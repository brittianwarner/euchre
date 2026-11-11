//! Shared test helpers: the TS regression positions (`scripts/fixtures/*.ts`,
//! converted by `scripts/fixtures.ts`) and Rust ports of their `playView` and
//! `bidView` builders.

#![allow(dead_code)]

use euchre_engine::{
    Card, GameInit, GamePhase, GameStatus, LegalMove, Move, Play, PublicGameView, Seat, Suit,
    TableRules, Trick, card_name, effective_suit, get_legal_plays, legal_moves, project,
    try_create_game,
};
use euchre_protocol::{PersonaConfig, default_personas};
use serde::Deserialize;
use serde_json::Value;

/// One converted case.
#[derive(Deserialize)]
pub struct Case {
    /// The TS case name.
    pub name: String,
    /// The seat-visible view.
    pub view: PublicGameView,
    /// The expected move id(s).
    pub expected: Value,
    /// TS `rankMoves(view)` line with `top=`.
    pub rank: String,
    /// TS Jev request body for Rita.
    pub jev: String,
}

fn load(text: &str) -> Vec<Case> {
    serde_json::from_str(text).unwrap()
}

/// `biddingCases`.
pub fn bidding_cases() -> Vec<Case> {
    load(include_str!("../fixtures/bidding.json"))
}
/// `acceptanceBiddingCases`.
pub fn acceptance_cases() -> Vec<Case> {
    load(include_str!("../fixtures/bidding-acceptance.json"))
}
/// `partnershipCases`.
pub fn partnership_cases() -> Vec<Case> {
    load(include_str!("../fixtures/partnership.json"))
}
/// `strategyCases`.
pub fn strategy_cases() -> Vec<Case> {
    load(include_str!("../fixtures/strategy.json"))
}

/// A card literal.
pub fn c(s: &str) -> Card {
    Card::parse(s).unwrap()
}
/// Card literals.
pub fn cards(list: &[&str]) -> Vec<Card> {
    list.iter().map(|s| c(s)).collect()
}
/// A seat literal.
pub fn seat(n: u8) -> Seat {
    Seat::new(n).unwrap()
}
/// A suit literal.
pub fn suit(ch: char) -> Suit {
    Suit::from_char(ch).unwrap()
}
/// A play literal.
pub fn play(s: u8, card: &str) -> Play {
    Play {
        seat: seat(s),
        card: c(card),
    }
}

/// The default personas' configs: Rita, Ned, Grandpa.
pub fn personas() -> Vec<PersonaConfig> {
    default_personas().into_iter().map(|a| a.persona).collect()
}

/// `playView(hand, plays, overrides)`: `trump` is `overrides.trump ?? 'H'`;
/// `patch` applies the remaining overrides before `legal` is derived.
pub fn play_view(
    hand: &[&str],
    plays: &[Play],
    trump: Option<Suit>,
    patch: impl FnOnce(&mut PublicGameView),
) -> PublicGameView {
    let state = try_create_game(GameInit {
        game_id: "partnership-check".to_owned(),
        seed: "partnership-check".to_owned(),
        cfg: None,
        internal_token: None,
        first_dealer: None,
    })
    .unwrap();
    let mut view = project(&state, seat(2));
    let trump = trump.unwrap_or(Suit::Hearts);
    let hand = cards(hand);
    let led = plays.first().map(|p| effective_suit(p.card, Some(trump)));
    view.phase = GamePhase::TrickPlay;
    view.status = GameStatus::Active;
    view.turn_seat = Some(seat(2));
    view.trump = Some(trump);
    view.maker_seat = Some(seat(0));
    view.hand = hand.clone();
    for s in Seat::ALL {
        view.hand_counts[s.index()] =
            (hand.len() - usize::from(plays.iter().any(|p| p.seat == s))) as u8;
    }
    view.trick = Trick {
        index: 0,
        led_suit: led,
        plays: plays.to_vec(),
        winner_seat: None,
    };
    patch(&mut view);
    view.legal = get_legal_plays(&hand, led, Some(trump))
        .into_iter()
        .map(|card| LegalMove {
            id: format!("play:{card}"),
            mv: Move::Play { card },
            label: card_name(card),
        })
        .collect();
    view
}

/// The state-shaping overrides of `bidView`.
#[derive(Clone, Copy, Default)]
pub struct BidOpts {
    /// `overrides.you` (default 1).
    pub you: Option<u8>,
    /// `overrides.dealerSeat` (default 0).
    pub dealer: Option<u8>,
    /// `overrides.phase === 'bid_round_2'`.
    pub round2: bool,
    /// `overrides.passes` (default 0).
    pub passes: Option<u8>,
    /// `overrides.rules`, merged into the config and the view.
    pub rules: Option<TableRules>,
}

/// `bidView(hand, upCard, overrides)`: legal options from the actual rule
/// engine; `patch` applies view-only overrides (score, bids, ...).
pub fn bid_view(
    hand: &[&str],
    up: &str,
    opts: BidOpts,
    patch: impl FnOnce(&mut PublicGameView),
) -> PublicGameView {
    let mut state = try_create_game(GameInit {
        game_id: "bidding-check".to_owned(),
        seed: "bidding-check".to_owned(),
        cfg: None,
        internal_token: None,
        first_dealer: None,
    })
    .unwrap();
    let you = seat(opts.you.unwrap_or(1));
    let up = c(up);
    if let Some(rules) = opts.rules {
        state.cfg.stick_the_dealer = rules.stick_the_dealer;
        state.cfg.require_natural_trump = Some(rules.require_natural_trump);
    }
    state.status = GameStatus::Active;
    state.hand.phase = if opts.round2 {
        GamePhase::BidRound2
    } else {
        GamePhase::BidRound1
    };
    state.hand.dealer_seat = seat(opts.dealer.unwrap_or(0));
    state.hand.turn_seat = Some(you);
    state.hand.up_card = Some(up);
    state.hand.up_card_turned_down = opts.round2;
    state.hand.turned_down_suit = opts.round2.then(|| up.printed_suit());
    state.hand.passes = opts.passes.unwrap_or(0);
    state.hand.hands[you.index()] = cards(hand);
    let mut view = project(&state, you);
    view.hand_counts = [5, 5, 5, 5];
    if opts.rules.is_some() {
        view.rules = opts.rules;
    }
    patch(&mut view);
    view.legal = legal_moves(&state, you);
    view
}

/// The legal ids of a view.
pub fn ids(view: &PublicGameView) -> Vec<String> {
    view.legal.iter().map(|m| m.id.clone()).collect()
}

/// A JSON round trip with extra (private-looking) keys merged in: the typed
/// view drops them, as the TS functions ignore them.
pub fn poisoned(view: &PublicGameView, extra: Value) -> PublicGameView {
    let mut v = serde_json::to_value(view).unwrap();
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    serde_json::from_value(v).unwrap()
}

/// `createGame({ gameId, seed, firstDealer })` with the default rules.
pub fn game(game_id: &str, seed_text: &str, first_dealer: Option<u8>) -> euchre_engine::GameState {
    try_create_game(GameInit {
        game_id: game_id.to_owned(),
        seed: seed_text.to_owned(),
        cfg: None,
        internal_token: None,
        first_dealer: first_dealer.map(seat),
    })
    .unwrap()
}

/// A rollout result looked up by move id (the TS `result[id]`).
pub fn entry<'a, T>(result: &'a [(String, T)], id: &str) -> Option<&'a T> {
    result.iter().find(|(k, _)| k == id).map(|(_, v)| v)
}
