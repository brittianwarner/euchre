//! Engine faults: states the engine can never produce make the `try_*`
//! functions return an `EngineFault` carrying the TS `Error` message (checked
//! against the TS engine), and the plain functions panic with it.

mod common;

use common::*;
use euchre_engine::{
    ApplyError, EngineFault, GamePhase, GameState, Move, advance, apply, legal_moves, try_advance,
    try_apply,
};

fn first_legal(s: &GameState) -> GameState {
    let t = turn(s);
    ok(s, t, legal_moves(s, t)[0].mv).state
}

/// Seed `f`, dealer 0: run 'em, seat 1 orders up, the dealer discards.
fn trick_play() -> GameState {
    let mut s = opened("f", Some(seat(0)), None);
    s = ok(&s, seat(3), Move::Cut { cut: false }).state;
    s = ok(&s, seat(1), Move::OrderUp { alone: false }).state;
    ok(
        &s,
        seat(0),
        Move::Discard {
            card: s.hand.hands[0][0],
        },
    )
    .state
}

fn fault(message: &str) -> EngineFault {
    EngineFault {
        message: message.to_owned(),
    }
}

#[test]
fn a_play_before_trump_is_set_is_a_fault() {
    let s = patch_hand(&trick_play(), |h| h.trump = None);
    let mv = legal_moves(&s, seat(1))[0].mv;
    let err = try_apply(&s, seat(1), mv, None).unwrap_err();
    assert_eq!(
        err,
        ApplyError::Fault(fault("a card was played before trump was set"))
    );
    assert_eq!(
        err.to_string(),
        "euchre engine invariant violated: a card was played before trump was set"
    );
}

#[test]
fn an_unresolved_trick_in_trick_resolve_is_a_fault() {
    let mut s = trick_play();
    while s.hand.phase != GamePhase::TrickResolve {
        s = first_legal(&s);
    }
    let s = patch_hand(&s, |h| h.trick.winner_seat = None);
    assert_eq!(
        try_advance(&s),
        Err(fault("trick_resolve with an unresolved trick"))
    );
}

#[test]
fn a_hand_scored_with_no_maker_is_a_fault() {
    let mut s = trick_play();
    while !(s.hand.phase == GamePhase::TrickResolve && s.hand.trick_log.len() == 5) {
        s = match s.hand.turn_seat {
            None => advance(&s).state,
            Some(_) => first_legal(&s),
        };
    }
    let s = patch_hand(&s, |h| h.maker_seat = None);
    assert_eq!(
        try_advance(&s),
        Err(fault("a hand was scored with no maker"))
    );
}

#[test]
fn a_discard_that_leaves_the_dealer_off_five_is_a_fault() {
    let tp = trick_play();
    let s = patch_hand(&tp, |h| {
        h.phase = GamePhase::DealerDiscard;
        h.turn_seat = Some(seat(0));
    });
    let card = tp.hand.hands[0][0];
    assert_eq!(
        try_apply(&s, seat(0), Move::Discard { card }, None),
        Err(ApplyError::Fault(fault(
            "dealer hand is not five cards after the discard"
        )))
    );
}

#[test]
#[should_panic(
    expected = "euchre engine invariant violated: a card was played before trump was set"
)]
fn apply_panics_with_the_fault_message() {
    let s = patch_hand(&trick_play(), |h| h.trump = None);
    let mv = legal_moves(&s, seat(1))[0].mv;
    let _ = apply(&s, seat(1), mv, None);
}

#[test]
fn a_rule_error_is_never_a_fault() {
    let s = trick_play();
    let err = try_apply(&s, seat(3), Move::Pass, None).unwrap_err();
    assert!(matches!(err, ApplyError::Rule(_)));
    assert_eq!(err.to_string(), "not_your_turn");
}
