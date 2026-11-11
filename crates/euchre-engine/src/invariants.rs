//! The reusable state validator (port of `invariants.ts`, spec 01 section 12).
//!
//! [`assert_invariants`] checks every invariant that is decidable from a single
//! [`GameState`]; [`assert_legal_non_empty`] checks V19 with the caller's legal
//! move function, so this module stays free of a dependency on `legal`.
//! Messages match the TS `InvariantError` messages byte for byte.

use std::collections::HashSet;
use std::fmt;

use crate::cards::{DECK, SUITS, effective_suit, suit_of};
use crate::seats::partner_of;
use crate::trick::card_value;
use crate::types::{Card, GamePhase, GameState, GameStatus, HandResult, Rank, Seat, Trick};

/// An impossible state produced by the engine: a programmer error, never a
/// rules event, never shown to a player. Display is `"{invariant} violated: {detail}"`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InvariantError {
    /// The invariant id, e.g. `"V4"`.
    pub invariant: String,
    /// What was wrong.
    pub detail: String,
}

impl InvariantError {
    /// An error for `invariant` with `detail`.
    pub fn new(invariant: impl Into<String>, detail: impl Into<String>) -> InvariantError {
        InvariantError {
            invariant: invariant.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for InvariantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} violated: {}", self.invariant, self.detail)
    }
}

impl std::error::Error for InvariantError {}

/// The TS `require_(ok, invariant, detail)`.
fn require(
    ok: bool,
    invariant: &str,
    detail: impl FnOnce() -> String,
) -> Result<(), InvariantError> {
    if ok {
        Ok(())
    } else {
        Err(InvariantError::new(invariant, detail()))
    }
}

/// A nullable value as JS template interpolation prints it (`null` for `None`).
fn js<T: fmt::Display>(v: Option<T>) -> String {
    match v {
        Some(v) => v.to_string(),
        None => "null".to_owned(),
    }
}

/// A pair as JS prints an array (`"a,b"`).
fn pair(v: [u8; 2]) -> String {
    format!("{},{}", v[0], v[1])
}

/// Every trick exactly once.
///
/// During `trick_resolve` the engine leaves `hand.trick` pointing at the trick
/// it already appended to `trick_log`, so it is deduped by index.
fn all_tricks(s: &GameState) -> Vec<&Trick> {
    let log = &s.hand.trick_log;
    let cur = &s.hand.trick;
    let mut out: Vec<&Trick> = log.iter().collect();
    if !cur.plays.is_empty() && !log.iter().any(|t| t.index == cur.index) {
        out.push(cur);
    }
    out
}

/// Asserts every state-decidable invariant (V4, V5, V8 to V11, V13, V14, V16
/// to V18). Returns the first violation as an [`InvariantError`].
pub fn assert_invariants(s: &GameState) -> Result<(), InvariantError> {
    let h = &s.hand;
    let dealt = h.phase != GamePhase::Lobby && h.phase != GamePhase::Cutting;

    // V4 - alone.
    if let Some(alone) = h.alone_seat {
        require(Some(alone) == h.maker_seat, "V4", || {
            format!("aloneSeat {alone} !== makerSeat {}", js(h.maker_seat))
        })?;
        require(h.sitting_seat == Some(partner_of(alone)), "V4", || {
            format!("sittingSeat {} !== partnerOf({alone})", js(h.sitting_seat))
        })?;
        require(h.trump.is_some(), "V4", || {
            "aloneSeat set without trump".to_owned()
        })?;
    }
    require(
        h.sitting_seat.is_none() || h.alone_seat.is_some(),
        "V4",
        || "sittingSeat set without an aloneSeat".to_owned(),
    )?;

    // V5 - dealer discard happens at most once, and leaves the dealer at five.
    if h.dealer_discard.is_some() && h.phase != GamePhase::DealerDiscard {
        let dealer_held = h.hands[h.dealer_seat.index()].len();
        let dealer_is_sitting = h.sitting_seat == Some(h.dealer_seat);
        require(dealer_is_sitting || dealer_held <= 5, "V5", || {
            format!("dealer holds {dealer_held} after discarding")
        })?;
    }

    // V8 - sitting out.
    if let Some(sitting) = h.sitting_seat {
        for t in all_tricks(s) {
            require(!t.plays.iter().any(|p| p.seat == sitting), "V8", || {
                format!("sitting seat {sitting} played in trick {}", t.index)
            })?;
        }
    }

    // V9 - lead succession: the winner of a trick leads the next one.
    let tricks = all_tricks(s);
    for w in tricks.windows(2) {
        let (prev, next) = (w[0], w[1]);
        let (Some(winner), Some(lead)) = (prev.winner_seat, next.plays.first()) else {
            continue;
        };
        require(lead.seat == winner, "V9", || {
            format!(
                "trick {} led by {}, but {winner} won trick {}",
                next.index, lead.seat, prev.index
            )
        })?;
        require(Some(winner) != h.sitting_seat, "V9", || {
            "the sitting seat won a trick".to_owned()
        })?;
    }

    // V10 - card accounting.
    if dealt {
        let n = h.deck_order.len();
        require(n == 24, "V10", || format!("deckOrder has {n} cards"))?;
        let distinct: HashSet<Card> = h.deck_order.iter().copied().collect();
        require(distinct.len() == 24, "V10", || {
            "deckOrder contains a duplicate".to_owned()
        })?;
        let k = h.kitty.len();
        require(k == 4, "V10", || format!("kitty has {k} cards"))?;

        let played: Vec<Card> = tricks
            .iter()
            .flat_map(|t| t.plays.iter().map(|p| p.card))
            .collect();
        let held: Vec<Card> = Seat::ALL
            .iter()
            .flat_map(|seat| h.hands[seat.index()].iter().copied())
            .collect();
        let mut universe: HashSet<Card> = HashSet::new();
        universe.extend(held.iter().copied());
        universe.extend(played.iter().copied());
        universe.extend(h.kitty.iter().copied());
        universe.extend(h.dealer_discard);
        require(universe.len() == 24, "V10", || {
            format!(
                "{} distinct cards accounted for, expected 24",
                universe.len()
            )
        })?;

        // The up-card is the one legal alias: kitty[0] is immutable, so after a
        // pickup the same card is legitimately in both kitty and a hand.
        let mut live: Vec<Card> = held.clone();
        live.extend(played.iter().copied());
        live.extend(h.dealer_discard);
        let mut seen: HashSet<Card> = HashSet::new();
        let dupe = live.iter().copied().find(|c| !seen.insert(*c));
        require(dupe.is_none(), "V10", || {
            format!("{} is in two live places at once", js(dupe))
        })?;

        if matches!(h.phase, GamePhase::HandScore | GamePhase::GameOver) {
            let expect_played = if h.sitting_seat.is_none() { 20 } else { 15 };
            if h.result.is_some_and(|r| r != HandResult::ThrowIn) {
                require(played.len() == expect_played, "V10", || {
                    format!("{} cards played, expected {expect_played}", played.len())
                })?;
            }
        }
    }

    // V11 - scoring. Scores and deltas are unsigned, so "negative" cannot occur.
    let game_to = s.cfg.game_to;
    require(
        s.score[0] <= game_to && s.score[1] <= game_to,
        "V11",
        || format!("score {} exceeds gameTo {game_to}", pair(s.score)),
    )?;
    if let Some(delta) = h.delta {
        require(delta[0] == 0 || delta[1] == 0, "V11", || {
            format!("both teams scored in one hand: {}", pair(delta))
        })?;
        for (team, &d) in delta.iter().enumerate() {
            // A four-point loner at seven is clamped to the three points needed
            // to finish. `delta` records the applied award, not the nominal one.
            let clamped_loner =
                d == 3 && h.result == Some(HandResult::LoneMarch) && s.score[team] == game_to;
            require([0, 1, 2, 4].contains(&d) || clamped_loner, "V11", || {
                format!("delta component {d} is not a legal award")
            })?;
        }
    }

    // V13 - kittyCount is a function of (upCardTurnedDown, dealerDiscard set).
    // The TS formula can only produce 3 or 4, so its check is vacuous here too.

    // V14 - up-card persistence: never nulled once turned, for the whole hand.
    if dealt && h.phase != GamePhase::Deal {
        let Some(up) = h.up_card else {
            return Err(InvariantError::new(
                "V14",
                format!("upCard is null in phase {}", h.phase.as_str()),
            ));
        };
        require(up == h.kitty[0], "V14", || {
            format!("upCard {up} !== kitty[0] {}", h.kitty[0])
        })?;
    }

    // V16 - trump census: 7 trump, 5 in the same-colour suit, 6 in each off-colour.
    if let Some(trump) = h.trump {
        let count = DECK
            .iter()
            .filter(|&&c| effective_suit(c, Some(trump)) == trump)
            .count();
        require(count == 7, "V16", || {
            format!("{count} cards are trump under {trump}, expected 7")
        })?;
        for &suit in SUITS.iter().filter(|&&x| x != trump) {
            let n = DECK
                .iter()
                .filter(|&&c| effective_suit(c, Some(trump)) == suit)
                .count();
            let jack = Card::new(Rank::Jack, suit);
            let is_mate = suit_of(jack) == suit && effective_suit(jack, Some(trump)) == trump;
            let expect = if is_mate { 5 } else { 6 };
            require(n == expect, "V16", || {
                format!("suit {suit} has {n} cards under trump {trump}, expected {expect}")
            })?;
        }
    }

    // V17 - comparator totality: no two cards in one trick share a value.
    if h.trump.is_some() {
        for t in &tricks {
            let Some(led) = t.led_suit else { continue };
            if t.plays.is_empty() {
                continue;
            }
            let values: Vec<u8> = t
                .plays
                .iter()
                .map(|p| card_value(p.card, led, h.trump))
                .collect();
            let non_zero: Vec<u8> = values.iter().copied().filter(|&v| v > 0).collect();
            let distinct: HashSet<u8> = non_zero.iter().copied().collect();
            require(distinct.len() == non_zero.len(), "V17", || {
                let joined: Vec<String> = values.iter().map(u8::to_string).collect();
                format!("tie in trick {}: values {}", t.index, joined.join(","))
            })?;
        }
    }

    // V18 - bounded collections. Nothing grows with hand_no.
    let n = h.trick_log.len();
    require(n <= 5, "V18", || format!("trickLog has {n} entries"))?;
    let n = h.bids.len();
    require(n <= 8, "V18", || format!("bids has {n} entries"))?;
    require(h.passes <= 4, "V18", || format!("passes is {}", h.passes))?;
    let n = h.trick.plays.len();
    require(n <= 4, "V18", || format!("trick has {n} plays"))?;

    Ok(())
}

/// V19: the seat to act (when active and not sitting out) has at least one
/// legal move under `legal_moves_fn` (pass `legal::legal_moves`).
pub fn assert_legal_non_empty<F, T>(s: &GameState, legal_moves_fn: F) -> Result<(), InvariantError>
where
    F: Fn(&GameState, Seat) -> Vec<T>,
{
    let Some(turn) = s.hand.turn_seat else {
        return Ok(());
    };
    if s.status != GameStatus::Active || Some(turn) == s.hand.sitting_seat {
        return Ok(());
    }
    let n = legal_moves_fn(s, turn).len();
    require(n >= 1, "V19", || {
        format!(
            "seat {turn} must act in phase {} but has no legal moves",
            h_phase(s)
        )
    })
}

/// The current phase's wire name.
fn h_phase(s: &GameState) -> &'static str {
    s.hand.phase.as_str()
}
