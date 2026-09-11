//! Python bindings for SoDEX enums.
//!
//! `#[pyclass]` decoration lives on the Rust definitions - [`crate::common::Market`] and
//! [`crate::http::Network`] - so Python and Rust share one type rather than a mirrored copy
//! that could drift.
//!
//! They reach Python under their Rust names, `Market` and `Network`, rather than prefixed ones.
//! That was tried and reverted: `pyclass(name = ...)` renames the runtime class but
//! `gen_stub_pyclass_enum` does not carry the rename into the generated stub, so the two
//! disagreed and the generator's own `__all__` check caught it. The alternative the ecosystem
//! uses - prefixing the *Rust* enum names, as dydx does - would touch 274 references across this
//! crate and make every internal use read `common::SodexMarket` inside the sodex crate.
//!
//! So the names stay generic, and the thing that made them a concern is handled by convention
//! instead: nothing here documents or uses `from ... import *`, and every example imports the
//! names it wants explicitly.
//!
//! Both default to the safer side: `Market.SPOT` carries no leverage and has no liquidation, and
//! `Network.TESTNET` spends no real funds. A configuration that omits either therefore cannot
//! silently open a leveraged mainnet position.
