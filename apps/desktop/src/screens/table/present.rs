//! What the table screen keeps beside the store (ARCHITECTURE §2.5): rack slots, the illegal
//! card message, the go-alone box, the trick gather, outcome banners and the deal-in. The store
//! owns the game; this owns only how it is being shown.
//!
//! Every deadline is stamped lazily by [`Presentation::poll`] on the frame clock (the toast
//! pattern), so handlers and bridge replies never need a clock.

use std::collections::VecDeque;

use euchre_protocol::{
    Card, GamePhase, GameStatus, HandResult, Play, PublicGameView, Seat, Step, Suit, TableStore,
};

use super::choreo::{Director, Feel, PlayFlightSpec, Shot, ShotKey};
use super::copy;
use crate::theme::felt::timing;
use crate::theme::motion;

/// One rack slot (06 §5.6): a card that keeps its place while it is out of the hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The card this slot was made for.
    pub card: Card,
    /// Whether it is in the hand now; an absent card leaves an invisible placeholder.
    pub present: bool,
}

/// The rack's slot list for one hand number (`reconcileHandSlots`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RackSlots {
    /// The hand the slots belong to.
    pub hand_no: Option<u32>,
    /// The slots, in the order cards first arrived.
    pub slots: Vec<Slot>,
}

impl RackSlots {
    /// Follows `hand` for `hand_no`: a new hand resets the slots; new cards append; cards that
    /// leave keep their slot. Returns true when the hand number changed.
    pub fn reconcile(&mut self, hand_no: u32, hand: &[Card]) -> bool {
        if self.hand_no != Some(hand_no) {
            self.hand_no = Some(hand_no);
            self.slots = hand
                .iter()
                .map(|&card| Slot {
                    card,
                    present: true,
                })
                .collect();
            return true;
        }
        for slot in &mut self.slots {
            slot.present = hand.contains(&slot.card);
        }
        for &card in hand {
            if !self.slots.iter().any(|s| s.card == card) {
                self.slots.push(Slot {
                    card,
                    present: true,
                });
            }
        }
        false
    }

    /// The card in slot `index` (0-based), if it is in the hand.
    pub fn card_at(&self, index: usize) -> Option<Card> {
        self.slots.get(index).filter(|s| s.present).map(|s| s.card)
    }

    /// The present cards, in slot order.
    pub fn present(&self) -> impl Iterator<Item = Card> + '_ {
        self.slots.iter().filter(|s| s.present).map(|s| s.card)
    }
}

/// The trick gather to the winner's stack (06 §6.4).
#[derive(Clone, Debug, PartialEq)]
pub struct Collect {
    /// The trick's hand.
    pub hand_no: u32,
    /// The trick's index.
    pub index: u8,
    /// Who took it.
    pub winner: Seat,
    /// Its plays in sweep order: the taker's card, then slot order (left, partner, right,
    /// you).
    pub plays: Vec<Play>,
    /// The `turnId` Continue answers.
    pub turn_id: String,
    /// Review mode: the gather waits for Continue.
    pub review: bool,
    /// Auto mode: when the gather starts (stamped `COLLECT_AFTER` after the winner shows).
    pub due: Option<f64>,
    /// When the gather started.
    pub started: Option<f64>,
    /// How many cards have left the trick for the stack.
    pub moved: usize,
    /// Continue was pressed.
    pub continuing: bool,
    /// The Continue request went out.
    pub sent: bool,
    /// The cards have begun to gather toward the winner (09 §4.4: `GATHER_LEAD` before the
    /// sweep, Full feel only; Lane B moves the slots with `.motion(GATHER)`).
    pub gather: bool,
    /// The last card has landed in the stack (`Cue::TrickTake` is out).
    pub took: bool,
}

impl Collect {
    fn new(view: &PublicGameView, winner: Seat) -> Collect {
        let mut plays = view.trick.plays.clone();
        // The taker's card leads the sweep into its stack, and lands on top there (stage.rs
        // `stack`); the rest follow in slot order: left (1), partner (2), right (3), you (0).
        plays.sort_by_key(|p| (p.seat != winner, (copy::rel(view, p.seat) + 3) % 4));
        Collect {
            hand_no: view.hand_no,
            index: view.trick.index,
            winner,
            plays,
            turn_id: view.turn_id.clone(),
            review: copy::review_mode(view),
            due: None,
            started: None,
            moved: 0,
            continuing: false,
            sent: false,
            gather: false,
            took: false,
        }
    }

    /// Every card has left for the stack.
    pub fn done(&self) -> bool {
        self.moved >= self.plays.len()
    }

    /// Whether `play` has left the trick.
    pub fn has_moved(&self, play: &Play) -> bool {
        self.plays.iter().take(self.moved).any(|p| p == play)
    }

    /// The gather is under way and Continue has not been answered (`Collecting… →`).
    pub fn collecting(&self) -> bool {
        self.continuing && !self.sent
    }

    /// When the last card lands: its send plus one `SWEEP` flight (09 §4.4), or the start
    /// when the sweep is instant (reduce motion, Skim).
    pub fn finish_at(&self, instant: bool) -> Option<f64> {
        let started = self.started?;
        if instant {
            return Some(started);
        }
        Some(started + self.last_send() + motion::SWEEP_SECS)
    }

    /// The last card's send, seconds after the sweep starts.
    fn last_send(&self) -> f64 {
        self.plays.len().saturating_sub(1) as f64 * timing::COLLECT_STAGGER
    }

    fn restore(&mut self) {
        self.started = None;
        self.moved = 0;
        self.continuing = false;
        self.sent = false;
        self.gather = false;
        self.took = false;
    }
}

/// An outcome banner (06 §9).
#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    /// Bumps per banner, so a new one remounts.
    pub id: u64,
    /// The line after the star.
    pub text: String,
    /// Confetti falls with it.
    pub confetti: bool,
    /// When it first showed on the frame clock (stamped lazily; the confetti's `t0`).
    pub since: Option<f64>,
    /// When it leaves.
    pub until: Option<f64>,
    /// When the confetti leaves.
    pub confetti_until: Option<f64>,
    /// The match's last banner (`GameWon`): it waits for the match-end panel, which shows its
    /// line, and its confetti rises from behind that panel (09 §4.10).
    pub match_end: bool,
}

/// A sound cue for the sound lane (06 §15; D14: off by default, nothing plays here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// A fresh hand dealt into the rack.
    Deal,
    /// A card landed in the trick (06 §15.2 `playPlace(cardId)`: the card sets the thud's pitch).
    Place(Card),
    /// A trick gathered to its winner.
    TrickTake,
    /// Trump was named.
    TrumpCalled(Suit),
    /// A march or a loner.
    March,
    /// The makers were euchred.
    Euchre,
    /// Any other scored hand.
    HandWon,
    /// The match ended.
    GameWon,
    /// A tap on an unavailable card.
    Error,
}

/// The table's open panels, the rules search and the queues the sound lane and the
/// screen-reader announcer drain.
#[derive(Clone, Debug, Default)]
pub struct Hooks {
    /// Settings rail (06 §5.9) open.
    pub settings_open: bool,
    /// Rules dialog (06 §5.10) open.
    pub rules_open: bool,
    /// First-run tour step (06 §14.3), when it shows.
    pub tour_step: Option<u8>,
    /// Hand review dialog (06 §11) open.
    pub hand_review_open: bool,
    /// The settings rail's Table talk disclosure is open.
    pub talk_open: bool,
    /// Coach hints (06 §13) on.
    pub hints: bool,
    /// Sound cues waiting for the sound lane (newest last; capped).
    pub sound: VecDeque<Cue>,
    /// Steps waiting for the screen-reader announcer (06 §10; newest last; capped).
    pub announce: VecDeque<Step>,
    /// Seats a presence event reported offline, by seat index (no UI yet).
    pub offline: [bool; 4],
    /// The rules dialog's search text (06 §5.10; Lane C draws the box).
    pub rules_query: String,
    /// Rules accordion sections the player has toggled away from Lane C's default, one bit per
    /// section (bit `n` = section `n` of the 8).
    pub rules_flipped: u8,
}

/// The most queued hook items kept before the oldest drop.
const HOOK_QUEUE: usize = 16;
/// The most step keys remembered for dedupe (06 §9).
const SEEN_STEPS: usize = 80;

impl Hooks {
    fn cue(&mut self, cue: Cue) {
        if self.sound.len() >= HOOK_QUEUE {
            self.sound.pop_front();
        }
        self.sound.push_back(cue);
    }

    fn announce(&mut self, step: &Step) {
        if self.announce.len() >= HOOK_QUEUE {
            self.announce.pop_front();
        }
        self.announce.push_back(step.clone());
    }
}

/// What a poll asks the screen to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PollOut {
    /// The picture changed.
    pub changed: bool,
    /// The next deadline on the frame clock.
    pub wake: Option<f64>,
    /// Send `reviewTrick({action: "continue", turnId})`.
    pub continue_turn: Option<String>,
    /// A poll-driven pose is live: wake for the next frame (`tx.wake_after(ZERO)`).
    pub frame: bool,
    /// This commit sends sweep cards: their matched flights draw outside every clip
    /// (`tx.unclipped_flights()`, 09 §4.4).
    pub unclipped: bool,
    /// This commit lands in place (`tx.instant()`): a Skim sweep in one commit (09 §4.4).
    pub instant: bool,
}

/// The header score as shown, stepping toward the view's score (09 §4.8; Lane C draws it with
/// `RollingText`): one point per `motion::SCORE_STEP` once the hand's `Score` shot starts. The
/// first view, a snap and reduce motion show the target at once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScoreRoll {
    /// The score the header shows now, by team (`[us, them]` as the view orders it).
    pub shown: [u8; 2],
    /// The view's score.
    pub target: [u8; 2],
    /// When `shown` takes its next step on the frame clock (`motion::SCORE_STEP` apart).
    pub next_step: Option<f64>,
    /// When the last step landed: the `PULSE` ring and `NUM_POP` fire from here.
    pub landed_at: Option<f64>,
}

/// How long `b`'s confetti waits after the banner shows: the match's waits for its panel to
/// land (09 §4.10), every other burst starts with its banner.
fn confetti_delay(b: &Banner) -> f64 {
    if b.match_end {
        f64::from(motion::CONFETTI_DELAY)
    } else {
        0.0
    }
}

/// The table's presentation state.
#[derive(Clone, Debug, Default)]
pub struct Presentation {
    /// Rack slots.
    pub rack: RackSlots,
    /// The illegal-card message and when it leaves (06 §5.7).
    pub illegal: Option<(String, Option<f64>)>,
    /// The go-alone box is ticked for this decision (`{handNo}:{phase}`).
    pub go_alone: Option<(u32, GamePhase)>,
    /// The trick gather.
    pub collect: Option<Collect>,
    /// The banner on screen.
    pub banner: Option<Banner>,
    /// The hand whose cards deal in (never the first view, never the discard).
    pub deal_in: Option<u32>,
    /// The rack card under the pointer.
    pub hover: Option<Card>,
    /// Seams for later lanes.
    pub hooks: Hooks,
    /// The choreography queue (09 §2.2): what flies, what waits, and at what feel.
    pub director: Director,
    /// The card that shakes after an illegal press, and when it started (09 §4.7; stamped
    /// lazily, cleared after `motion::SHAKE`). Lane C draws it.
    pub shake: Option<(Card, Option<f64>)>,
    /// When the open dialog (rules, hand review or tour) opened, on the frame clock; `None`
    /// while none is open. For poll-driven scrim or panel progress.
    pub dialog_since: Option<f64>,
    /// The header score roll (09 §4.8).
    pub score: ScoreRoll,
    /// The deck tile's last laid-out window rect, for the deal from the deck (09 §4.1). Set by
    /// `AppState::deck_laid_out` from the deck's `on_layout` (Lane B wires it).
    pub deck_at: Option<[f32; 4]>,
    /// The screen-reader line the announcer last made (06 §10): a bumped id and the text.
    pub announced: Option<(u64, String)>,
    /// The next change lands in place (a snapping sync): nothing new flies.
    quiet: bool,
    /// Reduce motion at the last poll.
    reduced: bool,
    /// The hand the score belongs to.
    score_hand: u32,
    /// The current trick's plays at the last change, so a new play of yours (the store's
    /// optimistic preview) flies from the rack at once.
    seen_plays: Vec<ShotKey>,
    seen_view: bool,
    seen_steps: VecDeque<String>,
    next_banner: u64,
    /// The displayed match is won (the match-end panel's game): its last trick sweeps at once
    /// when no `MatchEnd` waits for it.
    match_over: bool,
}

impl Presentation {
    /// Follows the store after any change to it.
    pub fn after_change(&mut self, store: &TableStore) {
        let Some(view) = store.display_view() else {
            return;
        };
        let first = !self.seen_view;
        self.seen_view = true;
        self.match_over = view.status == GameStatus::Complete;
        let quiet = first || std::mem::take(&mut self.quiet);
        // A live deal usually lands as a new hand number with an empty rack, then the cards a
        // step later, so "cards arrived in an empty rack" deals in as well as a new hand does.
        let was_empty = self.rack.present().next().is_none();
        let new_hand = self.rack.reconcile(view.hand_no, &view.hand);
        if new_hand {
            self.collect = None;
            self.go_alone = None;
            self.illegal = None;
            self.hover = None;
            self.deal_in = None;
        }
        if (new_hand || was_empty)
            && !first
            && view.phase != GamePhase::DealerDiscard
            && !view.hand.is_empty()
            && self.deal_in != Some(view.hand_no)
        {
            self.deal_in = Some(view.hand_no);
            self.hooks.cue(Cue::Deal);
        }
        if self
            .go_alone
            .is_some_and(|key| key != (view.hand_no, view.phase))
        {
            self.go_alone = None;
        }
        self.score.target = view.score;
        self.score_hand = view.hand_no;
        if quiet || self.reduced {
            self.score.shown = view.score;
            self.score.next_step = None;
        }
        let key = (view.hand_no, view.trick.index);
        if let Some(winner) = view.trick.winner_seat
            && self
                .collect
                .as_ref()
                .is_none_or(|c| (c.hand_no, c.index) != key)
        {
            self.collect = Some(Collect::new(view, winner));
        }
        // The table re-turns the trick when it opens it for review: answer the newest turn.
        if let Some(c) = &mut self.collect
            && (c.hand_no, c.index) == key
            && view.trick.winner_seat.is_some()
            && !c.sent
        {
            c.turn_id.clone_from(&view.turn_id);
            c.review = copy::review_mode(view);
        }
        // A trick that moved on before its gather finished gathers in this commit.
        if let Some(c) = &mut self.collect
            && ((c.hand_no, c.index) != key || view.trick.winner_seat.is_none())
            && !c.done()
        {
            c.moved = c.plays.len();
        }
        for step in store.last_steps() {
            let id = format!("{}:{step:?}", view.hand_no);
            if self.seen_steps.contains(&id) {
                continue;
            }
            if self.seen_steps.len() >= SEEN_STEPS {
                self.seen_steps.pop_front();
            }
            self.seen_steps.push_back(id);
            self.on_step(view, step);
        }
        self.follow_plays(view, quiet);
        if quiet {
            self.director.clear();
            self.deal_in = None;
        }
    }

    /// Lands the next change in place (a snapping sync, a resync, a reconnect; 09 §2.4):
    /// everything queued or flying lands now and nothing in the next change flies.
    pub fn snap(&mut self) {
        self.director.clear();
        self.quiet = true;
        self.score.shown = self.score.target;
        self.score.next_step = None;
    }

    /// Your plays in the displayed trick: a new one (the store's optimistic preview, or a
    /// step) flies from the rack now; one whose card went back to the rack (refused) stops.
    fn follow_plays(&mut self, view: &PublicGameView, quiet: bool) {
        for key in self.director.plays_back_in(view.you, &view.hand) {
            self.director.cancel(&key);
        }
        let keys: Vec<ShotKey> = view
            .trick
            .plays
            .iter()
            .map(|p| ShotKey::play(view.hand_no, view.trick.index, p.seat))
            .collect();
        if !quiet {
            for (play, key) in view.trick.plays.iter().zip(&keys) {
                if play.seat == view.you
                    && !self.seen_plays.contains(key)
                    && !self.director.knows(key)
                {
                    self.director.enqueue(Shot::Play(PlayFlightSpec {
                        hand_no: view.hand_no,
                        index: view.trick.index,
                        seat: play.seat,
                        card: play.card,
                        mine: true,
                    }));
                }
            }
        }
        self.seen_plays = keys;
    }

    fn on_step(&mut self, view: &PublicGameView, step: &Step) {
        self.hooks.announce(step);
        for shot in Shot::from_step(view, step) {
            self.director.enqueue(shot);
        }
        // `Cue::Place` waits for the card's touchdown and `Cue::TrickTake` for the sweep's
        // last landing (09 §4.3, §4.4; `poll`).
        match step {
            Step::TrumpSet {
                suit, alone_seat, ..
            } => {
                self.hooks.cue(Cue::TrumpCalled(*suit));
                if alone_seat.is_some() {
                    self.hooks.cue(Cue::March);
                }
            }
            Step::HandScored { result, .. } => self.hooks.cue(match result {
                HandResult::LoneMarch | HandResult::March => Cue::March,
                HandResult::Euchre => Cue::Euchre,
                HandResult::Point | HandResult::LonePoint | HandResult::ThrowIn => Cue::HandWon,
            }),
            Step::GameWon { .. } => self.hooks.cue(Cue::GameWon),
            _ => {}
        }
        if let Some(b) = copy::banner_for(view, step) {
            self.next_banner += 1;
            self.banner = Some(Banner {
                id: self.next_banner,
                text: b.text,
                confetti: b.confetti,
                since: None,
                until: None,
                confetti_until: None,
                match_end: matches!(step, Step::GameWon { .. }),
            });
        }
    }

    /// Shows the reason `card` cannot be played (06 §5.7) for [`timing::ILLEGAL`].
    pub fn show_illegal(&mut self, reason: String) {
        self.illegal = Some((reason, None));
        self.hooks.cue(Cue::Error);
    }

    /// Starts `card`'s illegal shake (09 §4.7); its start is stamped on the next poll.
    pub fn shake_card(&mut self, card: Card) {
        self.shake = Some((card, None));
    }

    /// The shaking card and seconds into its shake, while it shakes.
    pub fn shake_t(&self, now: f64) -> Option<(Card, f32)> {
        let (card, t0) = self.shake?;
        let t = now - t0?;
        (t < motion::SHAKE).then_some((card, t.max(0.0) as f32))
    }

    /// How far the open dialog has come in over `secs`: 0..=1, `None` while none is open.
    pub fn dialog_progress(&self, now: f64, secs: f64) -> Option<f32> {
        let since = self.dialog_since?;
        Some((((now - since) / secs.max(1e-6)) as f32).clamp(0.0, 1.0))
    }

    /// Records a new screen-reader line (06 §10) under a fresh id, so a repeat is re-read.
    pub fn announce_line(&mut self, text: String) {
        let id = self.announced.as_ref().map_or(1, |(id, _)| id + 1);
        self.announced = Some((id, text));
    }

    /// Continue (06 §5.4.5): gather now, then answer the trick once the cards land.
    pub fn press_continue(&mut self) -> bool {
        match &mut self.collect {
            Some(c) if c.review && !c.continuing => {
                c.continuing = true;
                true
            }
            _ => false,
        }
    }

    /// The review request failed or was refused: put the trick back and let Continue retry.
    pub fn review_failed(&mut self) {
        if let Some(c) = &mut self.collect {
            c.restore();
        }
    }

    /// Whether `view`'s trick winner shows (ring, pop, outline, seat plate): once its `Win`
    /// shot has started, which waits for the trick's last card to touch down (09 §4.4). A
    /// winner the Director never saw (a first view, a snap) shows at once.
    pub fn winner_shown(&self, view: &PublicGameView) -> bool {
        view.trick.winner_seat.is_some()
            && !self
                .director
                .pending(&ShotKey::win(view.hand_no, view.trick.index))
    }

    /// Whether `view`'s trick cards gather toward the winner (09 §4.4, from `GATHER_LEAD`
    /// before the sweep; never under reduce motion or a Brisk or Skim win).
    pub fn gathering(&self, view: &PublicGameView) -> bool {
        self.collect_for(view).is_some_and(|c| c.gather)
    }

    /// Seconds into the confetti burst, while it runs (09 §4.9; `None` under reduce motion),
    /// and whether it is the match's (it rises from behind the match-end panel, which it
    /// waits [`motion::CONFETTI_DELAY`] for).
    pub fn confetti_t(&self, now: f64) -> Option<(f32, bool)> {
        let b = self
            .banner
            .as_ref()
            .filter(|b| b.confetti && !self.reduced)?;
        let t = now - b.since? - confetti_delay(b);
        (0.0..motion::CONFETTI_SECS)
            .contains(&t)
            .then_some((t as f32, b.match_end))
    }

    /// Whether the match-end panel shows (09 §4.10): the match is won and its `MatchEnd`
    /// shot has started, which waits for the last trick's sweep and the hand's score roll.
    pub fn match_end_shown(&self, view: &PublicGameView) -> bool {
        view.status == GameStatus::Complete
            && view.winner_team.is_some()
            && !self.director.pending(&ShotKey::match_end())
    }

    /// Seconds into the match-end moment (the panel's score roll and pop), while its shot
    /// runs at a feel that animates.
    pub fn match_end_t(&self, now: f64) -> Option<f32> {
        let key = ShotKey::match_end();
        if self.reduced || self.director.feel_of(&key) == Some(Feel::Snap) {
            return None;
        }
        self.director.elapsed(&key, now)
    }

    /// Whether hand `hand_no`'s deal is still on its way (queued or flying): the seats and
    /// the hand area hold their turn and bid status until the cards land (09 §4.1).
    pub fn dealing(&self, hand_no: u32) -> bool {
        let key = ShotKey::deal(hand_no);
        !self.reduced && (self.director.pending(&key) || self.director.started(&key).is_some())
    }

    /// The trick gather for the trick in `view`, if it is that one.
    pub fn collect_for(&self, view: &PublicGameView) -> Option<&Collect> {
        self.collect
            .as_ref()
            .filter(|c| (c.hand_no, c.index) == (view.hand_no, view.trick.index))
    }

    /// Whether `seat` has taken a trick this hand (its stack shows).
    pub fn has_stack(&self, view: &PublicGameView, seat: Seat) -> bool {
        view.trick_log.iter().any(|t| t.winner_seat == Some(seat))
            || self
                .collect
                .as_ref()
                .is_some_and(|c| c.hand_no == view.hand_no && c.winner == seat && c.moved > 0)
    }

    /// Whether the go-alone box is ticked for `view`'s decision.
    pub fn go_alone_on(&self, view: &PublicGameView) -> bool {
        self.go_alone == Some((view.hand_no, view.phase))
    }

    /// Ticks or clears the go-alone box for `view`'s decision.
    pub fn toggle_go_alone(&mut self, view: &PublicGameView) {
        self.go_alone = if self.go_alone_on(view) {
            None
        } else {
            Some((view.hand_no, view.phase))
        };
    }

    /// Advances the timed pieces to `now` (frame clock, seconds).
    pub fn poll(&mut self, now: f64, reduced: bool) -> PollOut {
        self.reduced = reduced;
        // The sweep's due time is stamped before the Director polls, so a hand's `Score` that
        // arrives with its last trick still waits for that trick's sweep. A won match whose
        // `MatchEnd` does not wait (a first view, a snap) sweeps its last trick at once,
        // review or not, so the panel never sits over unswept cards.
        let match_now = self.match_over && !self.director.pending(&ShotKey::match_end());
        if let Some(c) = &mut self.collect
            && (!c.review || match_now)
            && c.started.is_none()
            && !c.done()
        {
            c.review &= !match_now;
            let due = c.due.get_or_insert(now + timing::COLLECT_AFTER);
            if match_now {
                *due = due.min(now);
            }
        }
        let sweep_until = self.sweep_until(reduced);
        self.director.set_sweep_until(sweep_until);
        let mut out = self.director.poll(now, reduced);
        let wake = |at: f64, out: &mut PollOut| {
            out.wake = Some(out.wake.map_or(at, |w: f64| w.min(at)));
        };
        // A deal that lands in place (a snap, a skimmed backlog) does not deal in.
        if let Some(h) = self.deal_in
            && self.director.feel_of(&ShotKey::deal(h)) == Some(Feel::Snap)
        {
            self.deal_in = None;
            out.changed = true;
        }
        // One thud per poll, pitched by the last card that landed in it.
        if let Some(card) = self.director.take_touchdowns().pop() {
            self.hooks.cue(Cue::Place(card));
        }
        if let Some((_, t0)) = &mut self.shake {
            let start = *t0.get_or_insert(now);
            if reduced || now >= start + motion::SHAKE {
                self.shake = None;
            } else {
                out.frame = true;
                out.changed = true;
            }
        }
        let dialog =
            self.hooks.rules_open || self.hooks.hand_review_open || self.hooks.tour_step.is_some();
        if !dialog {
            self.dialog_since = None;
        } else if self.dialog_since.is_none() {
            self.dialog_since = Some(now);
        }
        if let Some((_, until)) = &mut self.illegal {
            let due = *until.get_or_insert(now + timing::ILLEGAL);
            if now >= due {
                self.illegal = None;
                out.changed = true;
            } else {
                wake(due, &mut out);
            }
        }
        let match_waits = self.director.pending(&ShotKey::match_end());
        if let Some(b) = self
            .banner
            .as_mut()
            .filter(|b| !(b.match_end && match_waits))
        {
            let since = *b.since.get_or_insert(now);
            let until = *b.until.get_or_insert(since + timing::BANNER);
            let confetti_until = *b
                .confetti_until
                .get_or_insert(since + confetti_delay(b) + motion::CONFETTI_SECS);
            if b.confetti && now >= confetti_until {
                b.confetti = false;
                out.changed = true;
            } else if b.confetti && !reduced {
                // The burst is posed from the clock (09 §4.9).
                out.frame = true;
                out.changed = true;
            } else if b.confetti {
                wake(confetti_until, &mut out);
            }
            if now >= until {
                self.banner = None;
                out.changed = true;
            } else {
                wake(until, &mut out);
            }
        }
        if let Some(c) = &mut self.collect {
            let win = ShotKey::win(c.hand_no, c.index);
            let win_pending = self.director.pending(&win);
            let win_feel = self.director.feel_of(&win);
            // A match-now sweep lands without flights: the match-end panel is already
            // entering, and cards must not fly across it (09 §4.10).
            let full =
                !reduced && !match_now && !win_pending && win_feel.is_none_or(|f| f == Feel::Full);
            let instant = reduced || match_now || win_feel == Some(Feel::Skim);
            if !c.review && c.started.is_none() && !c.done() {
                let due = *c.due.get_or_insert(now + timing::COLLECT_AFTER);
                if full && !c.gather {
                    let lead = due - motion::GATHER_LEAD;
                    if now >= lead {
                        c.gather = true;
                        out.changed = true;
                    } else {
                        wake(lead, &mut out);
                    }
                }
                if now >= due {
                    c.started = Some(due.min(now));
                } else {
                    wake(due, &mut out);
                }
            }
            if c.continuing && c.started.is_none() {
                c.started = Some(now);
                c.gather = full;
            }
            if let Some(started) = c.started
                && !c.done()
            {
                let target = if instant {
                    c.plays.len()
                } else {
                    let elapsed = ((now - started) / timing::COLLECT_STAGGER).floor().max(0.0);
                    ((elapsed as usize) + 1).min(c.plays.len())
                };
                if target > c.moved {
                    c.moved = target;
                    out.changed = true;
                    if instant {
                        out.instant |= !reduced;
                    } else {
                        out.unclipped = true;
                    }
                }
                if !c.done() {
                    wake(started + c.moved as f64 * timing::COLLECT_STAGGER, &mut out);
                }
            }
            if c.done()
                && !c.took
                && let Some(landed) = c.finish_at(instant)
            {
                if now >= landed {
                    c.took = true;
                    self.hooks.cue(Cue::TrickTake);
                } else {
                    wake(landed, &mut out);
                }
            }
            if c.continuing
                && !c.sent
                && c.done()
                && let Some(finish) = c.finish_at(instant)
            {
                if now >= finish {
                    c.sent = true;
                    out.continue_turn = Some(c.turn_id.clone());
                    out.changed = true;
                } else {
                    wake(finish, &mut out);
                }
            }
        }
        self.roll_score(now, reduced, &mut out);
        if out.frame {
            out.changed = true;
        }
        out
    }

    /// When the current sweep's last card lands, or will (from its due time), while it has
    /// not: the hand's `Score` shot waits for it.
    fn sweep_until(&self, reduced: bool) -> Option<f64> {
        let c = self.collect.as_ref().filter(|c| !c.took)?;
        if reduced {
            return None;
        }
        c.finish_at(false)
            .or_else(|| c.due.map(|due| due + c.last_send() + motion::SWEEP_SECS))
    }

    /// Steps the header score one point per `SCORE_STEP` toward the view's score, once the
    /// hand's `Score` shot has started (09 §4.8).
    fn roll_score(&mut self, now: f64, reduced: bool, out: &mut PollOut) {
        let s = &mut self.score;
        if s.shown == s.target {
            s.next_step = None;
            return;
        }
        if reduced {
            s.shown = s.target;
            s.next_step = None;
            s.landed_at = Some(now);
            out.changed = true;
            return;
        }
        if self.director.pending(&ShotKey::score(self.score_hand)) {
            return;
        }
        let mut at = *s.next_step.get_or_insert(now);
        while now >= at && s.shown != s.target {
            for (shown, target) in s.shown.iter_mut().zip(s.target) {
                if *shown < target {
                    *shown += 1;
                } else {
                    // A new match (or a correction) never rolls backwards.
                    *shown = target;
                }
            }
            at += motion::SCORE_STEP;
            out.changed = true;
        }
        if s.shown == s.target {
            s.next_step = None;
            s.landed_at = Some(now);
        } else {
            s.next_step = Some(at);
            out.wake = Some(out.wake.map_or(at, |w| w.min(at)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> Card {
        Card::parse(id).expect("a card id")
    }

    #[test]
    fn slots_keep_their_places() {
        let (a, b, c) = (card("JS"), card("AH"), card("9D"));
        let mut rack = RackSlots::default();
        assert!(rack.reconcile(1, &[a, b, c]));
        assert!(!rack.reconcile(1, &[a, c]));
        assert_eq!(rack.card_at(1), None, "a played card leaves a hole");
        assert_eq!(rack.card_at(2), Some(c), "the rest never shift");
        let up = card("KC");
        rack.reconcile(1, &[a, c, up]);
        assert_eq!(rack.card_at(3), Some(up), "a picked-up card appends");
        rack.reconcile(1, &[a, b, c, up]);
        assert_eq!(
            rack.card_at(1),
            Some(b),
            "a refused play returns to its slot"
        );
        assert!(rack.reconcile(2, &[c]));
        assert_eq!(rack.slots.len(), 1, "a new hand resets");
    }

    #[test]
    fn a_live_deal_into_an_empty_rack_deals_in_once() {
        let (sync, _) = crate::scenes::drive_core("deal-in", false, |s| {
            s.view.phase == GamePhase::BidRound1 && !s.view.hand.is_empty()
        })
        .expect("a dealt hand");
        let mut store = TableStore::new();
        let _ = store.attach(true, 0);
        let mut p = Presentation::default();
        let deals = |p: &Presentation| p.hooks.sound.iter().filter(|c| **c == Cue::Deal).count();

        store.on_sync_event(sync.clone());
        p.after_change(&store);
        assert_eq!(p.deal_in, None, "the first view never deals in");

        // The next hand arrives as a number with an empty rack, then the cards a step later.
        let next = sync.view.hand_no + 1;
        let mut empty = sync.clone();
        empty.v += 1;
        empty.view.v = empty.v;
        empty.view.hand_no = next;
        empty.view.phase = GamePhase::Deal;
        empty.view.hand.clear();
        store.on_sync_event(empty);
        p.after_change(&store);
        assert_eq!(p.deal_in, None, "nothing to deal yet");

        let mut dealt = sync.clone();
        dealt.v += 2;
        dealt.view.v = dealt.v;
        dealt.view.hand_no = next;
        store.on_sync_event(dealt.clone());
        p.after_change(&store);
        assert_eq!(
            p.deal_in,
            Some(next),
            "cards landing in an empty rack deal in"
        );
        assert_eq!(deals(&p), 1);

        let mut played = dealt;
        played.v += 1;
        played.view.v = played.v;
        played.view.hand.pop();
        store.on_sync_event(played);
        p.after_change(&store);
        assert_eq!(deals(&p), 1, "a play inside the hand never deals again");
    }

    #[test]
    fn illegal_message_lasts_its_time() {
        let mut p = Presentation::default();
        p.show_illegal("It's not your turn yet.".to_owned());
        assert_eq!(p.poll(10.0, false).wake, Some(10.0 + timing::ILLEGAL));
        assert!(p.illegal.is_some());
        assert!(p.poll(10.0 + timing::ILLEGAL, false).changed);
        assert!(p.illegal.is_none());
        assert_eq!(p.hooks.sound.back(), Some(&Cue::Error));
    }
}
