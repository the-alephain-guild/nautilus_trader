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
//! - trading actions sign [`ExchangeAction`] under the `spot`/`futures` domain, prefix `0x01`
//! - account-level actions sign under the `universal` domain, prefix `0x02`
//!
//! The payload hash bound into [`ExchangeAction`] is `keccak256` over the *compact* JSON
//! encoding of `{type, params}`. The gateway verifies by parsing the request body into its
//! own Go structs and re-marshaling, so field order is part of the contract — see
//! [`signing`] for how that is preserved on this side.

//! # What is implemented
//!
//! | Area | State |
//! |------|-------|
//! | Instruments | Loaded from the venue listing for both engines |
//! | Historical bars | REST klines, with the still-forming tail removed |
//! | Streaming bars | `candle` channel, completed bars only |
//! | Quotes | `ticker` channel — a periodic sample of top of book, not every change |
//! | Trades | `trade` channel, with the aggressing side |
//! | Order submission | Market and limit, spot and perps |
//! | Order cancellation | By venue order id, falling back to the client order id |
//! | Order books | **Not implemented** — the venue publishes no book channel |
//! | Instrument reload | Hourly by default, configurable; `None` disables |
//! | Socket state reporting | Link state surfaced to the engine, reconnect requestable |
//! | Python bindings | `nautilus_trader.adapters.sodex` |
//! | Account state | **Not implemented** |
//! | Order status and fill reports | **Not implemented** |
//! | Position reports | **Not implemented** |
//!
//! The account, order-status and position gaps share one cause: the venue documentation this
//! adapter was built from covers the trading endpoints, and the paths for those reads have
//! not been verified against a live link. Guessing them would produce failures that read
//! like credential errors rather than missing endpoints — a diagnosis this integration has
//! already cost time on once.
//!
//! The consequence is worth stating rather than leaving to be discovered: **the execution
//! client cannot reconcile.** Orders it did not place, and fills that occurred while it was
//! disconnected, remain invisible to the engine. Fills are not reported at all, so a live run
//! learns that an order was accepted but never that it was filled.
//!
//! An `accountUpdate` stream channel does exist — the `probe_channels` example found it, and
//! the venue's own error text names an `accountID` field on its subscription parameters — but
//! eighteen candidate parameter shapes were all refused as `invalid params`, so its selector
//! remains unknown. That channel is the next thing to add, and finding its shape is a
//! question for the venue rather than for guesswork.
//!
//! # Reused rather than rebuilt
//!
//! The parts below come from the engine's own crates, and are listed because the alternative —
//! a hand-rolled equivalent — is easy to write by accident and hard to notice afterwards. One
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
//! One thing is deliberately *not* reused. The venue meters request **weight** — endpoints cost
//! between 1 and 20 against one shared per-minute budget — and the library's limiter consumes
//! exactly one cell per call with no weighted form, so [`http::WeightBudget`] is hand-rolled.
//! That is recorded here so the next reader does not spend time looking for the library
//! facility it duplicates.
//!
//! # Backtesting
//!
//! Historical bars come back through the same conversion the stream uses, so a backtest and
//! a live run see bars built by identical code. The one asymmetry that would otherwise
//! remain — the venue marks streamed bars closed but leaves historical ones unmarked — is
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
