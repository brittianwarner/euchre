//! Spec 01 section 15 reducer goldens: `create_game`, the first `advance` and
//! `apply`, and four full-match replays, all checked against values produced
//! by the TS engine.

mod common;

use common::*;
use euchre_engine::{
    EngineConfigOverrides, GameInit, Move, Step, advance, apply, assert_invariants,
    assert_legal_non_empty, create_game, fnv1a, legal_moves,
};

fn abc() -> euchre_engine::GameState {
    create_game(GameInit {
        game_id: "g-abc".to_owned(),
        seed: "abc".to_owned(),
        ..GameInit::default()
    })
}

#[test]
fn create_game_golden_token_turn_id_and_first_dealer() {
    let g = abc();
    assert_eq!(g.internal_token, "26xgdh1wxuoi1my8zj1cynak");
    assert_eq!(g.turn_id, "13swn7p");
    assert_eq!(g.first_dealer, seat(1));
    assert_eq!(g.hand.dealer_seat, seat(1));
}

#[test]
fn advance_golden_opens_cutting() {
    let s = advance(&abc()).state;
    assert_eq!(s.hand.phase.as_str(), "cutting");
    assert_eq!(s.hand.dealer_seat, seat(1));
    assert_eq!(s.hand.turn_seat, Some(seat(0)));
    assert_eq!(s.turn_id, "dz47z7e93to6");
}

#[test]
fn apply_golden_run_em() {
    let s = advance(&abc()).state;
    let res = ok(&s, seat(0), Move::Cut { cut: false });
    let t = &res.state;
    assert_eq!(t.turn_id, "pkpgihoqqnfk");
    assert_eq!(t.hand.up_card, Some(c("JH")));
    assert_eq!(t.hand.kitty.to_vec(), cards(&["JH", "QC", "KH", "AC"]));
    assert_eq!(t.hand.hands[0], cards(&["AH", "9D", "QS", "9S", "JC"]));
    assert_eq!(t.hand.hands[1], cards(&["TD", "TC", "QH", "9C", "KS"]));
    assert_eq!(t.hand.hands[2], cards(&["KC", "KD", "JD", "AS", "TH"]));
    assert_eq!(t.hand.hands[3], cards(&["AD", "9H", "QD", "TS", "JS"]));
    assert_eq!(
        json(&res.steps),
        concat!(
            r#"[{"t":"cut","seat":0,"cut":false},{"t":"dealt","packets":["#,
            r#"{"seat":2,"count":2,"cards":["KC","KD"]},{"seat":3,"count":3,"cards":["AD","9H","QD"]},"#,
            r#"{"seat":0,"count":2,"cards":["AH","9D"]},{"seat":1,"count":3,"cards":["TD","TC","QH"]},"#,
            r#"{"seat":2,"count":3,"cards":["JD","AS","TH"]},{"seat":3,"count":2,"cards":["TS","JS"]},"#,
            r#"{"seat":0,"count":3,"cards":["QS","9S","JC"]},{"seat":1,"count":2,"cards":["9C","KS"]}]},"#,
            r#"{"t":"upCardTurned","card":"JH"}]"#
        )
    );
}

#[test]
fn apply_golden_bump() {
    let s = advance(&abc()).state;
    let t = ok(&s, seat(0), Move::Cut { cut: true }).state;
    assert_eq!(t.hand.hands[0], cards(&["9S", "JC", "AD", "9H", "QD"]));
    assert_eq!(t.hand.hands[1], cards(&["9C", "KS", "JH", "AH", "9D"]));
    assert_eq!(t.hand.hands[2], cards(&["AS", "TH", "QC", "KH", "AC"]));
    assert_eq!(t.hand.hands[3], cards(&["TS", "JS", "QS", "KC", "KD"]));
    assert_eq!(t.hand.up_card, Some(c("TD")));
}

struct Replay {
    seed: &'static str,
    cfg: Option<EngineConfigOverrides>,
    first_dealer: Option<u8>,
    seat_moves: usize,
    transitions: usize,
    move_hash: u32,
    first_six: &'static str,
    score: [u8; 2],
    hand_no: u32,
    winner: u8,
    final_dealer: u8,
    /// The spec's `firstDealer` column: `state.first_dealer`, drawn or fixed.
    want_first_dealer: u8,
    final_turn_id: &'static str,
    hand_scored: &'static str,
}

fn check_replay(r: &Replay) {
    let g = create_game(GameInit {
        game_id: format!("g-{}", r.seed),
        seed: r.seed.to_owned(),
        cfg: r.cfg,
        internal_token: None,
        first_dealer: r.first_dealer.map(seat),
    });
    let mut pick = random_picker(r.seed);
    let mut s = g;
    // Spec section 15: "invariants asserted every step", the initial state included.
    assert_invariants(&s).unwrap_or_else(|e| panic!("{} initial: {e}", r.seed));
    let mut moves: Vec<String> = Vec::new();
    let mut transitions = 0;
    let mut scored: Vec<String> = Vec::new();
    while s.status == euchre_engine::GameStatus::Active {
        let res = match s.hand.turn_seat {
            None => advance(&s),
            Some(seat) => {
                let legal = legal_moves(&s, seat);
                let m = pick(&s, seat, &legal);
                moves.push(format!("{}{}", seat, m.id));
                apply(&s, seat, m.mv, None).unwrap_or_else(|e| panic!("{e}"))
            }
        };
        transitions += 1;
        for st in &res.steps {
            if let Step::HandScored {
                result,
                delta,
                score,
                ..
            } = st
            {
                scored.push(format!(
                    "{}:{}/{}:{}-{}",
                    result, delta[0], delta[1], score[0], score[1]
                ));
            }
        }
        s = res.state;
        assert_invariants(&s).unwrap_or_else(|e| panic!("{} #{transitions}: {e}", r.seed));
        assert_legal_non_empty(&s, legal_moves)
            .unwrap_or_else(|e| panic!("{} #{transitions}: {e}", r.seed));
        assert_eq!(
            s.first_dealer,
            seat(r.want_first_dealer),
            "{} firstDealer",
            r.seed
        );
    }
    assert_eq!(
        s.first_dealer,
        seat(r.want_first_dealer),
        "{} firstDealer",
        r.seed
    );
    assert_eq!(moves.len(), r.seat_moves, "{} seat moves", r.seed);
    assert_eq!(transitions, r.transitions, "{} transitions", r.seed);
    assert_eq!(moves[..6].join(","), r.first_six, "{} first moves", r.seed);
    assert_eq!(fnv1a(&moves.join(",")), r.move_hash, "{} moveHash", r.seed);
    assert_eq!(s.score, r.score, "{} score", r.seed);
    assert_eq!(s.hand_no, r.hand_no, "{} handNo", r.seed);
    assert_eq!(
        s.winner_team.map(|t| t.get()),
        Some(r.winner),
        "{} winner",
        r.seed
    );
    assert_eq!(
        s.hand.dealer_seat,
        seat(r.final_dealer),
        "{} dealer",
        r.seed
    );
    assert_eq!(s.turn_id, r.final_turn_id, "{} turnId", r.seed);
    assert_eq!(scored.join(", "), r.hand_scored, "{} handScored", r.seed);
}

#[test]
fn replay_abc_default_drawn_dealer() {
    check_replay(&Replay {
        seed: "abc",
        cfg: None,
        first_dealer: None,
        seat_moves: 213,
        transitions: 280,
        move_hash: 3877649361,
        first_six: "0cut:no,2orderUp,1discard:TD,2play:TH,3play:9H,0play:AH",
        score: [7, 10],
        hand_no: 10,
        winner: 1,
        final_dealer: 3,
        want_first_dealer: 1,
        final_turn_id: "1mxxun81nrwnq5",
        hand_scored: "point:1/0:1-0, euchre:2/0:3-0, euchre:0/2:3-2, lone_point:0/1:3-3, euchre:0/2:3-5, euchre:0/2:3-7, point:1/0:4-7, euchre:2/0:6-7, point:1/0:7-7, lone_point:0/1:7-8, euchre:0/2:7-10",
    });
}

#[test]
fn replay_full_10_default_drawn_dealer() {
    check_replay(&Replay {
        seed: "full-10",
        cfg: None,
        first_dealer: None,
        seat_moves: 244,
        transitions: 317,
        move_hash: 4165239875,
        first_six: "0cut:yes,2orderUp,1discard:TS,2play:JC,3play:9S,0play:KS",
        score: [9, 10],
        hand_no: 11,
        winner: 1,
        final_dealer: 0,
        want_first_dealer: 1,
        final_turn_id: "cejxtyc4kc4z",
        hand_scored: "euchre:0/2:0-2, lone_point:1/0:1-2, euchre:2/0:3-2, euchre:2/0:5-2, euchre:0/2:5-4, point:0/1:5-5, euchre:0/2:5-7, point:0/1:5-8, point:1/0:6-8, euchre:2/0:8-8, lone_point:1/0:9-8, euchre:0/2:9-10",
    });
}

#[test]
fn replay_x1_to_5_no_stick_dealer_2() {
    check_replay(&Replay {
        seed: "x1",
        cfg: Some(EngineConfigOverrides {
            game_to: Some(5),
            stick_the_dealer: Some(false),
            ..EngineConfigOverrides::default()
        }),
        first_dealer: Some(2),
        seat_moves: 86,
        transitions: 111,
        move_hash: 2702995405,
        first_six: "1cut:yes,3pass,0pass,1pass,2orderUp+alone,2discard:9C",
        score: [2, 5],
        hand_no: 3,
        winner: 1,
        final_dealer: 1,
        want_first_dealer: 2,
        final_turn_id: "1iqy1gi1igyfrj",
        hand_scored: "euchre:0/2:0-2, point:0/1:0-3, euchre:2/0:2-3, euchre:0/2:2-5",
    });
}

#[test]
fn replay_nat_natural_trump_dealer_0() {
    check_replay(&Replay {
        seed: "nat",
        cfg: Some(EngineConfigOverrides {
            require_natural_trump: Some(true),
            ..EngineConfigOverrides::default()
        }),
        first_dealer: Some(0),
        seat_moves: 133,
        transitions: 176,
        move_hash: 2991084946,
        first_six: "3cut:no,1pass,2orderUp+alone,1play:KH,2play:9H,3play:JD",
        score: [10, 3],
        hand_no: 6,
        winner: 0,
        final_dealer: 2,
        want_first_dealer: 0,
        final_turn_id: "1k11rk91j72yhc",
        hand_scored: "euchre:0/2:0-2, euchre:2/0:2-2, euchre:2/0:4-2, lone_point:0/1:4-3, euchre:2/0:6-3, euchre:2/0:8-3, euchre:2/0:10-3",
    });
}
