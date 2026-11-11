//! Seeded determinism and the physical deal (port of `deal.ts`).
//!
//! Every hand is a pure function of `(seed, hand_no)`. There is no ambient
//! randomness or time anywhere in this module (V20): a replay is
//! `{ seed, moves[] }` and nothing else.
//!
//! The PRNG is deliberately not part of `GameState`. The seed is persisted and
//! the generator is rebuilt from it on every wake.
//!
//! All integer arithmetic reproduces the JS `uint32` semantics of the TS engine
//! (`Math.imul`, `>>> 0`) and strings are hashed over their UTF-16 code units,
//! exactly like `String.prototype.charCodeAt`.

use std::fmt;

use crate::cards::DECK;
use crate::seats::next_seat;
use crate::types::{CardId, DealPacket, DealtPacket, Hands, Seat};

/* -------------------------------------------------------------------------- */
/* PRNG                                                                        */
/* -------------------------------------------------------------------------- */

/// FNV-1a over the UTF-16 code units of `s`, as an unsigned 32-bit integer.
pub fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for unit in s.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The 32-bit seed for one hand: FNV-1a over `"{seed}#{hand_no}"`.
pub fn hash_seed(seed: &str, hand_no: u32) -> u32 {
    fnv1a(&format!("{seed}#{hand_no}"))
}

/// The mulberry32 generator state. Yields numbers in `[0, 1)`; never persisted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mulberry32 {
    state: u32,
}

impl Mulberry32 {
    /// A generator seeded with `seed` (the TS `a >>> 0`).
    pub const fn new(seed: u32) -> Mulberry32 {
        Mulberry32 { state: seed }
    }

    /// The next draw in `[0, 1)`, bit-identical to the TS closure.
    pub fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut x = self.state;
        x = (x ^ (x >> 15)).wrapping_mul(x | 1);
        x ^= x.wrapping_add((x ^ (x >> 7)).wrapping_mul(x | 61));
        f64::from(x ^ (x >> 14)) / 4_294_967_296.0
    }
}

impl Iterator for Mulberry32 {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        Some(self.next_f64())
    }
}

/// mulberry32 as a stateful closure yielding numbers in `[0, 1)` (the TS shape).
pub fn mulberry32(seed: u32) -> impl FnMut() -> f64 + Clone {
    let mut rng = Mulberry32::new(seed);
    move || rng.next_f64()
}

/// The RNG stream for one hand, positioned at zero.
pub fn hand_rng(seed: &str, hand_no: u32) -> impl FnMut() -> f64 + Clone {
    mulberry32(hash_seed(seed, hand_no))
}

/// `Math.floor(r * n)` as an index, clamped to `0..=max` so an out-of-contract
/// draw can never index out of bounds (a `[0, 1)` draw is never clamped).
fn draw_index(r: f64, n: usize, max: usize) -> usize {
    // Float-to-int `as` saturates and maps NaN to 0.
    ((r * n as f64).floor() as usize).min(max)
}

/// Fisher-Yates driven by `rnd`. Returns a new vector; consumes exactly
/// `items.len() - 1` draws (none for fewer than two items).
pub fn shuffle<T: Clone>(items: &[T], rnd: &mut impl FnMut() -> f64) -> Vec<T> {
    let mut a = items.to_vec();
    for i in (1..a.len()).rev() {
        let j = draw_index(rnd(), i + 1, i);
        a.swap(i, j);
    }
    a
}

/// `[...deck.slice(i), ...deck.slice(0, i)]` with JS `slice` index rules
/// (negative counts from the end, out-of-range clamps).
fn js_rotate(deck: &[CardId], i: i64) -> Vec<CardId> {
    let len = i64::try_from(deck.len()).unwrap_or(i64::MAX);
    let at = if i < 0 { (len + i).max(0) } else { i.min(len) };
    let at = usize::try_from(at).unwrap_or(0);
    let mut out = Vec::with_capacity(deck.len());
    out.extend_from_slice(&deck[at..]);
    out.extend_from_slice(&deck[..at]);
    out
}

/// A single cut at a seeded index never within 4 of either end. Returns a new
/// vector; consumes exactly one draw.
pub fn cut_at(deck: &[CardId], rnd: &mut impl FnMut() -> f64) -> Vec<CardId> {
    let span = deck.len() as f64 - 8.0;
    let offset = (rnd() * span).floor();
    // `as` saturates; the deck is 24 cards in every real call.
    let i = 4 + offset as i64;
    js_rotate(deck, i)
}

/// The restart-safe cut: the index derives from `(seed, hand_no)` alone,
/// `4 + fnv1a("{seed}#{hand_no}#cut") % (len - 8)`. Returns a new vector.
pub fn cut_for_hand(deck: &[CardId], seed: &str, hand_no: u32) -> Vec<CardId> {
    let span = deck.len().abs_diff(8) as u64;
    if span == 0 {
        // JS: `h % 0` is NaN and `slice(NaN)` starts at 0, so the deck is unchanged.
        return deck.to_vec();
    }
    let h = u64::from(fnv1a(&format!("{seed}#{hand_no}#cut")));
    // JS `%` takes the dividend's sign, so a negative span acts as its magnitude.
    let i = 4 + i64::try_from(h % span).unwrap_or(0);
    js_rotate(deck, i)
}

/// The dealer for hand 0, drawn once at game creation. Consumes one draw.
pub fn pick_first_dealer(rnd: &mut impl FnMut() -> f64) -> Seat {
    // `floor(r * 4)` is 0..=3 for any `[0, 1)` draw; the clamp guards the rest.
    Seat::wrapping(draw_index(rnd(), 4, 3) as u8)
}

/// JS `Number.prototype.toString(36)` for an unsigned 32-bit integer.
pub fn to_base36(mut n: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(char::from(DIGITS[(n % 36) as usize]));
        n /= 36;
    }
    out.iter().rev().collect()
}

/* -------------------------------------------------------------------------- */
/* The deck                                                                    */
/* -------------------------------------------------------------------------- */

/// Why a deck order or deal plan was rejected. A programmer error (V10), never a rules event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DealError {
    /// The deck order is not 24 cards long.
    WrongLength {
        /// The length found.
        got: usize,
    },
    /// The deck order repeats a card.
    Duplicate,
    /// The deck order lacks a card of the canonical deck.
    Missing {
        /// The first missing card, in `DECK` order.
        card: CardId,
    },
    /// The plan does not deal exactly 20 cards.
    WrongPlanTotal {
        /// The total dealt by the plan.
        got: u32,
    },
}

impl fmt::Display for DealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DealError::WrongLength { got } => {
                write!(f, "deckOrder must hold {} cards, got {got}", DECK.len())
            }
            DealError::Duplicate => f.write_str("deckOrder contains duplicate cards"),
            DealError::Missing { card } => write!(f, "deckOrder is missing {card}"),
            DealError::WrongPlanTotal { got } => {
                write!(f, "deal plan must account for 20 cards, got {got}")
            }
        }
    }
}

impl std::error::Error for DealError {}

/// A fresh, mutable copy of the canonical 24-card deck.
pub fn new_deck() -> Vec<CardId> {
    DECK.to_vec()
}

/// The persisted 24-card permutation for one hand, before the cut.
pub fn shuffle_for_hand(seed: &str, hand_no: u32) -> Vec<CardId> {
    shuffle(&DECK, &mut hand_rng(seed, hand_no))
}

/// Checks that `deck_order` is a genuine permutation of the 24-card deck.
pub fn assert_deck_order(deck_order: &[CardId]) -> Result<(), DealError> {
    if deck_order.len() != DECK.len() {
        return Err(DealError::WrongLength {
            got: deck_order.len(),
        });
    }
    let mut seen = deck_order.to_vec();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != DECK.len() {
        return Err(DealError::Duplicate);
    }
    match DECK.iter().find(|c| !deck_order.contains(c)) {
        Some(&card) => Err(DealError::Missing { card }),
        None => Ok(()),
    }
}

/* -------------------------------------------------------------------------- */
/* The deal                                                                    */
/* -------------------------------------------------------------------------- */

/// Which alternation this hand uses: `hash_seed(seed, hand_no) & 1 == 1` means
/// the first packet is three cards (3-2-3-2, then 2-3-2-3).
pub fn starts_with_three(seed: &str, hand_no: u32) -> bool {
    hash_seed(seed, hand_no) & 1 == 1
}

/// The canonical 3-2 / 2-3 alternation in dealing order: two passes clockwise
/// from eldest, the second pass giving each seat the complement of its first.
pub fn deal_plan(dealer_seat: Seat, three_first: bool) -> Vec<DealPacket> {
    let mut order = [dealer_seat; 4];
    let mut s = next_seat(dealer_seat);
    for slot in &mut order {
        *slot = s;
        s = next_seat(s);
    }
    let first: Vec<DealPacket> = order
        .iter()
        .enumerate()
        .map(|(i, &seat)| DealPacket {
            seat,
            count: if (i % 2 == 0) == three_first { 3 } else { 2 },
        })
        .collect();
    let second = first.iter().map(|p| DealPacket {
        seat: p.seat,
        count: 5 - p.count,
    });
    let mut plan = first.clone();
    plan.extend(second);
    plan
}

/// The hands and kitty produced by [`deal_from`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DealtHands {
    /// All four hands, each in deal order (not sorted).
    pub hands: Hands,
    /// `deck_order[20..24]`; `kitty[0]` is the up-card.
    pub kitty: [CardId; 4],
}

/// Deals `deck_order` out according to `plan`, taking cards off the top in
/// packet order. Rejects a non-permutation deck or a plan that does not deal 20.
pub fn deal_from(deck_order: &[CardId], plan: &[DealPacket]) -> Result<DealtHands, DealError> {
    assert_deck_order(deck_order)?;
    let dealt: u32 = plan.iter().map(|p| u32::from(p.count)).sum();
    if dealt != 20 {
        return Err(DealError::WrongPlanTotal { got: dealt });
    }
    let mut hands: Hands = Default::default();
    let mut cards = deck_order.iter().copied();
    for p in plan {
        hands[p.seat.index()].extend(cards.by_ref().take(usize::from(p.count)));
    }
    let kitty = [
        deck_order[20],
        deck_order[21],
        deck_order[22],
        deck_order[23],
    ];
    Ok(DealtHands { hands, kitty })
}

/// The unredacted `dealt` step payload: the plan's packets, each carrying the
/// cards it delivered. Server-side only; `project_steps` redacts it (V12).
pub fn dealt_packets(deck_order: &[CardId], plan: &[DealPacket]) -> Vec<DealtPacket> {
    let mut i = 0usize;
    plan.iter()
        .map(|p| {
            let start = i.min(deck_order.len());
            let end = (i + usize::from(p.count)).min(deck_order.len());
            i += usize::from(p.count);
            DealtPacket {
                seat: p.seat,
                count: p.count,
                cards: Some(deck_order[start..end].to_vec()),
            }
        })
        .collect()
}

/// Everything one deal produces, ready to be written into a fresh `HandState`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DealResult {
    /// The 24-card permutation actually dealt from (post-cut, if a cut was taken).
    pub deck_order: Vec<CardId>,
    /// The eight packets in dealing order.
    pub plan: Vec<DealPacket>,
    /// All four hands.
    pub hands: Hands,
    /// The four undealt cards.
    pub kitty: [CardId; 4],
    /// `kitty[0]`, turned face up.
    pub up_card: CardId,
}

/// The inputs of [`deal_hand`].
#[derive(Clone, Copy, Debug)]
pub struct DealHandParams<'a> {
    /// The persisted (and already cut, if bumped) deck order.
    pub deck_order: &'a [CardId],
    /// The dealer.
    pub dealer_seat: Seat,
    /// The match seed.
    pub seed: &'a str,
    /// The hand number.
    pub hand_no: u32,
}

/// Deals one hand from an already-persisted, already-cut deck order. Neither
/// shuffles nor cuts; the alternation derives from `(seed, hand_no)`.
pub fn deal_hand(params: DealHandParams<'_>) -> Result<DealResult, DealError> {
    let plan = deal_plan(
        params.dealer_seat,
        starts_with_three(params.seed, params.hand_no),
    );
    let DealtHands { hands, kitty } = deal_from(params.deck_order, &plan)?;
    Ok(DealResult {
        deck_order: params.deck_order.to_vec(),
        plan,
        hands,
        kitty,
        up_card: kitty[0],
    })
}
