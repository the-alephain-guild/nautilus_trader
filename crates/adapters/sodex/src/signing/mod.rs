//! EIP-712 signing for SoDEX.
//!
//! # Why field order is a correctness concern here
//!
//! The signed digest is not over the typed struct alone. It is over
//! `keccak256(compact_json({type, params}))`, and the gateway verifies by parsing the
//! request body into its own Go structs and re-marshaling with `json.Marshal`, which
//! emits fields in struct-definition order. A payload whose keys are ordered differently
//! hashes differently and the signature is rejected - with no diagnostic beyond a
//! verification failure.
//!
//! `serde` serializes struct fields in declaration order, which matches Go's behavior, so
//! the contract holds as long as payloads are modeled as concrete structs whose field order
//! mirrors the Go SDK. It does **not** hold for [`serde_json::Value`], and the reason is worse
//! than a fixed ordering: `Value`'s object representation is chosen by a Cargo feature. With
//! `serde_json/preserve_order` it is an insertion-ordered map; without it, a `BTreeMap` that
//! sorts keys alphabetically. That feature is not this crate's to set - building with the
//! `python` feature turns it on transitively, and a plain Rust build leaves it off.
//!
//! So a payload routed through `Value` would hash one way in one build of this crate and
//! another way in another, producing signatures that verify in one configuration and are
//! rejected in the other, with no diagnostic beyond a verification failure. That is why
//! [`payload_hash`] is generic over `T: Serialize` and never takes a `Value`: the property it
//! needs must not be a function of which features happen to be enabled.
//!
//! Three further encoding rules travel with the order requirement:
//!
//! - fields typed `DecimalString` are quoted strings (`"quantity":"0.001"`), never numbers
//! - `omitempty` fields must be absent entirely when unset, i.e. `Option` +
//!   `skip_serializing_if`
//! - non-optional fields must be present even at their zero value

pub mod nonce;
pub mod signers;
pub mod universal;

pub use nonce::{NonceGenerator, is_within_window};
pub use signers::{ExchangeSigner, SigningError, payload_hash};
pub use universal::UniversalSigner;
