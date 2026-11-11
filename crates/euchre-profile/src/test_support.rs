//! Process-local start and sleep counters and the last persisted state per
//! profile owner, so engine tests can prove an actor really slept and woke and
//! read state that has no read action (`lifetime`, `dailyTokens`, D12).
//!
//! Recording is off until a test calls [`enable`]. It is a runtime switch, not
//! a Cargo feature, so feature unification in a workspace-wide build cannot
//! turn it on in a server binary. Never call [`enable`] outside a test.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use euchre_profile_model::ProfileActorState;
use serde_json::Value;

static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTS: LazyLock<parking_lot::Mutex<HashMap<String, (u64, u64)>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static STATES: LazyLock<parking_lot::Mutex<HashMap<String, Value>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

/// Turn recording on for this process. Tests only: the maps keep one entry per
/// owner and are never evicted.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether [`enable`] ran in this process.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn bump(owner: &str, sleep: bool) {
    if !enabled() {
        return;
    }
    let mut counts = COUNTS.lock();
    let entry = counts.entry(owner.to_owned()).or_default();
    if sleep {
        entry.1 += 1;
    } else {
        entry.0 += 1;
    }
}

/// Record that `owner`'s actor was created (every wake).
pub(crate) fn record_start(owner: &str) {
    bump(owner, false);
}

/// Record that `owner`'s actor went to sleep.
pub(crate) fn record_sleep(owner: &str) {
    bump(owner, true);
}

/// How many times `owner`'s actor started in this process (`0` until [`enable`]).
pub fn starts(owner: &str) -> u64 {
    COUNTS.lock().get(owner).map_or(0, |c| c.0)
}

/// How many times `owner`'s actor went to sleep in this process (`0` until [`enable`]).
pub fn sleeps(owner: &str) -> u64 {
    COUNTS.lock().get(owner).map_or(0, |c| c.1)
}

/// Record the state `owner`'s actor just persisted.
pub(crate) fn record_state(owner: &str, state: &ProfileActorState) {
    if !enabled() {
        return;
    }
    if let Ok(v) = serde_json::to_value(state) {
        STATES.lock().insert(owner.to_owned(), v);
    }
}

/// The JSON of the state `owner`'s actor last persisted (`None` until [`enable`]).
pub fn last_state(owner: &str) -> Option<Value> {
    STATES.lock().get(owner).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_nothing_until_enabled() {
        // Unit tests in this binary never call `enable` before this check.
        let owner = "test-support-off";
        if !enabled() {
            record_start(owner);
            record_sleep(owner);
            assert_eq!(starts(owner), 0);
            assert_eq!(sleeps(owner), 0);
        }
        enable();
        let owner = "test-support-on";
        record_start(owner);
        record_start(owner);
        record_sleep(owner);
        assert_eq!((starts(owner), sleeps(owner)), (2, 1));
    }
}
