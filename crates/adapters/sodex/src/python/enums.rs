//! Python bindings for SoDEX enums.
//!
//! `#[pyclass]` decoration lives on the Rust definitions — [`crate::common::Market`] and
//! [`crate::http::Network`] — so Python and Rust share one type rather than a mirrored copy
//! that could drift.
//!
//! Both default to the safer side: `Market::SPOT` carries no leverage and has no liquidation,
//! and `Network::TESTNET` spends no real funds. A config that omits either therefore cannot
//! silently open a leveraged mainnet position.
