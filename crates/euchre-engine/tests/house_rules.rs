//! Port of `__tests__/house-rules.test.ts`: the natural-trump house rule.

mod common;

use common::*;
use euchre_engine::{
    EngineConfigOverrides, GameInit, GamePhase, GameState, GameStatus, Move, Suit, advance,
    create_game, legal_move_ids, legal_moves,
};

fn bidding() -> GameState {
    let mut state = advance(&create_game(GameInit {
        game_id: "rules".to_owned(),
        seed: "rules".to_owned(),
        first_dealer: Some(seat(0)),
        cfg: Some(EngineConfigOverrides {
            require_natural_trump: Some(true),
            ..EngineConfigOverrides::default()
        }),
        internal_token: None,
    }))
    .state;
    state = ok(&state, turn(&state), Move::Cut { cut: false }).state;
    state = advance(&state).state;
    // TS rebuilds `hands` as a JS array; the seat-indexed lookups are identical.
    patch_hand(&state, |h| {
        h.up_card = Some(c("9H"));
        h.hands[1] = cards(&["JD", "9C", "TC", "QC", "AC"]);
    })
}

#[test]
fn does_not_let_the_left_bower_alone_order_up_trump() {
    let s = bidding();
    assert_eq!(legal_move_ids(&s, seat(1)), ["pass"]);
}

#[test]
fn does_not_let_the_left_bower_alone_name_that_suit_in_round_two() {
    let s = bidding();
    let next = patch_hand(&s, |h| {
        h.phase = GamePhase::BidRound2;
        h.turned_down_suit = Some(Suit::Spades);
    });
    assert!(!legal_move_ids(&next, seat(1)).contains(&"call:H".to_owned()));
    assert!(legal_move_ids(&next, seat(1)).contains(&"call:C".to_owned()));
}

#[test]
fn allows_the_dealer_to_count_the_natural_trump_up_card() {
    let s = bidding();
    let next = patch_hand(&s, |h| h.turn_seat = Some(seat(0)));
    assert!(legal_move_ids(&next, seat(0)).contains(&"orderUp".to_owned()));
}

#[test]
fn lets_a_stuck_dealer_choose_rather_than_creating_a_dead_end() {
    let s = bidding();
    let next = patch_hand(&s, |h| {
        h.phase = GamePhase::BidRound2;
        h.turn_seat = Some(seat(0));
        h.passes = 3;
        h.turned_down_suit = Some(Suit::Spades);
    });
    assert_eq!(
        legal_move_ids(&next, seat(0)),
        [
            "call:H",
            "call:H+alone",
            "call:D",
            "call:D+alone",
            "call:C",
            "call:C+alone"
        ]
    );
}

fn finishes_matches_with_natural_trump(stick_the_dealer: bool) {
    for seed in 0..20usize {
        let mut s = create_game(GameInit {
            game_id: format!("house-{seed}"),
            seed: format!("house-{seed}"),
            cfg: Some(EngineConfigOverrides {
                stick_the_dealer: Some(stick_the_dealer),
                require_natural_trump: Some(true),
                ..EngineConfigOverrides::default()
            }),
            ..GameInit::default()
        });
        let mut step = 0usize;
        while step < 3000 && s.status == GameStatus::Active {
            match s.hand.turn_seat {
                None => s = advance(&s).state,
                Some(seat) => {
                    let moves = legal_moves(&s, seat);
                    assert!(!moves.is_empty());
                    let chosen = moves
                        .iter()
                        .find(|m| matches!(m.mv, Move::Call { .. } | Move::OrderUp { .. }))
                        .unwrap_or(&moves[(seed + step) % moves.len()]);
                    s = ok(&s, seat, chosen.mv).state;
                }
            }
            step += 1;
        }
        assert_eq!(s.status, GameStatus::Complete);
    }
}

#[test]
fn finishes_matches_with_natural_trump_and_stick_true() {
    finishes_matches_with_natural_trump(true);
}

#[test]
fn finishes_matches_with_natural_trump_and_stick_false() {
    finishes_matches_with_natural_trump(false);
}
