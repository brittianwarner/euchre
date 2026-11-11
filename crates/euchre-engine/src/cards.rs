//! Card identity, colour, and the bower-aware ranking primitives (port of `cards.ts`).
//!
//! This is the **only** module permitted to compare the result of [`suit_of`].
//! Voidness, follow-suit legality, led-suit determination and the trick
//! comparator all route through [`effective_suit`] instead; a `suit_of()`
//! comparison anywhere else is a bug, because it silently gets the left bower wrong.

use crate::types::{Card, CardId, Rank, Suit};

/// Canonical suit order. Iteration order is load-bearing: it fixes [`DECK`].
pub const SUITS: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

/// Canonical rank order, low to high in a plain suit.
pub const RANKS: [Rank; 6] = [
    Rank::Nine,
    Rank::Ten,
    Rank::Jack,
    Rank::Queen,
    Rank::King,
    Rank::Ace,
];

/// Builds the canonical deck at compile time (suit-major, rank ascending).
const fn build_deck() -> [Card; 24] {
    let mut out = [Card::new(Rank::Nine, Suit::Spades); 24];
    let mut s = 0;
    while s < 4 {
        let mut r = 0;
        while r < 6 {
            out[s * 6 + r] = Card::new(RANKS[r], SUITS[s]);
            r += 1;
        }
        s += 1;
    }
    out
}

/// The 24-card euchre deck in canonical, unshuffled order (suit-major, rank
/// ascending). Deterministic shuffling starts from exactly this permutation.
pub const DECK: [CardId; 24] = build_deck();

/// Alias of [`DECK`], under the name used by the TS file manifest.
pub const CARD_IDS: [CardId; 24] = DECK;

/// The rank half of a card id. `rank_of("JD") == J`.
pub const fn rank_of(c: CardId) -> Rank {
    c.rank()
}

/// The **printed** suit of a card. Do not compare this outside `cards.rs`; use [`effective_suit`].
pub const fn suit_of(c: CardId) -> Suit {
    c.printed_suit()
}

/// The other suit of the same colour: `S <-> C` black, `H <-> D` red.
pub const fn color_mate(s: Suit) -> Suit {
    match s {
        Suit::Spades => Suit::Clubs,
        Suit::Clubs => Suit::Spades,
        Suit::Hearts => Suit::Diamonds,
        Suit::Diamonds => Suit::Hearts,
    }
}

/// Whether two suits share a colour. Reflexive: `same_color(s, s)` is `true`.
pub fn same_color(a: Suit, b: Suit) -> bool {
    a == b || color_mate(a) == b
}

/// The suit a card **behaves as**: identity, except the left bower reports as
/// trump. `effective_suit(c, None) == suit_of(c)` (V15).
pub fn effective_suit(card: CardId, trump: Option<Suit>) -> Suit {
    let s = suit_of(card);
    match trump {
        Some(t) if rank_of(card) == Rank::Jack && s == color_mate(t) => t,
        _ => s,
    }
}

/// Rank within the trump suit, or `0` for any card that is not trump.
/// Right bower 8, left bower 7, A 6, K 5, Q 4, T 3, 9 2.
pub fn trump_rank(card: CardId, trump: Suit) -> u8 {
    if effective_suit(card, Some(trump)) != trump {
        return 0;
    }
    match rank_of(card) {
        Rank::Jack => {
            if suit_of(card) == trump {
                8
            } else {
                7
            }
        }
        Rank::Ace => 6,
        Rank::King => 5,
        Rank::Queen => 4,
        Rank::Ten => 3,
        Rank::Nine => 2,
    }
}

/// Rank within a plain suit: A 6, K 5, Q 4, J 3, T 2, 9 1. Meaningful only for non-trump cards.
pub const fn plain_rank(card: CardId) -> u8 {
    match rank_of(card) {
        Rank::Ace => 6,
        Rank::King => 5,
        Rank::Queen => 4,
        Rank::Jack => 3,
        Rank::Ten => 2,
        Rank::Nine => 1,
    }
}

/// The two jacks that are trump under `trump`, as `[right, left]`.
pub const fn bowers_of(trump: Suit) -> [CardId; 2] {
    [
        Card::new(Rank::Jack, trump),
        Card::new(Rank::Jack, color_mate(trump)),
    ]
}

/// Whether `card` is the left bower under `trump`; `false` when `trump` is `None`.
pub fn is_left_bower(card: CardId, trump: Option<Suit>) -> bool {
    match trump {
        Some(t) => rank_of(card) == Rank::Jack && suit_of(card) == color_mate(t),
        None => false,
    }
}
