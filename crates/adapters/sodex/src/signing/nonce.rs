//! Nonce generation for signed requests.
//!
//! SoDEX tracks nonces **per signing address**, not per account, and keeps only the 100
//! highest values seen. A new nonce must exceed the smallest of that set and must never
//! have been used before. Valid values are additionally confined to `(T - 2 days, T + 1 day)`
//! where `T` is the block timestamp in Unix milliseconds.
//!
//! Two consequences shape this module:
//!
//! - Nonces must be monotonic per signing key, so the generator is a counter rather than a
//!   bare clock read: two calls within the same millisecond must still differ.
//! - Because tracking is per *address*, two concurrent processes sharing one API key share
//!   one nonce sequence and will race. The venue's own guidance is one API key per trading
//!   process; [`NonceGenerator`] cannot enforce that across processes, so it guards only
//!   the in-process case.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Lower bound of the acceptance window, relative to server time.
pub const WINDOW_BEHIND_MS: u64 = 2 * 24 * 60 * 60 * 1000;

/// Upper bound of the acceptance window, relative to server time.
pub const WINDOW_AHEAD_MS: u64 = 24 * 60 * 60 * 1000;

/// Monotonic nonce source for one signing address.
///
/// Values track wall-clock milliseconds when traffic is sparse, and degrade to a simple
/// increment under bursts so that a batch submitted within a single millisecond still
/// yields distinct nonces.
#[derive(Debug)]
pub struct NonceGenerator {
    last: AtomicU64,
}

impl NonceGenerator {
    /// Creates a generator seeded from the current clock.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last: AtomicU64::new(now_ms()),
        }
    }

    /// Creates a generator seeded from an explicit value.
    ///
    /// Useful when resuming against an address whose high-water mark is already known, and
    /// in tests where a fixed starting point is needed.
    #[must_use]
    pub fn starting_at(seed: u64) -> Self {
        Self {
            last: AtomicU64::new(seed),
        }
    }

    /// Returns the next nonce: the current millisecond if the clock has moved on, otherwise
    /// one more than the previous value.
    ///
    /// The venue permits fast-forwarding the counter to wall-clock time, which is what keeps
    /// nonces inside the acceptance window after an idle period rather than lagging behind by
    /// however many requests were issued earlier.
    pub fn next(&self) -> u64 {
        let now = now_ms();
        // `fetch_update` hands back the value *before* the update, so the issued nonce has
        // to be recomputed from it. Returning the previous value directly would hand out a
        // stale nonce after an idle period - outside the acceptance window, and already used.
        let previous = self
            .last
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                Some(last.saturating_add(1).max(now))
            })
            .expect("fetch_update with an infallible closure cannot fail");

        previous.saturating_add(1).max(now)
    }

    /// The most recently issued value, for diagnostics.
    pub fn last_issued(&self) -> u64 {
        self.last.load(Ordering::SeqCst)
    }
}

impl Default for NonceGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `nonce` falls inside the venue's acceptance window around `server_time_ms`.
///
/// The bounds are exclusive, matching the documented `(T - 2 days, T + 1 day)`.
#[must_use]
pub fn is_within_window(nonce: u64, server_time_ms: u64) -> bool {
    let lower = server_time_ms.saturating_sub(WINDOW_BEHIND_MS);
    let upper = server_time_ms.saturating_add(WINDOW_AHEAD_MS);
    nonce > lower && nonce < upper
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn successive_nonces_strictly_increase_within_one_millisecond() {
        // Seeding far in the future keeps the clock from advancing past the counter, which
        // is exactly the burst case: many nonces drawn inside a single millisecond.
        let future = now_ms() + 60_000;
        let nonces = NonceGenerator::starting_at(future);

        let drawn: Vec<u64> = (0..1000).map(|_| nonces.next()).collect();

        for pair in drawn.windows(2) {
            assert!(pair[1] > pair[0], "nonces must strictly increase: {pair:?}");
        }
    }

    #[rstest]
    fn nonce_fast_forwards_to_wall_clock_after_idling() {
        // A generator seeded in the distant past must jump to now rather than creeping up
        // one increment at a time, otherwise it would sit outside the acceptance window.
        let stale = now_ms() - WINDOW_BEHIND_MS * 2;
        let nonces = NonceGenerator::starting_at(stale);

        let issued = nonces.next();

        assert!(
            is_within_window(issued, now_ms()),
            "issued {issued} fell outside the window"
        );
    }

    #[rstest]
    fn window_bounds_are_exclusive() {
        let t = 1_760_373_925_000_u64;

        assert!(!is_within_window(t - WINDOW_BEHIND_MS, t), "lower bound");
        assert!(!is_within_window(t + WINDOW_AHEAD_MS, t), "upper bound");
        assert!(is_within_window(t - WINDOW_BEHIND_MS + 1, t));
        assert!(is_within_window(t + WINDOW_AHEAD_MS - 1, t));
        assert!(is_within_window(t, t));
    }

    #[rstest]
    fn window_rejects_values_outside_the_bounds() {
        let t = 1_760_373_925_000_u64;

        assert!(!is_within_window(t - WINDOW_BEHIND_MS - 1, t), "too old");
        assert!(
            !is_within_window(t + WINDOW_AHEAD_MS + 1, t),
            "too far ahead"
        );
    }

    #[rstest]
    fn concurrent_draws_never_collide() {
        use std::{sync::Arc, thread};

        let future = now_ms() + 60_000;
        let nonces = Arc::new(NonceGenerator::starting_at(future));

        #[allow(clippy::needless_collect)] // Collecting is what starts all eight threads
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let nonces = Arc::clone(&nonces);
                thread::spawn(move || (0..500).map(|_| nonces.next()).collect::<Vec<_>>())
            })
            .collect();

        let mut all: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread panicked"))
            .collect();

        let issued = all.len();
        all.sort_unstable();
        all.dedup();

        assert_eq!(all.len(), issued, "duplicate nonce handed to two threads");
    }
}
