// -------------------------------------------------------------------------------------------------
//  SoDEX integration adapter.
// -------------------------------------------------------------------------------------------------

//! [SoDEX](https://sodex.com) integration adapter.
//!
//! SoDEX is an on-chain orderbook DEX on ValueChain. Despite settling on-chain,
//! trading does not require broadcasting transactions: orders are submitted over
//! plain REST and authenticated with an offline EIP-712 signature, which makes the
//! integration shape closer to a centralized venue than to an AMM.
//!
//! # Credential model
//!
//! Two distinct keys, with deliberately different exposure:
//!
//! - The **master wallet** owns the account. It signs only account-level actions
//!   (`addAPIKey`, `revokeAPIKey`, `approveBuilderFee`) and is expected to stay offline.
//! - An **API key** is a named, revocable signing credential registered by the master
//!   wallet. It signs day-to-day trading actions and cannot read account data. This is
//!   the only key a running process needs to hold.
//!
//! # Signature layout
//!
//! Every signature carries a leading type byte, and the two families differ:
//!
//! - trading actions sign an `ExchangeAction` under the `spot`/`futures` domain, prefix `0x01`
//! - account-level actions sign under the `universal` domain, prefix `0x02`
//!
//! The payload hash bound into `ExchangeAction` is `keccak256` over the *compact* JSON
//! encoding of `{type, params}`. The gateway verifies by parsing the request body into its
//! own Go structs and re-marshaling, so field order is part of the contract - see
//! [`signing`] for how that is preserved on this side.

//! # Feature Flags
//!
//! This crate provides feature flags to control source code inclusion during compilation,
//! depending on the intended use case, i.e. whether to provide Python bindings
//! for the [nautilus_trader](https://pypi.org/project/nautilus_trader) Python package,
//! or as part of a Rust only build.
//!
//! - `extension-module`: Builds as a Python extension module.
//! - `high-precision` (default): Enables
//!   [high-precision mode](https://nautilustrader.io/docs/nightly/getting_started/installation/#precision-mode)
//!   to use 128-bit value types. Default here rather than opt-in because the venue settles
//!   on-chain and quotes token amounts at 18 decimals.
//! - `python`: Enables Python bindings from [PyO3](https://pyo3.rs).

//! # What is implemented
//!
//! | Area | State |
//! |------|-------|
//! | Instruments | Loaded from the venue listing for both engines |
//! | Historical bars | REST klines, with the still-forming tail removed |
//! | Streaming bars | `candle` channel, completed bars only |
//! | Quotes | `ticker` channel - a periodic sample of top of book, not every change |
//! | Trades | `trade` channel, with the aggressing side |
//! | Order submission | Market and limit, spot and perps |
//! | Order cancellation | By venue order id, falling back to the client order id |
//! | Ambiguous submissions | Resolved against the venue's order list, not guessed |
//! | Order books | **Not implemented** - the venue publishes no book channel |
//! | Instrument reload | Hourly by default, configurable; `None` disables |
//! | Socket state reporting | Link state surfaced to the engine, reconnect requestable |
//! | Python bindings | `nautilus_trader.adapters.sodex` |
//! | Account state | Coin balances, with free derived from total minus locked |
//! | Order status reports | Open orders and history, reconciled together |
//! | Fill reports | Per fill, with the venue's trade id, fee asset and liquidity side |
//! | Commission | Computed from the venue's maker/taker rates |
//! | Position reports | **Not implemented** (perps); correctly empty on spot |
//!
//! # How the account reads were found
//!
//! They were not in the documentation this adapter was built from. They were found by asking the
//! venue which paths it routes, which works because its gateway answers a path it does not route
//! differently from one it routes but cannot satisfy. Two things made the search converge:
//!
//! - **The router is per method.** A first sweep sent only `GET`, concluded almost nothing
//!   existed, and was refuted by its own control: `GET /trade/orders` also answers `404`, and
//!   orders are certainly placed there. There is no `405`, so a `404` means "not this method".
//! - **The paths carry the wallet address as a segment.** `/accounts/balances` is not a path;
//!   `/accounts/{address}/balances` is. One already-working endpoint had that shape documented in
//!   this crate all along, which is where the lead came from.
//!
//! The same technique settled the stream channel question too. `SubscriptionParams` has exactly
//! seven fields - the venue names each one in its unmarshal errors when sent a wrong type - and
//! all 127 non-empty subsets of them are refused for `accountUpdate`. That is not an exhausted
//! guess list but a closed search: its selector needs something outside that struct.
//!
//! # The fee is charged in the asset you receive
//!
//! Observed on both sides of a round trip, not inferred from one: a buy pays in the **base** asset
//! and a sell in the **quote** one. The buy's comes out of what arrives - ordering `0.001` vBTC
//! credited `0.00099935`, short by exactly the reported `0.00000065` - and the sell's comes out of
//! the proceeds, `notional × rate` to the last digit.
//!
//! Two consequences, both of which cost something to learn:
//!
//! - **A buy credits less than it ordered**, so anything selling back what it bought must sell
//!   what was *received*, rounded down to the step size. A flattening sell sized from the order is
//!   rejected for insufficient balance, which is how this was found. Each round trip also leaves
//!   base dust below the step size, which is unsellable by construction.
//! - **Fill reports keep the venue's asset** rather than converting. The quote-denominated estimate
//!   used for *inferred* fills is unaffected and exact: a sell's fee is `notional × rate` outright,
//!   and a buy's base-denominated fee converted at the fill price comes to the same number. Both
//!   are pinned against the observed trades.
//!
//! # A coin's listed precision is not its ledger precision
//!
//! The symbol listing reports `quoteCoinPrecision: 6` for vUSDC. The venue's ledger carries ten
//! places - a fee of `0.0498075435`, a balance of `999.3931824565`. Registering the currency at the
//! listed precision rounded that fee to `0.049808`, overstating it and leaving the recorded
//! commission unable to reconcile against the balance it came out of.
//!
//! So venue coins are registered at the engine's full width. The listed precision governs
//! *orders* (what price and size the venue accepts) and is carried separately on each instrument
//! as `price_precision` and `size_precision`, where it belongs.
//!
//! # What is left
//!
//! **Perps positions.** The endpoint exists; its payload needs an open perps position to observe,
//! and the testnet account holds no perps balance to open one with.
//! `generate_position_status_reports` therefore refuses on perps rather than returning an empty
//! list, because an empty list asserts the account is flat.
//!
//! Spot is complete: balances, open orders, history and fills all map, and each was verified
//! against a real response from the live testnet rather than from the documentation.
//!
//! **Perps positions.** The endpoint exists; its payload needs an open perps position to observe,
//! and the testnet account holds no perps balance to open one with. `generate_position_status_reports`
//! therefore refuses on perps rather than returning an empty list, because an empty list asserts
//! the account is flat.
//!
//! # Reused rather than rebuilt
//!
//! The parts below come from the engine's own crates, and are listed because the alternative -
//! a hand-rolled equivalent - is easy to write by accident and hard to notice afterwards. One
//! already happened here: a keepalive state machine was written before checking that
//! `WebSocketConfig` covered it.
//!
//! | Concern | Component |
//! |---------|-----------|
//! | Heartbeat, dead-peer detection, reconnect backoff | `nautilus_network::websocket::WebSocketConfig` |
//! | Per-endpoint request pacing | `HttpClient`'s GCRA limiter via `default_quota` |
//! | Order-count pacing | `nautilus_network::ratelimiter` |
//! | Transient failure retry | `nautilus_network::retry::RetryManager` |
//! | Task lifecycle | `nautilus_common::live::task::TaskHandles` |
//! | Instrument set snapshots | `nautilus_core::AtomicMap` |
//! | Socket state and reconnect control | `nautilus_live::SocketControlFactory` |
//!
//! One thing is deliberately *not* reused. The venue meters request **weight** - endpoints cost
//! between 1 and 20 against one shared per-minute budget - and the library's limiter consumes
//! exactly one cell per call with no weighted form, so [`http::WeightBudget`] is hand-rolled.
//! That is recorded here so the next reader does not spend time looking for the library
//! facility it duplicates.
//!
//! # Backtesting
//!
//! Historical bars come back through the same conversion the stream uses, so a backtest and
//! a live run see bars built by identical code. The one asymmetry that would otherwise
//! remain - the venue marks streamed bars closed but leaves historical ones unmarked - is
//! removed on both paths: the stream filters on the venue's flag, and history drops its
//! trailing bar by comparing the bar's open plus its interval against the clock.

#![allow(clippy::module_name_repetitions)]

pub mod common;
pub mod config;
pub mod data;
pub mod execution;
pub mod factories;
pub mod http;
pub mod providers;
#[cfg(feature = "python")]
pub mod python;
pub mod signing;
pub mod websocket;
