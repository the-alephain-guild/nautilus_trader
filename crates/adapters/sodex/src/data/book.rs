// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Order book snapshots.
//!
//! The venue publishes no order book *stream*: enumerating its subscription channels found three
//! market ones and no book among them. It does serve a REST snapshot, which is a different thing -
//! and the distinction was missed for a while, because an absent stream was read as an absent book.
//!
//! So depth is polled rather than streamed. The venue stamps each snapshot with its own
//! `updateID`, which is carried through as the book's sequence so a consumer can tell two reads at
//! one state from two reads at two states.

use std::collections::HashMap;

use nautilus_core::UnixNanos;
use nautilus_model::{
    data::order::BookOrder,
    enums::{BookType, OrderSide},
    identifiers::InstrumentId,
    orderbook::OrderBook,
};
use serde::Deserialize;

use super::parse::{price_at, quantity_at};
use crate::{
    data::history::HistoryError,
    http::{ClientError, SodexHttpClient, ratelimit::orderbook_weight},
};

/// Depth assumed when a request does not ask for one.
///
/// The venue answers ten levels per side unasked, so this is what it actually serves rather than a
/// number chosen here.
const DEFAULT_BOOK_DEPTH: usize = 10;

/// One order book snapshot, as the venue serves it.
///
/// Both engines answer the same shape. Levels arrive best-first as `[price, size]` pairs of
/// decimal strings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RpcOrderBook {
    /// Chain time the snapshot was taken at, milliseconds.
    #[serde(rename = "blockTime")]
    pub block_time_ms: u64,
    #[serde(rename = "blockHeight")]
    pub block_height: u64,
    /// The venue's own monotonic revision for this book.
    #[serde(rename = "updateID")]
    pub update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

/// Fetches one order book snapshot.
///
/// `depth` is pushed to the venue as its `limit` parameter rather than trimmed here, so the
/// unwanted levels are never transferred. The venue caps the response at the book's actual size,
/// so asking for more levels than exist is not an error.
///
/// # Errors
///
/// Returns [`HistoryError::Transport`] if the request fails.
pub async fn fetch_order_book(
    client: &SodexHttpClient,
    instrument_id: InstrumentId,
    depth: Option<usize>,
) -> Result<RpcOrderBook, HistoryError> {
    let mut params: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(depth) = depth {
        params.insert("limit".to_string(), vec![depth.to_string()]);
    }

    // The book endpoint addresses the instrument by name, like the klines endpoint does.
    let path = format!("/markets/{}/orderbook", instrument_id.symbol);
    let params = (!params.is_empty()).then_some(params);

    // The venue charges this read by requested depth, not at the flat endpoint rate, and the
    // limiter already knows the brackets - it just had no caller until now.
    let weight = orderbook_weight(depth.unwrap_or(DEFAULT_BOOK_DEPTH) as u32);

    client
        .get_public_weighted(&path, params.as_ref(), weight)
        .await
        .map_err(|e: ClientError| HistoryError::Transport(e.to_string()))
}

/// Builds a book from a snapshot.
///
/// Precisions come from the instrument rather than from each level's text, for the reason the bar
/// path had to learn: this venue writes one tick size several ways, and Nautilus rejects a book
/// whose prices disagree about scale.
///
/// # Errors
///
/// Returns [`HistoryError::Mapping`] if a level cannot be parsed at those precisions. A level is
/// not skipped on failure: a book missing a level silently misprices every depth calculation made
/// from it.
pub fn parse_order_book(
    raw: &RpcOrderBook,
    instrument_id: InstrumentId,
    price_precision: u8,
    size_precision: u8,
) -> Result<OrderBook, HistoryError> {
    let mut book = OrderBook::new(instrument_id, BookType::L2_MBP);
    let ts_event = UnixNanos::from(raw.block_time_ms * 1_000_000);

    for (side, levels) in [(OrderSide::Buy, &raw.bids), (OrderSide::Sell, &raw.asks)] {
        for (index, level) in levels.iter().enumerate() {
            let price = price_at(&level[0], price_precision, "book price")?;
            let size = quantity_at(&level[1], size_precision, "book size")?;
            // The order id only has to separate levels within one side; `L2_MBP` aggregates by
            // price, so the level's own position serves.
            let order = BookOrder::new(side, price, size, index as u64);
            book.add(order, 0, raw.update_id, ts_event);
        }
    }

    Ok(book)
}

#[cfg(test)]
mod tests {
    use nautilus_model::identifiers::{Symbol, Venue};
    use rstest::rstest;

    use super::*;
    use crate::config::{SODEX_PERPS, SODEX_SPOT};

    /// Verbatim from `/markets/BTC-USD/orderbook` on the perps engine, trimmed to three levels.
    fn perps_book() -> RpcOrderBook {
        RpcOrderBook {
            block_time_ms: 1_789_267_906_235,
            block_height: 245_859_906,
            update_id: 1_606_915_335,
            bids: vec![
                ["77256".to_string(), "0.12943".to_string()],
                ["77255".to_string(), "0.12943".to_string()],
                ["77254".to_string(), "0.12943".to_string()],
            ],
            asks: vec![
                ["77272".to_string(), "0.12941".to_string()],
                ["77273".to_string(), "0.12941".to_string()],
            ],
        }
    }

    fn perps_instrument() -> InstrumentId {
        InstrumentId::new(Symbol::from("BTC-USD"), Venue::from(SODEX_PERPS))
    }

    #[rstest]
    fn a_snapshot_becomes_a_book_with_both_sides() {
        let book = parse_order_book(&perps_book(), perps_instrument(), 0, 5).unwrap();

        assert_eq!(book.best_bid_price().unwrap().as_f64(), 77256.0);
        assert_eq!(book.best_ask_price().unwrap().as_f64(), 77272.0);
        assert_eq!(book.bids(None).count(), 3);
        assert_eq!(book.asks(None).count(), 2);
    }

    /// The venue's revision is carried through rather than discarded, so two reads at one state are
    /// distinguishable from two reads at two states.
    #[rstest]
    fn the_venue_revision_becomes_the_book_sequence() {
        let book = parse_order_book(&perps_book(), perps_instrument(), 0, 5).unwrap();

        assert_eq!(book.sequence, 1_606_915_335);
    }

    /// Chain time, not local time: two snapshots from one block describe one state.
    #[rstest]
    fn the_block_time_becomes_the_event_time() {
        let book = parse_order_book(&perps_book(), perps_instrument(), 0, 5).unwrap();

        assert_eq!(book.ts_last, UnixNanos::from(1_789_267_906_235_000_000));
    }

    /// Spot writes more decimals than perps, and the instrument's precision is what resolves it -
    /// the same rule the bar path had to learn after a runtime panic.
    #[rstest]
    fn levels_parse_at_the_instruments_precision() {
        let raw = RpcOrderBook {
            block_time_ms: 1_789_267_906_235,
            block_height: 1,
            update_id: 2,
            bids: vec![["77295.00".to_string(), "0.00155000".to_string()]],
            asks: vec![["77296".to_string(), "0.00113".to_string()]],
        };
        let instrument = InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT));

        let book = parse_order_book(&raw, instrument, 2, 8).unwrap();

        assert_eq!(book.best_bid_price().unwrap().precision, 2);
        assert_eq!(book.best_bid_size().unwrap().precision, 8);
    }

    /// A level this adapter cannot represent fails the read rather than vanishing from the book:
    /// silently dropping depth misprices everything computed from it.
    #[rstest]
    fn an_unparsable_level_fails_the_snapshot() {
        let raw = RpcOrderBook {
            block_time_ms: 1,
            block_height: 1,
            update_id: 1,
            bids: vec![["not-a-price".to_string(), "1".to_string()]],
            asks: Vec::new(),
        };

        assert!(parse_order_book(&raw, perps_instrument(), 0, 5).is_err());
    }

    /// An empty book is a legitimate state on a quiet market, not a failure.
    #[rstest]
    fn an_empty_book_parses() {
        let raw = RpcOrderBook {
            block_time_ms: 1,
            block_height: 1,
            update_id: 1,
            bids: Vec::new(),
            asks: Vec::new(),
        };

        let book = parse_order_book(&raw, perps_instrument(), 0, 5).unwrap();

        assert!(book.best_bid_price().is_none());
        assert!(book.best_ask_price().is_none());
    }

    /// Both engines answer the same shape, so one parser serves both.
    #[rstest]
    fn the_spot_engine_answers_the_same_shape() {
        let instrument = InstrumentId::new(Symbol::from("vBTC_vUSDC"), Venue::from(SODEX_SPOT));

        let book = parse_order_book(&perps_book(), instrument, 0, 5).unwrap();

        assert_eq!(book.instrument_id, instrument);
    }
}
