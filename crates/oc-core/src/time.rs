//! Wall-clock time behind a trait, so signed records and stored timestamps
//! can be tested with a clock the test controls.
//!
//! Intervals and timeouts do not use this: they use the async runtime's
//! timer, which tests can pause and advance.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of wall-clock time.
pub trait Clock: Send + Sync + 'static {
    /// Milliseconds since the Unix epoch.
    fn now_unix_ms(&self) -> u64;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to. For tests.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    /// A clock stopped at the given time.
    pub fn new(unix_ms: u64) -> Self {
        Self(AtomicU64::new(unix_ms))
    }

    /// Moves the clock forward.
    pub fn advance_ms(&self, by: u64) {
        self.0.fetch_add(by, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let clock = ManualClock::new(1_000);
        assert_eq!(clock.now_unix_ms(), 1_000);
        clock.advance_ms(250);
        assert_eq!(clock.now_unix_ms(), 1_250);
    }

    #[test]
    fn system_clock_is_after_2020() {
        assert!(SystemClock.now_unix_ms() > 1_577_836_800_000);
    }
}
