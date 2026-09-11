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
//! | Account state | Coin balances, with free derived from total minus locked |
//! | Order status reports | Open orders and history, reconciled together |
//! | Fill reports | **Not implemented** — one observed fill away |
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
//! seven fields — the venue names each one in its unmarshal errors when sent a wrong type — and
//! all 127 non-empty subsets of them are refused for `accountUpdate`. That is not an exhausted
//! guess list but a closed search: its selector needs something outside that struct.
//!
//! # What is left, and what it costs
//!
//! **Fills.** `/accounts/{wallet}/trades` exists and answers, but an account that has never
//! traded answers `[]`, so its wire shape cannot be read off it. Typing it by analogy to the
//! order shape is precisely the move that produced this integration's worst failures, so it waits
//! for one observed fill — which the `observe_fill` example produces in a single testnet
//! round-trip. Until then a live run learns that an order was accepted, and learns it was filled
//! only on the next reconciliation pass, from `executedQty` on the order record.
//!
//! **Perps positions.** The endpoint exists; its payload needs an open perps position to observe,
//! and the testnet account holds no perps balance to open one with. `generate_position_status_reports`
//! therefore refuses on perps rather than returning an empty list, because an empty list asserts
//! the account is flat.
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
