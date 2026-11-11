//! Public, deterministic partnership facts for Jev (`ai/partnership.ts`).
//! Never chooses a move. Field order is the TS key order.

use euchre_engine::{
    Card, DECK, LegalMove, LegalMoveId, Move, Play, PublicGameView, Seat, Suit, Team, card_name,
    card_value, currently_winning, effective_suit, next_active_seat, partner_of, plain_rank,
    plays_per_trick, team_of, trump_rank,
};
use serde::Serialize;

use crate::heuristic::void_reads;

/// The trick-level partnership facts (`context`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartnershipContext {
    /// This seat's team.
    pub your_team: Team,
    /// Partner's seat.
    pub partner_seat: Seat,
    /// Partner sits out a loner.
    pub partner_sitting_out: bool,
    /// The play currently winning the trick.
    pub current_winner: Option<Play>,
    /// Partner is winning.
    pub partner_winning: bool,
    /// Seats still to play after this one.
    pub seats_after_you: Vec<Seat>,
    /// The opponents among them.
    pub opponents_after_you: Vec<Seat>,
    /// Partner is winning and no unseen card can beat it.
    pub partner_trick_secured: bool,
    /// Unseen cards a later opponent might beat the winner with.
    pub possible_opponent_beaters: Vec<Card>,
    /// Always `true`: unseen cards are possibilities.
    pub unseen_cards_are_possibilities_not_known_holdings: bool,
    /// Our tricks.
    pub your_team_tricks: u8,
    /// Their tricks.
    pub opponent_tricks: u8,
    /// We have three.
    pub your_team_has_won_hand: bool,
    /// The makers' team.
    pub maker_team: Option<Team>,
    /// The makers have three.
    pub makers_have_three_tricks: bool,
    /// The defenders have none.
    pub makers_can_still_sweep: bool,
    /// A loner has lost a trick.
    pub loner_sweep_stopped: bool,
}

/// One card-play option with its consequences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartnershipOption {
    /// The legal move id.
    pub id: LegalMoveId,
    /// The card.
    pub card: Card,
    /// Its effective suit.
    pub effective_suit: Suit,
    /// Trump rank or plain rank.
    pub rank: u8,
    /// Plain-language resource class.
    pub resource: &'static str,
    /// The trick winner once played.
    pub winner_after_play: Play,
    /// Takes the lead from a winning partner.
    pub overtakes_partner: bool,
    /// Unseen cards that could still beat the winner.
    pub possible_opponent_beaters_after_play: Vec<Card>,
    /// What stays in hand.
    pub remaining_hand: Vec<Card>,
    /// Lower cards of the same suit with identical consequences.
    pub cheaper_equivalent_cards: Vec<Card>,
    /// One plain-language paragraph.
    pub description: String,
}

/// `analyzePartnership(view, legal)`; `None` outside trick play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PartnershipAnalysis {
    /// Trick-level facts.
    pub context: PartnershipContext,
    /// Per-option facts, in legal order.
    pub options: Vec<PartnershipOption>,
}

/// Parameters of [`analyze_partnership`].
#[derive(Clone, Copy, Debug)]
pub struct PartnershipParams<'a> {
    /// The redacted view.
    pub view: &'a PublicGameView,
    /// The legal (candidate) set.
    pub legal: &'a [LegalMove],
}

struct RawOption {
    id: LegalMoveId,
    card: Card,
    effective_suit: Suit,
    rank: u8,
    resource: &'static str,
    winner_after_play: Play,
    overtakes_partner: bool,
    beaters: Vec<Card>,
    remaining_hand: Vec<Card>,
}

/// `analyzePartnership(view, legal)`.
pub fn analyze_partnership(params: PartnershipParams<'_>) -> Option<PartnershipAnalysis> {
    let PartnershipParams { view, legal } = params;
    let trump = view.trump?;
    if view.phase != euchre_engine::GamePhase::TrickPlay {
        return None;
    }
    let team = team_of(view.you);
    let partner = partner_of(view.you);
    let plays = &view.trick.plays;
    let winner = currently_winning(plays, Some(trump));
    let mut seats_after_you = Vec::new();
    let mut next = view.you;
    let mut i = plays.len() + 1;
    while i < usize::from(plays_per_trick(view.sitting_seat)) {
        next = next_active_seat(next, view.sitting_seat);
        seats_after_you.push(next);
        i += 1;
    }
    let opponents_after_you: Vec<Seat> = seats_after_you
        .iter()
        .copied()
        .filter(|s| team_of(*s) != team)
        .collect();

    let mut known: Vec<Card> = view.hand.clone();
    for t in view.trick_log.iter().chain(std::iter::once(&view.trick)) {
        known.extend(t.plays.iter().map(|p| p.card));
    }
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
    let possible_beaters = |card: Card, led: Card| -> Vec<Card> {
        let suit = effective_suit(led, Some(trump));
        unseen
            .iter()
            .copied()
            .filter(|cand| {
                card_value(*cand, suit, Some(trump)) > card_value(card, suit, Some(trump))
                    && opponents_after_you
                        .iter()
                        .any(|s| !voids[s.index()].contains(&effective_suit(*cand, Some(trump))))
            })
            .collect()
    };
    let first = plays.first();
    let partner_winning = winner.is_some_and(|w| w.seat == partner);
    let threats = match (winner, first) {
        (Some(w), Some(f)) => possible_beaters(w.card, f.card),
        _ => Vec::new(),
    };
    let maker_team = view.maker_seat.map(team_of);
    let won = view.tricks_won;
    let t = team.index();
    let context = PartnershipContext {
        your_team: team,
        partner_seat: partner,
        partner_sitting_out: Some(partner) == view.sitting_seat,
        current_winner: winner,
        partner_winning,
        seats_after_you,
        opponents_after_you: opponents_after_you.clone(),
        partner_trick_secured: partner_winning && threats.is_empty(),
        possible_opponent_beaters: threats,
        unseen_cards_are_possibilities_not_known_holdings: true,
        your_team_tricks: won[t],
        opponent_tricks: won[1 - t],
        your_team_has_won_hand: won[t] >= 3,
        maker_team,
        makers_have_three_tricks: maker_team.is_some_and(|m| won[m.index()] >= 3),
        makers_can_still_sweep: maker_team.is_some_and(|m| won[1 - m.index()] == 0),
        loner_sweep_stopped: view.alone_seat.is_some()
            && maker_team.is_some_and(|m| won[1 - m.index()] > 0),
    };

    let mut raw: Vec<RawOption> = Vec::new();
    for m in legal {
        let Move::Play { card } = m.mv else { continue };
        let mut with_mine = plays.clone();
        with_mine.push(Play {
            seat: view.you,
            card,
        });
        let after = currently_winning(&with_mine, Some(trump)).unwrap_or(Play {
            seat: view.you,
            card,
        });
        let es = effective_suit(card, Some(trump));
        let is_trump = es == trump;
        let rank = if is_trump {
            trump_rank(card, trump)
        } else {
            plain_rank(card)
        };
        let resource = if is_trump {
            match rank {
                8 => "right bower, highest trump",
                7 => "left bower, second-highest trump",
                r if r >= 5 => "high trump",
                _ => "low trump",
            }
        } else if rank == 6 {
            "plain ace"
        } else if rank <= 3 {
            "low plain card"
        } else {
            "plain court card"
        };
        raw.push(RawOption {
            id: m.id.clone(),
            card,
            effective_suit: es,
            rank,
            resource,
            winner_after_play: after,
            overtakes_partner: partner_winning && after.seat == view.you,
            beaters: possible_beaters(after.card, first.map_or(card, |f| f.card)),
            remaining_hand: view.hand.iter().copied().filter(|h| *h != card).collect(),
        });
    }

    let names = |cards: &[Card], sep: &str| {
        cards
            .iter()
            .map(|c| card_name(*c))
            .collect::<Vec<_>>()
            .join(sep)
    };

    let options = raw
        .iter()
        .map(|option| {
            let cheaper: Vec<Card> = raw
                .iter()
                .filter(|other| {
                    other.effective_suit == option.effective_suit
                        && other.rank < option.rank
                        && other.winner_after_play.seat == option.winner_after_play.seat
                        && other.beaters == option.beaters
                })
                .map(|other| other.card)
                .collect();
            let team_winning = team_of(option.winner_after_play.seat) == team;
            let secured = team_winning && option.beaters.is_empty();
            let mut parts: Vec<String> = Vec::with_capacity(5);
            parts.push(format!(
                "Play {} ({}).",
                card_name(option.card),
                option.resource
            ));
            parts.push(
                if secured {
                    "Our team is guaranteed this trick against the remaining opponents."
                } else if team_winning {
                    "Our team leads, but a remaining opponent could still beat it."
                } else {
                    "An opponent remains ahead."
                }
                .to_owned(),
            );
            let third = if option.overtakes_partner {
                if context.partner_trick_secured {
                    "Overtakes an already secured partner winner without improving protection; only a justified transfer of the next lead could help.".to_owned()
                } else if secured {
                    "Protects our vulnerable partner by removing every possible opponent beater."
                        .to_owned()
                } else {
                    "Takes the lead away from our partner, but opponents can still beat it."
                        .to_owned()
                }
            } else if option.winner_after_play.seat == partner {
                if secured {
                    "Saves our stronger cards while our partner wins the trick.".to_owned()
                } else {
                    format!(
                        "Leaves our partner exposed to {} from a remaining opponent.",
                        names(&option.beaters, ", ")
                    )
                }
            } else {
                String::new()
            };
            parts.push(third);
            parts.push(if cheaper.is_empty() {
                String::new()
            } else {
                format!(
                    "Unnecessary expenditure: {} keeps the same winning seat and protection while saving this stronger card.",
                    names(&cheaper, " or ")
                )
            });
            parts.push(format!(
                "Keeps {} for later tricks.",
                names(&option.remaining_hand, ", ")
            ));
            let description = parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            PartnershipOption {
                id: option.id.clone(),
                card: option.card,
                effective_suit: option.effective_suit,
                rank: option.rank,
                resource: option.resource,
                winner_after_play: option.winner_after_play,
                overtakes_partner: option.overtakes_partner,
                possible_opponent_beaters_after_play: option.beaters.clone(),
                remaining_hand: option.remaining_hand.clone(),
                cheaper_equivalent_cards: cheaper,
                description,
            }
        })
        .collect();

    Some(PartnershipAnalysis { context, options })
}
