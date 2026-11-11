//! `aiSeat/memory`: card counting and cross-hand recall, both bounded, derived
//! from the redacted view alone.

use euchre_engine::{
    Bid, Card, HandResult, PublicGameView, Seat, Suit, Team, effective_suit, partner_of, team_of,
};
use euchre_protocol::EpisodeKind;

use super::state::{AiSeatEpisode, AiSeatHandMemory, HandSnapshot};
use crate::config::{
    MEM_BID_CAP, MEM_LEAD_CAP, MEM_NOTE_CAP, MEM_TRUMP_SEEN_CAP, MEM_VOID_SUITS_CAP, NOTE_MAX_CHARS,
};
use crate::heuristic::void_reads;
use crate::js::{utf16_len, utf16_prefix};

/// `pushCapped`: append with a cap, dropping the oldest; deduplicates.
fn push_capped<T: PartialEq>(arr: &mut Vec<T>, value: T, cap: usize) {
    if arr.contains(&value) {
        return;
    }
    arr.push(value);
    while arr.len() > cap {
        arr.remove(0);
    }
}

/// `emptyMemory(handNo)`.
pub fn empty_memory(hand_no: u32) -> AiSeatHandMemory {
    AiSeatHandMemory {
        hand_no,
        ..AiSeatHandMemory::default()
    }
}

/// `resetMemory(memory, handNo)`: clear the per-hand memory in place.
pub fn reset_memory(memory: &mut AiSeatHandMemory, hand_no: u32) {
    *memory = empty_memory(hand_no);
}

/// `tableCards(view)`: every card face up this hand, live trick deduped.
fn table_cards(view: &PublicGameView) -> Vec<Card> {
    let live = view.trick.index;
    let mut out: Vec<Card> = view
        .trick_log
        .iter()
        .filter(|t| t.index != live)
        .flat_map(|t| t.plays.iter().map(|p| p.card))
        .collect();
    out.extend(view.trick.plays.iter().map(|p| p.card));
    out
}

/// `absorbPublicState(memory, view)`: fold one view in. Idempotent.
pub fn absorb_public_state(memory: &mut AiSeatHandMemory, view: &PublicGameView) {
    if memory.hand_no != view.hand_no {
        reset_memory(memory, view.hand_no);
    }
    if let Some(trump) = view.trump {
        if let Some(up) = view
            .up_card
            .filter(|u| effective_suit(*u, Some(trump)) == trump)
        {
            push_capped(&mut memory.trump_seen, up, MEM_TRUMP_SEEN_CAP);
        }
        for c in table_cards(view) {
            if effective_suit(c, Some(trump)) == trump {
                push_capped(&mut memory.trump_seen, c, MEM_TRUMP_SEEN_CAP);
            }
        }
    }
    let reads = void_reads(view);
    for seat in Seat::ALL {
        for suit in &reads[seat.index()] {
            push_capped(memory.voids.get_mut(seat), *suit, MEM_VOID_SUITS_CAP);
        }
    }
    let live = view.trick.index;
    for t in view.trick_log.iter().filter(|t| t.index != live) {
        if let Some(first) = t.plays.first() {
            push_capped(&mut memory.lead_history, first.card, MEM_LEAD_CAP);
        }
    }
    if let Some(first) = view.trick.plays.first() {
        push_capped(&mut memory.lead_history, first.card, MEM_LEAD_CAP);
    }
    for b in &view.bids {
        push_capped(
            &mut memory.bid_history,
            format!("{}:{}", b.seat.get(), b.say),
            MEM_BID_CAP,
        );
    }
    memory.snapshot = Some(HandSnapshot {
        hand_no: view.hand_no,
        trump: view.trump,
        maker_seat: view.maker_seat,
        alone_seat: view.alone_seat,
        tricks_won: view.tricks_won,
        result: view.result,
    });
}

fn clamp_note(s: String) -> String {
    if utf16_len(&s) > NOTE_MAX_CHARS {
        utf16_prefix(&s, NOTE_MAX_CHARS).to_owned()
    } else {
        s
    }
}

fn other_team(t: Team) -> Team {
    if t == Team::NS { Team::EW } else { Team::NS }
}

/// `deriveHandNote(snapshot, seat)`: a one-line memory, or `None` when the
/// outcome was not certain from what this seat saw.
pub fn derive_hand_note(snapshot: Option<&HandSnapshot>, seat: Seat) -> Option<String> {
    let snapshot = snapshot?;
    let maker = snapshot.maker_seat?;
    let maker_team = team_of(maker);
    let maker_tricks = snapshot.tricks_won[maker_team.index()];
    let def_tricks = snapshot.tricks_won[other_team(maker_team).index()];
    let mine = team_of(seat);
    let who = |s: Seat| -> String {
        if s == seat {
            "I".to_owned()
        } else if s == partner_of(seat) {
            "my partner".to_owned()
        } else {
            format!("seat {}", s.get())
        }
    };
    let hand = format!("h{}", snapshot.hand_no);
    if let Some(alone) = snapshot.alone_seat {
        let tail = if def_tricks >= 3 {
            "went alone and got euchred"
        } else if maker_tricks == 5 {
            "went alone and marched it"
        } else if maker_tricks >= 3 {
            "went alone and made it"
        } else {
            return None;
        };
        return Some(clamp_note(format!("{hand}: {} {tail}", who(alone))));
    }
    let tail = if def_tricks >= 3 {
        if maker_team == mine {
            "called it and we got euchred"
        } else {
            "called it and we euchred them"
        }
    } else if maker_tricks == 5 {
        "called it and marched"
    } else if maker_tricks >= 3 {
        "called it and made the point"
    } else {
        return None;
    };
    Some(clamp_note(format!("{hand}: {} {tail}", who(maker))))
}

/// Parameters of [`remember_hand`].
#[derive(Debug)]
pub struct RememberParams<'a> {
    /// Cross-hand notes.
    pub notes: &'a mut Vec<String>,
    /// Hands already noted.
    pub noted_hands: &'a mut Vec<u32>,
    /// The hand's snapshot.
    pub snapshot: Option<&'a HandSnapshot>,
    /// This seat.
    pub seat: Seat,
}

/// `rememberHand(notes, notedHands, snapshot, seat)`: the note when new.
pub fn remember_hand(params: RememberParams<'_>) -> Option<String> {
    let snapshot = params.snapshot?;
    if params.noted_hands.contains(&snapshot.hand_no) {
        return None;
    }
    let note = derive_hand_note(Some(snapshot), params.seat);
    params.noted_hands.push(snapshot.hand_no);
    while params.noted_hands.len() > MEM_NOTE_CAP {
        params.noted_hands.remove(0);
    }
    let note = note?;
    params.notes.push(note.clone());
    while params.notes.len() > MEM_NOTE_CAP {
        params.notes.remove(0);
    }
    Some(note)
}

/// `episodeKindFor(snapshot)`.
pub fn episode_kind_for(snapshot: Option<&HandSnapshot>) -> EpisodeKind {
    let Some(snapshot) = snapshot else {
        return EpisodeKind::Play;
    };
    if snapshot.alone_seat.is_some() {
        return EpisodeKind::Loner;
    }
    let Some(maker) = snapshot.maker_seat else {
        return EpisodeKind::Play;
    };
    if snapshot.tricks_won[other_team(team_of(maker)).index()] >= 3 {
        EpisodeKind::Euchre
    } else {
        EpisodeKind::Bid
    }
}

/// Parameters of [`episode_for`].
#[derive(Clone, Copy, Debug)]
pub struct EpisodeParams<'a> {
    /// The hand's snapshot.
    pub snapshot: Option<&'a HandSnapshot>,
    /// This seat.
    pub seat: Seat,
    /// Caller-supplied timestamp.
    pub ts: u64,
}

/// `episodeFor(snapshot, seat, ts)`.
pub fn episode_for(params: EpisodeParams<'_>) -> Option<AiSeatEpisode> {
    let snapshot = params.snapshot?;
    let summary = derive_hand_note(Some(snapshot), params.seat)?;
    Some(AiSeatEpisode {
        kind: episode_kind_for(Some(snapshot)),
        summary,
        salience: if snapshot.alone_seat.is_some() {
            0.9
        } else {
            0.5
        },
        hand_no: snapshot.hand_no,
        ts: params.ts,
    })
}

/// `momentSalience(view, gameTo)`: `0`-`1`, from the public view only.
pub fn moment_salience(view: &PublicGameView, game_to: f64) -> f64 {
    let mut s: f64 = 0.2;
    if view.alone_seat.is_some() {
        s = s.max(1.0);
    }
    if view.result == Some(HandResult::Euchre) {
        s = s.max(0.95);
    }
    if matches!(view.result, Some(HandResult::March | HandResult::LoneMarch)) {
        s = s.max(0.9);
    }
    if view.phase == euchre_engine::GamePhase::TrickResolve {
        s = s.max(0.35);
    }
    if view.bids.last().is_some_and(|b| b.mv != Bid::Pass) {
        s = s.max(0.6);
    }
    let a = f64::from(view.score[0]);
    let b = f64::from(view.score[1]);
    if a >= game_to - 1.0 && b >= game_to - 1.0 {
        s = s.max(0.9);
    } else if a >= game_to - 1.0 || b >= game_to - 1.0 {
        s = s.max(0.55);
    }
    if s > 1.0 { 1.0 } else { s }
}

/// `situationLine(view, seat)`: a short, card-free description of the moment.
pub fn situation_line(view: &PublicGameView, seat: Seat) -> String {
    let mut parts = vec![
        format!("seat {}", seat.get()),
        format!("hand {}", view.hand_no),
        format!("score {}-{}", view.score[0], view.score[1]),
    ];
    if let Some(t) = view.trump {
        parts.push(format!("trump {}", t.as_char()));
    }
    if let Some(m) = view.maker_seat {
        parts.push(format!("maker seat {}", m.get()));
    }
    if let Some(a) = view.alone_seat {
        parts.push(format!("seat {} is alone", a.get()));
    }
    parts.push(format!(
        "tricks {}-{}",
        view.tricks_won[0], view.tricks_won[1]
    ));
    if let Some(r) = view.result {
        parts.push(format!("result {}", r.as_str()));
    }
    parts.join(", ")
}

/// `knownVoids(memory, seat)`.
pub fn known_voids(memory: &AiSeatHandMemory, seat: Seat) -> &[Suit] {
    memory.voids.get(seat)
}
