//! Outbox order, the one-in-flight `recordGame` guard and the `decide` budget.

use std::sync::Arc;
use std::time::Duration;

use euchre_engine::Seat;
use euchre_protocol::{AiSeatLifecycleKind, AiSeatLifecycleMessage};
use parking_lot::Mutex;
use serde_json::json;

use super::*;

fn seat(n: u8) -> Seat {
    match Seat::new(n) {
        Some(s) => s,
        None => panic!("seat {n}"),
    }
}

fn notify(seat_no: u8, kind: AiSeatLifecycleKind, hand_no: u32) -> Job {
    Job::Notify(LifecycleCall {
        seat: seat(seat_no),
        key: euchre_protocol::ai_seat_key("g", seat(seat_no)),
        create_input: json!({}),
        message: AiSeatLifecycleMessage {
            internal_token: "t".into(),
            game_id: "g".into(),
            kind,
            hand_no: Some(hand_no),
        },
    })
}

fn label(job: &Job) -> String {
    match job {
        Job::Notify(c) => format!("{}:{}", job.action(), c.message.hand_no.unwrap_or(0)),
        other => other.action().to_owned(),
    }
}

#[test]
fn each_ai_seat_and_the_profile_have_their_own_outbox() {
    assert_eq!(
        [seat_lane(seat(1)), seat_lane(seat(2)), seat_lane(seat(3))],
        [0, 1, 2]
    );
    assert_eq!(PROFILE_LANE, 3);
}

#[test]
fn only_the_first_push_to_an_idle_outbox_starts_a_drainer() {
    let outbox = Outbox::default();
    assert!(outbox.push(0, notify(1, AiSeatLifecycleKind::HandEnd, 1)));
    assert!(!outbox.push(0, notify(1, AiSeatLifecycleKind::ResetHand, 2)));
    // Another seat's outbox is independent.
    assert!(outbox.push(1, notify(2, AiSeatLifecycleKind::HandEnd, 1)));
    let order: Vec<String> = std::iter::from_fn(|| outbox.next(0))
        .map(|j| label(&j))
        .collect();
    assert_eq!(order, ["handEnd:1", "resetHand:2"]);
    // Empty: the outbox is idle again, so the next push starts a drainer.
    assert!(outbox.push(0, notify(1, AiSeatLifecycleKind::GameEnd, 0)));
    // An unknown lane takes nothing.
    assert!(!outbox.push(PROFILE_LANE + 1, notify(1, AiSeatLifecycleKind::GameEnd, 0)));
}

/// `requests/server-ai-seat-dispatch-order.md`: a slow `handEnd` to a seat
/// never lets that seat's next `resetHand` or `decide` overtake it, and a
/// slow seat never delays another seat.
#[tokio::test]
async fn a_slow_hand_end_is_never_overtaken_and_never_blocks_another_seat() {
    let outbox = Arc::new(Outbox::default());
    let done: Arc<Mutex<Vec<String>>> = Arc::default();

    let run = |outbox: Arc<Outbox>, lane: usize, done: Arc<Mutex<Vec<String>>>| async move {
        drain(&outbox, lane, |job| {
            let done = done.clone();
            async move {
                let name = label(&job);
                if name.starts_with("handEnd") && lane == 0 {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
                done.lock().push(format!("{lane}/{name}"));
            }
        })
        .await;
    };

    assert!(outbox.push(0, notify(1, AiSeatLifecycleKind::HandEnd, 1)));
    let seat1 = tokio::spawn(run(outbox.clone(), 0, done.clone()));
    tokio::time::sleep(Duration::from_millis(10)).await;
    // Pushed while the handEnd is still in flight: the drainer takes them.
    assert!(!outbox.push(0, notify(1, AiSeatLifecycleKind::ResetHand, 2)));
    assert!(!outbox.push(0, notify(1, AiSeatLifecycleKind::GameEnd, 2)));
    assert!(outbox.push(1, notify(2, AiSeatLifecycleKind::HandEnd, 1)));
    let seat2 = tokio::spawn(run(outbox.clone(), 1, done.clone()));
    let (a, b) = tokio::join!(seat1, seat2);
    assert!(a.is_ok() && b.is_ok());

    let done = done.lock().clone();
    assert_eq!(
        done,
        ["1/handEnd:1", "0/handEnd:1", "0/resetHand:2", "0/gameEnd:2"],
        "seat 2 finished first; seat 1 kept its order"
    );
}

#[test]
fn one_record_game_is_in_flight_at_a_time() {
    let outbox = Outbox::default();
    assert!(outbox.begin_record());
    assert!(!outbox.begin_record());
    outbox.end_record();
    assert!(outbox.begin_record());
}

#[test]
fn the_decide_budget_is_the_time_to_the_deadline_and_at_least_one_ms() {
    assert_eq!(decide_budget(5_000, 3_500), Duration::from_millis(1_500));
    assert_eq!(decide_budget(5_000, 5_000), Duration::from_millis(1));
    assert_eq!(decide_budget(5_000, 9_000), Duration::from_millis(1));
}

#[test]
fn seat_params_carry_the_match_token_only() {
    assert_eq!(seat_params("tok"), json!({ "internalToken": "tok" }));
}
