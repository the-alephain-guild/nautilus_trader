//! WebSocket streams for SoDEX.
//!
//! # Channels
//!
//! Three are implemented, and their selectors do not share a shape:
//!
//! | Channel  | Selector | Carries |
//! |----------|----------|---------|
//! | `candle` | `symbol` + `interval` | OHLC bars, republished while forming |
//! | `trade`  | `symbols` array | Public trades with the aggressing side |
//! | `ticker` | `symbols` array | 24h statistics plus top of book, on a fixed cadence |
//!
//! The set was established by asking the venue rather than by assumption: the
//! `probe_channels` example sweeps candidate names and reports which reply `unknown channel`
//! and which reply `invalid params`, the latter meaning the name exists and only the
//! selector was wrong. That sweep also found an `accountUpdate` channel whose selector is
//! still unknown - see the crate documentation.
//!
//! # Streams carry no authorization
//!
//! The venue states that user-specific streams require no subscription authorization and
//! that any client may subscribe to another user's data. Two consequences worth stating
//! plainly:
//!
//! - Consuming account streams needs no API key, which simplifies this layer.
//! - Account activity is **not private**. Anyone holding an account id can watch that
//!   account's orders, fills and balance changes in real time. A strategy whose behavior is
//!   inferable from its order flow - laddered entries, fixed grid spacing, predictable
//!   scale-ins - is exposed by merely trading here.
//!
//! # Liveness
//!
//! The venue drops a connection after 60 seconds without a subscription or pushed data, so
//! silence is indistinguishable from a dead link and must be probed.
//!
//! The probing itself is `nautilus-network`'s, configured with a **text** heartbeat payload:
//! this venue counts an application-level `{"op":"ping"}`, and the library's docs are
//! explicit that a text keepalive and an empty Ping control frame "are not interchangeable".
//! What remains here is the venue's own timing constraint - see [`DEFAULT_IDLE_PROBE_SECS`].

pub mod client;
pub mod messages;

pub use client::{SodexWebSocketClient, SodexWsEvent, Subscription, WsError};
pub use messages::{
    Candle, CandleParams, Heartbeat, Op, SymbolsParams, Ticker, Trade, WsAck, WsRequest, WsUpdate,
};

use crate::{common::Market, http::Network};

/// How long the venue tolerates silence before dropping the connection.
pub const SERVER_IDLE_DISCONNECT_SECS: u64 = 60;

/// Default idle period before probing with a ping.
///
/// The full detection cycle costs two of these - one waiting for traffic, one waiting for the
/// pong - so the value must stay under half of [`SERVER_IDLE_DISCONNECT_SECS`]. Otherwise the
/// venue would close the connection before this side concluded anything was wrong, turning
/// an orderly reconnect into a surprise disconnect. [`probe_interval_is_sound`] states the
/// rule, and the client rejects a configuration that breaks it.
///
/// Seconds, because that is the unit the transport's heartbeat fields take: expressing it
/// more finely here would only introduce a rounding step between the value that gets checked
/// and the value that gets used.
pub const DEFAULT_IDLE_PROBE_SECS: u64 = 20;

/// The bound this module's default is required to satisfy, checked at compile time.
///
/// `assert!` inside a test over two constants is a tautology the compiler can already settle, so
/// stating it here makes the build fail on a bad default rather than the test suite.
const _: () = assert!(DEFAULT_IDLE_PROBE_SECS * 2 < SERVER_IDLE_DISCONNECT_SECS);

/// Whether a probe interval leaves room to detect a dead link before the venue hangs up.
#[must_use]
pub const fn probe_interval_is_sound(probe_secs: u64) -> bool {
    probe_secs > 0 && probe_secs * 2 < SERVER_IDLE_DISCONNECT_SECS
}

/// WebSocket URL for one network and market.
#[must_use]
pub fn stream_url(network: Network, market: Market) -> String {
    format!("{}/{}", network.ws_base(), market.path_segment())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn the_default_probe_interval_can_detect_a_dead_link_in_time() {
        // Detection costs two intervals: one waiting for traffic, one waiting for the pong.
        // The arithmetic bound itself is asserted at compile time above.
        assert!(probe_interval_is_sound(DEFAULT_IDLE_PROBE_SECS));
    }

    #[rstest]
    fn a_probe_interval_at_half_the_disconnect_window_is_too_slow() {
        // 30s would conclude "dead" at exactly 60s - the moment the venue drops the link,
        // making every reconnect reactive rather than preemptive.
        assert!(!probe_interval_is_sound(SERVER_IDLE_DISCONNECT_SECS / 2));
        assert!(!probe_interval_is_sound(SERVER_IDLE_DISCONNECT_SECS));
        assert!(!probe_interval_is_sound(0));
    }

    #[rstest]
    fn stream_urls_pair_network_with_market() {
        assert_eq!(
            stream_url(Network::Testnet, Market::Perps),
            "wss://testnet-gw.sodex.dev/ws/perps"
        );
        assert_eq!(
            stream_url(Network::Mainnet, Market::Spot),
            "wss://mainnet-gw.sodex.dev/ws/spot"
        );
    }
}
