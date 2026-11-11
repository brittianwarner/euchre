//! Unit tests of the table core against an in-memory host.
//!
//! Ports `review.test.ts`, `decision-delivery.test.ts` (all seven cases) and
//! the table-shaped half of `state-proxy.test.ts`, plus the Rust-only ledger,
//! early-fire and `submitMove` check-order rules, profile recording (03 §11,
//! §13), the forged-publish test (03 §14) and the `sync` / `MoveAck` JSON
//! goldens (03 §16.8). `run` mirrors the TS test
//! harness: `reconcile`, then each message in order, each in its own lane turn.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Result, bail};
use async_trait::async_trait;
use euchre_engine::{
    AiDecisionSource, GamePhase, HUMAN_SEAT, Move, Seat, legal_moves, try_advance, try_apply,
};
use euchre_protocol::table_actions::{ON_AI_TIMEOUT, ON_REAP, ON_TEMPO_GATE, RELEASE_AI_MOVE};
use euchre_protocol::{
    AIDecision, AiSayMsg, CompletedHandReview, MoveAck, ReviewAction, ReviewMsg, TableCreateInput,
    TickKind, TickMsg, default_personas,
};
use euchre_server_core::rivet::parse_rivet_error;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::seams::{
    AiDispatch, DecideCall, LifecycleCall, ProfileSink, RecordGameCall, RecordHandsCall,
};
use crate::state::TableState;
use crate::tempo::Tempo;

const NOW: u64 = 100_000;
/// The TS stub returned the same id for every schedule; so does this host.
const TIMER_ID: &str = "timer";

#[derive(Default)]
struct RecordingAi {
    asks: bool,
    decides: Mutex<Vec<DecideCall>>,
    notifies: Mutex<Vec<LifecycleCall>>,
    /// Every call in order: `decide:<seat>:<handNo>` or `<kind>:<seat>:<handNo>`.
    log: Mutex<Vec<String>>,
}

impl AiDispatch for RecordingAi {
    fn asks(&self) -> bool {
        self.asks
    }
    fn decide(&self, call: DecideCall) {
        self.log.lock().push(format!(
            "decide:{}:{}",
            call.seat.get(),
            call.request.view.hand_no
        ));
        self.decides.lock().push(call);
    }
    fn notify(&self, call: LifecycleCall) {
        self.log.lock().push(format!(
            "{}:{}:{}",
            call.message.kind.as_str(),
            call.seat.get(),
            call.message
                .hand_no
                .map_or("-".to_owned(), |n| n.to_string())
        ));
        self.notifies.lock().push(call);
    }
}

#[derive(Default)]
struct RecordingProfile {
    available: AtomicBool,
    hands: Mutex<Vec<RecordHandsCall>>,
    games: Mutex<Vec<RecordGameCall>>,
}

impl ProfileSink for RecordingProfile {
    fn available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }
    fn record_hands(&self, call: RecordHandsCall) {
        self.hands.lock().push(call);
    }
    fn record_game(&self, call: RecordGameCall) {
        self.games.lock().push(call);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Scheduled {
    delay_ms: u64,
    action: &'static str,
    args: Vec<Value>,
}

struct TestHost {
    now: AtomicU64,
    scheduled: Mutex<Vec<Scheduled>>,
    live: Mutex<TableState>,
    reviews: Mutex<BTreeMap<u32, CompletedHandReview>>,
    fail_persist: AtomicBool,
    /// Persist calls made so far, and the first call index that fails.
    persists: AtomicU64,
    fail_from: AtomicU64,
    cancels: Mutex<Vec<String>>,
    events: Mutex<Vec<(String, Value)>>,
    syncs: Mutex<Vec<Value>>,
    connected: AtomicBool,
    destroyed: AtomicBool,
    ai: RecordingAi,
    profile: RecordingProfile,
}

impl TestHost {
    fn new(st: TableState, asks: bool) -> Self {
        TestHost {
            now: AtomicU64::new(NOW),
            scheduled: Mutex::new(Vec::new()),
            live: Mutex::new(st),
            reviews: Mutex::new(BTreeMap::new()),
            fail_persist: AtomicBool::new(false),
            persists: AtomicU64::new(0),
            fail_from: AtomicU64::new(u64::MAX),
            cancels: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            syncs: Mutex::new(Vec::new()),
            connected: AtomicBool::new(true),
            destroyed: AtomicBool::new(false),
            ai: RecordingAi {
                asks,
                ..RecordingAi::default()
            },
            profile: RecordingProfile::default(),
        }
    }

    fn st(&self) -> TableState {
        self.live.lock().clone()
    }

    fn set_time(&self, ms: u64) {
        self.now.store(ms, Ordering::SeqCst);
    }

    fn edit(&self, f: impl FnOnce(&mut TableState)) {
        f(&mut self.live.lock());
    }

    fn session(&self, tempo: Tempo) -> Session<'_, TestHost> {
        Session::new(self, self.st(), tempo)
    }

    fn event_count(&self, name: &str) -> usize {
        self.events.lock().iter().filter(|(n, _)| n == name).count()
    }
}

#[async_trait]
impl Host for TestHost {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
    async fn schedule(
        &self,
        delay_ms: u64,
        action: &'static str,
        args: Vec<Value>,
    ) -> Result<String> {
        self.scheduled.lock().push(Scheduled {
            delay_ms,
            action,
            args,
        });
        Ok(TIMER_ID.to_owned())
    }
    async fn cancel(&self, id: &str) {
        self.cancels.lock().push(id.to_owned());
    }
    async fn persist(&self, state: &TableState, reviews: &[CompletedHandReview]) -> Result<()> {
        let n = self.persists.fetch_add(1, Ordering::SeqCst);
        if self.fail_persist.load(Ordering::SeqCst) || n >= self.fail_from.load(Ordering::SeqCst) {
            bail!("sqlite is down");
        }
        let mut stored = self.reviews.lock();
        for r in reviews {
            stored.insert(r.hand_no, r.clone());
        }
        *self.live.lock() = state.clone();
        Ok(())
    }
    fn broadcast(&self, name: &str, payload: Value) {
        self.events.lock().push((name.to_owned(), payload));
    }
    fn send_sync(&self, build: &(dyn Fn(Seat) -> Option<Value> + Sync)) {
        if self.connected.load(Ordering::SeqCst)
            && let Some(v) = build(HUMAN_SEAT)
        {
            self.syncs.lock().push(v);
        }
    }
    fn player_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
    fn destroy(&self) {
        self.destroyed.store(true, Ordering::SeqCst);
    }
    fn ai(&self) -> &dyn AiDispatch {
        &self.ai
    }
    fn profile(&self) -> &dyn ProfileSink {
        &self.profile
    }
}

#[derive(Clone, Debug)]
enum Msg {
    Review(ReviewMsg),
    Tick(TickMsg),
    Ai(AIDecision),
    Say(AiSayMsg),
}

/// The TS harness `run(...messages)`: reconcile, then each message in its own
/// lane turn. A `RuleError` from an internal path is absorbed, as the run loop did.
async fn run(h: &TestHost, tempo: Tempo, msgs: Vec<Msg>) {
    let mut s = h.session(tempo);
    if let Err(e) = s.reconcile().await {
        panic!("reconcile: {e:#}");
    }
    if let Err(e) = s.flush().await {
        panic!("flush: {e:#}");
    }
    for msg in msgs {
        let mut s = h.session(tempo);
        let out = match &msg {
            Msg::Review(b) => s.on_review(b).await,
            Msg::Tick(b) => s.on_tick(b).await,
            Msg::Ai(b) => s.on_ai_decision(b).await,
            Msg::Say(b) => {
                s.on_ai_say(b, false);
                Ok(())
            }
        };
        match out {
            Ok(()) => {}
            Err(e) if rule_error(&e).is_some() => {}
            Err(e) => panic!("{msg:?}: {e:#}"),
        }
        if let Err(e) = s.flush().await {
            panic!("flush: {e:#}");
        }
    }
}

/// Drive a fresh table with first-legal moves until `done`.
fn drive(game_id: &str, personas: bool, done: impl Fn(&TableState) -> bool) -> TableState {
    let input = TableCreateInput {
        seed: Some(game_id.to_owned()),
        personas: personas.then(default_personas),
        ..TableCreateInput::default()
    };
    drive_input(game_id, &input, done)
}

/// [`drive`] from an explicit create input (its seed, as the actor passes it).
fn drive_input(
    game_id: &str,
    input: &TableCreateInput,
    done: impl Fn(&TableState) -> bool,
) -> TableState {
    let seed = input.seed.clone().unwrap_or_else(|| game_id.to_owned());
    let mut st = TableState::create(
        game_id.to_owned(),
        seed,
        "match-token-0123".to_owned(),
        input,
        NOW,
    )
    .unwrap_or_else(|e| panic!("create: {e}"));
    for _ in 0..5_000 {
        if done(&st) {
            return st;
        }
        let game = &st.game;
        let next = match game.hand.turn_seat {
            Some(seat) => match legal_moves(game, seat).first() {
                Some(m) => try_apply(game, seat, m.mv, None)
                    .map(|r| r.state)
                    .unwrap_or_else(|e| panic!("apply: {e}")),
                None => try_advance(game)
                    .map(|r| r.state)
                    .unwrap_or_else(|e| panic!("advance: {e}")),
            },
            None => try_advance(game)
                .map(|r| r.state)
                .unwrap_or_else(|e| panic!("advance: {e}")),
        };
        st.game = next;
    }
    panic!("fixture never reached its target");
}

// ------------------------------------------------------------ review.test.ts

struct ReviewFixture {
    h: TestHost,
    turn_id: String,
}

impl ReviewFixture {
    fn new(trick_index: u8) -> Self {
        let st = drive("review-test", false, |st| {
            st.game.hand.phase == GamePhase::TrickResolve && st.game.hand.trick.index == trick_index
        });
        let turn_id = st.game.turn_id.clone();
        ReviewFixture {
            h: TestHost::new(st, false),
            turn_id,
        }
    }

    fn review(&self, action: ReviewAction, enabled: Option<bool>) -> Msg {
        let st = self.h.st();
        Msg::Review(ReviewMsg {
            internal_token: st.game.internal_token.clone(),
            game_id: st.game.game_id.clone(),
            action,
            enabled,
            turn_id: self.turn_id.clone(),
        })
    }

    fn set(&self, enabled: bool) -> Msg {
        self.review(ReviewAction::Set, Some(enabled))
    }

    fn cont(&self) -> Msg {
        self.review(ReviewAction::Continue, None)
    }

    fn tick(&self, kind: TickKind, turn_id: &str) -> Msg {
        let st = self.h.st();
        Msg::Tick(TickMsg {
            internal_token: st.game.internal_token.clone(),
            game_id: st.game.game_id.clone(),
            kind,
            turn_id: turn_id.to_owned(),
            seat: None,
        })
    }

    fn tempo(&self) -> Msg {
        self.tick(TickKind::Tempo, &self.turn_id)
    }

    async fn run(&self, msgs: Vec<Msg>) {
        run(&self.h, Tempo::NORMAL, msgs).await;
    }
}

#[tokio::test]
async fn review_pauses_after_the_read_gate_and_stays_paused_across_wake() {
    let f = ReviewFixture::new(0);
    f.run(vec![f.set(true), f.tempo()]).await;
    let st = f.h.st();
    assert_eq!(st.game.hand.phase, GamePhase::TrickResolve);
    assert_eq!(st.review_ready_turn_id, Some(Some(f.turn_id.clone())));
    f.h.scheduled.lock().clear();
    f.run(vec![]).await;
    assert!(f.h.scheduled.lock().is_empty(), "a wake re-armed nothing");
    assert_eq!(
        table_view(&f.h.st(), HUMAN_SEAT).awaiting_trick_review,
        Some(true)
    );
}

#[tokio::test]
async fn review_ignores_an_early_continue() {
    let f = ReviewFixture::new(0);
    f.run(vec![f.set(true), f.cont()]).await;
    let st = f.h.st();
    assert_eq!(st.game.turn_id, f.turn_id);
    assert_eq!(st.review_ready_turn_id, None);
}

#[tokio::test]
async fn review_continues_once_and_ignores_a_duplicate_continue() {
    let f = ReviewFixture::new(0);
    f.run(vec![f.set(true), f.tempo(), f.cont(), f.cont()])
        .await;
    let st = f.h.st();
    assert_eq!(st.game.hand.trick.index, 1);
    assert_eq!(st.game.hand.phase, GamePhase::TrickPlay);
    assert_eq!(st.review_ready_turn_id, Some(None));
}

#[tokio::test]
async fn review_switched_off_releases_a_paused_trick() {
    let f = ReviewFixture::new(0);
    f.run(vec![f.set(true), f.tempo(), f.set(false)]).await;
    let st = f.h.st();
    assert_eq!(st.review_tricks, Some(false));
    assert_eq!(st.game.hand.trick.index, 1);
}

#[tokio::test]
async fn review_absent_preference_advances_automatically() {
    let f = ReviewFixture::new(0);
    f.run(vec![f.tempo()]).await;
    let st = f.h.st();
    assert_eq!(st.game.hand.phase, GamePhase::TrickPlay);
    assert_eq!(st.game.hand.trick.index, 1);
}

#[tokio::test]
async fn review_holds_the_fifth_trick_then_scores_the_hand_once() {
    let f = ReviewFixture::new(4);
    f.run(vec![f.set(true), f.tempo()]).await;
    assert_eq!(f.h.st().game.hand.phase, GamePhase::TrickResolve);
    f.run(vec![f.cont(), f.cont()]).await;
    let st = f.h.st();
    assert_eq!(st.game.hand.phase, GamePhase::HandScore);
    assert_eq!(st.hands_played, 1);
    let reviews = f.h.reviews.lock();
    let review = reviews
        .get(&st.game.hand.hand_no)
        .unwrap_or_else(|| panic!("hand archived"));
    assert_eq!(review.tricks.len(), 5);
    assert_eq!(review.buried.len(), 4);
}

#[tokio::test]
async fn review_archives_the_fifth_trick_whatever_the_ai_lifecycle_does() {
    // Lifecycle delivery is detached here (the seam returns at once), so a
    // hanging seat cannot hold the hand: it finishes and archives.
    let f = ReviewFixture::new(4);
    f.run(vec![f.set(true), f.tempo()]).await;
    f.run(vec![f.cont()]).await;
    let st = f.h.st();
    assert_eq!(st.game.hand.phase, GamePhase::HandScore);
    assert_eq!(st.hands_played, 1);
    let reviews = f.h.reviews.lock();
    let review = reviews
        .get(&st.game.hand.hand_no)
        .unwrap_or_else(|| panic!("hand archived"));
    assert_eq!(review.tricks.len(), 5);
    assert_eq!(json!(review.tricks_won), json!(st.game.hand.tricks_won));
    assert_eq!(json!(review.delta), json!(st.game.hand.delta));
}

#[tokio::test]
async fn review_buried_cards_stay_unarchived_until_the_hand_ends() {
    let f = ReviewFixture::new(0);
    f.run(vec![]).await;
    let hand_no = f.h.st().game.hand.hand_no;
    assert!(f.h.reviews.lock().get(&hand_no).is_none());
    // The non-player refusal of getHandReview is `auth::tests`.
}

#[tokio::test]
async fn review_old_human_timeouts_neither_play_nor_abandon() {
    let f = ReviewFixture::new(0);
    f.run(vec![f.tempo()]).await;
    let before = f.h.st();
    let game_json = serde_json::to_string(&before.game).unwrap_or_default();
    f.run(vec![f.tick(TickKind::Abandon, &before.game.turn_id)])
        .await;
    let after = f.h.st();
    assert_eq!(
        serde_json::to_string(&after.game).unwrap_or_default(),
        game_json
    );
    assert_eq!(after.abandon_strikes, 0);
}

#[tokio::test]
async fn review_forged_messages_are_dropped() {
    let f = ReviewFixture::new(0);
    let Msg::Review(mut forged) = f.set(true) else {
        panic!("review message");
    };
    forged.internal_token = "forged".into();
    f.run(vec![Msg::Review(forged)]).await;
    assert_eq!(f.h.st().review_tricks, None);
}

#[tokio::test]
async fn review_trick_builds_the_review_body_and_applies_it() {
    let f = ReviewFixture::new(0);
    let st = f.h.st();
    let body = review_body(&st, &json!({ "action": "set", "enabled": true }))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(body.turn_id, st.game.turn_id);
    assert_eq!(body.enabled, Some(true));
    // Building the body changes nothing; applying it does.
    assert_eq!(f.h.st().review_tricks, None);
    f.run(vec![Msg::Review(body)]).await;
    assert_eq!(f.h.st().review_tricks, Some(true));

    let cont = review_body(&st, &json!({ "action": "continue", "turnId": "t-9" }))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (cont.action, cont.turn_id.as_str()),
        (ReviewAction::Continue, "t-9")
    );
    for bad in [
        json!(null),
        json!({ "action": "set" }),
        json!({ "action": "set", "enabled": "yes" }),
        json!({ "action": "continue" }),
        json!({ "action": "skip" }),
    ] {
        let e = review_body(&st, &bad)
            .err()
            .unwrap_or_else(|| panic!("{bad}"));
        let u = parse_rivet_error(&e).unwrap_or_else(|| panic!("user error"));
        assert_eq!(
            (u.code.as_str(), u.message.as_str()),
            ("internal_error", "Invalid review request")
        );
    }
}

// --------------------------------------------------- decision-delivery.test.ts

struct DeliveryFixture {
    h: TestHost,
    request: euchre_protocol::AIDecideRequest,
}

impl DeliveryFixture {
    async fn new() -> Self {
        let st = drive("delivery-test", true, |st| {
            let game = &st.game;
            match game.hand.turn_seat {
                Some(seat) => {
                    game.hand.phase == GamePhase::TrickPlay
                        && seat != HUMAN_SEAT
                        && legal_moves(game, seat).len() >= 2
                }
                None => false,
            }
        });
        let h = TestHost::new(st, true);
        run(&h, Tempo::NORMAL, vec![]).await;
        let request =
            h.ai.decides
                .lock()
                .first()
                .map(|c| c.request.clone())
                .unwrap_or_else(|| panic!("no AI request"));
        DeliveryFixture { h, request }
    }

    fn reply(&self, move_id: &str) -> AIDecision {
        AIDecision {
            internal_token: self.request.internal_token.clone(),
            game_id: self.request.game_id.clone(),
            seat: self.request.seat,
            turn_id: self.request.turn_id.clone(),
            move_id: move_id.to_owned(),
            source: AiDecisionSource::Llm,
            banter: None,
            rationale: None,
            latency_ms: 0,
        }
    }

    fn tick(&self, kind: TickKind) -> Msg {
        Msg::Tick(TickMsg {
            internal_token: self.request.internal_token.clone(),
            game_id: self.request.game_id.clone(),
            kind,
            turn_id: self.request.turn_id.clone(),
            seat: Some(self.request.seat),
        })
    }

    fn pending(&self) -> euchre_engine::PendingDecision {
        self.h
            .st()
            .game
            .pending
            .unwrap_or_else(|| panic!("a parked decision"))
    }

    fn played(&self, mv: &Move) -> bool {
        self.h
            .st()
            .game
            .hand
            .trick
            .plays
            .iter()
            .any(|p| p.seat == self.request.seat && Move::Play { card: p.card } == *mv)
    }

    async fn run(&self, msgs: Vec<Msg>) {
        run(&self.h, Tempo::NORMAL, msgs).await;
    }
}

#[tokio::test]
async fn delivery_replaces_the_parked_fallback_with_the_model_choice() {
    let f = DeliveryFixture::new().await;
    let pending = f.pending();
    let chosen = f
        .request
        .legal
        .iter()
        .find(|m| Some(&m.id) != pending.decision.as_ref())
        .cloned()
        .unwrap_or_else(|| panic!("a second legal move"));
    let count = f.h.st().game.hand.trick.plays.len();
    f.h.set_time(pending.reveal_at + 1);
    f.run(vec![Msg::Ai(f.reply(&chosen.id))]).await;
    assert!(f.played(&chosen.mv));
    assert_eq!(f.h.st().game.hand.trick.plays.len(), count + 1);
    let st = f.h.st();
    let last = st.journal.last().unwrap_or_else(|| panic!("journaled"));
    assert_eq!(last.source, Some(AiDecisionSource::Llm));
}

#[tokio::test]
async fn delivery_keeps_waiting_at_the_floor_while_the_model_is_outstanding() {
    let f = DeliveryFixture::new().await;
    f.h.set_time(f.pending().reveal_at + 1);
    let before = f.h.st().game.hand.trick.plays.clone();
    f.run(vec![f.tick(TickKind::ReleaseAi)]).await;
    let st = f.h.st();
    assert_eq!(st.game.turn_id, f.request.turn_id);
    assert_eq!(st.game.hand.trick.plays, before);
    assert_eq!(f.pending().answered, Some(false));
}

#[tokio::test]
async fn delivery_holds_an_early_answer_behind_the_floor_and_ignores_a_duplicate() {
    let f = DeliveryFixture::new().await;
    let floor = f.pending().reveal_at;
    let first = f.request.legal[0].id.clone();
    let second = f.request.legal[1].id.clone();
    f.run(vec![Msg::Ai(f.reply(&first)), Msg::Ai(f.reply(&second))])
        .await;
    let p = f.pending();
    assert_eq!(
        (p.decision.as_deref(), p.answered, p.source),
        (
            Some(first.as_str()),
            Some(true),
            Some(AiDecisionSource::Llm)
        )
    );
    f.h.set_time(floor + 1);
    f.run(vec![f.tick(TickKind::ReleaseAi)]).await;
    assert_ne!(f.h.st().game.turn_id, f.request.turn_id);
}

#[tokio::test]
async fn delivery_keeps_a_pre_upgrade_fallback_pending_after_wake() {
    let f = DeliveryFixture::new().await;
    f.h.edit(|st| {
        if let Some(p) = st.game.pending.as_mut() {
            p.answered = None;
        }
    });
    let pending = f.pending();
    let chosen = f
        .request
        .legal
        .iter()
        .find(|m| Some(&m.id) != pending.decision.as_ref())
        .cloned()
        .unwrap_or_else(|| panic!("a second legal move"));
    f.h.set_time(pending.reveal_at + 1);
    f.run(vec![]).await;
    assert_eq!(f.h.st().game.turn_id, f.request.turn_id);
    f.run(vec![Msg::Ai(f.reply(&chosen.id))]).await;
    assert!(f.played(&chosen.mv));
}

#[tokio::test]
async fn delivery_releases_a_pre_upgrade_forced_move_at_its_floor() {
    let f = DeliveryFixture::new().await;
    f.h.edit(|st| {
        if let Some(p) = st.game.pending.as_mut() {
            p.answered = None;
            p.source = Some(AiDecisionSource::Forced);
        }
    });
    f.h.set_time(f.pending().reveal_at + 1);
    f.run(vec![]).await;
    assert_ne!(f.h.st().game.turn_id, f.request.turn_id);
}

#[tokio::test]
async fn delivery_releases_exactly_one_fallback_on_timeout() {
    let f = DeliveryFixture::new().await;
    let pending = f.pending();
    let fallback = f
        .request
        .legal
        .iter()
        .find(|m| Some(&m.id) == pending.decision.as_ref())
        .cloned()
        .unwrap_or_else(|| panic!("the parked move is legal"));
    let last = f
        .request
        .legal
        .last()
        .map(|m| m.id.clone())
        .unwrap_or_default();
    let mut forged = f.reply(&last);
    forged.internal_token = "forged".into();
    f.run(vec![Msg::Ai(forged)]).await;
    assert_eq!(f.pending().answered, Some(false));
    f.h.set_time(f.request.deadline_at + 1);
    f.run(vec![f.tick(TickKind::AiTimeout), Msg::Ai(f.reply(&last))])
        .await;
    assert!(f.played(&fallback.mv));
    let st = f.h.st();
    let by_seat = st
        .game
        .hand
        .trick
        .plays
        .iter()
        .filter(|p| p.seat == f.request.seat)
        .count();
    assert_eq!(by_seat, 1);
}

#[tokio::test]
async fn delivery_allows_chatter_for_this_hand_only() {
    let f = DeliveryFixture::new().await;
    let hand_no = f.h.st().game.hand.hand_no;
    let say = |hand_no: u32| {
        Msg::Say(AiSayMsg {
            internal_token: f.request.internal_token.clone(),
            game_id: f.request.game_id.clone(),
            seat: f.request.seat,
            turn_id: "previous-turn".into(),
            hand_no: Some(hand_no),
            msg_id: "talk".into(),
            delta: None,
            text: Some("Now that was a hand.".into()),
            is_final: true,
        })
    };
    let before = f.h.event_count("chat");
    f.run(vec![say(hand_no), say(hand_no + 1)]).await;
    assert_eq!(f.h.event_count("chat") - before, 1);
}

#[tokio::test]
async fn dispatch_asks_the_seat_with_ts_create_input_and_persists_first() {
    let f = DeliveryFixture::new().await;
    let calls = f.h.ai.decides.lock();
    let call = calls.first().unwrap_or_else(|| panic!("asked"));
    assert_eq!(call.seat, f.request.seat);
    assert_eq!(
        call.key,
        [
            "table".to_owned(),
            "delivery-test".to_owned(),
            "seat".to_owned(),
            call.seat.get().to_string()
        ]
    );
    let keys: Vec<&str> = call
        .create_input
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    // No owner yet, so no profileKey.
    assert_eq!(
        keys,
        [
            "gameId",
            "seat",
            "internalToken",
            "fenceNonce",
            "persona",
            "dossier",
            "matchId"
        ]
    );
    // The parked heuristic, the reveal and the watchdog were all persisted.
    let st = f.h.st();
    let p = st.game.pending.clone().unwrap_or_else(|| panic!("parked"));
    assert_eq!(p.source, Some(AiDecisionSource::Fallback));
    assert_eq!(p.answered, Some(false));
    assert_eq!(
        st.game.turn_deadline_at,
        Some(NOW + Tempo::NORMAL.ai_hard_cap_ms)
    );
    assert_eq!(f.request.deadline_at, NOW + Tempo::NORMAL.ai_hard_cap_ms);
    assert_eq!(f.h.event_count("thinking"), 1);
    let sched = f.h.scheduled.lock();
    assert!(sched.iter().any(|s| s.action == ON_AI_TIMEOUT
        && s.delay_ms == Tempo::NORMAL.ai_hard_cap_ms
        && s.args == vec![json!(p.turn_id), json!(p.seat.get())]));
    assert!(
        sched
            .iter()
            .any(|s| s.action == RELEASE_AI_MOVE && s.delay_ms == p.reveal_at - NOW)
    );
}

#[tokio::test]
async fn heuristic_dispatch_plays_the_parked_move_at_the_floor() {
    // D8: with no seat asked, the parked top move plays when the reveal fires.
    let st = drive("delivery-test", true, |st| {
        let game = &st.game;
        game.hand.phase == GamePhase::TrickPlay
            && game
                .hand
                .turn_seat
                .is_some_and(|s| s != HUMAN_SEAT && legal_moves(game, s).len() >= 2)
    });
    let h = TestHost::new(st, false);
    run(&h, Tempo::NORMAL, vec![]).await;
    let turn = h.st().game.turn_id.clone();
    let pending = h.st().game.pending.unwrap_or_else(|| panic!("parked"));
    h.set_time(pending.reveal_at);
    let mut s = h.session(Tempo::NORMAL);
    s.fire(TickKind::ReleaseAi, Some(turn.clone()), None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_ne!(h.st().game.turn_id, turn);
    let st = h.st();
    let last = st.journal.last().unwrap_or_else(|| panic!("journaled"));
    assert_eq!(Some(&last.id), pending.decision.as_ref());
    assert_eq!(last.source, Some(AiDecisionSource::Fallback));
}

// -------------------------------------------------- state-proxy.test.ts analog

#[tokio::test]
async fn repeated_commits_keep_one_value_and_the_journal_capped() {
    let f = ReviewFixture::new(0);
    let mut st = f.h.st();
    let v0 = st.game.v;
    let mv = legal_moves(&st.game, HUMAN_SEAT)
        .into_iter()
        .next()
        .unwrap_or_else(|| euchre_engine::LegalMove {
            id: "pass".into(),
            mv: Move::Pass,
            label: "Pass".into(),
        });
    let mut s = f.h.session(Tempo::NORMAL);
    for _ in 0..64 {
        let next = s.st.game.clone();
        commit(&mut s.st, next);
        s.journal(HUMAN_SEAT, &mv, Attribution::Human);
    }
    s.save().await.unwrap_or_else(|e| panic!("{e:#}"));
    st = f.h.st();
    assert_eq!(st.game.v, v0 + 64);
    assert_eq!(st.journal.len(), JOURNAL_CAP);
}

#[tokio::test]
async fn a_failed_save_leaves_the_live_state_unchanged() {
    let f = ReviewFixture::new(0);
    let before = f.h.st();
    f.h.fail_persist.store(true, Ordering::SeqCst);
    let Msg::Review(body) = f.set(true) else {
        panic!("review");
    };
    let mut s = f.h.session(Tempo::NORMAL);
    assert!(s.on_review(&body).await.is_err());
    assert_eq!(f.h.st(), before);
    assert_eq!(f.h.syncs.lock().len(), 0, "nothing emitted before a save");
    // The retry applies the write once.
    f.h.fail_persist.store(false, Ordering::SeqCst);
    let mut s = f.h.session(Tempo::NORMAL);
    s.on_review(&body).await.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(f.h.st().game.v, before.game.v + 1);
}

// ------------------------------------------------------ ledger and early fires

#[tokio::test]
async fn a_wake_resumes_the_read_pause_where_it_was() {
    let f = ReviewFixture::new(0);
    f.run(vec![]).await;
    let first = f.h.scheduled.lock().last().cloned();
    let full = Tempo::NORMAL.trick_resolve(false);
    assert_eq!(
        first.map(|s| (s.action, s.delay_ms)),
        Some((ON_TEMPO_GATE, full))
    );
    let due = f.h.st().ledger.tempo.map(|d| d.at);
    assert_eq!(due, Some(NOW + full));
    // Sleep 1 s, wake: only the remainder is armed.
    f.h.set_time(NOW + 1000);
    f.run(vec![]).await;
    let again = f.h.scheduled.lock().last().cloned();
    assert_eq!(
        again.map(|s| (s.action, s.delay_ms, s.args)),
        Some((ON_TEMPO_GATE, full - 1000, vec![json!(f.turn_id)]))
    );
}

#[tokio::test]
async fn an_early_fire_rearms_the_remainder_and_changes_nothing_else() {
    let f = ReviewFixture::new(0);
    f.run(vec![]).await;
    let full = Tempo::NORMAL.trick_resolve(false);
    let before = f.h.st().game;
    f.h.set_time(NOW + 500);
    let mut s = f.h.session(Tempo::NORMAL);
    s.fire(TickKind::Tempo, Some(f.turn_id.clone()), None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(f.h.st().game, before);
    let last = f.h.scheduled.lock().last().cloned();
    assert_eq!(
        last.map(|s| (s.action, s.delay_ms)),
        Some((ON_TEMPO_GATE, full - 500))
    );
    // Within tolerance counts as due.
    f.h.set_time(NOW + full - EARLY_FIRE_TOLERANCE_MS);
    let mut s = f.h.session(Tempo::NORMAL);
    s.fire(TickKind::Tempo, Some(f.turn_id.clone()), None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(f.h.st().game.hand.phase, GamePhase::TrickPlay);
    assert_eq!(f.h.st().ledger.tempo, None);
}

#[tokio::test]
async fn a_stale_fire_does_nothing() {
    let f = ReviewFixture::new(0);
    f.run(vec![]).await;
    let before = f.h.st();
    let n = f.h.scheduled.lock().len();
    let mut s = f.h.session(Tempo::NORMAL);
    s.fire(TickKind::Tempo, Some("old-turn".into()), None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(f.h.st(), before);
    assert_eq!(f.h.scheduled.lock().len(), n);
}

#[tokio::test]
async fn match_timers_are_rearmed_from_the_ledger_and_the_reap_waits_for_a_record() {
    let f = ReviewFixture::new(0);
    f.h.edit(|st| {
        st.recorded_at = Some(NOW);
        st.reap_id = Some("lost".into());
        st.ledger.reap_at = Some(NOW + 10_000);
    });
    f.h.set_time(NOW + 4_000);
    f.run(vec![]).await;
    let reaps: Vec<u64> =
        f.h.scheduled
            .lock()
            .iter()
            .filter(|s| s.action == ON_REAP)
            .map(|s| s.delay_ms)
            .collect();
    assert_eq!(reaps, vec![6_000]);
    // Due: a recorded table destroys itself.
    f.h.set_time(NOW + 10_000);
    let mut s = f.h.session(Tempo::NORMAL);
    s.fire(TickKind::Reap, None, None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert!(f.h.destroyed.load(Ordering::SeqCst));

    // Not yet recorded: the reap re-arms instead of destroying history.
    let g = ReviewFixture::new(0);
    g.h.edit(|st| st.ledger.reap_at = Some(NOW));
    let mut s = g.h.session(Tempo::NORMAL);
    s.fire(TickKind::Reap, None, None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert!(!g.h.destroyed.load(Ordering::SeqCst));
    assert_eq!(g.h.st().ledger.reap_at, Some(NOW + Tempo::NORMAL.reap_ms));
}

#[tokio::test]
async fn a_finished_match_without_a_profile_is_marked_recorded_with_no_reap() {
    let mut f = ReviewFixture::new(0);
    f.h.edit(|st| {
        st.owner_user_id = "guest-owner".into();
        st.game.status = euchre_engine::GameStatus::Complete;
    });
    f.turn_id.clear();
    f.run(vec![]).await;
    let st = f.h.st();
    assert_eq!(st.recorded_at, Some(NOW));
    assert_eq!(st.reap_id, None);
}

// ----------------------------------------------------------- submitMove order

fn user_error_of(r: Result<impl std::fmt::Debug>) -> (String, String, Option<Value>) {
    let e = r.err().unwrap_or_else(|| panic!("expected an error"));
    let u = parse_rivet_error(&e).unwrap_or_else(|| panic!("user error: {e}"));
    (u.code, u.message, u.metadata)
}

fn human_turn() -> TableState {
    drive("order-test", false, |st| {
        st.game.hand.turn_seat == Some(HUMAN_SEAT)
    })
}

#[test]
fn submit_move_checks_run_in_ts_order() {
    let st = human_turn();
    let legal = legal_moves(&st.game, HUMAN_SEAT);
    let good = legal[0].id.clone();

    // A finished match wins over an illegal move and a missing clientMoveId.
    let mut over = st.clone();
    over.game.status = euchre_engine::GameStatus::Complete;
    let (code, msg, _) = user_error_of(submit_move_body(&over, "u", &json!({ "moveId": "nope" })));
    assert_eq!(
        (code.as_str(), msg.as_str()),
        ("game_complete", "This match is over")
    );

    // An illegal move wins over a missing clientMoveId, and carries the legal set.
    let (code, msg, meta) = user_error_of(submit_move_body(&st, "u", &json!({ "moveId": "nope" })));
    assert_eq!(
        (code.as_str(), msg.as_str()),
        ("illegal_move", "That move is not legal right now")
    );
    assert_eq!(meta, Some(json!({ "legal": legal })));
    let (code, _, _) = user_error_of(submit_move_body(&st, "u", &json!(null)));
    assert_eq!(code, "illegal_move");

    // Then the clientMoveId.
    let (code, msg, _) = user_error_of(submit_move_body(&st, "u", &json!({ "moveId": good })));
    assert_eq!(
        (code.as_str(), msg.as_str()),
        ("internal_error", "Missing clientMoveId")
    );

    // A good request becomes the move body; the id is cut to 64 UTF-16 units.
    let long = "x".repeat(80);
    let body = submit_move_body(
        &st,
        "guest-u",
        &json!({ "moveId": good, "turnId": st.game.turn_id, "clientMoveId": long }),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(body.seq, st.game.applied_seq + 1);
    assert_eq!(body.client_move_id.len(), 64);
    assert_eq!(body.turn_id.as_deref(), Some(st.game.turn_id.as_str()));
    assert_eq!(body.user_id, "guest-u");
}

#[tokio::test]
async fn on_move_is_idempotent_and_refuses_stale_turns() {
    let st = human_turn();
    let h = TestHost::new(st, false);
    let st = h.st();
    let good = legal_moves(&st.game, HUMAN_SEAT)[0].id.clone();
    let req = json!({ "moveId": good, "turnId": st.game.turn_id, "clientMoveId": "c-1" });
    let body = submit_move_body(&st, "guest-u", &req).unwrap_or_else(|e| panic!("{e}"));

    // A stale turn id is a RuleError (the actor turns it into a rejected ack).
    let mut stale = body.clone();
    stale.turn_id = Some("old".into());
    let mut s = h.session(Tempo::NORMAL);
    let e = s
        .on_move(&stale)
        .await
        .err()
        .unwrap_or_else(|| panic!("stale"));
    assert_eq!(
        rule_error(&e).map(|r| r.code),
        Some(euchre_engine::RuleCode::StaleTurn)
    );
    assert_eq!(h.st(), st, "a refused move changes nothing");

    let mut s = h.session(Tempo::NORMAL);
    let ack = s.on_move(&body).await.unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    let v = h.st().game.v;
    assert_eq!(ack, MoveAck::Ok { v });
    assert!(v > st.game.v);
    assert_eq!(h.syncs.lock().len(), 1, "one sync per applied move");

    // The same message again (redelivery, double click): ok, nothing applied.
    let mut s = h.session(Tempo::NORMAL);
    let again = s.on_move(&body).await.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(again, MoveAck::Ok { v });
    assert_eq!(h.st().game.v, v);
    // A new seq with the same clientMoveId is a duplicate too.
    let mut dup = body.clone();
    dup.seq += 5;
    let mut s = h.session(Tempo::NORMAL);
    assert_eq!(
        s.on_move(&dup).await.unwrap_or_else(|e| panic!("{e:#}")),
        MoveAck::Ok { v }
    );
    assert_eq!(h.st().game.v, v);
}

#[test]
fn observations_round_human_think_time_and_mark_ai_as_paced() {
    let st = human_turn();
    let h = TestHost::new(st, false);
    let mut s = h.session(Tempo::NORMAL);
    let before = s.st.game.clone();
    s.st.turn_opened = Some(crate::state::TurnOpened {
        turn_id: before.turn_id.clone(),
        at: NOW - 1_130,
        connected: true,
    });
    let mv = legal_moves(&before, HUMAN_SEAT)[0].clone();
    s.observe_move(&before, HUMAN_SEAT, &mv);
    let o =
        s.st.observations
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("observed"));
    assert_eq!(o.elapsed_ms, Some(1_250));
    assert_eq!(o.timing, euchre_engine::ObservationTiming::HumanObserved);
    // Disconnected: unreliable, no time.
    if let Some(t) = s.st.turn_opened.as_mut() {
        t.connected = false;
    }
    s.observe_move(&before, HUMAN_SEAT, &mv);
    let o =
        s.st.observations
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("observed"));
    assert_eq!(
        (o.elapsed_ms, o.timing),
        (None, euchre_engine::ObservationTiming::Unreliable)
    );
    s.observe_move(&before, Seat::WEST, &mv);
    let o =
        s.st.observations
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("observed"));
    assert_eq!(o.timing, euchre_engine::ObservationTiming::ComputerPaced);
}

// ------------------------------------- profile recording (03 §11, §13, §16.7)

const OWNER: &str = "guest-owner";
const PROFILE_TOKEN: &str = "deployment-internal-token";

/// A table with personas, an owner and a writable profile, driven to `done`.
fn owned(game_id: &str, done: impl Fn(&TableState) -> bool) -> TestHost {
    let mut st = drive(game_id, true, done);
    st.owner_user_id = OWNER.to_owned();
    let h = TestHost::new(st, true);
    h.profile.available.store(true, Ordering::SeqCst);
    h
}

fn game_over() -> TestHost {
    owned("profile-test", |st| {
        st.game.status != euchre_engine::GameStatus::Active
    })
}

fn flush_profile_schedules(h: &TestHost) -> Vec<u64> {
    h.scheduled
        .lock()
        .iter()
        .filter(|s| s.action == FLUSH_PROFILE)
        .map(|s| s.delay_ms)
        .collect()
}

fn game_ends(h: &TestHost) -> Vec<Option<u32>> {
    h.ai.notifies
        .lock()
        .iter()
        .filter(|c| c.message.kind == AiSeatLifecycleKind::GameEnd)
        .map(|c| c.message.hand_no)
        .collect()
}

async fn settle(h: &TestHost, outcome: std::result::Result<(), String>) {
    let mut s = h.session(Tempo::NORMAL);
    s.on_record_game(outcome)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
}

#[tokio::test]
async fn a_finished_match_is_sent_once_off_the_lane_then_settles_and_arms_the_reap() {
    let h = game_over();
    run(&h, Tempo::NORMAL, vec![]).await;

    // Sent (detached) with the retry already persisted; not recorded yet.
    let st = h.st();
    assert_eq!(h.profile.games.lock().len(), 1);
    assert_eq!(st.recorded_at, None);
    assert_eq!(
        st.ledger.profile_retry_at,
        Some(NOW + Tempo::NORMAL.profile_retry_ms)
    );
    assert_eq!(flush_profile_schedules(&h), vec![30_000]);
    assert!(game_ends(&h).is_empty(), "gameEnd waits for the record");

    // The body `playerProfile.recordGame` reads (TS `RecordGameMessage`).
    let call = h.profile.games.lock()[0].clone();
    assert_eq!(call.profile_user, OWNER);
    let body = serde_json::to_value(crate::remote::RecordGameBody {
        internal_token: PROFILE_TOKEN,
        record: &call.record,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let keys: Vec<&str> = body
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(keys, ["internalToken", "record"]);
    let record_keys: Vec<&str> = body["record"]
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        record_keys,
        [
            "schema",
            "matchId",
            "userId",
            "seed",
            "cfg",
            "firstDealer",
            "startedAt",
            "endedAt",
            "status",
            "outcome",
            "score",
            "winnerTeam",
            "handsPlayed",
            "personas",
            "stats",
            "tokens"
        ]
    );
    assert_eq!(
        body["record"]["tokens"],
        json!({ "tokensIn": 0, "tokensOut": 0, "cacheReadTokens": 0, "calls": 0 })
    );
    let parsed = euchre_profile_model::parse_record_game(&body, OWNER, NOW as i64)
        .unwrap_or_else(|e| panic!("playerProfile refuses the body: {e:?}"));
    assert_eq!(parsed.record.match_id, "profile-test");
    assert!(parsed.hands.is_none());

    // The outcome: recorded, the retry cancelled, gameEnd to every seat, the reap.
    h.set_time(NOW + 700);
    settle(&h, Ok(())).await;
    let st = h.st();
    assert_eq!(st.recorded_at, Some(NOW + 700));
    assert_eq!(
        (st.profile_retry_id.clone(), st.ledger.profile_retry_at),
        (None, None)
    );
    assert_eq!(game_ends(&h), vec![None, None, None]);
    assert_eq!(st.ledger.reap_at, Some(NOW + 700 + Tempo::NORMAL.reap_ms));
    assert!(st.reap_id.is_some());

    // Settled: a wake, a late duplicate outcome and a stray flushProfile send nothing.
    settle(&h, Ok(())).await;
    run(&h, Tempo::NORMAL, vec![]).await;
    let mut s = h.session(Tempo::NORMAL);
    s.fire(TickKind::FlushProfile, None, None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(h.profile.games.lock().len(), 1);
    assert_eq!(game_ends(&h).len(), 3);
}

/// A create input past every bound `TableState::create` applies: five
/// personas (a seat-0 one and a duplicate seat), every text field at three
/// times its cap in 4-byte characters, and the persona ids, the profile id and
/// the seed in control characters (6 JSON bytes per unit, the costliest
/// escape). Every number is at the most digits its type holds.
fn oversized_create_input() -> TableCreateInput {
    let emoji = |n: usize| "\u{1F0CF}".repeat(n);
    let control = |n: usize| "\u{1}".repeat(n);
    let persona = |seat: Seat| {
        let mut a = default_personas()
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("a default persona"));
        a.seat = seat;
        let p = &mut a.persona;
        p.id = control(3 * crate::create_input::PERSONA_ID_MAX_UNITS);
        p.version = u32::MAX;
        p.name = emoji(3 * euchre_protocol::PERSONA_NAME_MAX_CHARS);
        p.blurb = emoji(3 * euchre_protocol::PERSONA_BLURB_MAX_CHARS);
        p.prompt = emoji(3 * euchre_protocol::PERSONA_PROMPT_MAX_CHARS);
        p.house_prompt = emoji(3 * euchre_protocol::HOUSE_PROMPT_MAX_CHARS);
        p.aggression = -f64::MIN_POSITIVE;
        p.risk = -f64::MIN_POSITIVE;
        p.chattiness = -f64::MIN_POSITIVE;
        p.temperature = -f64::MIN_POSITIVE;
        p.model_id = euchre_protocol::AnthropicModelId::ClaudeHaiku45;
        p.bid_model_id = euchre_protocol::AnthropicModelId::ClaudeHaiku45;
        a.dossier = emoji(3 * euchre_protocol::PERSONA_DOSSIER_MAX_CHARS);
        a
    };
    TableCreateInput {
        owner_user_id: Some(OWNER.to_owned()),
        profile_user_id: Some(control(crate::create_input::USER_ID_MAX_UNITS)),
        seed: Some(control(3 * crate::create_input::SEED_MAX_UNITS)),
        personas: Some(vec![
            persona(Seat::SOUTH),
            persona(Seat::WEST),
            persona(Seat::NORTH),
            persona(Seat::WEST),
            persona(Seat::EAST),
        ]),
        ..TableCreateInput::default()
    }
}

/// W3 follow-up (`reports/w3-crates.md` §3): no create input can make the
/// match's `recordGame` exceed rivetkit's 65,536-byte incoming-message limit,
/// which would fail every send the same way and retry it forever.
#[tokio::test]
async fn worst_case_record_game_fits_the_action_limit() {
    let limit = usize::try_from(rivetkit::ActorConfig::default().max_incoming_message_size)
        .unwrap_or(usize::MAX);
    assert_eq!(limit, 65_536, "rivetkit 3.1.0 default");

    let input = oversized_create_input();
    let unbounded = serde_json::to_vec(&input.personas)
        .unwrap_or_default()
        .len();
    assert!(unbounded > limit, "the input alone is {unbounded} B");

    // The longest match id the create accepts (the profile's id pattern).
    let match_id = "m".repeat(128);
    let st = drive_input(&match_id, &input, |st| {
        st.game.status != euchre_engine::GameStatus::Active
    });
    assert_eq!(
        st.personas.iter().map(|p| p.seat.get()).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let h = TestHost::new(st, true);
    h.profile.available.store(true, Ordering::SeqCst);
    run(&h, Tempo::NORMAL, vec![]).await;
    let mut call = h
        .profile
        .games
        .lock()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("recordGame was not sent"));

    // Widen every number to the most digits the profile accepts, past what
    // play reaches, so the bound does not depend on how this game went.
    let max_ms = u64::try_from(euchre_profile_model::js::MAX_SAFE_INTEGER).unwrap_or(u64::MAX);
    let r = &mut call.record;
    r.cfg.game_to = u8::MAX;
    r.cfg.misdeal_limit = u8::MAX;
    r.started_at = max_ms;
    r.ended_at = Some(max_ms);
    r.score = [u8::MAX; 2];
    r.hands_played = 10_000;
    r.stats = euchre_protocol::MatchStats {
        euchres_for: u32::MAX,
        euchres_against: u32::MAX,
        loners_attempted: u32::MAX,
        loners_made: u32::MAX,
        marches: u32::MAX,
        throw_ins: u32::MAX,
        tricks: [u32::MAX; 2],
    };

    // The deployment token is the operator's secret (generated ones are 43
    // characters); 64 leaves room for a hand-set one.
    let token = "t".repeat(64);
    let body = serde_json::to_value(crate::remote::RecordGameBody {
        internal_token: &token,
        record: &call.record,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let wire = serde_json::to_vec(&json!({ "args": [body.clone()] }))
        .unwrap_or_else(|e| panic!("{e}"))
        .len();
    // Measured at 63,617 B (1,919 B of headroom). Keep at least 1 KiB spare,
    // so a new record field or envelope byte shows up here first.
    assert!(
        wire + 1_024 <= limit,
        "worst-case recordGame is {wire} B, within 1 KiB of the {limit} B action limit"
    );

    // And the profile accepts it, so it records instead of retrying.
    let parsed = euchre_profile_model::parse_record_game(&body, &call.profile_user, NOW as i64)
        .unwrap_or_else(|e| panic!("playerProfile refuses the worst-case body: {e:?}"));
    assert_eq!(parsed.record.match_id, match_id);
    assert_eq!(parsed.record.personas.len(), 3);
}

#[tokio::test]
async fn a_failed_record_game_retries_on_flush_profile_and_never_reaps_unrecorded() {
    let h = game_over();
    run(&h, Tempo::NORMAL, vec![]).await;
    settle(&h, Err("Actor queue delivery interrupted".into())).await;
    let st = h.st();
    assert_eq!(st.recorded_at, None);
    assert!(st.profile_retry_id.is_some());
    assert_eq!(st.reap_id, None);
    assert!(game_ends(&h).is_empty());
    // TS: a retry is armed only when none is (`profileRetryId === null`).
    assert_eq!(flush_profile_schedules(&h), vec![30_000]);

    // The retry fires: a second send, with the next retry armed first.
    h.set_time(NOW + 30_000);
    let mut s = h.session(Tempo::NORMAL);
    s.fire(TickKind::FlushProfile, None, None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(h.profile.games.lock().len(), 2);
    assert_eq!(h.st().ledger.profile_retry_at, Some(NOW + 60_000));
    settle(&h, Ok(())).await;
    assert_eq!(h.st().recorded_at, Some(NOW + 30_000));
    assert_eq!(game_ends(&h).len(), 3);
}

#[tokio::test]
async fn a_crash_before_the_record_game_outcome_sends_again_on_wake() {
    let h = game_over();
    run(&h, Tempo::NORMAL, vec![]).await;
    assert_eq!(h.profile.games.lock().len(), 1);
    // The process dies before the outcome: wake on the persisted row.
    let woke = TestHost::new(h.st(), true);
    woke.profile.available.store(true, Ordering::SeqCst);
    woke.set_time(NOW + 5_000);
    run(&woke, Tempo::NORMAL, vec![]).await;
    assert_eq!(woke.profile.games.lock().len(), 1, "resent on wake");
    // The persisted retry was re-armed from the ledger (25 s left).
    assert_eq!(flush_profile_schedules(&woke), vec![25_000]);
    settle(&woke, Ok(())).await;
    assert_eq!(woke.st().recorded_at, Some(NOW + 5_000));
}

#[tokio::test]
async fn without_an_owner_nothing_is_recorded_or_sent() {
    let h = game_over();
    h.edit(|st| st.owner_user_id.clear());
    run(&h, Tempo::NORMAL, vec![]).await;
    assert!(h.profile.games.lock().is_empty());
    assert_eq!(h.st().recorded_at, None);
    assert!(flush_profile_schedules(&h).is_empty());
}

#[tokio::test]
async fn each_finished_hand_goes_to_the_profile_then_seats_hear_hand_end_reset_hand_decide() {
    let h = owned("hand-journal-test", |st| {
        st.game.hand.phase == GamePhase::HandScore
    });
    let hand_no = h.st().game.hand.hand_no;
    run(&h, Tempo::NORMAL, vec![]).await;

    // recordHands: one entry, for this hand, with the TS body shape.
    let calls = h.profile.hands.lock().clone();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(
        (
            call.profile_user.as_str(),
            call.match_id.as_str(),
            call.entry.hand_no
        ),
        (OWNER, "hand-journal-test", hand_no)
    );
    let body = serde_json::to_value(crate::remote::RecordHandsBody {
        internal_token: PROFILE_TOKEN,
        match_id: &call.match_id,
        hands: std::slice::from_ref(&call.entry),
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let keys: Vec<&str> = body
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(keys, ["internalToken", "matchId", "hands"]);
    let rows = euchre_profile_model::parse_record_hands(&body)
        .unwrap_or_else(|e| panic!("playerProfile refuses the body: {e:?}"));
    assert_eq!(rows.len(), 1);

    // The read pause ends: the next hand opens.
    let turn_id = h.st().game.turn_id.clone();
    let tick = Msg::Tick(TickMsg {
        internal_token: h.st().game.internal_token.clone(),
        game_id: "hand-journal-test".into(),
        kind: TickKind::Tempo,
        turn_id,
        seat: None,
    });
    run(&h, Tempo::NORMAL, vec![tick]).await;
    assert_eq!(h.profile.hands.lock().len(), 1, "a hand is sent once");

    // Per seat: handEnd(N), then resetHand(N+1), then any decide of N+1.
    let log = h.ai.log.lock().clone();
    for seat in 1..=3u8 {
        let mine: Vec<&String> = log
            .iter()
            .filter(|l| l.split(':').nth(1) == Some(seat.to_string().as_str()))
            .collect();
        let at = |needle: String| mine.iter().position(|l| **l == needle);
        let end = at(format!("handEnd:{seat}:{hand_no}"));
        let reset = at(format!("resetHand:{seat}:{}", hand_no + 1));
        assert!(end.is_some() && reset.is_some(), "seat {seat}: {mine:?}");
        assert!(end < reset, "seat {seat}: {mine:?}");
        if let Some(decide) = at(format!("decide:{seat}:{}", hand_no + 1)) {
            assert!(reset < Some(decide), "seat {seat}: {mine:?}");
        }
    }
}

// -------------------------------------------------- forged publish (03 §14)

/// 01-ARCHITECTURE §3.2: the per-match token is the only thing between a
/// forged message and the game. The same bodies with the right token land.
#[tokio::test]
async fn forged_publish_is_refused_by_the_token_alone() {
    let f = DeliveryFixture::new().await;
    let chosen = f.request.legal[1].id.clone();
    let mut forged = f.reply(&chosen);
    forged.internal_token = "forged-token-0123".into();
    let say = |token: &str| {
        Msg::Say(AiSayMsg {
            internal_token: token.to_owned(),
            game_id: f.request.game_id.clone(),
            seat: f.request.seat,
            turn_id: f.request.turn_id.clone(),
            hand_no: None,
            msg_id: "talk".into(),
            delta: None,
            text: Some("Your lead.".into()),
            is_final: true,
        })
    };
    let chat = f.h.event_count("chat");
    f.run(vec![Msg::Ai(forged), say("forged-token-0123")]).await;
    assert_eq!(f.pending().answered, Some(false), "forged decision dropped");
    assert_eq!(f.h.event_count("chat"), chat, "forged chat dropped");

    let token = f.request.internal_token.clone();
    f.run(vec![say(&token), Msg::Ai(f.reply(&chosen))]).await;
    assert_eq!(f.h.event_count("chat"), chat + 1);
    let p = f.pending();
    assert_eq!(
        (p.decision.as_deref(), p.answered),
        (Some(chosen.as_str()), Some(true))
    );
}

// ------------------------------------------------ JSON goldens (03 §16.8)

fn fixture(json: &str, key: &str) -> Value {
    let all: Value = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
    all[key].clone()
}

fn keys_of(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// The TS-emitted goldens of `euchre-protocol` (`scripts/gen-fixtures.ts`).
const SYNC_FIXTURES: &str = include_str!("../../../euchre-protocol/tests/fixtures/sync.json");
const ACTION_FIXTURES: &str = include_str!("../../../euchre-protocol/tests/fixtures/actions.json");

#[test]
fn the_sync_payload_has_the_ts_shape() {
    let st = human_turn();
    let payload = sync_payload(&st, HUMAN_SEAT, &[]).unwrap_or_else(|| panic!("a payload"));
    let golden = fixture(SYNC_FIXTURES, "first");
    assert_eq!(keys_of(&payload), keys_of(&golden));
    assert_eq!(keys_of(&payload["view"]), keys_of(&golden["view"]));
    assert_eq!(payload["v"], payload["view"]["v"]);
    assert_eq!(payload["steps"], json!([]));
    let back: euchre_protocol::SyncEvent =
        serde_json::from_value(payload.clone()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(serde_json::to_value(back).unwrap_or(Value::Null), payload);
}

#[tokio::test]
async fn move_acks_match_the_ts_goldens() {
    let h = TestHost::new(human_turn(), false);
    h.edit(|st| st.game.status = euchre_engine::GameStatus::Complete);
    let mut s = h.session(Tempo::NORMAL);
    let body = euchre_protocol::MoveMsg {
        seq: 1,
        turn_id: None,
        move_id: "pass".into(),
        client_move_id: "c-1".into(),
        user_id: OWNER.into(),
    };
    let ack = s.on_move(&body).await.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(
        serde_json::to_value(ack).unwrap_or(Value::Null),
        fixture(ACTION_FIXTURES, "ackGameComplete")
    );
    let ok = serde_json::to_value(MoveAck::Ok { v: 17 }).unwrap_or(Value::Null);
    assert_eq!(ok, fixture(ACTION_FIXTURES, "ackOk"));
    let err = fixture(ACTION_FIXTURES, "ackErr");
    let back: MoveAck = serde_json::from_value(err.clone()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(serde_json::to_value(back).unwrap_or(Value::Null), err);
}

// ------------------------------------------------------------ review fixes

/// Let the next `n` persists succeed and fail every one after them.
fn fail_after(h: &TestHost, n: u64) {
    h.fail_from
        .store(h.persists.load(Ordering::SeqCst) + n, Ordering::SeqCst);
}

fn heal(h: &TestHost) {
    h.fail_from.store(u64::MAX, Ordering::SeqCst);
}

#[tokio::test]
async fn a_second_move_validated_against_the_same_view_is_refused_once_the_first_applied() {
    // Two clicks (or two tabs) on one view: the body is built inside the lane
    // turn, so the second is checked against the state the first left.
    let st = drive("double-move", true, |st| {
        st.game.hand.turn_seat == Some(HUMAN_SEAT)
            && st.game.hand.phase == GamePhase::BidRound1
            && legal_moves(&st.game, HUMAN_SEAT).len() >= 2
    });
    let h = TestHost::new(st, true);
    let st = h.st();
    let legal = legal_moves(&st.game, HUMAN_SEAT);
    let first = json!({ "moveId": legal[0].id, "turnId": st.game.turn_id, "clientMoveId": "c-1" });
    let second = json!({ "moveId": legal[1].id, "turnId": st.game.turn_id, "clientMoveId": "c-2" });
    // The old order built both bodies from one view; the second then read as a
    // redelivery and was acked without being applied.
    let stale = submit_move_body(&st, "guest-u", &second).unwrap_or_else(|e| panic!("{e}"));

    let mut s = h.session(Tempo::NORMAL);
    let body = submit_move_body(&s.st, "guest-u", &first).unwrap_or_else(|e| panic!("{e}"));
    s.on_move(&body).await.unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    let after = h.st();
    assert!(after.game.v > st.game.v);

    let s = h.session(Tempo::NORMAL);
    let (code, msg, _) = user_error_of(submit_move_body(&s.st, "guest-u", &second));
    assert_eq!(
        (code.as_str(), msg.as_str()),
        ("illegal_move", "That move is not legal right now")
    );
    assert!(
        stale.seq <= after.game.applied_seq,
        "the stale body is behind"
    );
}

#[test]
fn the_move_action_body_is_rebuilt_server_side() {
    let st = human_turn();
    let msg = MoveMsg {
        seq: u64::MAX,
        turn_id: Some(st.game.turn_id.clone()),
        move_id: "pass".into(),
        client_move_id: "y".repeat(500),
        user_id: "someone-else".into(),
    };
    let body = queue_move_body(&st, "guest-u", &msg);
    assert_eq!(body.seq, st.game.applied_seq + 1);
    assert_eq!(body.client_move_id.len(), 64);
    assert_eq!(body.user_id, "guest-u");
    assert_eq!(body.move_id, "pass");
    // A table already at the top cannot overflow.
    let mut top = st.clone();
    top.game.applied_seq = u64::MAX;
    assert_eq!(queue_move_body(&top, "u", &msg).seq, u64::MAX);
    let good = legal_moves(&top.game, HUMAN_SEAT)[0].id.clone();
    let req = json!({ "moveId": good, "clientMoveId": "c" });
    let body = submit_move_body(&top, "u", &req).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(body.seq, u64::MAX);
}

/// A table in round 2 where the human deals and three have passed, with
/// stick-the-dealer off: the human's pass throws the hand in.
fn throw_in_table() -> TableState {
    let input = TableCreateInput {
        seed: Some("throw-in".into()),
        owner_user_id: Some("guest-owner".into()),
        personas: Some(default_personas()),
        ..TableCreateInput::default()
    };
    let mut st = TableState::create(
        "throw-in".into(),
        "throw-in".into(),
        "match-token-0123".into(),
        &input,
        NOW,
    )
    .unwrap_or_else(|e| panic!("create: {e}"));
    st.game.cfg.stick_the_dealer = false;
    for _ in 0..5_000 {
        let game = &st.game;
        if game.hand.phase == GamePhase::BidRound2
            && game.hand.turn_seat == Some(HUMAN_SEAT)
            && game.hand.dealer_seat == HUMAN_SEAT
        {
            return st;
        }
        let next = match game.hand.turn_seat {
            Some(seat) => {
                let legal = legal_moves(game, seat);
                let mv = legal
                    .iter()
                    .find(|m| m.mv == Move::Pass)
                    .or(legal.first())
                    .unwrap_or_else(|| panic!("a legal move"));
                try_apply(game, seat, mv.mv, None)
                    .map(|r| r.state)
                    .unwrap_or_else(|e| panic!("apply: {e}"))
            }
            None => try_advance(game)
                .map(|r| r.state)
                .unwrap_or_else(|e| panic!("advance: {e}")),
        };
        st.game = next;
    }
    panic!("never reached a human dealer in round 2");
}

#[tokio::test]
async fn a_thrown_in_hand_lands_in_the_same_save_as_the_move_and_is_sent_after_it() {
    let st = throw_in_table();
    let before_no = st.game.hand.hand_no;
    let h = TestHost::new(st, true);
    h.profile.available.store(true, Ordering::SeqCst);
    let st = h.st();
    let req = json!({ "moveId": "pass", "turnId": st.game.turn_id, "clientMoveId": "c-throw" });
    let body = submit_move_body(&st, "guest-owner", &req).unwrap_or_else(|e| panic!("{e}"));

    // The move's save is the only one that succeeds: the actor is "killed"
    // before anything after it persists.
    fail_after(&h, 1);
    let mut s = h.session(Tempo::NORMAL);
    assert!(s.on_move(&body).await.is_err(), "a later save failed");
    assert!(s.committed());
    let live = h.st();
    assert!(move_persisted(&live, &body), "the move is durable");
    assert_eq!(live.last_journaled_hand_no, i64::from(before_no));
    assert_eq!(live.stats.throw_ins, 1);
    assert!(
        h.reviews.lock().contains_key(&before_no),
        "archived with it"
    );
    assert_eq!(
        live.last_reset_hand_no,
        i64::from(live.game.hand.hand_no),
        "the new hand's reset is saved too"
    );
    // The boundary calls went out after that save, in TS order.
    let hand_nos: Vec<u32> = h
        .profile
        .hands
        .lock()
        .iter()
        .map(|c| c.entry.hand_no)
        .collect();
    assert_eq!(hand_nos, vec![before_no]);
    assert!(h.ai.log.lock().iter().any(|l| l.starts_with("resetHand:")));

    // The recovery reconcile re-derives nothing twice: no second archive.
    heal(&h);
    let mut s = h.session(Tempo::NORMAL);
    s.reconcile().await.unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(h.st().stats.throw_ins, 1);
    assert_eq!(h.profile.hands.lock().len(), 1);
}

#[tokio::test]
async fn a_turn_that_fails_after_its_first_save_is_recovered_by_reconcile() {
    // The human plays, the move is saved, then arming the next AI turn fails.
    let st = drive("recover", true, |st| {
        let game = &st.game;
        game.hand.phase == GamePhase::TrickPlay
            && game.hand.turn_seat == Some(HUMAN_SEAT)
            && game.hand.trick.plays.len() < 3
    });
    let h = TestHost::new(st, true);
    let st = h.st();
    let good = legal_moves(&st.game, HUMAN_SEAT)[0].id.clone();
    let req = json!({ "moveId": good, "turnId": st.game.turn_id, "clientMoveId": "c-r" });
    let body = submit_move_body(&st, "guest-u", &req).unwrap_or_else(|e| panic!("{e}"));
    fail_after(&h, 1);
    let mut s = h.session(Tempo::NORMAL);
    assert!(s.on_move(&body).await.is_err());
    assert!(s.committed());
    let live = h.st();
    assert!(move_persisted(&live, &body));
    assert!(live.game.pending.is_none(), "no AI turn was armed");
    let ai_turn = live.game.turn_id.clone();

    // A retry of the original trigger is stale against the live state.
    heal(&h);
    let asked = h.ai.decides.lock().len();
    let mut s = h.session(Tempo::NORMAL);
    s.fire(TickKind::Tempo, Some(st.game.turn_id.clone()), None)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    assert!(h.st().game.pending.is_none());

    // The recovery reconcile arms the AI turn from the persisted state.
    let mut s = h.session(Tempo::NORMAL);
    s.reconcile().await.unwrap_or_else(|e| panic!("{e:#}"));
    s.flush().await.unwrap_or_else(|e| panic!("{e:#}"));
    let st = h.st();
    let pending = st
        .game
        .pending
        .unwrap_or_else(|| panic!("the AI turn is armed"));
    assert_eq!(pending.turn_id, ai_turn);
    assert!(st.game.watchdog_id.is_some() && st.game.reveal_id.is_some());
    assert_eq!(h.ai.decides.lock().len(), asked + 1, "the seat is asked");
}

#[tokio::test]
async fn an_early_answer_keeps_the_old_timers_until_its_reveal_is_saved() {
    let f = DeliveryFixture::new().await;
    let before = f.h.st();
    let answer = f.reply(&f.request.legal[0].id);
    f.h.fail_persist.store(true, Ordering::SeqCst);
    let mut s = f.h.session(Tempo::NORMAL);
    assert!(s.on_ai_decision(&answer).await.is_err());
    assert!(f.h.cancels.lock().is_empty(), "no timer was cancelled");
    let live = f.h.st();
    assert_eq!(live.game.watchdog_id, before.game.watchdog_id);
    assert_eq!(live.game.reveal_id, before.game.reveal_id);

    f.h.fail_persist.store(false, Ordering::SeqCst);
    let mut s = f.h.session(Tempo::NORMAL);
    s.on_ai_decision(&answer)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    let live = f.h.st();
    assert_eq!(live.game.watchdog_id, None);
    assert!(live.game.reveal_id.is_some());
    assert_eq!(
        f.h.cancels.lock().len(),
        2,
        "old watchdog and reveal, after the save"
    );
}

#[tokio::test]
async fn a_line_with_a_null_hand_no_is_dropped_as_ts_did() {
    let f = DeliveryFixture::new().await;
    let say = AiSayMsg {
        internal_token: f.request.internal_token.clone(),
        game_id: f.request.game_id.clone(),
        seat: f.request.seat,
        turn_id: f.request.turn_id.clone(),
        hand_no: None,
        msg_id: "talk".into(),
        delta: None,
        text: Some("Your lead.".into()),
        is_final: true,
    };
    let s = f.h.session(Tempo::NORMAL);
    let before = f.h.event_count("chat");
    s.on_ai_say(&say, true);
    assert_eq!(
        f.h.event_count("chat"),
        before,
        "handNo: null is a mismatch"
    );
    s.on_ai_say(&say, false);
    assert_eq!(
        f.h.event_count("chat"),
        before + 1,
        "absent handNo checks the turn"
    );
}

#[tokio::test]
async fn an_internal_tick_cannot_fire_a_timer_before_it_is_due() {
    // Reap: a recorded table with its reap a day away is not destroyed.
    let f = ReviewFixture::new(0);
    f.h.edit(|st| {
        st.recorded_at = Some(NOW);
        st.ledger.reap_at = Some(NOW + 86_400_000);
    });
    let st = f.h.st();
    let tick = |kind: TickKind, turn_id: &str, seat: Option<Seat>| TickMsg {
        internal_token: st.game.internal_token.clone(),
        game_id: st.game.game_id.clone(),
        kind,
        turn_id: turn_id.to_owned(),
        seat,
    };
    let mut s = f.h.session(Tempo::NORMAL);
    s.on_tick_message(&tick(TickKind::Reap, "", None))
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert!(!f.h.destroyed.load(Ordering::SeqCst));

    // Tempo: the read pause is not skipped before its due time.
    f.run(vec![]).await;
    let mut s = f.h.session(Tempo::NORMAL);
    s.on_tick_message(&tick(TickKind::Tempo, &f.turn_id, None))
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(f.h.st().game.turn_id, f.turn_id, "still paused");

    // AI timeout: an asked turn is not forced before its deadline.
    let d = DeliveryFixture::new().await;
    let mut s = d.h.session(Tempo::NORMAL);
    let body = TickMsg {
        internal_token: d.request.internal_token.clone(),
        game_id: d.request.game_id.clone(),
        kind: TickKind::AiTimeout,
        turn_id: d.request.turn_id.clone(),
        seat: Some(d.request.seat),
    };
    s.on_tick_message(&body)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(d.h.st().game.turn_id, d.request.turn_id);
    // A forged tick is dropped before anything.
    let mut forged = body.clone();
    forged.internal_token = "forged".into();
    d.h.set_time(d.request.deadline_at + 1);
    let mut s = d.h.session(Tempo::NORMAL);
    s.on_tick_message(&forged)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(d.h.st().game.turn_id, d.request.turn_id);
    // Once due, the same tick plays the parked move.
    let mut s = d.h.session(Tempo::NORMAL);
    s.on_tick_message(&body)
        .await
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_ne!(d.h.st().game.turn_id, d.request.turn_id);
}

#[test]
fn an_internal_reader_never_sees_a_private_card() {
    let st = human_turn();
    let player = table_view(&st, HUMAN_SEAT);
    assert!(!player.hand.is_empty() && !player.legal.is_empty());
    let internal = internal_view(&st);
    assert!(internal.hand.is_empty());
    assert!(internal.legal.is_empty());
    assert_eq!(internal.you, HUMAN_SEAT);
    assert_eq!(internal.hand_counts, player.hand_counts);
    assert_eq!(internal.turn_id, player.turn_id);
}
