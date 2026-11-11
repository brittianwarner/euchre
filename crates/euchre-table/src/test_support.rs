//! Process-local start and sleep counters and the last persisted state per
//! match, so engine tests can prove a table really slept and woke, and read
//! server-only state (the per-match internal token, the timer ledger) that no
//! action returns.
//!
//! Recording is off until a test calls [`enable`]. It is a runtime switch, not
//! a Cargo feature, so feature unification in a workspace-wide build cannot
//! turn it on in a server binary. Never call [`enable`] outside a test.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::state::TableState;

static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTS: LazyLock<parking_lot::Mutex<HashMap<String, (u64, u64)>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static STATES: LazyLock<parking_lot::Mutex<HashMap<String, Value>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static SLEEP_AFTER_DISPATCH: LazyLock<parking_lot::Mutex<HashSet<String>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashSet::new()));

/// Turn recording on for this process. Tests only: the maps keep one entry per
/// match and are never evicted.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether [`enable`] ran in this process.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn bump(match_id: &str, sleep: bool) {
    if !enabled() {
        return;
    }
    let mut counts = COUNTS.lock();
    let entry = counts.entry(match_id.to_owned()).or_default();
    if sleep {
        entry.1 += 1;
    } else {
        entry.0 += 1;
    }
}

/// Record that the table was created (every wake).
pub(crate) fn record_start(match_id: &str) {
    bump(match_id, false);
}

/// Record that the table went to sleep.
pub(crate) fn record_sleep(match_id: &str) {
    bump(match_id, true);
}

/// How many times the table started in this process (`0` until [`enable`]).
pub fn starts(match_id: &str) -> u64 {
    COUNTS.lock().get(match_id).map_or(0, |c| c.0)
}

/// How many times the table went to sleep in this process (`0` until [`enable`]).
pub fn sleeps(match_id: &str) -> u64 {
    COUNTS.lock().get(match_id).map_or(0, |c| c.1)
}

/// Record the state the table just persisted.
pub(crate) fn record_state(match_id: &str, state: &TableState) {
    if !enabled() {
        return;
    }
    if let Ok(v) = serde_json::to_value(state) {
        STATES.lock().insert(match_id.to_owned(), v);
    }
}

/// The JSON of the state the table last persisted (`None` until [`enable`]).
pub fn last_state(match_id: &str) -> Option<Value> {
    STATES.lock().get(match_id).cloned()
}

/// The per-match internal token of the table, from its last persisted state.
pub fn internal_token(match_id: &str) -> Option<String> {
    last_state(match_id)?["game"]["internalToken"]
        .as_str()
        .map(str::to_owned)
}

/// Make the table of `match_id` call `ctx.sleep()` right after its next
/// `decide` to an `aiSeat` is sent, so a test can crash it between the
/// dispatch and the decision. One-shot; ignored until [`enable`].
pub fn request_sleep_after_dispatch(match_id: &str) {
    if enabled() {
        SLEEP_AFTER_DISPATCH.lock().insert(match_id.to_owned());
    }
}

/// Take a pending [`request_sleep_after_dispatch`] for `match_id`.
pub(crate) fn take_sleep_after_dispatch(match_id: &str) -> bool {
    enabled() && SLEEP_AFTER_DISPATCH.lock().remove(match_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_nothing_until_enabled() {
        let id = "test-support-off";
        if !enabled() {
            record_start(id);
            record_sleep(id);
            assert_eq!((starts(id), sleeps(id)), (0, 0));
        }
        enable();
        let id = "test-support-on";
        record_start(id);
        record_start(id);
        record_sleep(id);
        assert_eq!((starts(id), sleeps(id)), (2, 1));
        request_sleep_after_dispatch(id);
        assert!(take_sleep_after_dispatch(id));
        assert!(!take_sleep_after_dispatch(id), "one-shot");
    }
}
