//! Python bindings for SoDEX enums.
//!
//! `#[pyclass]` decoration lives on the Rust definitions — [`crate::common::Market`] and
//! [`crate::http::Network`] — so Python and Rust share one type rather than a mirrored copy
//! that could drift.
//!
//! They reach Python as `SodexMarket` and `SodexNetwork`. The prefix is not decoration: the
//! package is imported with `*`, and a bare `Market` or `Network` would shadow whatever else a
//! strategy has by those names.
//!
//! Both default to the safer side: `SodexMarket.SPOT` carries no leverage and has no
//! liquidation, and `SodexNetwork.TESTNET` spends no real funds. A configuration that omits
//! either therefore cannot silently open a leveraged mainnet position.
