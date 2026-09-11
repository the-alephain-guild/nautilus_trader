//! Instrument definitions and the venue-to-engine symbol mapping.
//!
//! # Two identifiers for one instrument
//!
//! Nautilus addresses instruments by [`InstrumentId`] (a symbol plus a venue), while SoDEX
//! addresses them by a numeric `symbolID`. Every order therefore needs the numeric form, and
//! every inbound message needs the reverse.
//!
//! The mapping is built from the venue's own symbol listing rather than derived locally, so
//! it is **fully reconstructible**: `load_all` rebuilds it from the authority. Nothing here
//! depends on process-local state surviving a restart, which is the failure mode a locally
//! derived mapping would introduce.
//!
//! # Precision is preserved end to end
//!
//! Tick sizes, quantities and prices arrive as decimal strings and are parsed straight into
//! [`Price`] and [`Quantity`] via `FromStr`. No binary float sits between the venue's value
//! and the engine's fixed-point type.

use std::{collections::HashMap, str::FromStr};

use async_trait::async_trait;
use nautilus_common::providers::{InstrumentProvider, InstrumentStore};
use nautilus_core::{AtomicMap, UnixNanos};
use nautilus_model::{
    currencies::CURRENCY_MAP,
    enums::CurrencyType,
    identifiers::{InstrumentId, Symbol, Venue},
    instruments::{Instrument, CryptoPerpetual, CurrencyPair, InstrumentAny},
    types::{Currency, Money, Price, Quantity, fixed::FIXED_PRECISION},
};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::Deserialize;

use crate::{
    common::{Market, decimal::normalize as normalize_decimal},
    config::venue_for,
    http::{Network, SodexHttpClient, client::DEFAULT_TIMEOUT_SECS},
};

/// Status string the venue uses for a tradable symbol.
pub const STATUS_TRADING: &str = "TRADING";

/// Spot symbol definition.
///
/// `baseCoin` and its precision are optional in the venue's schema even though the coin ids
/// are not, so the parser has to tolerate their absence rather than assume they are present.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotSymbol {
    pub id: u64,
    pub name: String,
    pub display_name: String,
    pub base_coin: Option<String>,
    pub base_coin_precision: Option<u8>,
    pub quote_coin: Option<String>,
    pub quote_coin_precision: Option<u8>,
    pub price_precision: i32,
    pub tick_size: String,
    pub min_price: String,
    pub max_price: String,
    pub quantity_precision: i32,
    pub step_size: String,
    pub min_quantity: String,
    pub max_quantity: String,
    pub min_notional: String,
    pub max_notional: String,
    pub maker_fee: String,
    pub taker_fee: String,
    pub status: String,
}

/// Perpetual symbol definition.
///
/// Unlike spot there is no base coin id or precision: the base is an index, not a settled
/// asset. Settlement happens in the quote coin.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerpsSymbol {
    pub id: u64,
    pub name: String,
    pub display_name: String,
    pub base_coin: String,
    pub quote_coin: String,
    pub quote_coin_precision: u8,
    pub price_precision: i32,
    pub tick_size: String,
    pub min_price: String,
    pub max_price: String,
    pub quantity_precision: i32,
    pub step_size: String,
    pub min_quantity: String,
    pub max_quantity: String,
    pub min_notional: String,
    pub max_notional: String,
    pub max_leverage: u32,
    pub maker_fee: String,
    pub taker_fee: String,
    pub status: String,
}

/// Resolves a currency, registering it when the venue trades something Nautilus has no
/// built-in definition for.
///
/// SoDEX testnet trades `vBTC` and `vUSDC`, which are not in any standard currency table.
/// Failing on an unknown code would make the adapter unusable there, so unknown codes are
/// registered as crypto with the venue's stated precision.
///
/// # Precision is clamped, and that is safe here
///
/// The venue reports coin precision as on-chain token decimals, which reach 18 — beyond
/// Nautilus's fixed-point maximum, where an unclamped value panics. Clamping loses nothing
/// that matters for trading: this precision describes the currency's own denomination, while
/// order prices and sizes take their precision from the symbol's `tickSize` and `stepSize`,
/// which are far coarser and are carried separately.
fn resolve_currency(code: &str, precision: u8) -> Currency {
    if let Some(existing) = CURRENCY_MAP.lock().get(code) {
        return *existing;
    }
    Currency::new(
        code,
        precision.min(FIXED_PRECISION),
        0,
        code,
        CurrencyType::Crypto,
    )
}

/// Parses a decimal string into a `Decimal`, defaulting to zero.
///
/// Fee ratios are the only place this is used; a malformed fee should not prevent an
/// instrument from loading, since it does not affect order validity.
fn parse_decimal_or_zero(raw: &str) -> Decimal {
    Decimal::from_str(raw).unwrap_or_default()
}

/// Treats a zero bound as "unbounded", matching the venue's own convention.
///
/// The venue documents each filter as inactive when its value is `0`, so mapping `0` onto a
/// real limit would reject orders the venue would have accepted.
fn optional_price(raw: &str) -> Option<Price> {
    let price = Price::from_str(&normalize_decimal(raw).ok()?).ok()?;
    (price.as_f64() != 0.0).then_some(price)
}

fn optional_quantity(raw: &str) -> Option<Quantity> {
    let quantity = Quantity::from_str(&normalize_decimal(raw).ok()?).ok()?;
    (quantity.as_f64() != 0.0).then_some(quantity)
}

/// Notional bounds are the one place a float is unavoidable: `Money` is constructed from
/// `f64`. These are filter thresholds rather than traded values, so the rounding `Money`
/// applies is harmless — unlike on a price or size, where it would break a lot filter.
fn optional_notional(raw: &str, currency: Currency) -> Option<Money> {
    let amount = Decimal::from_str(raw).ok()?;
    (!amount.is_zero()).then(|| Money::new(amount.to_f64().unwrap_or(0.0), currency))
}

/// Parses a required decimal string, attributing failures to the field that caused them.
///
/// Normalises first: the venue emits on-chain precision, which the engine's fixed-point
/// types reject outright.
fn parse_price(raw: &str, field: &'static str) -> anyhow::Result<Price> {
    let normalized = normalize_decimal(raw)?;
    Price::from_str(&normalized).map_err(|e| anyhow::anyhow!("invalid {field} {raw:?}: {e}"))
}

fn parse_quantity(raw: &str, field: &'static str) -> anyhow::Result<Quantity> {
    let normalized = normalize_decimal(raw)?;
    Quantity::from_str(&normalized).map_err(|e| anyhow::anyhow!("invalid {field} {raw:?}: {e}"))
}

/// Builds a Nautilus instrument id for a venue symbol.
#[must_use]
pub fn instrument_id_for(raw_symbol: &str, venue: Venue) -> InstrumentId {
    InstrumentId::new(Symbol::from(raw_symbol), venue)
}

/// Converts a spot symbol definition into a Nautilus instrument.
///
/// # Errors
///
/// Returns an error if a required numeric field cannot be parsed.
pub fn parse_spot_instrument(
    symbol: &SpotSymbol,
    venue: Venue,
    ts_init: UnixNanos,
) -> anyhow::Result<CurrencyPair> {
    let price_precision = u8::try_from(symbol.price_precision.max(0))?;
    let size_precision = u8::try_from(symbol.quantity_precision.max(0))?;

    let base = resolve_currency(
        symbol.base_coin.as_deref().unwrap_or("UNKNOWN"),
        symbol.base_coin_precision.unwrap_or(size_precision),
    );
    let quote = resolve_currency(
        symbol.quote_coin.as_deref().unwrap_or("UNKNOWN"),
        symbol.quote_coin_precision.unwrap_or(price_precision),
    );

    CurrencyPair::builder()
        .instrument_id(instrument_id_for(&symbol.name, venue))
        .raw_symbol(Symbol::from(symbol.name.as_str()))
        .base_currency(base)
        .quote_currency(quote)
        .price_precision(price_precision)
        .size_precision(size_precision)
        .price_increment(parse_price(&symbol.tick_size, "tickSize")?)
        .size_increment(parse_quantity(&symbol.step_size, "stepSize")?)
        .maybe_max_quantity(optional_quantity(&symbol.max_quantity))
        .maybe_min_quantity(optional_quantity(&symbol.min_quantity))
        .maybe_max_notional(optional_notional(&symbol.max_notional, quote))
        .maybe_min_notional(optional_notional(&symbol.min_notional, quote))
        .maybe_max_price(optional_price(&symbol.max_price))
        .maybe_min_price(optional_price(&symbol.min_price))
        .maker_fee(parse_decimal_or_zero(&symbol.maker_fee))
        .taker_fee(parse_decimal_or_zero(&symbol.taker_fee))
        .ts_event(ts_init)
        .ts_init(ts_init)
        .build()
        .map_err(Into::into)
}

/// Converts a perpetual symbol definition into a Nautilus instrument.
///
/// # Errors
///
/// Returns an error if a required numeric field cannot be parsed.
pub fn parse_perps_instrument(
    symbol: &PerpsSymbol,
    venue: Venue,
    ts_init: UnixNanos,
) -> anyhow::Result<CryptoPerpetual> {
    let price_precision = u8::try_from(symbol.price_precision.max(0))?;
    let size_precision = u8::try_from(symbol.quantity_precision.max(0))?;

    let base = resolve_currency(&symbol.base_coin, size_precision);
    let quote = resolve_currency(&symbol.quote_coin, symbol.quote_coin_precision);

    CryptoPerpetual::builder()
        .instrument_id(instrument_id_for(&symbol.name, venue))
        .raw_symbol(Symbol::from(symbol.name.as_str()))
        .base_currency(base)
        .quote_currency(quote)
        // Linear contracts settled in the quote coin, not inverse.
        .settlement_currency(quote)
        .is_inverse(false)
        .price_precision(price_precision)
        .size_precision(size_precision)
        .price_increment(parse_price(&symbol.tick_size, "tickSize")?)
        .size_increment(parse_quantity(&symbol.step_size, "stepSize")?)
        .maybe_max_quantity(optional_quantity(&symbol.max_quantity))
        .maybe_min_quantity(optional_quantity(&symbol.min_quantity))
        .maybe_max_notional(optional_notional(&symbol.max_notional, quote))
        .maybe_min_notional(optional_notional(&symbol.min_notional, quote))
        .maybe_max_price(optional_price(&symbol.max_price))
        .maybe_min_price(optional_price(&symbol.min_price))
        .maker_fee(parse_decimal_or_zero(&symbol.maker_fee))
        .taker_fee(parse_decimal_or_zero(&symbol.taker_fee))
        .ts_event(ts_init)
        .ts_init(ts_init)
        .build()
        .map_err(Into::into)
}

/// Loads instrument definitions from one SoDEX engine.
pub struct SodexInstrumentProvider {
    client: SodexHttpClient,
    market: Market,
    venue: Venue,
    store: InstrumentStore,
    /// Reverse map for order submission, rebuilt on every load from the venue's listing.
    symbol_ids: HashMap<InstrumentId, u64>,
}

/// One reading of the venue's instrument listing.
///
/// Carried as a value rather than applied to a provider because the refresh runs on a spawned
/// task: [`InstrumentProvider`] is an `?Send` trait, so its futures cannot be spawned onto the
/// multi-threaded runtime. Fetching through a plain function keeps the reload on the same code
/// path as the initial load without routing it through the trait.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub instruments: Vec<InstrumentAny>,
    pub symbol_ids: HashMap<InstrumentId, u64>,
}

/// Reads the venue's instrument listing.
///
/// Skips anything not in trading status, so a halted or delisted pair is absent rather than
/// present and unusable.
///
/// # Errors
///
/// Returns the transport failure, or a parse failure for a symbol whose numeric fields the
/// engine's types cannot hold.
pub async fn fetch_instruments(
    client: &SodexHttpClient,
    market: Market,
    venue: Venue,
) -> anyhow::Result<Listing> {
    let ts = UnixNanos::default();
    let mut listing = Listing::default();

    match market {
        Market::Spot => {
            let symbols: Vec<SpotSymbol> = client
                .get_public("/markets/symbols", None)
                .await
                .map_err(|e| anyhow::anyhow!("failed to load spot symbols: {e}"))?;
            ingest_spot(&mut listing, symbols, venue, ts)?;
        }
        Market::Perps => {
            let symbols: Vec<PerpsSymbol> = client
                .get_public("/markets/symbols", None)
                .await
                .map_err(|e| anyhow::anyhow!("failed to load perps symbols: {e}"))?;
            ingest_perps(&mut listing, symbols, venue, ts)?;
        }
    }

    Ok(listing)
}

/// The loaded instrument set, shared between the clients that read it and the task that
/// refreshes it.
///
/// Reads happen on synchronous trait methods — `request_instruments`, and the precision lookup
/// a subscription needs — while the refresh happens on a task that must `await` the venue.
/// Holding the provider behind a lock would force those readers to block on a network call, so
/// the provider stays on the refresh side and publishes whole snapshots here instead: readers
/// see either the previous set or the next one, never a half-built one.
#[derive(Debug, Default)]
pub struct InstrumentCatalog {
    instruments: AtomicMap<InstrumentId, InstrumentAny>,
    symbol_ids: AtomicMap<InstrumentId, u64>,
}

impl InstrumentCatalog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the published set with what the provider currently holds.
    ///
    /// A replacement rather than a merge, for the same reason the provider rebuilds rather
    /// than merges: the venue's listing is the authority, and a stale local entry for a
    /// delisted symbol is worse than an absent one.
    pub fn publish(&self, listing: &Listing) {
        self.instruments.store(
            listing
                .instruments
                .iter()
                .map(|instrument| (Instrument::id(instrument), instrument.clone()))
                .collect(),
        );
        self.symbol_ids.store(
            listing
                .symbol_ids
                .iter()
                .map(|(id, symbol_id)| (*id, *symbol_id))
                .collect(),
        );
    }

    /// Every published instrument.
    #[must_use]
    pub fn all(&self) -> Vec<InstrumentAny> {
        self.instruments.load().values().cloned().collect()
    }

    /// One published instrument.
    #[must_use]
    pub fn find(&self, instrument_id: &InstrumentId) -> Option<InstrumentAny> {
        self.instruments.get_cloned(instrument_id)
    }

    /// The numeric symbol id an order must carry.
    ///
    /// `None` before the instrument has been published. Callers must treat that as an error
    /// rather than a default — submitting without it would mean guessing an id.
    #[must_use]
    pub fn symbol_id(&self, instrument_id: &InstrumentId) -> Option<u64> {
        self.symbol_ids.get_cloned(instrument_id)
    }

    /// Number of published instruments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instruments.len()
    }

    /// Whether nothing has been published yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instruments.is_empty()
    }
}

impl std::fmt::Debug for SodexInstrumentProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SodexInstrumentProvider")
            .field("market", &self.market)
            .field("venue", &self.venue)
            .field("loaded", &self.symbol_ids.len())
            .finish()
    }
}

impl SodexInstrumentProvider {
    /// Creates a provider for one engine.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be built.
    pub fn new(network: Network, market: Market) -> anyhow::Result<Self> {
        Self::with_options(network, market, DEFAULT_TIMEOUT_SECS)
    }

    /// Creates a provider with an explicit HTTP timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be built.
    pub fn with_options(
        network: Network,
        market: Market,
        timeout_secs: u64,
    ) -> anyhow::Result<Self> {
        let client = SodexHttpClient::public_with_options(network, market, timeout_secs, None)
            .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))?;
        Ok(Self {
            client,
            market,
            venue: venue_for(market),
            store: InstrumentStore::default(),
            symbol_ids: HashMap::new(),
        })
    }

    /// The venue these instruments belong to.
    #[must_use]
    pub const fn venue(&self) -> Venue {
        self.venue
    }

    /// The numeric symbol id an order must carry for this instrument.
    ///
    /// Returns `None` before the instrument has been loaded — submitting without it would
    /// mean guessing an id, so callers must treat the absence as an error rather than a
    /// default.
    #[must_use]
    pub fn symbol_id(&self, instrument_id: &InstrumentId) -> Option<u64> {
        self.symbol_ids.get(instrument_id).copied()
    }

    /// The full reverse map, for publishing into a shared catalogue.
    #[must_use]
    pub const fn symbol_ids(&self) -> &HashMap<InstrumentId, u64> {
        &self.symbol_ids
    }

    /// Number of instruments currently mapped.
    #[must_use]
    pub fn len(&self) -> usize {
        self.symbol_ids.len()
    }

    /// Whether nothing has been loaded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.symbol_ids.is_empty()
    }

}

/// Folds a spot symbol listing into a [`Listing`], skipping anything not trading.
///
/// # Errors
///
/// Returns a parse failure for a symbol whose numeric fields the engine's types cannot hold.
pub fn ingest_spot(
    listing: &mut Listing,
    symbols: Vec<SpotSymbol>,
    venue: Venue,
    ts: UnixNanos,
) -> anyhow::Result<()> {
    for symbol in symbols {
        if symbol.status != STATUS_TRADING {
            continue;
        }
        let instrument = parse_spot_instrument(&symbol, venue, ts)?;
        listing.symbol_ids.insert(instrument.id, symbol.id);
        listing
            .instruments
            .push(InstrumentAny::CurrencyPair(instrument));
    }
    Ok(())
}

/// Folds a perps symbol listing into a [`Listing`], skipping anything not trading.
///
/// # Errors
///
/// Returns a parse failure for a symbol whose numeric fields the engine's types cannot hold.
pub fn ingest_perps(
    listing: &mut Listing,
    symbols: Vec<PerpsSymbol>,
    venue: Venue,
    ts: UnixNanos,
) -> anyhow::Result<()> {
    for symbol in symbols {
        if symbol.status != STATUS_TRADING {
            continue;
        }
        let instrument = parse_perps_instrument(&symbol, venue, ts)?;
        listing.symbol_ids.insert(instrument.id, symbol.id);
        listing
            .instruments
            .push(InstrumentAny::CryptoPerpetual(instrument));
    }
    Ok(())
}

#[async_trait(?Send)]
impl InstrumentProvider for SodexInstrumentProvider {
    fn store(&self) -> &InstrumentStore {
        &self.store
    }

    fn store_mut(&mut self) -> &mut InstrumentStore {
        &mut self.store
    }

    async fn load_all(&mut self, _filters: Option<&HashMap<String, String>>) -> anyhow::Result<()> {
        let listing = fetch_instruments(&self.client, self.market, self.venue).await?;

        // Rebuilding rather than merging: the venue's listing is the authority, and a stale
        // local entry for a delisted symbol is worse than an absent one.
        self.symbol_ids = listing.symbol_ids;
        self.store = InstrumentStore::default();
        for instrument in listing.instruments {
            self.store.add(instrument);
        }

        Ok(())
    }

    async fn load(
        &mut self,
        instrument_id: &InstrumentId,
        filters: Option<&HashMap<String, String>>,
    ) -> anyhow::Result<()> {
        // The venue's symbol endpoint accepts a name filter, but the full listing is small
        // and already cached per call; loading everything keeps one code path and guarantees
        // the reverse map stays complete.
        if instrument_id.venue != self.venue {
            anyhow::bail!(
                "instrument {} belongs to {}, not {}",
                instrument_id,
                instrument_id.venue,
                self.venue
            );
        }
        self.load_all(filters).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SODEX_PERPS, SODEX_SPOT};

    /// Field values taken from the live testnet listing for `vBTC_vUSDC`.
    fn spot_symbol() -> SpotSymbol {
        SpotSymbol {
            id: 1,
            name: "vBTC_vUSDC".to_string(),
            display_name: "BTC/USDC".to_string(),
            base_coin: Some("vBTC".to_string()),
            base_coin_precision: Some(8),
            quote_coin: Some("vUSDC".to_string()),
            quote_coin_precision: Some(6),
            price_precision: 0,
            tick_size: "1".to_string(),
            min_price: "0".to_string(),
            max_price: "0".to_string(),
            quantity_precision: 5,
            step_size: "0.00001".to_string(),
            min_quantity: "0.00001".to_string(),
            max_quantity: "1000".to_string(),
            min_notional: "5".to_string(),
            max_notional: "4000000".to_string(),
            maker_fee: "0.00065".to_string(),
            taker_fee: "0.00035".to_string(),
            status: STATUS_TRADING.to_string(),
        }
    }

    fn perps_symbol() -> PerpsSymbol {
        PerpsSymbol {
            id: 1,
            name: "BTC-USD".to_string(),
            display_name: "BTC-USD".to_string(),
            base_coin: "BTC".to_string(),
            quote_coin: "vUSDC".to_string(),
            quote_coin_precision: 6,
            price_precision: 0,
            tick_size: "1".to_string(),
            min_price: "0".to_string(),
            max_price: "0".to_string(),
            quantity_precision: 5,
            step_size: "0.00001".to_string(),
            min_quantity: "0.00001".to_string(),
            max_quantity: "1000".to_string(),
            min_notional: "10".to_string(),
            max_notional: "4000000".to_string(),
            max_leverage: 40,
            maker_fee: "0.0002".to_string(),
            taker_fee: "0.0005".to_string(),
            status: STATUS_TRADING.to_string(),
        }
    }

    #[test]
    fn spot_instrument_preserves_the_venue_tick_and_step() {
        let venue = Venue::from(SODEX_SPOT);
        let instrument = parse_spot_instrument(&spot_symbol(), venue, UnixNanos::default()).unwrap();

        assert_eq!(instrument.price_increment.to_string(), "1");
        assert_eq!(instrument.size_increment.to_string(), "0.00001");
        assert_eq!(instrument.price_precision, 0);
        assert_eq!(instrument.size_precision, 5);
    }

    #[test]
    fn decimal_strings_do_not_pass_through_a_float() {
        // 0.00001 has no exact binary representation; round-tripping it through f64 and back
        // is how step sizes acquire trailing noise and orders start failing lot filters.
        let venue = Venue::from(SODEX_SPOT);
        let instrument = parse_spot_instrument(&spot_symbol(), venue, UnixNanos::default()).unwrap();

        assert_eq!(instrument.size_increment.to_string(), "0.00001");
        assert!(!instrument.size_increment.to_string().contains("9999"));
    }

    #[test]
    fn zero_bounds_map_to_unbounded_not_to_a_literal_zero() {
        // The venue documents a filter as inactive when its value is 0. Mapping that onto a
        // real maximum price of zero would reject every order.
        let venue = Venue::from(SODEX_SPOT);
        let instrument = parse_spot_instrument(&spot_symbol(), venue, UnixNanos::default()).unwrap();

        assert!(instrument.min_price.is_none());
        assert!(instrument.max_price.is_none());
        assert!(instrument.min_quantity.is_some(), "0.00001 is a real bound");
    }

    #[test]
    fn on_chain_token_decimals_are_clamped_to_the_fixed_point_maximum() {
        // The venue reports coin precision as token decimals, which reach 18. Passing that
        // through panics inside Nautilus. Found by fetching the live symbol listing, not by
        // the earlier tests, which happened to use in-range values.
        let venue = Venue::from(SODEX_SPOT);
        let mut symbol = spot_symbol();
        symbol.base_coin = Some("wSOMETOKEN".to_string());
        symbol.base_coin_precision = Some(18);

        let instrument = parse_spot_instrument(&symbol, venue, UnixNanos::default()).unwrap();

        assert_eq!(instrument.base_currency.precision, FIXED_PRECISION);
    }

    #[test]
    fn clamping_does_not_touch_order_precision() {
        // The clamp applies to the currency's denomination only; order price and size
        // precision come from tickSize and stepSize and must be unaffected.
        let venue = Venue::from(SODEX_SPOT);
        let mut symbol = spot_symbol();
        symbol.base_coin = Some("wOTHERTOKEN".to_string());
        symbol.base_coin_precision = Some(18);

        let instrument = parse_spot_instrument(&symbol, venue, UnixNanos::default()).unwrap();

        assert_eq!(instrument.price_precision, 0);
        assert_eq!(instrument.size_precision, 5);
        assert_eq!(instrument.size_increment.to_string(), "0.00001");
    }

    #[test]
    fn unknown_venue_coins_are_registered_rather_than_rejected() {
        // vBTC and vUSDC are testnet tokens absent from any standard currency table.
        let venue = Venue::from(SODEX_SPOT);
        let instrument = parse_spot_instrument(&spot_symbol(), venue, UnixNanos::default()).unwrap();

        assert_eq!(instrument.base_currency.code.as_str(), "vBTC");
        assert_eq!(instrument.quote_currency.code.as_str(), "vUSDC");
    }

    #[test]
    fn perps_settle_in_the_quote_currency_and_are_linear() {
        let venue = Venue::from(SODEX_PERPS);
        let instrument =
            parse_perps_instrument(&perps_symbol(), venue, UnixNanos::default()).unwrap();

        assert_eq!(instrument.settlement_currency, instrument.quote_currency);
        assert!(!instrument.is_inverse);
    }

    #[test]
    fn the_same_symbol_name_on_each_engine_is_a_different_instrument() {
        // The venue split shows up here: identical names must not collide across engines.
        let spot = instrument_id_for("BTC-USD", Venue::from(SODEX_SPOT));
        let perps = instrument_id_for("BTC-USD", Venue::from(SODEX_PERPS));

        assert_ne!(spot, perps);
        assert_eq!(spot.symbol, perps.symbol);
    }

    #[test]
    fn halted_symbols_are_not_loaded() {
        let mut listing = Listing::default();
        let mut halted = spot_symbol();
        halted.status = "HALT".to_string();

        ingest_spot(
            &mut listing,
            vec![halted],
            Venue::from(SODEX_SPOT),
            UnixNanos::default(),
        )
        .unwrap();

        assert!(listing.instruments.is_empty());
    }

    #[test]
    fn loading_populates_the_reverse_map_for_order_submission() {
        let mut listing = Listing::default();
        ingest_spot(
            &mut listing,
            vec![spot_symbol()],
            Venue::from(SODEX_SPOT),
            UnixNanos::default(),
        )
        .unwrap();

        let catalog = InstrumentCatalog::new();
        catalog.publish(&listing);

        let id = instrument_id_for("vBTC_vUSDC", Venue::from(SODEX_SPOT));
        assert_eq!(catalog.symbol_id(&id), Some(1));
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn an_unloaded_instrument_has_no_symbol_id() {
        // Callers must treat this as an error: guessing an id would submit an order against
        // whatever instrument happens to hold that number.
        let catalog = InstrumentCatalog::new();
        let id = instrument_id_for("vBTC_vUSDC", Venue::from(SODEX_SPOT));

        assert_eq!(catalog.symbol_id(&id), None);
        assert!(catalog.is_empty());
    }

    #[test]
    fn a_refresh_replaces_the_published_set_rather_than_adding_to_it() {
        // The venue listing is the authority, so a delisted symbol must disappear rather than
        // linger from an earlier reload. The refresh publishes whole snapshots for exactly
        // this reason.
        let catalog = InstrumentCatalog::new();
        let venue = Venue::from(SODEX_SPOT);

        let mut first = Listing::default();
        ingest_spot(
            &mut first,
            vec![spot_symbol()],
            venue,
            UnixNanos::default(),
        )
        .unwrap();
        catalog.publish(&first);
        assert_eq!(catalog.len(), 1);

        // The same pair listed again: a merge would double-count, a replacement will not.
        catalog.publish(&first);
        assert_eq!(catalog.len(), 1, "a reload must not accumulate");

        // And the pair delisted: it must be gone, not stale.
        catalog.publish(&Listing::default());
        assert!(
            catalog.is_empty(),
            "a delisted pair must disappear from the catalogue"
        );
        let id = instrument_id_for("vBTC_vUSDC", venue);
        assert_eq!(
            catalog.symbol_id(&id),
            None,
            "its symbol id must go with it, or an order could still be addressed to it"
        );
    }
}

/// Loads the venue listing and publishes it.
///
/// # Errors
///
/// Returns the load failure. Called on connect, where failing is the right outcome: nothing can
/// be subscribed or submitted without the listing, so continuing would only defer the error to
/// the first request.
pub async fn load_instruments(
    client: &SodexHttpClient,
    market: Market,
    venue: Venue,
    catalog: &InstrumentCatalog,
) -> anyhow::Result<()> {
    let listing = fetch_instruments(client, market, venue).await?;
    catalog.publish(&listing);
    Ok(())
}

/// Spawns the periodic reload, or returns `None` when it is disabled.
///
/// A reload failure is logged and the loop continues: the previously published listing is still
/// the best available answer, and tearing the client down over a transient venue hiccup would
/// be a worse outcome than trading one interval on slightly stale instruments.
///
/// Cancellation is checked in the same `select!` as the sleep, so a shutdown does not wait out
/// a full interval.
#[must_use]
pub fn spawn_instrument_refresh(
    interval_mins: Option<u64>,
    client: std::sync::Arc<SodexHttpClient>,
    market: Market,
    venue: Venue,
    catalog: std::sync::Arc<InstrumentCatalog>,
    cancellation: tokio_util::sync::CancellationToken,
    client_id: nautilus_model::identifiers::ClientId,
) -> Option<tokio::task::JoinHandle<()>> {
    let minutes = interval_mins.filter(|minutes| *minutes > 0)?;
    let interval = std::time::Duration::from_secs(minutes.saturating_mul(60));

    Some(nautilus_common::live::get_runtime().spawn(async move {
        loop {
            let sleep = tokio::time::sleep(interval);
            tokio::pin!(sleep);

            tokio::select! {
                () = cancellation.cancelled() => {
                    log::debug!("sodex_instrument_refresh_cancelled client_id={client_id}");
                    break;
                }
                () = &mut sleep => match load_instruments(&client, market, venue, &catalog).await {
                    Ok(()) => log::debug!(
                        "sodex_instruments_refreshed client_id={client_id} count={}",
                        catalog.len()
                    ),
                    Err(e) => log::warn!(
                        "sodex_instrument_refresh_failed client_id={client_id} error={e}"
                    ),
                },
            }
        }
    }))
}

#[cfg(test)]
mod refresh_tests {
    use nautilus_model::identifiers::ClientId;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{config::SODEX_SPOT, http::SodexHttpClient};

    fn client() -> std::sync::Arc<SodexHttpClient> {
        std::sync::Arc::new(
            SodexHttpClient::new_public(Network::Testnet, Market::Spot).expect("client builds"),
        )
    }

    #[tokio::test]
    async fn no_interval_means_no_refresh_task() {
        // A deployment that asks for no reload must not get one. The `None` and `0` forms are
        // both spellings of "off" and a config round-tripped through Python can produce either.
        for interval in [None, Some(0)] {
            let task = spawn_instrument_refresh(
                interval,
                client(),
                Market::Spot,
                Venue::from(SODEX_SPOT),
                std::sync::Arc::new(InstrumentCatalog::new()),
                CancellationToken::new(),
                ClientId::from("SODEX-TEST"),
            );

            assert!(task.is_none(), "interval {interval:?} must not spawn a task");
        }
    }

    #[tokio::test]
    async fn cancelling_stops_the_refresh_without_waiting_out_the_interval() {
        // The loop selects on the token alongside the sleep. Without that, a shutdown would
        // block for up to a full interval — an hour, at the default.
        let cancellation = CancellationToken::new();
        let task = spawn_instrument_refresh(
            Some(60),
            client(),
            Market::Spot,
            Venue::from(SODEX_SPOT),
            std::sync::Arc::new(InstrumentCatalog::new()),
            cancellation.clone(),
            ClientId::from("SODEX-TEST"),
        )
        .expect("a positive interval spawns a task");

        cancellation.cancel();

        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
        assert!(
            stopped.is_ok(),
            "the task should exit on cancellation, not at the next interval"
        );
    }
}
