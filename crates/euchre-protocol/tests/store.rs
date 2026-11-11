//! `TableStore` (02 section 6): a 1:1 port of `game/table.test.ts`, then one
//! test per rule in 02 sections 6.1-6.3 and 6.5.

mod common;

use std::collections::VecDeque;

use euchre_protocol::store::{
    ACTION_TIMEOUT_MS, MSG_CONNECTION_INTERRUPTED, MSG_COULD_NOT_CONFIRM, MSG_HAND_LOAD_FAILED,
    MSG_ILLEGAL_MOVE, MSG_LOADING_CONNECTED_BODY, MSG_LOADING_CONNECTED_HEADING,
    MSG_LOADING_DISCONNECTED_BODY, MSG_LOADING_DISCONNECTED_HEADING, MSG_OLDER_HAND_NOT_SAVED,
    MSG_RECONNECTING, MSG_REVIEW_FAILED, MSG_TABLE_OPEN_FAILED, MSG_WAITING_FOR_TABLE, RequestId,
    initial_review_hand, loading_error_line,
};
use euchre_protocol::*;

/* -------------------------------------------------------------------------- */
/* Fixture                                                                     */
/* -------------------------------------------------------------------------- */

/// `project(createGame({ gameId: 'store-test', seed: 'store-test' }), 0)` from TS.
fn base_view() -> PublicGameView {
    serde_json::from_value(common::entry("view", "storeTest")).expect("storeTest view")
}

/// The TS test's `view(v, turnId)`: the base view with one legal play of 9H.
fn view(v: u64, turn_id: &str) -> PublicGameView {
    let mut view = base_view();
    view.v = v;
    view.turn_id = turn_id.to_owned();
    view.legal = vec![LegalMove {
        id: "play:9H".to_owned(),
        mv: Move::Play {
            card: Card::parse("9H").expect("9H"),
        },
        label: "Nine of hearts".to_owned(),
    }];
    view
}

fn sync(view: PublicGameView) -> SyncEvent {
    SyncEvent {
        v: view.v,
        view,
        steps: vec![],
    }
}

/// A scripted host: runs the store's commands against canned replies, the way
/// the TS test's `vi.fn` handle does, and counts calls.
struct Host {
    store: TableStore,
    now: u64,
    submit_replies: VecDeque<SubmitOutcome>,
    snapshot: Option<PublicGameView>,
    submits: Vec<SubmitMoveRequest>,
    snapshots: usize,
    reviews: Vec<ReviewTrickRequest>,
    review_reply: Option<ReviewTrickAck>,
    hand_reviews: Vec<u32>,
    hand_review_reply: HandReviewOutcome,
    events: Vec<StoreEvent>,
    /// When false, commands are left in `parked` instead of being answered.
    auto: bool,
    parked: VecDeque<StoreCommand>,
}

impl Host {
    fn new(snapshot: PublicGameView) -> Host {
        Host {
            store: TableStore::new(),
            now: 1_000,
            submit_replies: VecDeque::new(),
            snapshot: Some(snapshot),
            submits: vec![],
            snapshots: 0,
            reviews: vec![],
            review_reply: Some(ReviewTrickAck::default()),
            hand_reviews: vec![],
            hand_review_reply: HandReviewOutcome::Loaded(None),
            events: vec![],
            auto: true,
            parked: VecDeque::new(),
        }
    }

    /// Runs effects to quiescence; returns the events produced along the way.
    fn drive(&mut self, fx: StoreEffects) -> Vec<StoreEvent> {
        let start = self.events.len();
        self.events.extend(fx.events);
        let mut queue: VecDeque<StoreCommand> = fx.commands.into();
        while let Some(cmd) = queue.pop_front() {
            if !self.auto {
                self.parked.push_back(cmd);
                continue;
            }
            let next = self.answer(cmd);
            self.events.extend(next.events);
            queue.extend(next.commands);
        }
        self.events[start..].to_vec()
    }

    fn answer(&mut self, cmd: StoreCommand) -> StoreEffects {
        match cmd {
            StoreCommand::FetchSnapshot { id } => {
                self.snapshots += 1;
                let snap = self.snapshot.clone();
                self.store.on_snapshot(id, snap, self.now)
            }
            StoreCommand::SubmitMove { id, request } => {
                self.submits.push(request);
                let reply = self
                    .submit_replies
                    .pop_front()
                    .unwrap_or(SubmitOutcome::NoAck);
                self.store.on_submit(id, reply, self.now)
            }
            StoreCommand::ReviewTrick { id, request } => {
                self.reviews.push(request);
                self.store.on_review(id, self.review_reply)
            }
            StoreCommand::GetHandReview { id, hand_no } => {
                self.hand_reviews.push(hand_no);
                let reply = self.hand_review_reply.clone();
                self.store.on_hand_review(id, reply);
                StoreEffects::default()
            }
        }
    }

    fn play(&mut self, move_id: &str) -> Option<bool> {
        let fx = self.store.play(move_id, "client-move-1", self.now);
        self.settle(fx)
    }

    /// Drives a play's effects: `Some(false)` for a refusal, else the
    /// settlement of the submitted request.
    fn settle(&mut self, fx: StoreEffects) -> Option<bool> {
        if fx.play_refused() {
            return Some(false);
        }
        let id = fx.submit_id()?;
        settled(&self.drive(fx), id)
    }

    fn resync(&mut self) -> Vec<StoreEvent> {
        let fx = self.store.resync(self.now);
        self.drive(fx)
    }
}

fn settled(events: &[StoreEvent], id: RequestId) -> Option<bool> {
    events.iter().find_map(|e| match e {
        StoreEvent::PlaySettled { id: got, ok } if *got == id => Some(*ok),
        _ => None,
    })
}

/// The TS `fixture(submitMove, snapshot)`: bound, connected, at `view(1, 'turn')`.
fn fixture(submit: SubmitOutcome, snapshot: PublicGameView) -> Host {
    let mut host = Host::new(snapshot);
    host.submit_replies.push_back(submit);
    host.store.bind(true, host.now);
    host.store.apply_sync(&sync(view(1, "turn")));
    host
}

fn stale_turn() -> SubmitOutcome {
    SubmitOutcome::Ack(MoveAck::Rejected {
        code: ErrorCode::Rule(RuleCode::StaleTurn),
        legal: vec![],
    })
}

/* -------------------------------------------------------------------------- */
/* table.test.ts — "uncertain move acknowledgements"                           */
/* -------------------------------------------------------------------------- */

#[test]
fn recognizes_a_committed_move_after_its_acknowledgement_is_lost_without_resubmitting() {
    let mut host = fixture(SubmitOutcome::NoAck, view(2, "next"));
    assert_eq!(host.play("play:9H"), Some(true));
    assert_eq!(host.submits.len(), 1);
    assert_eq!(host.snapshots, 1);
    assert_eq!(host.store.view().map(|v| v.turn_id.as_str()), Some("next"));
    assert_eq!(host.store.error(), None);
}

#[test]
fn offers_a_retry_only_when_the_same_authoritative_turn_is_still_current() {
    let mut host = fixture(stale_turn(), view(1, "turn"));
    assert_eq!(host.play("play:9H"), Some(false));
    assert!(
        host.store
            .error()
            .is_some_and(|e| e.contains("Please try again"))
    );
    assert!(!host.store.submitting());
}

#[test]
fn holds_newer_sync_state_when_an_older_snapshot_arrives() {
    let mut host = fixture(SubmitOutcome::NoAck, view(1, "turn"));
    host.store.apply_sync(&sync(view(3, "newer")));
    host.resync();
    assert_eq!(host.store.view().map(|v| v.v), Some(3));
    assert_eq!(host.store.view().map(|v| v.turn_id.as_str()), Some("newer"));
}

/* -------------------------------------------------------------------------- */
/* 6.1 state                                                                   */
/* -------------------------------------------------------------------------- */

#[test]
fn a_new_store_is_idle_and_empty() {
    let store = TableStore::new();
    assert_eq!(store.status(), StoreStatus::Idle);
    assert!(store.view().is_none() && store.display_view().is_none());
    assert_eq!(store.error(), None);
    assert!(!store.submitting());
    assert!(store.chat().is_empty());
}

#[test]
fn bind_reports_ready_when_connected_and_connecting_otherwise() {
    let mut a = TableStore::new();
    a.bind(true, 0);
    assert_eq!(a.status(), StoreStatus::Ready);
    let mut b = TableStore::new();
    b.bind(false, 0);
    assert_eq!(b.status(), StoreStatus::Connecting);
}

#[test]
fn bind_issues_no_snapshot() {
    let mut host = Host::new(view(1, "t"));
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(1, "t")));
    assert_eq!(host.snapshots, 0);
}

/* -------------------------------------------------------------------------- */
/* 6.2 applySync / applyChat                                                   */
/* -------------------------------------------------------------------------- */

#[test]
fn apply_sync_accepts_equal_v_and_drops_older() {
    let mut store = TableStore::new();
    store.bind(false, 0);
    store.apply_sync(&sync(view(5, "a")));
    assert_eq!(store.status(), StoreStatus::Ready);
    store.apply_sync(&sync(view(5, "b")));
    assert_eq!(
        store.view().map(|v| v.turn_id.as_str()),
        Some("b"),
        "equal v wins"
    );
    store.apply_sync(&sync(view(4, "c")));
    assert_eq!(
        store.view().map(|v| v.turn_id.as_str()),
        Some("b"),
        "older v dropped"
    );
}

#[test]
fn apply_sync_clears_the_error() {
    let mut host = fixture(stale_turn(), view(1, "turn"));
    host.play("play:9H");
    assert!(host.store.error().is_some());
    host.store.apply_sync(&sync(view(2, "next")));
    assert_eq!(host.store.error(), None);
}

#[test]
fn sync_events_record_steps_even_when_the_view_is_dropped() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.on_sync_event(sync(view(5, "a")));
    let steps: Vec<Step> = serde_json::from_value(common::fixture("steps")).expect("steps fixture");
    store.on_sync_event(SyncEvent {
        v: 4,
        view: view(4, "old"),
        steps: steps.clone(),
    });
    assert_eq!(store.view().map(|v| v.v), Some(5));
    assert_eq!(store.last_steps(), steps.as_slice());
}

#[test]
fn snapshots_never_touch_the_recorded_steps() {
    let mut host = Host::new(view(9, "snap"));
    host.store.bind(true, 0);
    let steps: Vec<Step> = serde_json::from_value(common::fixture("steps")).expect("steps fixture");
    host.store.on_sync_event(SyncEvent {
        v: 1,
        view: view(1, "t"),
        steps: steps.clone(),
    });
    host.resync();
    assert_eq!(host.store.view().map(|v| v.v), Some(9));
    assert_eq!(host.store.last_steps(), steps.as_slice());
}

fn chat(msg_id: &str, kind: ChatKind) -> ChatEvent {
    ChatEvent {
        msg_id: msg_id.to_owned(),
        seat: Seat::new(2).expect("seat"),
        kind,
        text: format!("line {msg_id}"),
        is_final: True,
    }
}

#[test]
fn apply_chat_ignores_a_redelivered_msg_id() {
    let mut store = TableStore::new();
    store.apply_chat(ChatLine::from(&chat("a", ChatKind::Call)));
    store.apply_chat(ChatLine::from(&chat("a", ChatKind::Call)));
    assert_eq!(store.chat().len(), 1);
}

#[test]
fn apply_chat_keeps_only_the_last_six_lines() {
    let mut store = TableStore::new();
    for i in 0..9 {
        store.apply_chat(ChatLine::from(&chat(&i.to_string(), ChatKind::Call)));
    }
    let ids: Vec<&str> = store.chat().iter().map(|l| l.msg_id.as_str()).collect();
    assert_eq!(ids, ["3", "4", "5", "6", "7", "8"]);
}

/* -------------------------------------------------------------------------- */
/* 6.2 resync                                                                  */
/* -------------------------------------------------------------------------- */

#[test]
fn concurrent_resyncs_share_one_snapshot() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    let first = store.resync(0);
    let second = store.resync(0);
    assert_eq!(first.commands.len(), 1);
    assert!(second.is_empty(), "joins the in-flight snapshot");
    assert!(store.is_resyncing());
}

#[test]
fn resync_does_nothing_while_disconnected() {
    let mut store = TableStore::new();
    store.bind(false, 0);
    let fx = store.resync(0);
    assert!(fx.commands.is_empty());
    assert_eq!(fx.events, [StoreEvent::ResyncSettled { id: None }]);
    assert_eq!(store.error(), None);
}

#[test]
fn a_snapshot_applies_when_not_older_including_equal_v() {
    let mut host = Host::new(view(5, "snap"));
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(5, "live")));
    host.resync();
    assert_eq!(host.store.view().map(|v| v.turn_id.as_str()), Some("snap"));
    assert_eq!(host.store.status(), StoreStatus::Ready);
}

#[test]
fn an_empty_snapshot_reports_waiting_for_the_table() {
    let mut host = Host::new(view(1, "t"));
    host.snapshot = None;
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(1, "t")));
    host.resync();
    assert_eq!(host.store.error(), Some(MSG_WAITING_FOR_TABLE));
    assert_eq!(host.store.view().map(|v| v.v), Some(1), "last view kept");
}

#[test]
fn a_snapshot_times_out_after_4000ms_and_its_late_reply_is_ignored() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "t")));
    let fx = store.resync(100);
    let id = match fx.commands.as_slice() {
        [StoreCommand::FetchSnapshot { id }] => *id,
        other => panic!("expected one snapshot, got {other:?}"),
    };
    assert!(store.tick(4_099, true).is_empty(), "not yet");
    let fx = store.tick(4_100, true);
    assert!(
        fx.events
            .contains(&StoreEvent::ResyncSettled { id: Some(id) })
    );
    assert_eq!(store.error(), Some(MSG_WAITING_FOR_TABLE));
    assert!(
        store
            .on_snapshot(id, Some(view(7, "late")), 4_200)
            .is_empty()
    );
    assert_eq!(store.view().map(|v| v.v), Some(1));
    assert_eq!(
        store.resync(4_300).commands.len(),
        1,
        "a new resync can start"
    );
}

#[test]
fn a_snapshot_reply_at_its_deadline_counts_as_timed_out() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    let fx = store.resync(0);
    let StoreCommand::FetchSnapshot { id } = fx.commands[0].clone() else {
        panic!("snapshot");
    };
    store.on_snapshot(id, Some(view(3, "x")), 4_000);
    assert!(store.view().is_none());
    assert_eq!(store.error(), Some(MSG_WAITING_FOR_TABLE));
}

/* -------------------------------------------------------------------------- */
/* 6.2 play                                                                    */
/* -------------------------------------------------------------------------- */

#[test]
fn play_refuses_when_unbound_or_viewless() {
    let mut store = TableStore::new();
    assert!(store.play("play:9H", "c", 0).play_refused());
    store.bind(true, 0);
    assert!(store.play("play:9H", "c", 0).play_refused());
    let mut unbound = TableStore::new();
    unbound.apply_sync(&sync(view(1, "t")));
    let fx = unbound.play("play:9H", "c", 0);
    assert!(fx.play_refused());
    assert!(fx.commands.is_empty());
}

#[test]
fn play_refuses_a_second_move_while_submitting() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "t")));
    let first = store.play("play:9H", "c1", 0);
    assert_eq!(first.commands.len(), 1);
    assert!(store.submitting());
    let second = store.play("play:9H", "c2", 0);
    assert!(second.commands.is_empty());
    assert!(second.play_refused());
    assert!(store.submitting(), "the first play is still in flight");
}

#[test]
fn play_rejects_an_id_not_in_legal() {
    let mut host = fixture(SubmitOutcome::NoAck, view(2, "next"));
    assert_eq!(host.play("play:AS"), Some(false));
    assert_eq!(host.store.error(), Some(MSG_ILLEGAL_MOVE));
    assert!(host.submits.is_empty());
}

#[test]
fn play_submits_against_the_current_turn_and_clears_the_error() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "turn")));
    store.play("play:AS", "x", 0);
    assert!(store.error().is_some());
    let fx = store.play("play:9H", "uuid-1", 0);
    assert_eq!(store.error(), None);
    match fx.commands.as_slice() {
        [StoreCommand::SubmitMove { request, .. }] => {
            assert_eq!(
                request,
                &SubmitMoveRequest {
                    move_id: "play:9H".into(),
                    turn_id: Some("turn".into()),
                    client_move_id: "uuid-1".into(),
                }
            );
        }
        other => panic!("expected a submit, got {other:?}"),
    }
}

#[test]
fn an_ok_ack_already_seen_settles_without_a_resync() {
    let mut host = fixture(SubmitOutcome::Ack(MoveAck::Ok { v: 1 }), view(9, "x"));
    assert_eq!(host.play("play:9H"), Some(true));
    assert_eq!(host.snapshots, 0);
    assert!(!host.store.submitting());
}

#[test]
fn an_ok_ack_ahead_of_the_view_resyncs_then_succeeds() {
    let mut host = fixture(SubmitOutcome::Ack(MoveAck::Ok { v: 2 }), view(2, "next"));
    assert_eq!(host.play("play:9H"), Some(true));
    assert_eq!(host.snapshots, 1);
    assert_eq!(host.store.view().map(|v| v.v), Some(2));
}

#[test]
fn an_ok_ack_succeeds_even_when_its_resync_fails() {
    let mut host = fixture(SubmitOutcome::Ack(MoveAck::Ok { v: 2 }), view(2, "next"));
    host.snapshot = None;
    assert_eq!(host.play("play:9H"), Some(true));
    assert_eq!(host.store.error(), Some(MSG_WAITING_FOR_TABLE));
}

#[test]
fn a_rejection_after_the_turn_moved_on_counts_as_landed() {
    let mut host = fixture(stale_turn(), view(2, "next"));
    assert_eq!(host.play("play:9H"), Some(true));
    assert_eq!(host.store.error(), None);
}

#[test]
fn a_lost_ack_with_the_same_turn_offers_a_retry_even_if_the_resync_failed() {
    let mut host = fixture(SubmitOutcome::NoAck, view(1, "turn"));
    host.snapshot = None;
    assert_eq!(host.play("play:9H"), Some(false));
    assert_eq!(host.store.error(), Some(MSG_COULD_NOT_CONFIRM));
}

#[test]
fn a_thrown_submit_with_the_same_turn_reports_an_interruption() {
    let mut host = fixture(SubmitOutcome::Failed, view(1, "turn"));
    assert_eq!(host.play("play:9H"), Some(false));
    assert_eq!(host.store.error(), Some(MSG_CONNECTION_INTERRUPTED));
    assert!(!host.store.submitting());
}

#[test]
fn a_thrown_submit_after_the_turn_moved_on_counts_as_landed() {
    let mut host = fixture(SubmitOutcome::Failed, view(2, "next"));
    assert_eq!(host.play("play:9H"), Some(true));
}

#[test]
fn a_reply_for_another_request_is_ignored() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "t")));
    store.play("play:9H", "c", 0);
    assert!(
        store
            .on_submit(RequestId(999), SubmitOutcome::NoAck, 0)
            .is_empty()
    );
    assert!(store.submitting());
}

#[test]
fn a_play_waiting_on_resync_joins_the_in_flight_snapshot() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "turn")));
    let snap = store.resync(0);
    let StoreCommand::FetchSnapshot { id: snap_id } = snap.commands[0].clone() else {
        panic!("snapshot");
    };
    let fx = store.play("play:9H", "c", 0);
    let StoreCommand::SubmitMove { id, .. } = fx.commands[0].clone() else {
        panic!("submit");
    };
    let fx = store.on_submit(id, SubmitOutcome::NoAck, 10);
    assert!(fx.commands.is_empty(), "no second snapshot");
    let fx = store.on_snapshot(snap_id, Some(view(2, "next")), 20);
    assert_eq!(fx.play_settled(id), Some(true));
    assert!(
        fx.events
            .contains(&StoreEvent::ResyncSettled { id: Some(snap_id) })
    );
}

#[test]
fn a_play_resyncing_while_disconnected_settles_at_once() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "turn")));
    let fx = store.play("play:9H", "c", 0);
    let StoreCommand::SubmitMove { id, .. } = fx.commands[0].clone() else {
        panic!("submit");
    };
    store.set_connected(false, 0);
    let fx = store.on_submit(id, SubmitOutcome::NoAck, 0);
    assert!(fx.commands.is_empty());
    assert_eq!(fx.play_settled(id), Some(false));
    assert_eq!(store.error(), Some(MSG_COULD_NOT_CONFIRM));
}

/* -------------------------------------------------------------------------- */
/* 6.3 page wiring                                                             */
/* -------------------------------------------------------------------------- */

#[test]
fn attach_resyncs_only_when_connected() {
    let mut a = TableStore::new();
    assert_eq!(a.attach(true, 0).commands.len(), 1);
    let mut b = TableStore::new();
    assert!(b.attach(false, 0).is_empty());
}

#[test]
fn reconnecting_triggers_one_resync() {
    let mut store = TableStore::new();
    store.attach(false, 0);
    assert!(store.set_connected(false, 1).is_empty());
    assert_eq!(store.set_connected(true, 2).commands.len(), 1);
    assert!(
        store.set_connected(true, 3).is_empty(),
        "no edge, no resync"
    );
}

#[test]
fn the_poll_resyncs_every_10s_only_while_connected_and_visible() {
    let mut host = Host::new(view(1, "t"));
    host.store.bind(true, 0);
    assert!(host.store.tick(9_999, true).is_empty());
    let fx = host.store.tick(10_000, true);
    assert_eq!(fx.commands.len(), 1);
    host.now = 10_000;
    host.drive(fx);
    assert!(
        host.store.tick(20_000, false).commands.is_empty(),
        "hidden tab"
    );
    host.store.set_connected(false, 25_000);
    assert!(
        host.store.tick(30_000, true).commands.is_empty(),
        "disconnected"
    );
    let mut quiet = TableStore::new();
    quiet.bind(true, 0);
    assert_eq!(
        quiet.tick(35_000, true).commands.len(),
        1,
        "late tick fires once"
    );
    assert!(quiet.tick(39_999, true).commands.is_empty());
}

#[test]
fn banter_shows_as_table_talk_for_6500ms_on_its_hand_only() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "t")));
    store.on_chat_event(&chat("c1", ChatKind::Call), 0);
    assert!(
        store.visible_table_talk().is_none(),
        "calls are not bubbles"
    );
    store.on_chat_event(&chat("b1", ChatKind::Banter), 1_000);
    let talk = store.visible_table_talk().expect("bubble");
    assert_eq!((talk.msg_id.as_str(), talk.hand_no), ("b1", Some(0)));
    store.tick(7_499, true);
    assert!(store.visible_table_talk().is_some());
    store.on_chat_event(&chat("b1", ChatKind::Banter), 7_000);
    store.tick(7_500, true);
    assert!(
        store.visible_table_talk().is_some(),
        "a redelivery restarts the timer"
    );
    assert_eq!(store.chat().len(), 2, "but is not logged twice");
    store.tick(13_500, true);
    assert!(store.visible_table_talk().is_none());

    store.on_chat_event(&chat("b2", ChatKind::Banter), 20_000);
    let mut next_hand = view(2, "t2");
    next_hand.hand_no = 1;
    store.apply_sync(&sync(next_hand));
    assert!(
        store.visible_table_talk().is_none(),
        "hidden once the hand changes"
    );
}

#[test]
fn banter_before_any_view_is_never_shown() {
    let mut store = TableStore::new();
    store.on_chat_event(&chat("b", ChatKind::Banter), 0);
    store.apply_sync(&sync(view(1, "t")));
    assert!(store.visible_table_talk().is_none());
}

fn connected_at(v: u64, turn: &str) -> TableStore {
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(view(v, turn)));
    store
}

#[test]
fn submit_move_previews_until_the_live_view_moves_on() {
    let mut store = connected_at(1, "turn");
    let fx = store.submit_move("play:9H", "c", 0);
    assert_eq!(fx.commands.len(), 1);
    let shown = store.display_view().expect("display");
    let preview = euchre_engine::apply_optimistic(
        &view(1, "turn"),
        &Move::Play {
            card: Card::parse("9H").expect("card"),
        },
    );
    assert_eq!(shown, &preview);
    assert_eq!(store.optimistic().map(|o| o.for_v), Some(1));
    store.apply_sync(&sync(view(2, "next")));
    assert_eq!(
        store.display_view().map(|v| v.v),
        Some(2),
        "a newer sync supersedes it"
    );
}

#[test]
fn submit_move_records_the_decision() {
    let mut store = connected_at(1, "turn");
    store.submit_move("play:9H", "c", 0);
    let d = store.last_decision().expect("decision");
    assert_eq!(d.move_id, "play:9H");
    assert_eq!(d.view.turn_id, "turn");
}

#[test]
fn a_failed_submit_clears_the_preview() {
    let mut host = Host::new(view(1, "turn"));
    host.submit_replies.push_back(stale_turn());
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(1, "turn")));
    let fx = host.store.submit_move("play:9H", "c", 0);
    assert_eq!(host.settle(fx), Some(false));
    assert!(host.store.optimistic().is_none());
    assert_eq!(host.store.display_view().map(|v| v.legal.len()), Some(1));
}

#[test]
fn a_successful_submit_keeps_the_preview_until_superseded() {
    let mut host = Host::new(view(1, "turn"));
    host.submit_replies
        .push_back(SubmitOutcome::Ack(MoveAck::Ok { v: 1 }));
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(1, "turn")));
    let fx = host.store.submit_move("play:9H", "c", 0);
    assert_eq!(host.settle(fx), Some(true));
    assert!(host.store.optimistic().is_some());
}

#[test]
fn submit_move_guards_on_connection_submitting_and_displayed_legal() {
    let mut store = connected_at(1, "turn");
    store.set_connected(false, 0);
    assert!(store.submit_move("play:9H", "c", 0).is_empty());
    store.set_connected(true, 0);
    assert!(
        store.submit_move("play:AS", "c", 0).is_empty(),
        "not in legal"
    );
    assert!(store.last_decision().is_none());
    store.submit_move("play:9H", "c", 0);
    assert!(
        store.submit_move("play:9H", "d", 0).is_empty(),
        "refused while submitting"
    );
    assert!(TableStore::new().submit_move("play:9H", "c", 0).is_empty());
}

#[test]
fn controls_and_notice_follow_the_connection() {
    let mut store = connected_at(1, "turn");
    assert!(store.controls_enabled());
    assert_eq!(store.reconnect_notice(), None);
    store.play("play:9H", "c", 0);
    assert!(!store.controls_enabled(), "submitting");
    store.set_connected(false, 0);
    assert_eq!(store.reconnect_notice(), Some(MSG_RECONNECTING));
    assert!(
        store.display_view().is_some(),
        "the last good view stays on screen"
    );
}

#[test]
fn review_succeeds_only_on_a_queued_ack() {
    let mut host = Host::new(view(1, "t"));
    host.store.bind(true, 0);
    let fx = host
        .store
        .review_trick(ReviewTrickRequest::Set { enabled: true }, 0);
    let id = fx.review_id().expect("review sent");
    assert_eq!(host.drive(fx), [StoreEvent::ReviewSettled { id, ok: true }]);
    assert_eq!(host.reviews.len(), 1);
    host.review_reply = None;
    let fx = host.store.review_trick(
        ReviewTrickRequest::Continue {
            turn_id: "t".into(),
        },
        0,
    );
    let id = fx.review_id().expect("review sent");
    assert_eq!(
        host.drive(fx),
        [StoreEvent::ReviewSettled { id, ok: false }]
    );
    assert_eq!(host.store.review_error(), Some(MSG_REVIEW_FAILED));
    host.review_reply = Some(ReviewTrickAck::default());
    let fx = host
        .store
        .review_trick(ReviewTrickRequest::Set { enabled: false }, 0);
    host.drive(fx);
    assert_eq!(
        host.store.review_error(),
        None,
        "cleared when a new request starts"
    );
}

#[test]
fn review_is_guarded_by_connection_and_busy() {
    let mut host = Host::new(view(1, "t"));
    host.auto = false;
    host.store.bind(true, 0);
    let fx = host
        .store
        .review_trick(ReviewTrickRequest::Set { enabled: true }, 0);
    host.drive(fx);
    assert!(host.store.review_busy());
    let busy = host
        .store
        .review_trick(ReviewTrickRequest::Set { enabled: true }, 0);
    assert!(busy.review_refused());
    assert!(busy.commands.is_empty());
    assert!(
        host.store.review_busy(),
        "the first review is still in flight"
    );
    let mut offline = TableStore::new();
    offline.bind(false, 0);
    let fx = offline.review_trick(ReviewTrickRequest::Set { enabled: true }, 0);
    assert!(fx.review_refused());
    assert!(fx.commands.is_empty());
}

/* -------------------------------------------------------------------------- */
/* 6.5 reconnect                                                               */
/* -------------------------------------------------------------------------- */

#[test]
fn a_reconnect_converges_on_the_highest_v() {
    let mut host = Host::new(view(4, "snap"));
    host.auto = false;
    host.store.attach(false, 0);
    host.store.apply_sync(&sync(view(2, "old")));
    let fx = host.store.set_connected(true, 100);
    host.drive(fx);
    // The connect-time push lands before the snapshot reply.
    host.store.on_sync_event(sync(view(5, "push")));
    let parked = host.parked.pop_front().expect("snapshot");
    host.auto = true;
    let fx = host.answer(parked);
    host.drive(fx);
    assert_eq!(host.store.view().map(|v| v.v), Some(5));
    assert!(
        host.store.last_steps().is_empty(),
        "reconnect pushes carry no steps"
    );
}

/* -------------------------------------------------------------------------- */
/* Action timeouts (rivetkit actionDefaults.timeout = 12 000 ms)                */
/* -------------------------------------------------------------------------- */

#[test]
fn a_submit_that_never_answers_times_out_after_12s_as_a_lost_ack() {
    let mut host = Host::new(view(1, "turn"));
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(1, "turn")));
    let fx = host.store.submit_move("play:9H", "c", 500);
    let id = fx.submit_id().expect("submitted");
    assert_eq!(
        host.store.next_deadline_ms(),
        Some(10_000),
        "the poll comes first"
    );
    assert!(
        host.store.tick(10_000, false).is_empty(),
        "hidden: no resync"
    );
    assert_eq!(host.store.next_deadline_ms(), Some(500 + ACTION_TIMEOUT_MS));
    assert!(
        host.store
            .tick(500 + ACTION_TIMEOUT_MS - 1, false)
            .is_empty(),
        "not yet"
    );
    host.now = 500 + ACTION_TIMEOUT_MS;
    let fx = host.store.tick(host.now, false);
    assert!(
        matches!(fx.commands.as_slice(), [StoreCommand::FetchSnapshot { .. }]),
        "a timeout resyncs like NoAck: {fx:?}"
    );
    let events = host.drive(fx);
    assert_eq!(settled(&events, id), Some(false));
    assert_eq!(host.store.error(), Some(MSG_COULD_NOT_CONFIRM));
    assert!(!host.store.submitting());
    assert!(host.store.controls_enabled(), "controls come back");
    assert!(
        host.store.optimistic().is_none(),
        "the page preview is dropped"
    );
    assert!(
        host.store
            .on_submit(id, SubmitOutcome::Ack(MoveAck::Ok { v: 2 }), host.now + 1)
            .is_empty(),
        "the late reply is ignored"
    );
    assert_eq!(host.store.error(), Some(MSG_COULD_NOT_CONFIRM));
}

#[test]
fn a_timed_out_submit_whose_move_landed_counts_as_landed() {
    let mut host = Host::new(view(2, "next"));
    host.store.bind(true, 0);
    host.store.apply_sync(&sync(view(1, "turn")));
    let fx = host.store.play("play:9H", "c", 0);
    let id = fx.submit_id().expect("submitted");
    host.now = ACTION_TIMEOUT_MS;
    let fx = host.store.tick(host.now, false);
    assert_eq!(settled(&host.drive(fx), id), Some(true));
    assert_eq!(host.store.error(), None);
}

#[test]
fn a_review_that_never_answers_times_out_after_12s() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    let fx = store.review_trick(ReviewTrickRequest::Set { enabled: true }, 1_000);
    let id = fx.review_id().expect("sent");
    assert!(store.review_busy());
    assert!(store.tick(10_000, false).is_empty(), "the poll, hidden");
    assert_eq!(store.next_deadline_ms(), Some(1_000 + ACTION_TIMEOUT_MS));
    assert!(store.tick(1_000 + ACTION_TIMEOUT_MS - 1, true).is_empty());
    let fx = store.tick(1_000 + ACTION_TIMEOUT_MS, true);
    assert_eq!(fx.review_settled(id), Some(false));
    assert!(!store.review_busy(), "the review toggle comes back");
    assert_eq!(store.review_error(), Some(MSG_REVIEW_FAILED));
    assert!(
        store
            .on_review(id, Some(ReviewTrickAck::default()))
            .is_empty(),
        "the late reply is ignored"
    );
    assert_eq!(store.review_error(), Some(MSG_REVIEW_FAILED));
}

/* -------------------------------------------------------------------------- */
/* Settlement correlation                                                      */
/* -------------------------------------------------------------------------- */

#[test]
fn a_refused_play_never_reads_as_the_in_flight_play_settling() {
    let mut store = connected_at(1, "turn");
    let mut merged = store.play("play:9H", "c1", 0);
    let id = merged.submit_id().expect("submitted");
    merged.extend(store.play("play:9H", "c2", 0));
    assert!(merged.play_refused());
    assert_eq!(
        merged.play_settled(id),
        None,
        "the first play is not settled"
    );
    assert!(store.submitting());
    let fx = store.on_submit(id, SubmitOutcome::Ack(MoveAck::Ok { v: 1 }), 5);
    assert_eq!(fx.play_settled(id), Some(true));
    assert!(!fx.play_refused());
}

#[test]
fn a_refused_review_never_reads_as_the_in_flight_review_settling() {
    let mut store = TableStore::new();
    store.bind(true, 0);
    let mut merged = store.review_trick(ReviewTrickRequest::Set { enabled: true }, 0);
    let id = merged.review_id().expect("sent");
    merged.extend(store.review_trick(ReviewTrickRequest::Set { enabled: false }, 0));
    assert!(merged.review_refused());
    assert_eq!(merged.review_settled(id), None);
    let fx = store.on_review(id, Some(ReviewTrickAck::default()));
    assert_eq!(fx.review_settled(id), Some(true));
}

/* -------------------------------------------------------------------------- */
/* Time contract                                                               */
/* -------------------------------------------------------------------------- */

#[test]
fn next_deadline_is_the_earliest_pending_timer() {
    let mut store = TableStore::new();
    assert_eq!(store.next_deadline_ms(), None, "nothing armed before bind");
    store.bind(true, 0);
    store.apply_sync(&sync(view(1, "turn")));
    assert_eq!(store.next_deadline_ms(), Some(10_000), "the poll");
    store.on_chat_event(&chat("b", ChatKind::Banter), 1_000);
    assert_eq!(store.next_deadline_ms(), Some(7_500), "table talk");
    store.resync(3_000);
    assert_eq!(store.next_deadline_ms(), Some(7_000), "the snapshot");
    store.tick(7_000, true);
    store.play("play:9H", "c", 0);
    assert_eq!(store.next_deadline_ms(), Some(7_500), "table talk again");
    store.tick(7_500, true);
    assert_eq!(store.next_deadline_ms(), Some(10_000));
    store.tick(10_000, false);
    assert_eq!(
        store.next_deadline_ms(),
        Some(ACTION_TIMEOUT_MS),
        "the submit"
    );
}

/* -------------------------------------------------------------------------- */
/* Loading screen                                                              */
/* -------------------------------------------------------------------------- */

#[test]
fn before_the_first_view_the_loading_screen_shows_and_not_the_reconnect_notice() {
    let mut store = TableStore::new();
    store.attach(false, 0);
    assert_eq!(store.reconnect_notice(), None);
    let notice = store.loading_notice().expect("loading");
    assert_eq!(notice.heading, MSG_LOADING_DISCONNECTED_HEADING);
    assert_eq!(notice.body, MSG_LOADING_DISCONNECTED_BODY);
    assert_eq!(notice.heading, "A seat at the table.");
    assert_eq!(notice.body, "Connecting to your game…");
    store.set_connected(true, 1);
    let notice = store.loading_notice().expect("loading");
    assert_eq!(notice.heading, MSG_LOADING_CONNECTED_HEADING);
    assert_eq!(notice.body, MSG_LOADING_CONNECTED_BODY);
    assert_eq!(notice.heading, "Your hand is on its way.");
    assert_eq!(notice.body, "Dealing your cards…");
    store.apply_sync(&sync(view(1, "t")));
    assert_eq!(store.loading_notice(), None);
    store.set_connected(false, 2);
    assert_eq!(store.reconnect_notice(), Some(MSG_RECONNECTING));
}

#[test]
fn the_loading_error_line_follows_the_page() {
    assert_eq!(loading_error_line(false, None), None);
    assert_eq!(loading_error_line(true, None), Some(MSG_TABLE_OPEN_FAILED));
    assert_eq!(
        MSG_TABLE_OPEN_FAILED,
        "We couldn’t open this table. Try reconnecting, or open Your saved games."
    );
    assert_eq!(loading_error_line(false, Some("boom")), Some("boom"));
    assert_eq!(loading_error_line(true, Some("boom")), Some("boom"));
    assert_eq!(loading_error_line(false, Some("")), None, "'' is falsy");
    assert_eq!(loading_error_line(true, Some("")), Some(""), "?? keeps ''");
}

/* -------------------------------------------------------------------------- */
/* Hand review (HandReview.svelte)                                             */
/* -------------------------------------------------------------------------- */

fn archive(key: &str) -> CompletedHandReview {
    serde_json::from_value(common::entry("review", key)).expect("review fixture")
}

#[test]
fn the_review_opens_on_the_previous_hand_until_the_current_one_has_play() {
    let mut v = view(1, "t");
    assert_eq!(initial_review_hand(&v), 0, "hand 0, nothing played");
    v.hand_no = 3;
    assert_eq!(initial_review_hand(&v), 2);
    v.trick_log = vec![Trick::empty(0)];
    assert_eq!(initial_review_hand(&v), 3, "a trick exists");
    v.trick_log.clear();
    v.result = Some(HandResult::ThrowIn);
    assert_eq!(initial_review_hand(&v), 3, "a result exists");

    let mut host = Host::new(view(1, "t"));
    assert!(
        host.store.open_hand_review(0).is_empty(),
        "no view, no panel"
    );
    host.store.bind(true, 0);
    let mut current = view(1, "t");
    current.hand_no = 2;
    host.store.apply_sync(&sync(current));
    host.hand_review_reply = HandReviewOutcome::Loaded(Some(archive("scored")));
    let fx = host.store.open_hand_review(0);
    host.drive(fx);
    assert_eq!(host.hand_reviews, [1]);
    let panel = host.store.hand_review();
    assert_eq!(panel.hand_no, 1);
    assert!(!panel.busy);
    assert_eq!(panel.error, None);
    assert_eq!(panel.archive.as_ref().map(|a| a.hand_no), Some(1));
    assert_eq!(host.store.hand_review_totals(), [4, 1]);
    assert_eq!(host.store.hand_review_tricks().len(), 5);
}

#[test]
fn the_latest_hand_review_request_wins() {
    let mut store = connected_at(1, "t");
    let first = store.choose_hand_review(0, 0);
    let StoreCommand::GetHandReview { id: a, hand_no: 0 } = first.commands[0] else {
        panic!("load hand 0");
    };
    assert!(store.hand_review().busy);
    let second = store.choose_hand_review(0, 10);
    let StoreCommand::GetHandReview { id: b, .. } = second.commands[0] else {
        panic!("load again");
    };
    store.on_hand_review(a, HandReviewOutcome::Loaded(Some(archive("throwIn"))));
    assert!(
        store.hand_review().archive.is_none(),
        "an earlier reply is dropped"
    );
    assert!(store.hand_review().busy, "and does not clear busy");
    store.on_hand_review(b, HandReviewOutcome::Failed);
    assert_eq!(store.hand_review().error, Some(MSG_HAND_LOAD_FAILED));
    assert!(!store.hand_review().busy);
    store.on_hand_review(b, HandReviewOutcome::Loaded(Some(archive("throwIn"))));
    assert!(store.hand_review().archive.is_none(), "settled once");
}

#[test]
fn a_missing_archive_reads_as_unsaved_only_for_an_older_hand() {
    let mut current = view(1, "t");
    current.hand_no = 2;
    current.trick_log = vec![Trick {
        index: 0,
        led_suit: None,
        plays: vec![],
        winner_seat: Some(Seat::new(1).expect("seat")),
    }];
    let mut store = TableStore::new();
    store.bind(true, 0);
    store.apply_sync(&sync(current));

    let fx = store.choose_hand_review(1, 0);
    let StoreCommand::GetHandReview { id, .. } = fx.commands[0] else {
        panic!("load");
    };
    store.on_hand_review(id, HandReviewOutcome::Loaded(None));
    assert_eq!(store.hand_review().error, Some(MSG_OLDER_HAND_NOT_SAVED));
    assert_eq!(
        MSG_OLDER_HAND_NOT_SAVED,
        "This older hand was not saved by the previous version."
    );
    assert!(store.hand_review_tricks().is_empty());

    let fx = store.choose_hand_review(2, 0);
    assert_eq!(store.hand_review().error, None, "a new choice clears it");
    let StoreCommand::GetHandReview { id, .. } = fx.commands[0] else {
        panic!("load");
    };
    store.on_hand_review(id, HandReviewOutcome::Loaded(None));
    assert_eq!(
        store.hand_review().error,
        None,
        "the current hand falls back"
    );
    assert_eq!(store.hand_review_tricks().len(), 1, "to the live trickLog");
    assert_eq!(store.hand_review_totals(), [0, 1]);
}

#[test]
fn a_hand_review_that_never_answers_times_out_as_no_archive() {
    let mut store = connected_at(1, "t");
    store.choose_hand_review(0, 100);
    store.tick(10_000, false);
    assert_eq!(store.next_deadline_ms(), Some(100 + ACTION_TIMEOUT_MS));
    store.tick(100 + ACTION_TIMEOUT_MS - 1, false);
    assert!(store.hand_review().busy);
    store.tick(100 + ACTION_TIMEOUT_MS, false);
    let panel = store.hand_review();
    assert!(!panel.busy);
    assert_eq!(panel.error, None, "hand 0 is the current hand");
    assert!(panel.archive.is_none());
}

/* -------------------------------------------------------------------------- */
/* Engine cross-check                                                          */
/* -------------------------------------------------------------------------- */

#[test]
fn the_store_fixture_matches_the_rust_engine_projection() {
    let game = euchre_engine::create_game(euchre_engine::GameInit {
        game_id: "store-test".into(),
        seed: "store-test".into(),
        cfg: None,
        internal_token: None,
        first_dealer: None,
    });
    let rust = euchre_engine::project(&game, Seat::new(0).expect("seat 0"));
    assert_eq!(rust, base_view());
}
