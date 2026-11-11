//! Clocks for the injected [`Clock`] seam.
//!
//! [`SystemClock`] reads wall time on every call (`Date.now`). It is the clock
//! production uses: the table stamps `deadlineAt` on the wall clock, so the
//! seat must measure budgets against the same clock.
//!
//! [`TokioClock`] anchors an epoch to a tokio `Instant`, so under
//! `tokio::time::pause` the ladder's `now()` advances with the paused timer
//! wheel and every deadline edge is testable exactly. It is for tests only.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::Clock;

/// Wall-clock epoch milliseconds (`Date.now`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

/// Current wall-clock epoch milliseconds (0 before 1970).
pub fn system_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        system_now_ms()
    }
}

/// `epoch_ms + (tokio::time::Instant::now() - origin)`. Test-only.
///
/// It reads the wall clock once and then adds monotonic elapsed time, so it
/// drifts from `Date.now` after a wall-clock step (NTP at boot) or a host sleep.
/// Never use it to measure a table `deadlineAt`; use [`SystemClock`].
#[derive(Clone, Copy, Debug)]
pub struct TokioClock {
    epoch_ms: u64,
    origin: tokio::time::Instant,
}

impl TokioClock {
    /// A clock reading `epoch_ms` now.
    pub fn starting_at(epoch_ms: u64) -> TokioClock {
        TokioClock {
            epoch_ms,
            origin: tokio::time::Instant::now(),
        }
    }

    /// A clock anchored to the current wall time. Test-only; production code
    /// uses [`SystemClock`], which tracks wall-clock steps.
    pub fn from_system() -> TokioClock {
        TokioClock::starting_at(system_now_ms())
    }
}

impl Clock for TokioClock {
    fn now_ms(&self) -> u64 {
        let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.epoch_ms.saturating_add(elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production clock reads `SystemTime` on every call, so a wall-clock
    /// step shows up at once instead of drifting like an anchored clock.
    #[test]
    fn system_clock_reads_wall_time_on_every_call() {
        let before = system_now_ms();
        let read = SystemClock.now_ms();
        let after = system_now_ms();
        assert!(before <= read && read <= after);
    }
}
