//! Port of `__tests__/optimistic.test.ts`: V22, optimistic agreement.
//!
//! `apply_optimistic(project(s, seat), move)` must equal
//! `project(apply(s, seat, move).state, seat)` on every field in
//! `OPTIMISTIC_FIELDS`, for every reachable state and every legal move.

mod common;

use common::*;
use euchre_engine::{
    GameInit, GamePhase, GameState, GameStatus, Move, OPTIMISTIC_FIELDS, Seat, advance,
    apply_optimistic, create_game, legal_moves, mulberry32, project,
};

/// Which seat, if any, must act now.
fn acting_seat(s: &GameState) -> Option<Seat> {
    SEATS_ALL
        .into_iter()
        .find(|&seat| !legal_moves(s, seat).is_empty())
}

struct Mismatch {
    field: &'static str,
    move_id: String,
    phase: GamePhase,
    seat: Seat,
    expected: String,
    actual: String,
}

/// A view field by its JSON key, serialized (`JSON.stringify(view[field])`).
fn field_json(view: &serde_json::Value, field: &str) -> String {
    view.get(field)
        .map_or_else(|| "undefined".to_owned(), |x| x.to_string())
}

fn to_value(view: &euchre_engine::PublicGameView) -> serde_json::Value {
    serde_json::to_value(view).unwrap()
}

fn new_state(id: &str) -> GameState {
    create_game(GameInit {
        game_id: id.to_owned(),
        seed: id.to_owned(),
        ..GameInit::default()
    })
}

fn sweep(games: u32, seed_base: u32) -> (usize, Vec<Mismatch>) {
    let mut mismatches = Vec::new();
    let mut checks = 0;

    for g in 0..games {
        let mut rng = mulberry32(seed_base.wrapping_add(g.wrapping_mul(7919)));
        let mut s = new_state(&format!("v22-{g}"));

        let mut step = 0;
        while step < 4000 && s.status == GameStatus::Active {
            step += 1;
            let Some(seat) = acting_seat(&s) else {
                let r = advance(&s);
                if r.state == s {
                    break;
                }
                s = r.state;
                continue;
            };

            let moves = legal_moves(&s, seat);
            let view = project(&s, seat);

            // Check every legal move from this state, not merely the one we take.
            for lm in &moves {
                // `cut` triggers the deal from the server-held seeded shuffle.
                if matches!(lm.mv, Move::Cut { .. }) {
                    continue;
                }
                let expected = to_value(&project(&ok(&s, seat, lm.mv).state, seat));
                let actual = to_value(&apply_optimistic(&view, &lm.mv));
                checks += 1;

                for field in OPTIMISTIC_FIELDS {
                    let e = field_json(&expected, field.as_str());
                    let a = field_json(&actual, field.as_str());
                    if e != a && mismatches.len() < 25 {
                        mismatches.push(Mismatch {
                            field: field.as_str(),
                            move_id: lm.id.clone(),
                            phase: s.hand.phase,
                            seat,
                            expected: e,
                            actual: a,
                        });
                    }
                }
            }

            let chosen = moves[pick_index(rng(), moves.len())].mv;
            s = ok(&s, seat, chosen).state;
        }
    }

    (checks, mismatches)
}

#[test]
fn agrees_with_the_authoritative_reducer_on_every_legal_move_across_many_games() {
    let (checks, mismatches) = sweep(60, 1);

    if !mismatches.is_empty() {
        let detail: Vec<String> = mismatches
            .iter()
            .map(|m| {
                format!(
                    "  {} after {} (phase {}, seat {})\n    expected {}\n    actual   {}",
                    m.field, m.move_id, m.phase, m.seat, m.expected, m.actual
                )
            })
            .collect();
        panic!(
            "{} field mismatches over {checks} checks:\n{}",
            mismatches.len(),
            detail.join("\n")
        );
    }
    // Guard against the sweep silently doing nothing.
    assert!(checks > 5000, "{checks}");
}

/// Plays first-legal moves until `trick_play` (or `limit` steps).
fn to_trick_play(id: &str, limit: usize) -> GameState {
    let mut s = new_state(id);
    let mut i = 0;
    while i < limit && s.hand.phase != GamePhase::TrickPlay {
        s = match acting_seat(&s) {
            None => advance(&s).state,
            Some(seat) => ok(&s, seat, legal_moves(&s, seat)[0].mv).state,
        };
        i += 1;
    }
    s
}

#[test]
fn is_identity_for_a_card_the_seat_does_not_hold() {
    let s = to_trick_play("v22-id", 50);
    let Some(seat) = acting_seat(&s) else { return };
    let view = project(&s, seat);
    let Some(not_held) = cards(&["9S", "TS", "JS", "QS"])
        .into_iter()
        .find(|card| !view.hand.contains(card))
    else {
        return;
    };
    // TS asserts reference identity (`toBe(view)`); Rust asserts value identity.
    assert_eq!(
        apply_optimistic(&view, &Move::Play { card: not_held }),
        view
    );
}

#[test]
fn never_mutates_the_view_it_was_given() {
    let s = to_trick_play("v22-imm", 60);
    let Some(seat) = acting_seat(&s) else { return };
    let view = project(&s, seat);
    let snapshot = json(&view);
    for lm in legal_moves(&s, seat) {
        apply_optimistic(&view, &lm.mv);
    }
    assert_eq!(json(&view), snapshot);
}
