//! Process-local start and sleep counters and the last persisted state per
//! seat, so engine tests can prove a seat really slept and woke and read the
//! state that no read action exposes (memory, budget, meter).
//!
//! Recording is off until a test calls [`enable`]. It is a runtime switch, not
//! a Cargo feature, so a workspace-wide build cannot turn it on in a server
//! binary. Never call [`enable`] outside a test.
//!
//! Seats are labelled `"<gameId>/<seat>"` (see [`label`]).

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use euchre_ai::ai_seat::state::AiSeatState;
use serde_json::Value;

static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTS: LazyLock<parking_lot::Mutex<HashMap<String, (u64, u64)>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static STATES: LazyLock<parking_lot::Mutex<HashMap<String, Value>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static INLINE: LazyLock<parking_lot::Mutex<HashMap<String, u64>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static SLEEP_REQUESTS: LazyLock<parking_lot::Mutex<HashSet<String>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashSet::new()));

/// Turn recording on for this process. Tests only: the maps keep one entry per
/// seat and are never evicted.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether [`enable`] ran in this process.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The label of seat `seat` of match `game_id`.
pub fn label(game_id: &str, seat: u8) -> String {
    format!("{game_id}/{seat}")
}

fn bump(seat: &str, sleep: bool) {
    if !enabled() {
        return;
    }
    let mut counts = COUNTS.lock();
    let entry = counts.entry(seat.to_owned()).or_default();
    if sleep {
        entry.1 += 1;
    } else {
        entry.0 += 1;
    }
}

/// Record that a seat was created (every wake).
pub(crate) fn record_start(seat: &str) {
    bump(seat, false);
}

/// Record that a seat went to sleep.
pub(crate) fn record_sleep(seat: &str) {
    bump(seat, true);
}

/// How many times the seat started in this process (`0` until [`enable`]).
pub fn starts(seat: &str) -> u64 {
    COUNTS.lock().get(seat).map_or(0, |c| c.0)
}

/// How many times the seat went to sleep in this process (`0` until [`enable`]).
pub fn sleeps(seat: &str) -> u64 {
    COUNTS.lock().get(seat).map_or(0, |c| c.1)
}

/// Record the state a seat just persisted.
pub(crate) fn record_state(seat: &str, state: &AiSeatState) {
    if !enabled() {
        return;
    }
    if let Ok(v) = serde_json::to_value(state) {
        STATES.lock().insert(seat.to_owned(), v);
    }
}

/// The JSON of the state the seat last persisted (`None` until [`enable`]).
pub fn last_state(seat: &str) -> Option<Value> {
    STATES.lock().get(seat).cloned()
}

/// Ask the seat to request its own sleep (`ctx.sleep()`) when it next starts
/// handling a message, so a test can send messages during sleep grace. No-op
/// until [`enable`].
pub fn sleep_on_next_message(seat: &str) {
    if enabled() {
        SLEEP_REQUESTS.lock().insert(seat.to_owned());
    }
}

/// Record that a message arrived after the worker closed the inbox (sleep
/// grace) and was handled inline in its action.
pub(crate) fn record_inline(seat: &str) {
    if enabled() {
        *INLINE.lock().entry(seat.to_owned()).or_default() += 1;
    }
}

/// How many messages the seat handled inline during sleep grace (`0` until
/// [`enable`]).
pub fn inline_messages(seat: &str) -> u64 {
    INLINE.lock().get(seat).copied().unwrap_or(0)
}

/// Take a pending [`sleep_on_next_message`] request (always `false` until
/// [`enable`]).
pub(crate) fn take_sleep_request(seat: &str) -> bool {
    enabled() && SLEEP_REQUESTS.lock().remove(seat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_nothing_until_enabled() {
        let seat = "test-support-off/1";
        if !enabled() {
            record_start(seat);
            record_sleep(seat);
            assert_eq!(starts(seat), 0);
            assert_eq!(sleeps(seat), 0);
        }
        enable();
        let seat = label("test-support-on", 2);
        assert_eq!(seat, "test-support-on/2");
        record_start(&seat);
        record_start(&seat);
        record_sleep(&seat);
        assert_eq!((starts(&seat), sleeps(&seat)), (2, 1));
        assert!(!take_sleep_request(&seat));
        sleep_on_next_message(&seat);
        assert!(take_sleep_request(&seat));
        assert!(!take_sleep_request(&seat));
    }
}
