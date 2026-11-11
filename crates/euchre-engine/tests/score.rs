//! Port of `__tests__/score.test.ts`: the scoring table, the clamp, and game end.

mod common;

use common::seat;
use euchre_engine::{
    HandResult, ScoreHandParams, ScoreOutcome, Seat, Team, apply_delta, is_euchre, is_game_over,
    is_in_the_barn, opponent_team, partner_of, raw_points, score_hand, team_of, throw_in_outcome,
    winner_team,
};

const ALL_SEATS: [Seat; 4] = Seat::ALL;

/// `tricks_won` with `made` going to `makers` and the rest to the defenders.
fn tricks(makers: Team, made: u8) -> [u8; 2] {
    if makers == Team::NS {
        [made, 5 - made]
    } else {
        [5 - made, made]
    }
}

/// The positional TS call `scoreHand(maker, alone, tricksWon, score, gameTo)`.
fn sh(
    maker: u8,
    alone: Option<u8>,
    tricks_won: [u8; 2],
    score: [u8; 2],
    game_to: u8,
) -> ScoreOutcome {
    score_hand(ScoreHandParams {
        maker_seat: seat(maker),
        alone_seat: alone.map(seat),
        tricks_won,
        score,
        game_to,
    })
}

fn out(result: HandResult, delta: [u8; 2]) -> ScoreOutcome {
    ScoreOutcome { result, delta }
}

#[derive(Clone, Copy)]
enum To {
    Makers,
    Defenders,
}

struct Row {
    made: u8,
    alone: bool,
    result: HandResult,
    points: u8,
    to: To,
}

const TABLE: [Row; 12] = [
    Row {
        made: 5,
        alone: false,
        result: HandResult::March,
        points: 2,
        to: To::Makers,
    },
    Row {
        made: 4,
        alone: false,
        result: HandResult::Point,
        points: 1,
        to: To::Makers,
    },
    Row {
        made: 3,
        alone: false,
        result: HandResult::Point,
        points: 1,
        to: To::Makers,
    },
    Row {
        made: 2,
        alone: false,
        result: HandResult::Euchre,
        points: 2,
        to: To::Defenders,
    },
    Row {
        made: 1,
        alone: false,
        result: HandResult::Euchre,
        points: 2,
        to: To::Defenders,
    },
    Row {
        made: 0,
        alone: false,
        result: HandResult::Euchre,
        points: 2,
        to: To::Defenders,
    },
    Row {
        made: 5,
        alone: true,
        result: HandResult::LoneMarch,
        points: 4,
        to: To::Makers,
    },
    Row {
        made: 4,
        alone: true,
        result: HandResult::LonePoint,
        points: 1,
        to: To::Makers,
    },
    Row {
        made: 3,
        alone: true,
        result: HandResult::LonePoint,
        points: 1,
        to: To::Makers,
    },
    Row {
        made: 2,
        alone: true,
        result: HandResult::Euchre,
        points: 2,
        to: To::Defenders,
    },
    Row {
        made: 1,
        alone: true,
        result: HandResult::Euchre,
        points: 2,
        to: To::Defenders,
    },
    Row {
        made: 0,
        alone: true,
        result: HandResult::Euchre,
        points: 2,
        to: To::Defenders,
    },
];

/// The body of each generated TS `it` for one table row.
fn check_row(i: usize) {
    let row = &TABLE[i];
    for maker in ALL_SEATS {
        let makers = team_of(maker);
        let defenders = opponent_team(makers);
        let alone = row.alone.then_some(maker.get());
        let got = sh(maker.get(), alone, tricks(makers, row.made), [0, 0], 10);
        let mut want = [0u8; 2];
        let to = match row.to {
            To::Makers => makers,
            To::Defenders => defenders,
        };
        want[to.index()] = row.points;
        assert_eq!((maker, got.result), (maker, row.result));
        assert_eq!((maker, got.delta), (maker, want));
    }
}

/* scoreHand: the full table (one test per row, as the TS loop generates) */

#[test]
fn makers_take_5_march_2_to_the_makers() {
    check_row(0);
}
#[test]
fn makers_take_4_point_1_to_the_makers() {
    check_row(1);
}
#[test]
fn makers_take_3_point_1_to_the_makers() {
    check_row(2);
}
#[test]
fn makers_take_2_euchre_2_to_the_defenders() {
    check_row(3);
}
#[test]
fn makers_take_1_euchre_2_to_the_defenders() {
    check_row(4);
}
#[test]
fn makers_take_0_euchre_2_to_the_defenders() {
    check_row(5);
}
#[test]
fn lone_maker_take_5_lone_march_4_to_the_makers() {
    check_row(6);
}
#[test]
fn lone_maker_take_4_lone_point_1_to_the_makers() {
    check_row(7);
}
#[test]
fn lone_maker_take_3_lone_point_1_to_the_makers() {
    check_row(8);
}
#[test]
fn lone_maker_take_2_euchre_2_to_the_defenders() {
    check_row(9);
}
#[test]
fn lone_maker_take_1_euchre_2_to_the_defenders() {
    check_row(10);
}
#[test]
fn lone_maker_take_0_euchre_2_to_the_defenders() {
    check_row(11);
}

#[test]
fn a_lone_maker_who_is_euchred_concedes_exactly_2() {
    let lone = sh(0, Some(0), [1, 4], [0, 0], 10);
    let pair = sh(0, None, [1, 4], [0, 0], 10);
    assert_eq!(lone.result, HandResult::Euchre);
    assert_eq!(pair.result, HandResult::Euchre);
    assert_eq!(lone.delta, [0, 2]);
    assert_eq!(pair.delta, [0, 2]);
}

#[test]
fn going_alone_changes_the_label_at_3_4_tricks_but_not_the_points() {
    assert_eq!(
        sh(0, None, [3, 2], [0, 0], 10),
        out(HandResult::Point, [1, 0])
    );
    assert_eq!(
        sh(0, Some(0), [3, 2], [0, 0], 10),
        out(HandResult::LonePoint, [1, 0])
    );
}

#[test]
fn going_alone_doubles_the_march_from_2_to_4() {
    assert_eq!(
        sh(0, None, [5, 0], [0, 0], 10),
        out(HandResult::March, [2, 0])
    );
    assert_eq!(
        sh(0, Some(0), [5, 0], [0, 0], 10),
        out(HandResult::LoneMarch, [4, 0])
    );
}

#[test]
fn there_is_no_super_euchre_row_defenders_taking_all_5_is_still_worth_2() {
    assert_eq!(
        sh(0, None, [0, 5], [0, 0], 10),
        out(HandResult::Euchre, [0, 2])
    );
    assert_eq!(
        sh(0, Some(0), [0, 5], [0, 0], 10),
        out(HandResult::Euchre, [0, 2])
    );
}

#[test]
fn reads_made_off_the_makers_index_not_index_0() {
    assert_eq!(
        sh(1, None, [0, 5], [0, 0], 10),
        out(HandResult::March, [0, 2])
    );
    assert_eq!(
        sh(3, None, [0, 5], [0, 0], 10),
        out(HandResult::March, [0, 2])
    );
    assert_eq!(
        sh(0, None, [0, 5], [0, 0], 10),
        out(HandResult::Euchre, [0, 2])
    );
}

#[test]
fn is_identical_for_a_seat_and_its_partner_teams_score_not_seats() {
    for s in ALL_SEATS {
        for made in 0..=5u8 {
            let t = tricks(team_of(s), made);
            let a = sh(s.get(), None, t, [2, 3], 10);
            let b = sh(partner_of(s).get(), None, t, [2, 3], 10);
            assert_eq!((s, made, a), (s, made, b));
        }
    }
}

#[test]
fn the_alone_seat_value_itself_is_irrelevant_only_null_vs_non_null_matters() {
    let a = sh(0, Some(0), [5, 0], [0, 0], 10);
    let b = sh(0, Some(2), [5, 0], [0, 0], 10);
    assert_eq!(a, b);
    assert_eq!(a.delta, [4, 0]);
}

/* the clamp (V11) */

#[test]
fn case_34_a_march_at_9_wins_at_10_not_11() {
    let o = sh(0, None, [5, 0], [9, 4], 10);
    assert_eq!(o.result, HandResult::March);
    assert_eq!(o.delta, [1, 0]);
    assert_eq!(apply_delta([9, 4], o.delta), [10, 4]);
}

#[test]
fn case_35_a_lone_march_from_7_gives_3_landing_on_10_exactly() {
    let o = sh(0, Some(0), [5, 0], [7, 2], 10);
    assert_eq!(o.result, HandResult::LoneMarch);
    assert_eq!(o.delta, [3, 0]);
    assert_eq!(apply_delta([7, 2], o.delta), [10, 2]);
}

#[test]
fn case_36_a_euchre_credits_the_defenders_even_when_the_makers_are_the_humans_team() {
    let o = sh(0, None, [1, 4], [5, 5], 10);
    assert_eq!(o.result, HandResult::Euchre);
    assert_eq!(o.delta, [0, 2]);
    assert_eq!(apply_delta([5, 5], o.delta), [5, 7]);
}

#[test]
fn a_euchre_at_9_also_clamps_the_defenders_win_at_10_not_11() {
    let o = sh(0, None, [1, 4], [3, 9], 10);
    assert_eq!(o.delta, [0, 1]);
    assert_eq!(apply_delta([3, 9], o.delta), [3, 10]);
}

#[test]
fn a_team_already_at_game_to_gains_nothing() {
    assert_eq!(sh(0, Some(0), [5, 0], [10, 3], 10).delta, [0, 0]);
}

#[test]
fn clamps_against_non_default_game_to_values() {
    assert_eq!(sh(0, Some(0), [5, 0], [3, 0], 5).delta, [2, 0]);
    assert_eq!(sh(0, None, [5, 0], [6, 0], 7).delta, [1, 0]);
    assert_eq!(sh(0, Some(0), [5, 0], [8, 0], 11).delta, [3, 0]);
}

#[test]
fn v11_exhaustively_delta_is_non_negative_single_component_and_equals_the_clamped_raw() {
    let mut checked = 0u32;
    for game_to in [5u8, 7, 10, 11] {
        for maker in ALL_SEATS {
            for alone in [false, true] {
                for made in 0..=5u8 {
                    for s0 in 0..=game_to {
                        for s1 in 0..=game_to {
                            let makers = team_of(maker);
                            let score = [s0, s1];
                            let o = sh(
                                maker.get(),
                                alone.then_some(maker.get()),
                                tricks(makers, made),
                                score,
                                game_to,
                            );
                            checked += 1;

                            let mut raw = [0u8; 2];
                            let beneficiary = if made <= 2 {
                                opponent_team(makers)
                            } else {
                                makers
                            };
                            raw[beneficiary.index()] = raw_points(o.result);

                            // Non-negative is structural (u8); at most one non-zero component.
                            assert!(o.delta[0] == 0 || o.delta[1] == 0);
                            for i in 0..2 {
                                assert_eq!(
                                    (
                                        game_to,
                                        maker,
                                        alone,
                                        made,
                                        score,
                                        i,
                                        u16::from(score[i]) + u16::from(o.delta[i])
                                    ),
                                    (
                                        game_to,
                                        maker,
                                        alone,
                                        made,
                                        score,
                                        i,
                                        u16::from(game_to)
                                            .min(u16::from(score[i]) + u16::from(raw[i]))
                                    )
                                );
                            }
                            let next = apply_delta(score, o.delta);
                            assert!(next[0] >= score[0] && next[1] >= score[1]);
                            assert!(next[0] <= game_to && next[1] <= game_to);
                        }
                    }
                }
            }
        }
    }
    let want: u32 = [5u32, 7, 10, 11]
        .iter()
        .map(|g| 4 * 2 * 6 * (g + 1) * (g + 1))
        .sum();
    assert_eq!(checked, want);
}

#[test]
fn never_mutates_its_score_or_tricks_won_arguments() {
    // TS freezes the inputs; Rust passes them by value, so mutation is impossible.
    let score = [9u8, 4];
    let won = [5u8, 0];
    let _ = sh(0, None, won, score, 10);
    assert_eq!(score, [9, 4]);
    assert_eq!(won, [5, 0]);
}

/* throwInOutcome */

#[test]
fn case_37_nobody_scores() {
    assert_eq!(throw_in_outcome(), out(HandResult::ThrowIn, [0, 0]));
}

#[test]
fn leaves_the_score_untouched_when_applied() {
    assert_eq!(apply_delta([4, 7], throw_in_outcome().delta), [4, 7]);
}

#[test]
fn returns_a_fresh_outcome_each_call() {
    // TS checks reference inequality; a Rust value is a fresh copy by construction.
    let mut a = throw_in_outcome();
    a.delta[0] = 9;
    assert_eq!(a.delta, [9, 0]);
    assert_eq!(throw_in_outcome().delta, [0, 0]);
}

/* applyDelta */

#[test]
fn apply_delta_adds_component_wise() {
    assert_eq!(apply_delta([3, 4], [2, 0]), [5, 4]);
    assert_eq!(apply_delta([0, 0], [0, 2]), [0, 2]);
}

#[test]
fn apply_delta_returns_a_fresh_tuple_and_never_mutates_the_input() {
    let score = [3u8, 4];
    let next = apply_delta(score, [2, 0]);
    assert_eq!(next, [5, 4]);
    assert_eq!(score, [3, 4]);
}

#[test]
fn composes_with_score_hand_to_reproduce_the_post_clamp_score_exactly() {
    let mut score = [0u8, 0];
    score = apply_delta(score, sh(0, None, [5, 0], score, 10).delta);
    score = apply_delta(score, sh(0, None, [5, 0], score, 10).delta);
    score = apply_delta(score, sh(1, None, [4, 1], score, 10).delta);
    score = apply_delta(score, sh(1, None, [4, 1], score, 10).delta);
    score = apply_delta(score, sh(0, None, [3, 2], score, 10).delta);
    assert_eq!(score, [9, 0]);
    let last = sh(0, Some(0), [5, 0], score, 10);
    assert_eq!(last.delta, [1, 0]);
    assert_eq!(apply_delta(score, last.delta), [10, 0]);
}

/* winnerTeam / isGameOver */

#[test]
fn is_null_while_both_teams_are_short_of_game_to() {
    assert_eq!(winner_team([0, 0], 10), None);
    assert_eq!(winner_team([9, 9], 10), None);
    assert!(!is_game_over([9, 9], 10));
}

#[test]
fn names_team_0_the_instant_it_reaches_game_to() {
    assert_eq!(winner_team([10, 4], 10), Some(Team::NS));
    assert!(is_game_over([10, 4], 10));
}

#[test]
fn names_team_1_the_instant_it_reaches_game_to() {
    assert_eq!(winner_team([4, 10], 10), Some(Team::EW));
    assert!(is_game_over([4, 10], 10));
}

#[test]
fn respects_a_non_default_game_to() {
    assert_eq!(winner_team([5, 0], 10), None);
    assert_eq!(winner_team([5, 0], 5), Some(Team::NS));
    assert_eq!(winner_team([6, 7], 7), Some(Team::EW));
}

#[test]
fn agrees_with_is_game_over_at_every_score_up_to_game_to() {
    for a in 0..=10u8 {
        for b in 0..=10u8 {
            let w = winner_team([a, b], 10);
            assert_eq!((a, b, is_game_over([a, b], 10)), (a, b, w.is_some()));
        }
    }
}

/* isInTheBarn */

#[test]
fn is_true_at_exactly_game_to_minus_1() {
    assert!(is_in_the_barn([9, 3], Team::NS, 10));
    assert!(is_in_the_barn([3, 9], Team::EW, 10));
}

#[test]
fn is_false_at_every_other_score() {
    for s in 0..=10u8 {
        assert_eq!((s, is_in_the_barn([s, 0], Team::NS, 10)), (s, s == 9));
    }
}

#[test]
fn is_per_team_not_per_score_array() {
    assert!(!is_in_the_barn([9, 3], Team::EW, 10));
    assert!(!is_in_the_barn([3, 9], Team::NS, 10));
}

#[test]
fn tracks_a_non_default_game_to() {
    assert!(is_in_the_barn([4, 0], Team::NS, 5));
    assert!(!is_in_the_barn([9, 0], Team::NS, 5));
}

/* isEuchre / rawPoints */

#[test]
fn is_euchre_is_true_only_for_euchre() {
    for &r in HandResult::ALL {
        assert_eq!((r, is_euchre(r)), (r, r == HandResult::Euchre));
    }
    assert_eq!(HandResult::ALL.len(), 6);
}

#[test]
fn raw_points_matches_the_table_for_all_six_results() {
    assert_eq!(raw_points(HandResult::Point), 1);
    assert_eq!(raw_points(HandResult::LonePoint), 1);
    assert_eq!(raw_points(HandResult::March), 2);
    assert_eq!(raw_points(HandResult::Euchre), 2);
    assert_eq!(raw_points(HandResult::LoneMarch), 4);
    assert_eq!(raw_points(HandResult::ThrowIn), 0);
}

#[test]
fn raw_points_equals_the_unclamped_delta_score_hand_produces_from_a_fresh_score() {
    for row in &TABLE {
        let makers = team_of(seat(0));
        let o = sh(
            0,
            row.alone.then_some(0),
            tricks(makers, row.made),
            [0, 0],
            10,
        );
        let total = o.delta[0] + o.delta[1];
        assert_eq!(
            (row.made, row.alone, total),
            (row.made, row.alone, raw_points(o.result))
        );
    }
}
