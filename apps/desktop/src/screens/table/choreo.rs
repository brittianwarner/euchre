//! The choreography queue (09 §2.2-§2.5): sync steps become [`Shot`]s, the [`Director`] starts
//! them on the frame clock, and the views ask it what is flying, what has landed and how far a
//! shot has got.
//!
//! How it runs:
//!
//! - [`Director::enqueue`] queues a shot (deduped by its [`ShotKey`]). Your own card play
//!   starts at once, so the card that leaves the rack is in the air in the same commit.
//! - [`Director::poll`] (from `Presentation::poll`) picks the [`Feel`], starts every queued
//!   shot whose wait is met (strictly in order), and retires finished ones into `landed`.
//! - A view asks [`Director::flyers`] for the Full-tier play flyers (and resolves each with
//!   [`resolve_flight`] and [`play_pose`]), [`Director::play_slot`] for how a trick slot
//!   renders, [`Director::started`] / [`Director::progress`] for the other moments.
//!
//! Rules (09 §2.1): every pose is a pure function of a shot and `AppState.clock`; every start
//! time is the poll clock (lazily stamped in [`Director::poll`], or the last poll's clock for a
//! shot that starts on enqueue); nothing here reads a wall clock.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::fmt;

use euchre_protocol::{Card, HandResult, PublicGameView, Seat, Step, Suit};
use rune::Cx;

use super::present::PollOut;
use super::{hand, stage};
use crate::app::AppState;
use crate::theme::card;
use crate::theme::motion::{
    CONFETTI_DELAY, CONFETTI_SECS, FF_SECS, FLIP, FLIP_BRISK, FLIP_GLINT, MATCH_ROLL_AT, NUM_POP,
    PICKUP_LAND, PICKUP_TURN, PLAY_ARC_OPP, PLAY_ARC_SHARE, PLAY_ARC_YOU, PLAY_DRAG, PLAY_END,
    PLAY_LIFT_SCALE, PLAY_SKID, PLAY_SQUASH, PLAY_TRAVEL, RING_DELAY, RING_IN, ROLL, ROT_SETTLE,
    SPARKS, SQUASH_T, STAMP, STAMP_HOLD, TRUMP_FADE, TRUMP_IN, WIN_POP, beats_secs, deal_delay,
};

/// How much motion the queue can afford right now (09 §2.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Feel {
    /// Queue of at most one shot and the oldest waits under 0.5 s: everything in 09 §4.
    Full,
    /// Queue of 2-3 or the oldest waits 0.5-1.5 s: `*_BRISK` tokens, matched plays, no flips.
    Brisk,
    /// Queue of 4+ or the oldest waits over 1.5 s: only the last play of each trick flies.
    Skim,
    /// A snapping sync, a resync, a reconnect or reduce motion: everything lands at once. A
    /// shot that ran as `Snap` mounted in place.
    #[default]
    Snap,
}

/// How a trick slot renders while its play is choreographed (09 §4.3). Every trick slot also
/// carries `.transition(Transition::none())`, so a hand-off from a flyer never cross-fades.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaySlot {
    /// A Full-tier flyer owns the card (its Play is queued or flying): render no slot.
    Hidden,
    /// The default: `.matched(stage::play_id(..)).motion(motion::PLAY_BRISK)`. A Brisk or Skim
    /// play flies in from its fan back or rack card this way, and a landed card keeps the id so
    /// the sweep can fly it to the stack.
    Matched,
    /// Mounts at rest: no `.matched` (Skim's older plays, and plays that a fast-forward or
    /// reduce motion landed, for the commit they mount in).
    Still,
}

/// A shot's identity, stable across polls; views key and gate on it (`fly-{key}`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ShotKey(String);

impl ShotKey {
    /// The deal of hand `hand_no`.
    pub fn deal(hand_no: u32) -> ShotKey {
        ShotKey(format!("deal-{hand_no}"))
    }

    /// The up-card turn (or turn-down) of hand `hand_no`.
    pub fn flip(hand_no: u32) -> ShotKey {
        ShotKey(format!("flip-{hand_no}"))
    }

    /// `seat`'s bid bubble for bid `id` in hand `hand_no`.
    pub fn bubble(hand_no: u32, seat: Seat, id: &str) -> ShotKey {
        ShotKey(format!("bubble-{hand_no}-{}-{id}", seat.index()))
    }

    /// Trump named in hand `hand_no`.
    pub fn trump(hand_no: u32) -> ShotKey {
        ShotKey(format!("trump-{hand_no}"))
    }

    /// The dealer taking the up card in hand `hand_no`.
    pub fn pickup(hand_no: u32) -> ShotKey {
        ShotKey(format!("pickup-{hand_no}"))
    }

    /// `seat`'s card in trick `index` of hand `hand_no`.
    pub fn play(hand_no: u32, index: u8, seat: Seat) -> ShotKey {
        ShotKey(format!("play-{hand_no}-{index}-{}", seat.index()))
    }

    /// Trick `index` of hand `hand_no` won (ring, pop, gather).
    pub fn win(hand_no: u32, index: u8) -> ShotKey {
        ShotKey(format!("win-{hand_no}-{index}"))
    }

    /// Hand `hand_no` scored (result card, chip, score roll).
    pub fn score(hand_no: u32) -> ShotKey {
        ShotKey(format!("score-{hand_no}"))
    }

    /// The euchre, march or lone-march stamp of hand `hand_no`.
    pub fn stamp(hand_no: u32) -> ShotKey {
        ShotKey(format!("stamp-{hand_no}"))
    }

    /// The match ending.
    pub fn match_end() -> ShotKey {
        ShotKey("match-end".to_owned())
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the Director knows about a card play; the view resolves it into a [`PlayFlight`] from
/// layout rects (`seat-{i}-fan` or `hole-{card}` to the trick slot, 09 §2.10).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayFlightSpec {
    /// The hand.
    pub hand_no: u32,
    /// The trick, 0-based.
    pub index: u8,
    /// Who played.
    pub seat: Seat,
    /// The card.
    pub card: Card,
    /// Whether it is your card (it leaves the rack, not a fan).
    pub mine: bool,
}

/// A play flyer's endpoints in window points, resolved by the view (09 §4.3).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PlayFlight {
    /// The source centre.
    pub from: [f32; 2],
    /// The trick slot's centre.
    pub to: [f32; 2],
    /// The arc: the flyer peaks `arc / 2` above the chord, half way through its travel
    /// (opponent 110, you 40).
    pub arc: f32,
    /// The source rotation, radians.
    pub r0: f32,
    /// The slot's rotation, radians.
    pub r1: f32,
    /// The source scale relative to the trick card.
    pub s0: f32,
    /// The trick card's size `[w, h]`: the flyer's size at scale 1.
    pub size: [f32; 2],
}

/// A flyer's pose at one moment (09 §4.3).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pose {
    /// Centre x, window points.
    pub x: f32,
    /// Centre y, window points.
    pub y: f32,
    /// Rotation, radians.
    pub rot: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Elevation for `motion::card_shadows`, 0 to 1.
    pub lift: f32,
}

/// One choreographed moment (09 §2.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Shot {
    /// The deal: 20 cards leave the deck in packets.
    Deal {
        /// The hand.
        hand_no: u32,
        /// Who dealt (cards go out from their left).
        dealer: Seat,
    },
    /// The up card turns face up, or (`down`) turns back down.
    Flip {
        /// The hand.
        hand_no: u32,
        /// The up card, when it is known.
        card: Option<Card>,
        /// Turned down after four passes.
        down: bool,
    },
    /// A bid bubble under a seat (opacity and offset only; runs alongside).
    Bubble {
        /// The hand.
        hand_no: u32,
        /// The bidder.
        seat: Seat,
        /// The bid's wire id.
        id: String,
        /// The verbatim table line.
        say: String,
    },
    /// Trump named: the badge, the maker chip.
    Trump {
        /// The hand.
        hand_no: u32,
        /// The suit.
        suit: Suit,
        /// The maker.
        maker: Seat,
        /// The loner, if any.
        alone: Option<Seat>,
    },
    /// The dealer takes the up card (matched; 09 §4.6).
    Pickup {
        /// The hand.
        hand_no: u32,
    },
    /// A card play.
    Play(PlayFlightSpec),
    /// A trick won.
    Win {
        /// The hand.
        hand_no: u32,
        /// The trick.
        index: u8,
        /// Who took it.
        seat: Seat,
    },
    /// The hand scored.
    Score {
        /// The hand.
        hand_no: u32,
        /// The table row.
        result: HandResult,
        /// Points by team.
        delta: [u8; 2],
        /// The new match score.
        score: [u8; 2],
    },
    /// A euchre, march or lone-march stamp.
    Stamp {
        /// The hand.
        hand_no: u32,
        /// The row it stamps.
        result: HandResult,
    },
    /// The match ended.
    MatchEnd {
        /// Your team won.
        you_won: bool,
    },
}

impl Shot {
    /// This shot's key.
    pub fn key(&self) -> ShotKey {
        match self {
            Shot::Deal { hand_no, .. } => ShotKey::deal(*hand_no),
            Shot::Flip { hand_no, .. } => ShotKey::flip(*hand_no),
            Shot::Bubble {
                hand_no, seat, id, ..
            } => ShotKey::bubble(*hand_no, *seat, id),
            Shot::Trump { hand_no, .. } => ShotKey::trump(*hand_no),
            Shot::Pickup { hand_no } => ShotKey::pickup(*hand_no),
            Shot::Play(p) => ShotKey::play(p.hand_no, p.index, p.seat),
            Shot::Win { hand_no, index, .. } => ShotKey::win(*hand_no, *index),
            Shot::Score { hand_no, .. } => ShotKey::score(*hand_no),
            Shot::Stamp { hand_no, .. } => ShotKey::stamp(*hand_no),
            Shot::MatchEnd { .. } => ShotKey::match_end(),
        }
    }

    /// The hand the shot belongs to (the match end belongs to none).
    fn hand_no(&self) -> Option<u32> {
        match self {
            Shot::Deal { hand_no, .. }
            | Shot::Flip { hand_no, .. }
            | Shot::Bubble { hand_no, .. }
            | Shot::Trump { hand_no, .. }
            | Shot::Pickup { hand_no }
            | Shot::Win { hand_no, .. }
            | Shot::Score { hand_no, .. }
            | Shot::Stamp { hand_no, .. } => Some(*hand_no),
            Shot::Play(p) => Some(p.hand_no),
            Shot::MatchEnd { .. } => None,
        }
    }

    /// The shots one sync step asks for (09 §2.2 mapping); `Cut` and `ThrowIn` ask for none.
    pub fn from_step(view: &PublicGameView, step: &Step) -> Vec<Shot> {
        let hand_no = view.hand_no;
        match step {
            Step::Dealt { .. } => vec![Shot::Deal {
                hand_no,
                dealer: view.dealer_seat,
            }],
            Step::UpCardTurned { card } => vec![Shot::Flip {
                hand_no,
                card: Some(*card),
                down: false,
            }],
            Step::TurnedDown { .. } => vec![Shot::Flip {
                hand_no,
                card: None,
                down: true,
            }],
            Step::Bid { seat, id, say } => vec![Shot::Bubble {
                hand_no,
                seat: *seat,
                id: id.clone(),
                say: say.clone(),
            }],
            Step::TrumpSet {
                suit,
                maker_seat,
                alone_seat,
                via_order_up,
                ..
            } => {
                let mut shots = vec![Shot::Trump {
                    hand_no,
                    suit: *suit,
                    maker: *maker_seat,
                    alone: *alone_seat,
                }];
                if *via_order_up {
                    shots.push(Shot::Pickup { hand_no });
                }
                shots
            }
            Step::DealerDiscarded { .. } => Vec::new(),
            Step::CardPlayed { seat, card } => vec![Shot::Play(PlayFlightSpec {
                hand_no,
                index: trick_of(view, *seat, *card),
                seat: *seat,
                card: *card,
                mine: *seat == view.you,
            })],
            Step::TrickWon {
                index, winner_seat, ..
            } => vec![Shot::Win {
                hand_no,
                index: *index,
                seat: *winner_seat,
            }],
            Step::HandScored {
                result,
                delta,
                score,
                ..
            } => {
                let mut shots = vec![Shot::Score {
                    hand_no,
                    result: *result,
                    delta: *delta,
                    score: *score,
                }];
                if matches!(
                    result,
                    HandResult::Euchre | HandResult::March | HandResult::LoneMarch
                ) {
                    shots.push(Shot::Stamp {
                        hand_no,
                        result: *result,
                    });
                }
                shots
            }
            Step::GameWon { team, .. } => vec![Shot::MatchEnd {
                you_won: team.index() == view.you.index() % 2,
            }],
            Step::Cut { .. } | Step::ThrowIn => Vec::new(),
        }
    }
}

/// The trick a play belongs to: the current trick if it holds the card, else the logged trick
/// that does, else the current index.
fn trick_of(view: &PublicGameView, seat: Seat, card: Card) -> u8 {
    let holds =
        |plays: &[euchre_protocol::Play]| plays.iter().any(|p| p.seat == seat && p.card == card);
    if holds(&view.trick.plays) {
        return view.trick.index;
    }
    view.trick_log
        .iter()
        .find(|t| holds(&t.plays))
        .map_or(view.trick.index, |t| t.index)
}

/// The whole deal: the last card's delay plus its 0.5 s presence spring (09 §4.1).
fn deal_secs() -> f64 {
    f64::from(deal_delay(19)) + 0.5
}
/// The Flip starts this long before the deal ends (09 §2.2).
const FLIP_OVERLAP: f64 = 0.10;
/// A bid's shot lands at once: the seat's detail line says "Passed" (and its banter bubble
/// owns the spot under the name), so no separate bid bubble draws (09 §4.6's 1.6 s chip).
const BUBBLE: f64 = 0.0;
/// A Brisk or Skim matched play at `PLAY_BRISK`; it touches down at 0.6 of the travel.
const BRISK_PLAY: f64 = 0.55;
/// Brisk durations are this share of Full ones (09 §2.3).
const BRISK: f64 = 0.6;
/// The hand-result card entering.
const SCORE: f64 = 1.0;
/// Tier thresholds (09 §2.3): the most queued shots for Full and for Brisk, and the waits.
const FULL_MAX: usize = 1;
const BRISK_MAX: usize = 3;
const FULL_WAIT: f64 = 0.5;
const BRISK_WAIT: f64 = 1.5;

/// A shot waiting to start.
#[derive(Clone, PartialEq, Debug)]
struct Queued {
    shot: Shot,
    key: ShotKey,
    /// When a poll first saw it queued.
    since: Option<f64>,
}

/// A started shot.
#[derive(Clone, PartialEq, Debug)]
pub struct Running {
    /// The shot.
    pub shot: Shot,
    /// Its key, kept so views can borrow it.
    pub key: ShotKey,
    /// When it started on the frame clock.
    pub t0: Option<f64>,
    /// How long it runs at the feel it started with, seconds.
    pub dur: f64,
    /// The feel it started with.
    pub feel: Feel,
    /// Fast-forward: when it began and the progress it began from.
    ff: Option<(f64, f32)>,
    /// A play has touched down (its `Cue::Place` is out).
    touched: bool,
}

impl Running {
    fn new(shot: Shot, key: ShotKey, t0: f64, feel: Feel) -> Running {
        let dur = duration(&shot, feel);
        Running {
            shot,
            key,
            t0: Some(t0),
            dur,
            feel,
            ff: None,
            touched: false,
        }
    }

    /// 0..=1. Fast-forward remaps the rest of the shot into `FF_SECS`, continuously.
    pub fn progress(&self, now: f64) -> f32 {
        let Some(t0) = self.t0 else { return 0.0 };
        if self.dur <= 0.0 {
            return 1.0;
        }
        let raw = (((now - t0) / self.dur) as f32).clamp(0.0, 1.0);
        match self.ff {
            None => raw,
            Some((at, p)) => {
                (p + (1.0 - p) * (((now - at) / FF_SECS) as f32).clamp(0.0, 1.0)).min(1.0)
            }
        }
    }

    /// Seconds into the shot's own timeline: progress times duration, so fast-forward runs the
    /// same pose faster, never jumping.
    pub fn elapsed(&self, now: f64) -> f32 {
        self.progress(now) * self.dur as f32
    }

    /// When progress reaches `p` (0..=1), fast-forward included.
    fn at_progress(&self, p: f32) -> f64 {
        let t0 = self.t0.unwrap_or(0.0);
        match self.ff {
            None => t0 + f64::from(p) * self.dur,
            Some((at, from)) if p > from => {
                at + f64::from((p - from) / (1.0 - from).max(1e-6)) * FF_SECS
            }
            Some((at, _)) => at,
        }
    }

    /// When the shot ends.
    fn end(&self) -> f64 {
        self.at_progress(1.0)
    }

    /// When a play touches down (the end of its travel); other shots: their end.
    fn touchdown(&self) -> f64 {
        match (&self.shot, self.feel) {
            (Shot::Play(_), Feel::Full) => self.at_progress(PLAY_TRAVEL / PLAY_END),
            (Shot::Play(_), Feel::Brisk | Feel::Skim) => {
                self.at_progress((f64::from(PLAY_TRAVEL) * BRISK / BRISK_PLAY) as f32)
            }
            _ => self.end(),
        }
    }

    /// Whether the view poses this shot from the clock every frame (mechanism A).
    fn poll_driven(&self, now: f64) -> bool {
        let t = now - self.t0.unwrap_or(now);
        match (&self.shot, self.feel) {
            (Shot::Play(_) | Shot::Flip { .. }, Feel::Full) => true,
            // The picked-up card shrinks to a back late in its flight, then turns (09 §4.6).
            (Shot::Pickup { .. }, f) => f != Feel::Snap,
            (Shot::Stamp { .. }, Feel::Full | Feel::Brisk) => t < f64::from(SPARKS),
            // The panel's final roll and pop, and on a win the confetti behind it.
            (Shot::MatchEnd { .. }, f) => f != Feel::Snap,
            _ => false,
        }
    }
}

/// How long `shot` runs at `feel`, seconds.
fn duration(shot: &Shot, feel: Feel) -> f64 {
    if feel == Feel::Snap {
        return 0.0;
    }
    let brisk = feel == Feel::Brisk;
    match shot {
        Shot::Deal { .. } => match feel {
            Feel::Full => deal_secs(),
            // Delays halved, the same last spring.
            Feel::Brisk => f64::from(deal_delay(19)) * 0.5 + 0.5,
            Feel::Skim | Feel::Snap => 0.0,
        },
        Shot::Flip { .. } => match feel {
            Feel::Full => f64::from(FLIP + FLIP_GLINT),
            Feel::Brisk => f64::from(FLIP_BRISK.0 + FLIP_BRISK.1),
            Feel::Skim | Feel::Snap => 0.0,
        },
        Shot::Bubble { .. } => BUBBLE,
        // The old words fade, then the line's `TRUMP_IN` (stage.rs `status_row`).
        Shot::Trump { .. } => f64::from(TRUMP_FADE) + beats_secs(TRUMP_IN),
        // The face-up flight at `PICKUP`, then its turn face down on the fan (stage.rs `fan`).
        Shot::Pickup { .. } => f64::from(PICKUP_LAND + PICKUP_TURN),
        Shot::Play(_) => match feel {
            Feel::Full => f64::from(PLAY_END),
            _ => BRISK_PLAY,
        },
        // The ring and the pop.
        Shot::Win { .. } => beats_secs(WIN_POP).max(beats_secs(RING_IN)),
        Shot::Score { .. } => SCORE * if brisk { BRISK } else { 1.0 },
        // The stamp's beat plus its hold.
        Shot::Stamp { .. } => STAMP_HOLD + beats_secs(STAMP),
        Shot::MatchEnd { you_won } => {
            let roll = f64::from(MATCH_ROLL_AT + ROLL.duration) + beats_secs(NUM_POP);
            if *you_won {
                roll.max(f64::from(CONFETTI_DELAY) + CONFETTI_SECS)
            } else {
                roll
            }
        }
    }
}

/// The most landed keys remembered: more than one hand's worth of shots.
const LANDED: usize = 64;

/// The choreography queue and the running shots (09 §2.2).
#[derive(Clone, Debug, Default)]
pub struct Director {
    /// Not started, in arrival order; each starts only after the one before it.
    queue: VecDeque<Queued>,
    /// Started; removed at their end.
    live: Vec<Running>,
    /// Finished, newest last, capped at [`LANDED`], with the feel each ran at.
    landed: VecDeque<(ShotKey, Feel)>,
    /// Plays that landed in place since the last poll: their slots mount [`PlaySlot::Still`].
    fresh: Vec<ShotKey>,
    /// The feel picked at the last poll.
    pub feel: Feel,
    /// The last poll's clock.
    now: f64,
    /// Reduce motion at the last poll.
    reduced: bool,
    /// When the trick sweep's last card lands (set by `Presentation::poll`): the hand's
    /// `Score` waits for it (09 §2.2).
    sweep_until: Option<f64>,
    /// The cards that touched down since the presentation last asked
    /// ([`Self::take_touchdowns`]), oldest first.
    touchdowns: Vec<Card>,
}

impl Director {
    /// Queues `shot`, unless a shot with its key is already queued, live or landed. Under
    /// reduce motion it lands at once; your own card play starts at once (on the last poll's
    /// clock) when nothing waits ahead of it.
    pub fn enqueue(&mut self, shot: Shot) {
        let key = shot.key();
        if self.knows(&key) {
            return;
        }
        if let Shot::Deal { hand_no, .. } = &shot
            && self.holds_older_than(*hand_no)
        {
            self.fast_forward(self.now);
        }
        if self.reduced {
            let play = matches!(shot, Shot::Play(_));
            if let Shot::Play(p) = &shot {
                self.touchdowns.push(p.card);
            }
            self.land_still(key, play);
            return;
        }
        let mine = matches!(&shot, Shot::Play(p) if p.mine);
        if mine && self.queue.is_empty() {
            self.live
                .push(Running::new(shot, key, self.now, Feel::Full));
            return;
        }
        self.queue.push_back(Queued {
            shot,
            key,
            since: None,
        });
    }

    /// Whether `key` is queued, live or landed.
    pub fn knows(&self, key: &ShotKey) -> bool {
        self.queue.iter().any(|q| &q.key == key)
            || self.live.iter().any(|r| &r.key == key)
            || self.landed.iter().any(|(k, _)| k == key)
    }

    /// Drops `key` without landing it, and forgets it (a refused play: the card went back to
    /// the rack, and a retry flies again).
    pub fn cancel(&mut self, key: &ShotKey) {
        self.queue.retain(|q| &q.key != key);
        self.live.retain(|r| &r.key != key);
        self.landed.retain(|(k, _)| k != key);
        self.fresh.retain(|k| k != key);
    }

    /// `seat`'s queued or live plays of a card in `cards` (a refused play).
    pub fn plays_back_in(&self, seat: Seat, cards: &[Card]) -> Vec<ShotKey> {
        let back =
            |s: &Shot| matches!(s, Shot::Play(p) if p.seat == seat && cards.contains(&p.card));
        self.queue
            .iter()
            .filter(|q| back(&q.shot))
            .map(|q| q.key.clone())
            .chain(
                self.live
                    .iter()
                    .filter(|r| back(&r.shot))
                    .map(|r| r.key.clone()),
            )
            .collect()
    }

    /// Lands every queued and live shot at once (a snapping sync, a resync, a reconnect, the
    /// first view). Their plays mount in place.
    pub fn clear(&mut self) {
        let queued: Vec<(ShotKey, bool)> = self
            .queue
            .drain(..)
            .map(|q| (q.key, matches!(q.shot, Shot::Play(_))))
            .collect();
        let live: Vec<(ShotKey, bool)> = self
            .live
            .drain(..)
            .map(|r| (r.key, matches!(r.shot, Shot::Play(_))))
            .collect();
        for (key, play) in live.into_iter().chain(queued) {
            self.land_still(key, play);
        }
    }

    /// The 140 ms catch-up (09 §2.3): every live shot runs to its end inside `FF_SECS` from
    /// where it is, with no jump; every queued shot lands in place.
    pub fn fast_forward(&mut self, now: f64) {
        for r in &mut self.live {
            if r.ff.is_none() && r.progress(now) < 1.0 {
                r.ff = Some((now, r.progress(now)));
            }
        }
        let queued: Vec<Queued> = self.queue.drain(..).collect();
        for q in queued {
            let play = matches!(q.shot, Shot::Play(_));
            self.land_still(q.key, play);
        }
    }

    /// Where the trick sweep's last card lands (`Presentation::poll` sets it before polling).
    pub fn set_sweep_until(&mut self, at: Option<f64>) {
        self.sweep_until = at;
    }

    /// The cards that touched down since the last call, oldest first (the presentation cues
    /// `Place` with them).
    pub fn take_touchdowns(&mut self) -> Vec<Card> {
        std::mem::take(&mut self.touchdowns)
    }

    /// Advances to `now`: picks the feel, starts shots whose wait is met, retires finished
    /// ones, and says when to come back.
    pub fn poll(&mut self, now: f64, reduced: bool) -> PollOut {
        self.now = now;
        self.reduced = reduced;
        self.fresh.clear();
        let mut out = PollOut::default();
        for q in &mut self.queue {
            q.since.get_or_insert(now);
        }
        if reduced {
            // 09 §2.5: each shot starts and finishes now.
            self.feel = Feel::Snap;
            if self.busy() {
                out.changed = true;
                let queued = self.queue.iter().map(|q| &q.shot);
                let live = self.live.iter().filter(|r| !r.touched).map(|r| &r.shot);
                let cards: Vec<Card> = live
                    .chain(queued)
                    .filter_map(|s| match s {
                        Shot::Play(p) => Some(p.card),
                        _ => None,
                    })
                    .collect();
                self.touchdowns.extend(cards);
                self.clear();
            }
            return out;
        }
        self.feel = self.pick_feel(now);
        let mut wake: Option<f64> = None;
        let mut soonest = |at: f64| wake = Some(wake.map_or(at, |w: f64| w.min(at)));
        while let Some(q) = self.queue.front() {
            match self.wait(q, now) {
                Some(at) => {
                    if let Some(at) = at {
                        soonest(at);
                    }
                    break;
                }
                None => {
                    let Some(q) = self.queue.pop_front() else {
                        break;
                    };
                    let feel = self.start_feel(&q.shot);
                    self.live.push(Running::new(q.shot, q.key, now, feel));
                    out.changed = true;
                }
            }
        }
        for r in &mut self.live {
            if matches!(r.shot, Shot::Play(_)) && !r.touched {
                if now >= r.touchdown() {
                    r.touched = true;
                    if let Shot::Play(p) = &r.shot {
                        self.touchdowns.push(p.card);
                    }
                } else {
                    soonest(r.touchdown());
                }
            }
        }
        let mut done = Vec::new();
        for r in &self.live {
            if r.progress(now) >= 1.0 {
                done.push((r.key.clone(), r.feel, matches!(r.shot, Shot::Play(_))));
            } else {
                soonest(r.end());
                if r.poll_driven(now) {
                    out.frame = true;
                }
            }
        }
        if !done.is_empty() {
            self.live.retain(|r| r.progress(now) < 1.0);
            for (key, feel, play) in done {
                if play && feel == Feel::Snap {
                    self.fresh.push(key.clone());
                }
                self.land(key, feel);
            }
            out.changed = true;
        }
        if out.frame {
            out.changed = true;
        }
        out.wake = wake.filter(|_| !out.frame);
        out
    }

    /// The feel for the queue as it stands (09 §2.3). Only plays and deals count as backlog: a
    /// trick's `Win` rides with its 4th card, a `Flip` with its deal, bubbles alongside.
    fn pick_feel(&self, now: f64) -> Feel {
        let backlog: Vec<&Queued> = self
            .queue
            .iter()
            .filter(|q| matches!(q.shot, Shot::Play(_) | Shot::Deal { .. }))
            .collect();
        let wait = backlog
            .first()
            .and_then(|q| q.since)
            .map_or(0.0, |since| now - since);
        if backlog.len() <= FULL_MAX && wait < FULL_WAIT {
            Feel::Full
        } else if backlog.len() <= BRISK_MAX && wait <= BRISK_WAIT {
            Feel::Brisk
        } else {
            Feel::Skim
        }
    }

    /// The feel a starting shot takes: the poll's, except that under Skim a play mounts in
    /// place unless it is the last queued play of its trick.
    fn start_feel(&self, shot: &Shot) -> Feel {
        match (shot, self.feel) {
            (Shot::Play(p), Feel::Skim) => {
                let later = self.queue.iter().any(|q| {
                    matches!(&q.shot, Shot::Play(o) if o.hand_no == p.hand_no && o.index == p.index)
                });
                if later { Feel::Snap } else { Feel::Skim }
            }
            (Shot::Deal { .. }, Feel::Skim) => Feel::Snap,
            (_, feel) => feel,
        }
    }

    /// `None` when `q` may start now; else `Some(when)` (`Some(None)`: no known time).
    #[allow(clippy::option_option)]
    fn wait(&self, q: &Queued, now: f64) -> Option<Option<f64>> {
        let until = |at: f64| (now < at).then_some(Some(at));
        match &q.shot {
            Shot::Deal { .. } | Shot::Bubble { .. } | Shot::Pickup { .. } | Shot::Stamp { .. } => {
                None
            }
            Shot::Flip { hand_no, .. } => self
                .live
                .iter()
                .filter(|r| matches!(&r.shot, Shot::Deal { hand_no: h, .. } if h == hand_no))
                .map(|r| r.end() - FLIP_OVERLAP)
                .reduce(f64::max)
                .and_then(until),
            Shot::Trump { .. } => self
                .live
                .iter()
                .filter(|r| !matches!(r.shot, Shot::Bubble { .. }))
                .map(Running::end)
                .reduce(f64::max)
                .and_then(until),
            Shot::Play(p) if p.mine => None,
            Shot::Play(_) => self
                .live
                .iter()
                .filter(|r| matches!(r.shot, Shot::Play(_)))
                .map(Running::touchdown)
                .reduce(f64::max)
                .and_then(until),
            Shot::Win { hand_no, index, .. } => self
                .live
                .iter()
                .filter(|r| {
                    matches!(&r.shot, Shot::Play(p) if p.hand_no == *hand_no && p.index == *index)
                })
                .map(|r| r.touchdown() + RING_DELAY)
                .reduce(f64::max)
                .and_then(until),
            Shot::Score { .. } => self.sweep_until.and_then(until),
            Shot::MatchEnd { .. } => self
                .live
                .iter()
                .filter(|r| matches!(r.shot, Shot::Score { .. }))
                .map(Running::end)
                .reduce(f64::max)
                .and_then(until),
        }
    }

    fn holds_older_than(&self, hand_no: u32) -> bool {
        let older = |s: &Shot| s.hand_no().is_some_and(|h| h < hand_no);
        self.queue.iter().any(|q| older(&q.shot))
            || self.live.iter().any(|r| older(&r.shot) && r.ff.is_none())
    }

    fn land_still(&mut self, key: ShotKey, play: bool) {
        if play {
            self.fresh.push(key.clone());
        }
        self.land(key, Feel::Snap);
    }

    fn land(&mut self, key: ShotKey, feel: Feel) {
        if self.landed.iter().any(|(k, _)| *k == key) {
            return;
        }
        if self.landed.len() >= LANDED {
            self.landed.pop_front();
        }
        self.landed.push_back((key, feel));
    }

    /// The live Full-tier play flyers: key, spec and seconds into the flight (fast-forward
    /// included). Resolve each with [`resolve_flight`] and pose it with [`play_pose`].
    pub fn flyers(&self, now: f64) -> impl Iterator<Item = (&ShotKey, &PlayFlightSpec, f32)> {
        self.live
            .iter()
            .filter_map(move |r| match (&r.shot, r.feel) {
                (Shot::Play(spec), Feel::Full) if !self.win_began(spec.hand_no, spec.index) => {
                    Some((&r.key, spec, r.elapsed(now)))
                }
                _ => None,
            })
    }

    /// Whether trick `index` of hand `hand`'s win has started (or landed). Its cards then rest
    /// in their slots: the winner's pop and ring sit on the static card from the Win beat
    /// (09 §4.4), so a winning 4th card hands off from its flyer there, after its touchdown.
    fn win_began(&self, hand: u32, index: u8) -> bool {
        let win = ShotKey::win(hand, index);
        self.started(&win).is_some() || self.landed(&win)
    }

    /// How the trick slot of `seat`'s card in trick `index` of hand `hand` renders now.
    pub fn play_slot(&self, hand: u32, index: u8, seat: Seat) -> PlaySlot {
        let key = ShotKey::play(hand, index, seat);
        if let Some(r) = self.live.iter().find(|r| r.key == key) {
            return match r.feel {
                Feel::Full if self.win_began(hand, index) => PlaySlot::Still,
                Feel::Full => PlaySlot::Hidden,
                Feel::Brisk | Feel::Skim => PlaySlot::Matched,
                Feel::Snap => PlaySlot::Still,
            };
        }
        if self.queue.iter().any(|q| q.key == key) {
            return PlaySlot::Hidden;
        }
        if self.fresh.contains(&key) {
            return PlaySlot::Still;
        }
        PlaySlot::Matched
    }

    /// Whether the trick slot of `seat`'s card in trick `index` of hand `hand` stays empty
    /// because its Play shot is queued or still flying (09 §4.3).
    pub fn hides_slot(&self, hand: u32, index: u8, seat: Seat) -> bool {
        self.play_slot(hand, index, seat) == PlaySlot::Hidden
    }

    /// How many of `seat`'s plays are still queued: its fan keeps that many extra backs.
    pub fn unlaunched_from(&self, seat: Seat) -> u8 {
        let n = self
            .queue
            .iter()
            .filter(|q| matches!(&q.shot, Shot::Play(p) if p.seat == seat))
            .count();
        u8::try_from(n).unwrap_or(u8::MAX)
    }

    /// `seat`'s oldest queued play: its fan's top back carries this play's matched id, so a
    /// Brisk or Skim play flies from there.
    pub fn unlaunched_play(&self, seat: Seat) -> Option<&PlayFlightSpec> {
        self.queue.iter().find_map(|q| match &q.shot {
            Shot::Play(p) if p.seat == seat => Some(p),
            _ => None,
        })
    }

    /// When the live shot `key` started (ring, pop and stamp `t0`).
    pub fn started(&self, key: &ShotKey) -> Option<f64> {
        self.live.iter().find(|r| &r.key == key).and_then(|r| r.t0)
    }

    /// Seconds into the live shot `key`'s timeline (fast-forward included).
    pub fn elapsed(&self, key: &ShotKey, now: f64) -> Option<f32> {
        self.live
            .iter()
            .find(|r| &r.key == key)
            .map(|r| r.elapsed(now))
    }

    /// How far `key` has got: 0 while queued, 0..=1 while live, 1 once landed, `None` if never
    /// seen.
    pub fn progress(&self, key: &ShotKey, now: f64) -> Option<f32> {
        if let Some(r) = self.live.iter().find(|r| &r.key == key) {
            return Some(r.progress(now));
        }
        if self.pending(key) {
            return Some(0.0);
        }
        self.landed(key).then_some(1.0)
    }

    /// Whether `key` is queued and not started.
    pub fn pending(&self, key: &ShotKey) -> bool {
        self.queue.iter().any(|q| &q.key == key)
    }

    /// Whether `key` has landed (recently: the last [`LANDED`] shots).
    pub fn landed(&self, key: &ShotKey) -> bool {
        self.landed.iter().any(|(k, _)| k == key)
    }

    /// The feel `key` runs or ran at: `None` if queued or never seen.
    pub fn feel_of(&self, key: &ShotKey) -> Option<Feel> {
        self.live
            .iter()
            .find(|r| &r.key == key)
            .map(|r| r.feel)
            .or_else(|| self.landed.iter().find(|(k, _)| k == key).map(|(_, f)| *f))
    }

    /// The feel in force.
    pub fn feel(&self) -> Feel {
        self.feel
    }

    /// Whether anything is queued or live.
    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || !self.live.is_empty()
    }
}

/// Where `spec`'s flyer starts and lands, in window points (09 §4.3), from this frame's
/// layout: an opponent's card from the centre of its `seat-{i}-fan` (or `seat-{i}` when the
/// fan is not shown), yours from its `hole-{card}`, both to the slot `stage::slot_rect` gives
/// in the `trick` box. `None` before the trick box is laid out; render the slot
/// [`PlaySlot::Matched`] then.
pub fn resolve_flight(
    spec: &PlayFlightSpec,
    view: &PublicGameView,
    cx: &Cx<AppState>,
    height: f32,
) -> Option<PlayFlight> {
    let trick = cx.layout("trick")?;
    let ([x, y, w, h], turn) = stage::slot_rect(view, spec.seat, trick, height);
    let to = [x + w / 2.0, y + h / 2.0];
    let w = w.max(1.0);
    if spec.mine {
        let [hx, hy, hw, hh] = cx.layout(&hand::hole_key(spec.card))?;
        // A card-sized hole is the card; a thin one marks the rack row's top edge.
        let (cw, cy) = if hh >= 4.0 {
            ((hh * card::ASPECT).min(hw), hy + hh / 2.0)
        } else {
            let cw = (hw - 3.0 * card::HAND_GAP).clamp(w, card::HAND_W);
            (cw, hy + cw / card::ASPECT / 2.0)
        };
        return Some(PlayFlight {
            from: [hx + hw / 2.0, cy],
            to,
            arc: PLAY_ARC_YOU,
            r0: 0.0,
            r1: turn,
            s0: cw / w,
            size: [w, h],
        });
    }
    let i = spec.seat.index();
    let (from, s0) = match cx.layout(&format!("seat-{i}-fan")) {
        Some([fx, fy, fw, fh]) => {
            let back_w = (fh * card::ASPECT).min(fw);
            ([fx + fw / 2.0, fy + fh / 2.0], back_w / w)
        }
        None => {
            let [sx, sy, sw, sh] = cx.layout(&format!("seat-{i}"))?;
            ([sx + sw / 2.0, sy + sh / 2.0], 0.4)
        }
    };
    let chord = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
    // The top back's tilt in the fan the card left, so the flyer leaves at the back's angle.
    let backs = (usize::from(view.hand_counts[i]) + 1).min(stage::FAN_MAX);
    Some(PlayFlight {
        from,
        to,
        arc: PLAY_ARC_OPP.min(PLAY_ARC_SHARE * chord),
        r0: stage::fan_tilt(backs - 1, backs),
        r1: turn,
        s0,
        size: [w, h],
    })
}

/// How far along the chord a flyer is `t` seconds after launch, 0 to 1: thrown with drag (fast
/// off the hand, slowing as it falls), touching down at `PLAY_SKID` short of the slot, then
/// skidding the rest of the way to a stop exactly at `PLAY_END`, with no jump in speed.
fn ground(t: f32) -> f32 {
    let k = PLAY_DRAG;
    let norm = 1.0 - (-k).exp();
    let air = 1.0 - PLAY_SKID;
    if t <= PLAY_TRAVEL {
        let tau = (t / PLAY_TRAVEL).max(0.0);
        return air * (1.0 - (-k * tau).exp()) / norm;
    }
    // Landing speed in chord shares per second, matched by the skid's start.
    let v = air * k * (-k).exp() / norm / PLAY_TRAVEL;
    let tc = PLAY_SKID / v;
    let span = (PLAY_END - PLAY_TRAVEL) / tc;
    let x = ((t - PLAY_TRAVEL) / tc).min(span);
    air + PLAY_SKID * (1.0 - (-x).exp()) / (1.0 - (-span).exp())
}

/// A play flyer's pose `t` seconds after launch (09 §4.3). It travels the chord on
/// [`ground`] and rises on a parabola in time, `4τ(1−τ)`, peaking `arc / 2` above the chord
/// half way through `PLAY_TRAVEL`: the toss leaves fast, hangs, and drops onto the slot. The
/// rotation springs on `ROT_SETTLE`, the scale grows from `s0` with the ground and swells
/// `PLAY_LIFT_SCALE` at the top, and a touchdown squash runs over `SQUASH_T`.
///
/// Deviation from the 09 §4.3 sketch (a Bézier eased by `PLAY_EASE`): that ease reaches 0.92
/// of the way at half the travel, so the card is already falling at t = 0.275 (about 0.15 ×
/// arc above the chord, against A-play-arcs' 0.3) and arrives flat. Splitting ground and
/// height keeps the peak at `arc / 2` but puts it at mid-travel, with the drop at the end.
pub fn play_pose(f: &PlayFlight, t: f32) -> Pose {
    let t = t.max(0.0);
    let g = ground(t);
    let tau = (t / PLAY_TRAVEL).min(1.0);
    let lift = 4.0 * tau * (1.0 - tau);
    let x = f.from[0] + (f.to[0] - f.from[0]) * g;
    let y = f.from[1] + (f.to[1] - f.from[1]) * g - 0.5 * f.arc * lift;
    let (rp, _) = ROT_SETTLE.sample(0.0, 0.0, 1.0, t);
    let land = ((t - PLAY_TRAVEL) / SQUASH_T).clamp(0.0, 1.0);
    let squash = if t > PLAY_TRAVEL {
        PLAY_SQUASH * (PI * land).sin()
    } else {
        0.0
    };
    let grow = (t / PLAY_TRAVEL).min(1.0);
    Pose {
        x,
        y,
        rot: f.r0 + (f.r1 - f.r0) * rp,
        scale: (f.s0 + (1.0 - f.s0) * grow) * (1.0 + PLAY_LIFT_SCALE * lift) * (1.0 - squash),
        lift,
    }
}

/// The illegal-card shake's x offset `t` seconds in (09 §4.7, S5): `11·exp(−t/0.11)·sin(2πt/0.12)`
/// for `motion::SHAKE`; rotate by `x · 0.004`.
pub fn shake_x(t: f32) -> f32 {
    11.0 * (-t / 0.11).exp() * (2.0 * PI * t / 0.12).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flight() -> PlayFlight {
        PlayFlight {
            from: [100.0, 100.0],
            to: [500.0, 300.0],
            arc: 110.0,
            r0: -0.1,
            r1: 0.05,
            s0: 0.4,
            size: [70.0, 98.0],
        }
    }

    fn above_chord(f: &PlayFlight, p: &Pose) -> f32 {
        let k = (p.x - f.from[0]) / (f.to[0] - f.from[0]);
        f.from[1] + k * (f.to[1] - f.from[1]) - p.y
    }

    fn spec(index: u8, seat: Seat, card: &str, mine: bool) -> PlayFlightSpec {
        PlayFlightSpec {
            hand_no: 3,
            index,
            seat,
            card: Card::parse(card).expect("a card"),
            mine,
        }
    }

    #[test]
    fn the_pose_starts_at_the_source_and_lands_on_the_slot() {
        let f = flight();
        let p = play_pose(&f, 0.0);
        assert!((p.x - 100.0).abs() < 0.01 && (p.y - 100.0).abs() < 0.01);
        assert!((p.scale - 0.4).abs() < 0.01);
        let end = play_pose(&f, PLAY_END);
        assert!((end.x - 500.0).abs() < 0.01 && (end.y - 300.0).abs() < 0.01);
        assert!((end.scale - 1.0).abs() < 0.01 && end.lift.abs() < 0.01);
        assert!((end.rot - 0.05).abs() < 0.02, "rot {}", end.rot);
        let after = play_pose(&f, 2.0);
        assert!((after.x - 500.0).abs() < 0.01 && (after.y - 300.0).abs() < 0.01);
    }

    #[test]
    fn the_pose_arcs_above_the_chord_and_peaks_mid_travel() {
        let f = flight();
        let mut peak = (0.0f32, 0.0f32);
        for i in 1..55 {
            let t = i as f32 * 0.01;
            let above = above_chord(&f, &play_pose(&f, t));
            assert!(above > 0.0, "above the chord at {t}");
            if above > peak.0 {
                peak = (above, t);
            }
        }
        assert!((peak.0 - 0.5 * f.arc).abs() < 0.02 * f.arc, "peak {peak:?}");
        assert!((peak.1 - PLAY_TRAVEL / 2.0).abs() < 0.02, "peak {peak:?}");
        // 09 §6.2 A-play-arcs: at t = 0.275 at least 0.3 x arc above the chord.
        assert!(above_chord(&f, &play_pose(&f, 0.275)) >= 0.3 * f.arc);
    }

    #[test]
    fn the_toss_slows_as_it_falls_and_skids_without_a_jump() {
        let dt = 1.0 / 240.0;
        let speed = |t: f32| (ground(t + dt) - ground(t)) / dt;
        assert!(
            speed(0.0) > 2.0 * speed(PLAY_TRAVEL - dt),
            "fast off the hand"
        );
        let before = speed(PLAY_TRAVEL - dt);
        let after = speed(PLAY_TRAVEL);
        assert!((before - after).abs() < 0.1 * before, "{before} vs {after}");
        assert!((ground(PLAY_TRAVEL) - (1.0 - PLAY_SKID)).abs() < 1e-5);
        assert!((ground(PLAY_END) - 1.0).abs() < 1e-5);
        assert!((1..240).all(|i| ground(i as f32 * dt) >= ground((i - 1) as f32 * dt)));
    }

    #[test]
    fn the_touchdown_squashes_and_recovers() {
        let f = flight();
        let mid = play_pose(&f, PLAY_TRAVEL + SQUASH_T / 2.0);
        assert!(mid.scale < 1.0 - 0.5 * PLAY_SQUASH);
        let out = play_pose(&f, PLAY_TRAVEL + SQUASH_T + 0.01);
        assert!((out.scale - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_shake_dies_out() {
        assert!(shake_x(0.03).abs() >= 4.0);
        assert!(shake_x(0.4).abs() < 0.5);
    }

    #[test]
    fn fast_forward_finishes_inside_its_window() {
        let mut r = Running::new(
            Shot::Pickup { hand_no: 0 },
            ShotKey::pickup(0),
            1.0,
            Feel::Full,
        );
        r.dur = 1.0;
        assert!((r.progress(1.3) - 0.3).abs() < 1e-4);
        r.ff = Some((1.3, 0.3));
        assert!(r.progress(1.3 + FF_SECS / 2.0) < 1.0);
        assert!((r.progress(1.3 + FF_SECS) - 1.0).abs() < 1e-6);
        assert!((r.end() - (1.3 + FF_SECS)).abs() < 1e-9);
    }

    #[test]
    fn plays_take_turns_and_the_ring_waits_for_the_touchdown() {
        let mut d = Director::default();
        let _ = d.poll(0.0, false);
        d.enqueue(Shot::Play(spec(1, Seat::WEST, "QH", false)));
        d.enqueue(Shot::Play(spec(1, Seat::NORTH, "KH", false)));
        d.enqueue(Shot::Win {
            hand_no: 3,
            index: 1,
            seat: Seat::NORTH,
        });
        let out = d.poll(1.0, false);
        assert_eq!(d.feel(), Feel::Brisk, "two plays queued");
        assert!(out.changed);
        assert_eq!(d.unlaunched_from(Seat::NORTH), 1);
        assert_eq!(d.play_slot(3, 1, Seat::WEST), PlaySlot::Matched);
        assert_eq!(d.play_slot(3, 1, Seat::NORTH), PlaySlot::Hidden);
        let td = out.wake.expect("the touchdown wakes the queue");
        assert!((td - (1.0 + f64::from(PLAY_TRAVEL) * BRISK)).abs() < 1e-6);
        let _ = d.poll(td, false);
        assert_eq!(d.unlaunched_from(Seat::NORTH), 0);
        // The backlog drained, so the second card flies Full.
        assert_eq!(
            d.feel_of(&ShotKey::play(3, 1, Seat::NORTH)),
            Some(Feel::Full)
        );
        assert!(d.pending(&ShotKey::win(3, 1)), "the ring waits");
        let ring = td + f64::from(PLAY_TRAVEL) + RING_DELAY;
        let _ = d.poll(ring - 0.01, false);
        assert!(d.pending(&ShotKey::win(3, 1)), "still in the air");
        let _ = d.poll(ring, false);
        assert!(d.started(&ShotKey::win(3, 1)).is_some());
        let landed: Vec<String> = d
            .take_touchdowns()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            landed,
            ["QH", "KH"],
            "each landing carries its card, in order"
        );
        assert!(d.take_touchdowns().is_empty());
    }

    #[test]
    fn one_play_flies_full_and_idles_after() {
        let mut d = Director::default();
        d.enqueue(Shot::Play(spec(0, Seat::EAST, "9S", false)));
        assert_eq!(d.play_slot(3, 0, Seat::EAST), PlaySlot::Hidden);
        let out = d.poll(0.5, false);
        assert_eq!(d.feel(), Feel::Full);
        assert!(out.frame && out.changed);
        let (key, _, t) = d.flyers(0.75).next().expect("a flyer");
        assert_eq!(key, &ShotKey::play(3, 0, Seat::EAST));
        assert!((t - 0.25).abs() < 1e-4);
        let out = d.poll(0.5 + f64::from(PLAY_END), false);
        assert!(out.changed && !out.frame);
        assert_eq!(d.flyers(2.0).count(), 0);
        assert_eq!(d.play_slot(3, 0, Seat::EAST), PlaySlot::Matched);
        assert!(!d.busy());
        assert_eq!(d.poll(3.0, false), PollOut::default());
    }

    #[test]
    fn your_play_starts_on_enqueue() {
        let mut d = Director::default();
        let _ = d.poll(2.0, false);
        d.enqueue(Shot::Play(spec(0, Seat::SOUTH, "AS", true)));
        assert_eq!(d.started(&ShotKey::play(3, 0, Seat::SOUTH)), Some(2.0));
        assert_eq!(d.flyers(2.0).count(), 1);
        d.enqueue(Shot::Play(spec(0, Seat::SOUTH, "AS", true)));
        assert_eq!(d.flyers(2.0).count(), 1, "deduped");
    }

    #[test]
    fn a_backlog_skims_to_the_last_play() {
        let mut d = Director::default();
        let _ = d.poll(0.0, false);
        for (seat, c) in [
            (Seat::WEST, "9H"),
            (Seat::NORTH, "TH"),
            (Seat::EAST, "JH"),
            (Seat::SOUTH, "QH"),
        ] {
            d.enqueue(Shot::Play(spec(2, seat, c, false)));
        }
        let _ = d.poll(DT, false);
        assert_eq!(d.feel(), Feel::Skim);
        assert_eq!(d.flyers(DT).count(), 0);
        for seat in [Seat::WEST, Seat::NORTH, Seat::EAST] {
            assert_eq!(d.play_slot(3, 2, seat), PlaySlot::Still, "{seat:?}");
        }
        assert_eq!(d.play_slot(3, 2, Seat::SOUTH), PlaySlot::Matched);
        let _ = d.poll(2.0 * DT, false);
        assert_eq!(d.play_slot(3, 2, Seat::WEST), PlaySlot::Matched);
    }

    const DT: f64 = 1.0 / 60.0;

    #[test]
    fn fast_forward_lands_the_queue_in_place() {
        let mut d = Director::default();
        d.enqueue(Shot::Play(spec(0, Seat::EAST, "9S", false)));
        let _ = d.poll(0.0, false);
        d.enqueue(Shot::Play(spec(0, Seat::NORTH, "TS", false)));
        d.fast_forward(0.2);
        assert!(!d.pending(&ShotKey::play(3, 0, Seat::NORTH)));
        assert_eq!(d.play_slot(3, 0, Seat::NORTH), PlaySlot::Still);
        let out = d.poll(0.2 + FF_SECS, false);
        assert!(out.changed);
        assert_eq!(d.flyers(0.2 + FF_SECS).count(), 0);
        assert!(!d.busy());
    }

    #[test]
    fn reduce_motion_lands_everything_at_once() {
        let mut d = Director::default();
        let _ = d.poll(0.0, true);
        assert_eq!(d.feel(), Feel::Snap);
        d.enqueue(Shot::Play(spec(1, Seat::WEST, "QH", false)));
        let key = ShotKey::play(3, 1, Seat::WEST);
        assert_eq!(d.play_slot(3, 1, Seat::WEST), PlaySlot::Still);
        assert_eq!(d.unlaunched_from(Seat::WEST), 0);
        assert_eq!(d.started(&key), None);
        assert_eq!(d.progress(&key, 0.0), Some(1.0));
        assert_eq!(d.flyers(0.0).count(), 0);
        assert!(!d.busy());
        assert_eq!(d.poll(0.1, true), PollOut::default());
    }

    #[test]
    fn a_new_deal_fast_forwards_the_old_hand() {
        let mut d = Director::default();
        d.enqueue(Shot::Score {
            hand_no: 3,
            result: HandResult::Point,
            delta: [1, 0],
            score: [1, 0],
        });
        let _ = d.poll(0.0, false);
        d.enqueue(Shot::Deal {
            hand_no: 4,
            dealer: Seat::NORTH,
        });
        let _ = d.poll(0.05, false);
        assert!(d.started(&ShotKey::deal(4)).is_some());
        let _ = d.poll(0.05 + FF_SECS, false);
        assert!(d.landed(&ShotKey::score(3)));
    }

    #[test]
    fn the_score_waits_for_the_sweep() {
        let mut d = Director::default();
        d.set_sweep_until(Some(2.9));
        d.enqueue(Shot::Score {
            hand_no: 3,
            result: HandResult::Euchre,
            delta: [0, 2],
            score: [3, 6],
        });
        d.enqueue(Shot::Stamp {
            hand_no: 3,
            result: HandResult::Euchre,
        });
        let out = d.poll(2.0, false);
        assert_eq!(out.wake, Some(2.9));
        assert!(d.pending(&ShotKey::stamp(3)));
        let out = d.poll(2.9, false);
        assert!(d.started(&ShotKey::stamp(3)).is_some());
        assert!(out.frame, "sparks are poll-driven");
    }
}
