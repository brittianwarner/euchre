//! Evidence a person in this chair could remember (`ai/table-context.ts`).
//! Reads only the public view; field order is the TS key order.

use euchre_engine::{
    Bid, Card, DECK, PublicGameView, SUITS, Seat, Suit, TableObservation, TableRules,
    effective_suit, partner_of, same_color, team_of, trump_rank,
};
use serde::Serialize;

use crate::heuristic::void_reads;
use crate::notation::split_tricks;

/// Cards in hand by effective suit, keyed `S H D C`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SuitCounts {
    /// Spades.
    #[serde(rename = "S")]
    pub s: usize,
    /// Hearts.
    #[serde(rename = "H")]
    pub h: usize,
    /// Diamonds.
    #[serde(rename = "D")]
    pub d: usize,
    /// Clubs.
    #[serde(rename = "C")]
    pub c: usize,
}

impl SuitCounts {
    /// The count for one suit.
    pub fn get(&self, suit: Suit) -> usize {
        match suit {
            Suit::Spades => self.s,
            Suit::Hearts => self.h,
            Suit::Diamonds => self.d,
            Suit::Clubs => self.c,
        }
    }
}

/// What one other seat has shown publicly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRead {
    /// The seat.
    pub seat: Seat,
    /// `"partner"` or `"opponent"`.
    pub relation: &'static str,
    /// Suits it has failed to follow.
    pub proved_voids: Vec<Suit>,
    /// Cards it has played this hand.
    pub cards_played: Vec<Card>,
    /// Completed tricks it won.
    pub tricks_taken: usize,
    /// Cards it still holds.
    pub remaining_cards: u8,
}

/// One auction entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AuctionCall {
    /// `1` or `2`.
    pub round: u8,
    /// The bidder.
    pub seat: Seat,
    /// The bid.
    #[serde(rename = "move")]
    pub mv: Bid,
}

/// The auction summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Auction {
    /// Partner deals.
    pub dealer_is_partner: bool,
    /// You deal.
    pub dealer_is_you: bool,
    /// The up-card.
    pub up_card: Option<Card>,
    /// Where the up-card is.
    pub up_card_location: &'static str,
    /// The same-colour suit of the turned-down one.
    pub next_suit: Option<Suit>,
    /// Every bid so far.
    pub calls: Vec<AuctionCall>,
    /// Public rule flags.
    pub rules: TableRules,
}

/// `tableContext(view)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableContext {
    /// Partner's seat.
    pub partner_seat: Seat,
    /// `auction`, `maker` or `defender`.
    pub your_role: &'static str,
    /// The hand objective in one sentence.
    pub objective: &'static str,
    /// `[us, them]`, literal 10 as in TS.
    pub points_needed_to_ten: [u8; 2],
    /// Hand counts by effective suit.
    pub cards_in_each_effective_suit: SuitCounts,
    /// Suits with zero cards.
    pub your_voids: Vec<Suit>,
    /// Unseen trump, highest first.
    pub unseen_trump: Vec<Card>,
    /// Caveat for `unseen_trump`.
    pub unseen_trump_warning: &'static str,
    /// The three other seats.
    pub public_reads: Vec<PublicRead>,
    /// The auction.
    pub auction: Auction,
    /// This hand's public timing log, last 32.
    pub observed_actions: Vec<TableObservation>,
    /// Caveat for `observed_actions`.
    pub timing_caution: &'static str,
}

/// `tableContext(view)`.
pub fn table_context(view: &PublicGameView) -> TableContext {
    let (current, past) = split_tricks(view);
    let played: Vec<euchre_engine::Play> = past
        .iter()
        .copied()
        .chain(current)
        .flat_map(|t| t.plays.iter().copied())
        .collect();
    let mut known: Vec<Card> = view.hand.clone();
    known.extend(played.iter().map(|p| p.card));
    if view.up_card_turned_down
        && let Some(up) = view.up_card
    {
        known.push(up);
    }
    let unseen: Vec<Card> = DECK
        .iter()
        .copied()
        .filter(|c| !known.contains(c))
        .collect();
    let voids = void_reads(view);
    let team = team_of(view.you).index();
    let maker_team = view.maker_seat.map(|m| team_of(m).index());
    let count_of = |suit: Suit| {
        view.hand
            .iter()
            .filter(|c| effective_suit(**c, view.trump) == suit)
            .count()
    };
    let suit_counts = SuitCounts {
        s: count_of(Suit::Spades),
        h: count_of(Suit::Hearts),
        d: count_of(Suit::Diamonds),
        c: count_of(Suit::Clubs),
    };
    let unseen_trump = match view.trump {
        None => Vec::new(),
        Some(trump) => {
            let mut t: Vec<Card> = unseen
                .iter()
                .copied()
                .filter(|c| effective_suit(*c, Some(trump)) == trump)
                .collect();
            t.sort_by_key(|c| std::cmp::Reverse(trump_rank(*c, trump)));
            t
        }
    };
    let won = view.tricks_won;
    let objective = match maker_team {
        None => "Choose a bid for expected partnership points; account for who deals.",
        Some(m) if m == team => {
            if won[team] >= 3 {
                if won[1 - team] == 0 {
                    "Hand made; pursue the sweep for the extra points."
                } else {
                    "Hand made; conserve resources and finish soundly."
                }
            } else {
                "Take three tricks to make the hand; keep a route to the next lead."
            }
        }
        Some(_) => {
            if view.alone_seat.is_some() && won[team] == 0 {
                "Stop the four-point loner sweep; then pursue three tricks for a euchre."
            } else {
                "Take three tricks to euchre; partner winners count equally."
            }
        }
    };
    let partner = partner_of(view.you);
    let public_reads = Seat::ALL
        .iter()
        .copied()
        .filter(|s| *s != view.you)
        .map(|seat| PublicRead {
            seat,
            relation: if seat == partner {
                "partner"
            } else {
                "opponent"
            },
            proved_voids: voids[seat.index()].clone(),
            cards_played: played
                .iter()
                .filter(|p| p.seat == seat)
                .map(|p| p.card)
                .collect(),
            tricks_taken: past.iter().filter(|t| t.winner_seat == Some(seat)).count(),
            remaining_cards: view.hand_counts[seat.index()],
        })
        .collect();
    let up_card_location = if view.up_card_turned_down {
        "out of play"
    } else if view.maker_seat.is_none() {
        "on the table"
    } else if view.turned_down_suit.is_none() {
        "dealer picked it up; it may subsequently have been discarded or played"
    } else {
        "out of play"
    };
    let next_suit = view.turned_down_suit.and_then(|down| {
        SUITS
            .iter()
            .copied()
            .find(|s| *s != down && same_color(*s, down))
    });
    let calls = view
        .bids
        .iter()
        .enumerate()
        .map(|(i, b)| AuctionCall {
            round: if i < 4 { 1 } else { 2 },
            seat: b.seat,
            mv: b.mv,
        })
        .collect();
    let observed: Vec<TableObservation> = view
        .observations
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|o| o.hand_no == view.hand_no)
        .cloned()
        .collect();
    let skip = observed.len().saturating_sub(32);
    TableContext {
        partner_seat: partner,
        your_role: match maker_team {
            None => "auction",
            Some(m) if m == team => "maker",
            Some(_) => "defender",
        },
        objective,
        points_needed_to_ten: [
            10u8.saturating_sub(view.score[team]),
            10u8.saturating_sub(view.score[1 - team]),
        ],
        cards_in_each_effective_suit: suit_counts,
        your_voids: SUITS
            .iter()
            .copied()
            .filter(|s| suit_counts.get(*s) == 0)
            .collect(),
        unseen_trump,
        unseen_trump_warning: "Possible cards in opponents, partner, or buried kitty/discard. Not known holdings or calibrated probabilities.",
        public_reads,
        auction: Auction {
            dealer_is_partner: view.dealer_seat == partner,
            dealer_is_you: view.dealer_seat == view.you,
            up_card: view.up_card,
            up_card_location,
            next_suit,
            calls,
            rules: view.rules.unwrap_or(TableRules {
                stick_the_dealer: false,
                require_natural_trump: false,
            }),
        },
        observed_actions: observed.into_iter().skip(skip).collect(),
        timing_caution: "Elapsed time is server-observed, includes network and distraction. Computer timing is deliberately paced, not a tell. Human timing is weak context only: never infer a specific card, override card evidence, delay deliberately to signal, or punish slow players.",
    }
}
