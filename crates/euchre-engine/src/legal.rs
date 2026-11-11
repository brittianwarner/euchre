//! What may happen next (port of `legal.ts`).
//!
//! Two layers. [`get_legal_plays`] is the card-level follow-suit primitive.
//! [`legal_moves`] is the move-level API every consumer calls: the table (to
//! validate), the AI seats (to build their choice set), the client (to render
//! and hit-test), and the coach. Because the legal set is computed here and
//! nowhere else, reneging is structurally impossible.

use crate::cards::{SUITS, effective_suit, is_left_bower, rank_of, suit_of};
use crate::seats::partner_of;
use crate::types::{CardId, GamePhase, GameState, GameStatus, LegalMove, Move, Rank, Seat, Suit};

/* -------------------------------------------------------------------------- */
/* Naming                                                                      */
/* -------------------------------------------------------------------------- */

/// Display name of each suit, indexed by `Suit::index()` (`SUITS` order). Also the round-2 call label.
pub const SUIT_NAME: [&str; 4] = ["Spades", "Hearts", "Diamonds", "Clubs"];

/// Singular lowercase suit noun, indexed by `Suit::index()`, for mid-sentence copy.
pub const SUIT_NOUN: [&str; 4] = ["spade", "heart", "diamond", "club"];

/// Display name of each rank, indexed by `Rank::index()` (`RANKS` order).
pub const RANK_NAME: [&str; 6] = ["Nine", "Ten", "Jack", "Queen", "King", "Ace"];

/// Display name of a suit, e.g. `"Hearts"`.
pub const fn suit_name(s: Suit) -> &'static str {
    SUIT_NAME[s.index()]
}

/// Singular lowercase suit noun, e.g. `"heart"`.
pub const fn suit_noun(s: Suit) -> &'static str {
    SUIT_NOUN[s.index()]
}

/// Display name of a rank, e.g. `"Queen"`.
pub const fn rank_name(r: Rank) -> &'static str {
    RANK_NAME[r.index()]
}

/// `"Jack of diamonds"`: the spoken/UI label for a card (always the printed card).
pub fn card_name(c: CardId) -> String {
    format!(
        "{} of {}",
        rank_name(rank_of(c)),
        suit_name(suit_of(c)).to_lowercase()
    )
}

/// `"jack of diamonds"`: [`card_name`] lowercased, for mid-sentence copy.
pub fn card_name_lower(c: CardId) -> String {
    format!(
        "{} of {}",
        rank_name(rank_of(c)).to_lowercase(),
        suit_name(suit_of(c)).to_lowercase()
    )
}

/// The round-1 label for accepting the up-card; only the spoken line differs by role.
pub fn order_label(seat: Seat, dealer: Seat) -> &'static str {
    if seat == dealer {
        "I take it"
    } else if seat == partner_of(dealer) {
        "I assist"
    } else {
        "Order it up"
    }
}

/// Joins names as `"a"`, `"a and b"`, `"a, b and c"`.
fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/* -------------------------------------------------------------------------- */
/* The card-level primitive                                                    */
/* -------------------------------------------------------------------------- */

/// The cards this hand may legally play: those of the led **effective** suit if
/// any, otherwise everything. `led_suit == None` means on lead. Never returns
/// `[]` for a non-empty hand (V19).
pub fn get_legal_plays(
    hand: &[CardId],
    led_suit: Option<Suit>,
    trump: Option<Suit>,
) -> Vec<CardId> {
    let Some(led) = led_suit else {
        return hand.to_vec();
    };
    let following: Vec<CardId> = hand
        .iter()
        .copied()
        .filter(|&c| effective_suit(c, trump) == led)
        .collect();
    if following.is_empty() {
        hand.to_vec()
    } else {
        following
    }
}

/// The inputs of [`why_illegal`].
#[derive(Clone, Copy, Debug)]
pub struct WhyIllegalParams<'a> {
    /// The seat's hand.
    pub hand: &'a [CardId],
    /// The card the seat tried to play.
    pub card: CardId,
    /// The led effective suit, or `None` on lead.
    pub led_suit: Option<Suit>,
    /// Trump, if named.
    pub trump: Option<Suit>,
}

/// Why a specific card may not be played, or `None` when the play is legal.
pub fn why_illegal(params: WhyIllegalParams<'_>) -> Option<String> {
    let WhyIllegalParams {
        hand,
        card,
        led_suit,
        trump,
    } = params;
    if !hand.contains(&card) {
        return Some("That card isn't in your hand.".to_owned());
    }
    let led = led_suit?;
    if effective_suit(card, trump) == led {
        return None;
    }
    let following: Vec<CardId> = hand
        .iter()
        .copied()
        .filter(|&c| effective_suit(c, trump) == led)
        .collect();
    if following.is_empty() {
        return None;
    }
    let opener = format!("{} were led.", suit_name(led));
    if let Some(&bower) = following.iter().find(|&&c| is_left_bower(c, trump)) {
        return Some(format!(
            "{opener} Your {} is a {} right now \u{2014} you have to follow.",
            card_name(bower),
            suit_noun(led)
        ));
    }
    let held: Vec<String> = following
        .iter()
        .map(|&c| format!("the {}", card_name_lower(c)))
        .collect();
    Some(format!(
        "{opener} You still hold {} \u{2014} you have to follow suit.",
        join_list(&held)
    ))
}

/* -------------------------------------------------------------------------- */
/* Stick the dealer                                                            */
/* -------------------------------------------------------------------------- */

/// The stuck-dealer predicate (V3): stick-the-dealer on, round 2, this seat is
/// the dealer, and the other three have passed. `pass` is then removed.
pub fn is_stuck_dealer(state: &GameState, seat: Seat) -> bool {
    let h = &state.hand;
    state.cfg.stick_the_dealer
        && h.phase == GamePhase::BidRound2
        && seat == h.dealer_seat
        && h.passes == 3
}

/* -------------------------------------------------------------------------- */
/* The move-level API                                                          */
/* -------------------------------------------------------------------------- */

/// A `LegalMove` whose id is derived from the move.
fn offer(mv: Move, label: impl Into<String>) -> LegalMove {
    LegalMove {
        id: mv.id(),
        mv,
        label: label.into(),
    }
}

/// Whether `hand` holds a card whose **printed** suit is `s` (the natural-trump test).
fn holds_printed(hand: &[CardId], s: Suit) -> bool {
    hand.iter().any(|&c| suit_of(c) == s)
}

/// The single source of truth for what `seat` may do right now. `[]` for any
/// seat not to act and for the loner's sitting partner; never empty for the
/// acting seat (V19). Pure.
pub fn legal_moves(state: &GameState, seat: Seat) -> Vec<LegalMove> {
    let h = &state.hand;
    if state.status != GameStatus::Active
        || Some(seat) == h.sitting_seat
        || h.turn_seat != Some(seat)
    {
        return Vec::new();
    }
    let hand = &h.hands[seat.index()];
    let natural = state.cfg.natural_trump_required();

    match h.phase {
        GamePhase::Cutting => vec![
            offer(Move::Cut { cut: true }, "Bump"),
            offer(Move::Cut { cut: false }, "Run 'em"),
        ],

        GamePhase::BidRound1 => {
            let order = order_label(seat, h.dealer_seat);
            if let Some(up) = h.up_card
                && natural
                && seat != h.dealer_seat
                && !holds_printed(hand, suit_of(up))
            {
                return vec![offer(Move::Pass, "Pass")];
            }
            vec![
                offer(Move::Pass, "Pass"),
                offer(Move::OrderUp { alone: false }, order),
                offer(Move::OrderUp { alone: true }, format!("{order}, alone")),
            ]
        }

        // Six cards after the pickup; discarding the up-card itself is legal.
        GamePhase::DealerDiscard => hand
            .iter()
            .map(|&c| offer(Move::Discard { card: c }, card_name(c)))
            .collect(),

        GamePhase::BidRound2 => {
            let stuck = is_stuck_dealer(state, seat);
            let mut out = if stuck {
                Vec::new()
            } else {
                vec![offer(Move::Pass, "Pass")]
            };
            for s in SUITS {
                if Some(s) == h.turned_down_suit {
                    continue;
                }
                if natural && !stuck && !holds_printed(hand, s) {
                    continue;
                }
                out.push(offer(
                    Move::Call {
                        suit: s,
                        alone: false,
                    },
                    suit_name(s),
                ));
                out.push(offer(
                    Move::Call {
                        suit: s,
                        alone: true,
                    },
                    format!("{}, alone", suit_name(s)),
                ));
            }
            out
        }

        GamePhase::TrickPlay => get_legal_plays(hand, h.trick.led_suit, h.trump)
            .into_iter()
            .map(|c| offer(Move::Play { card: c }, card_name(c)))
            .collect(),

        // Engine-only phases.
        GamePhase::Lobby
        | GamePhase::Deal
        | GamePhase::TrickResolve
        | GamePhase::HandScore
        | GamePhase::GameOver => Vec::new(),
    }
}

/// The legal move ids for `seat`, in [`legal_moves`] order.
pub fn legal_move_ids(state: &GameState, seat: Seat) -> Vec<String> {
    legal_moves(state, seat).into_iter().map(|m| m.id).collect()
}

/// Looks up one offered move by id, or `None` if it is not legal for `seat` now.
pub fn find_legal_move(state: &GameState, seat: Seat, id: &str) -> Option<LegalMove> {
    legal_moves(state, seat).into_iter().find(|m| m.id == id)
}
