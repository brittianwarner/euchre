//! Port of `__tests__/game.test.ts`: whole-hand and whole-match behaviour —
//! going alone end to end, seeded matches against the score table, and the
//! headless fuzz harness.

mod common;

use std::collections::{BTreeSet, HashSet};

use common::*;
use euchre_engine::{
    Card, DECK, EngineConfigOverrides, GameInit, GamePhase, GameState, GameStatus, HandResult,
    Move, RuleCode, ScoreHandParams, Seat, Step, Suit, Team, advance, apply, assert_invariants,
    assert_legal_non_empty, create_game, cutter_of, eldest_of, first_active_from, legal_moves,
    new_game, next_seat, partner_of, project, raw_points, score_hand, team_of,
};

/* -------------------------------------------------------------------------- */
/* Harness                                                                     */
/* -------------------------------------------------------------------------- */

/// Cut, then bid: `skip` passes and then an order-up, optionally alone.
fn order_up_after(seed: &str, dealer: Seat, skip: usize, alone: bool) -> GameState {
    let mut s = opened(seed, Some(dealer), None);
    s = ok(&s, cutter_of(dealer), Move::Cut { cut: false }).state;
    for _ in 0..skip {
        s = ok(&s, turn(&s), Move::Pass).state;
    }
    ok(&s, turn(&s), Move::OrderUp { alone }).state
}

fn is_engine_only(p: GamePhase) -> bool {
    matches!(
        p,
        GamePhase::Lobby
            | GamePhase::Deal
            | GamePhase::TrickResolve
            | GamePhase::HandScore
            | GamePhase::GameOver
    )
}

fn is_dealt(p: GamePhase) -> bool {
    matches!(
        p,
        GamePhase::BidRound1
            | GamePhase::DealerDiscard
            | GamePhase::BidRound2
            | GamePhase::TrickPlay
            | GamePhase::TrickResolve
            | GamePhase::HandScore
    )
}

fn js_opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "null".to_owned(), |v| v.to_string())
}

/// Every structural invariant that must hold of a single reachable state.
fn state_violations(s: &GameState) -> Vec<String> {
    let mut bad = Vec::new();
    let h = &s.hand;
    let at = format!("h{}/{}", s.hand_no, h.phase);

    // V1 - turn/phase agreement.
    if is_engine_only(h.phase) && h.turn_seat.is_some() {
        bad.push(format!("{at}: engine phase has a turnSeat"));
    }
    if !is_engine_only(h.phase) && h.turn_seat.is_none() {
        bad.push(format!("{at}: seat phase has no turnSeat"));
    }

    // V4 - going alone is coherent and simultaneous with trump.
    if let Some(alone) = h.alone_seat {
        if Some(alone) != h.maker_seat {
            bad.push(format!("{at}: aloneSeat !== makerSeat"));
        }
        if h.sitting_seat != Some(partner_of(alone)) {
            bad.push(format!("{at}: sittingSeat is not the partner"));
        }
        if h.trump.is_none() {
            bad.push(format!("{at}: alone without trump"));
        }
    }
    if h.sitting_seat.is_some() && h.alone_seat.is_none() {
        bad.push(format!("{at}: sitting without a loner"));
    }

    // V8 - the sitting partner never acts and never appears in a trick.
    if let Some(sit) = h.sitting_seat {
        if h.turn_seat == Some(sit) {
            bad.push(format!("{at}: the sitting seat is on turn"));
        }
        if !legal_moves(s, sit).is_empty() {
            bad.push(format!("{at}: the sitting seat has legal moves"));
        }
        if h.trick.plays.iter().any(|p| p.seat == sit) {
            bad.push(format!("{at}: sitting seat played"));
        }
        for t in &h.trick_log {
            if t.plays.iter().any(|p| p.seat == sit) {
                bad.push(format!("{at}: sitting seat played (log)"));
            }
            if t.winner_seat == Some(sit) {
                bad.push(format!("{at}: sitting seat took a trick (V9)"));
            }
        }
    }

    // V19 - the acting seat always has something to do.
    if let Some(t) = h.turn_seat
        && s.status == GameStatus::Active
        && legal_moves(s, t).is_empty()
    {
        bad.push(format!("{at}: empty legal set for seat {t}"));
    }

    // V18 - bounded collections.
    if h.trick_log.len() > 5 {
        bad.push(format!("{at}: trickLog {}", h.trick_log.len()));
    }
    if h.bids.len() > 8 {
        bad.push(format!("{at}: bids {}", h.bids.len()));
    }
    if h.passes > 4 {
        bad.push(format!("{at}: passes {}", h.passes));
    }
    if h.trick.plays.len() > 4 {
        bad.push(format!("{at}: trick.plays {}", h.trick.plays.len()));
    }
    if s.misdeal_streak > s.cfg.misdeal_limit {
        bad.push(format!("{at}: misdealStreak {}", s.misdeal_streak));
    }

    // V5 - a discard exists exactly when a round-1 call succeeded.
    let round_one_call = h.trump.is_some() && !h.up_card_turned_down;
    if h.dealer_discard.is_some() && !round_one_call {
        bad.push(format!("{at}: discard without a round-1 call"));
    }
    if h.up_card_turned_down && h.dealer_discard.is_some() {
        bad.push(format!("{at}: discard after a turn-down"));
    }

    // V11 - the score is bounded and the winner flag agrees with it.
    for i in 0..2 {
        if s.score[i] > s.cfg.game_to {
            bad.push(format!("{at}: score {}-{}", s.score[0], s.score[1]));
        }
    }
    let leader = if s.score[0] >= s.cfg.game_to {
        Some(Team::NS)
    } else if s.score[1] >= s.cfg.game_to {
        Some(Team::EW)
    } else {
        None
    };
    if s.winner_team != leader {
        bad.push(format!(
            "{at}: winnerTeam {} vs score",
            js_opt(s.winner_team)
        ));
    }

    // V10 - card accounting. Twenty cards are in play, twenty-one once the
    // up-card has been taken into the dealer's hand by a round-1 call.
    if is_dealt(h.phase) {
        let in_flight: Vec<Card> = if h.trick.winner_seat.is_none() {
            h.trick.plays.iter().map(|p| p.card).collect()
        } else {
            Vec::new()
        };
        let mut played: Vec<Card> = h
            .trick_log
            .iter()
            .flat_map(|t| t.plays.iter().map(|p| p.card))
            .collect();
        played.extend(in_flight);
        let mut owned: Vec<Card> = h.hands.iter().flatten().copied().collect();
        owned.extend(played.iter().copied());
        owned.extend(h.dealer_discard);
        let expected = 20 + usize::from(round_one_call);
        if owned.len() != expected {
            bad.push(format!(
                "{at}: {} cards in play, want {expected}",
                owned.len()
            ));
        }
        let owned_set: HashSet<Card> = owned.iter().copied().collect();
        if owned_set.len() != owned.len() {
            bad.push(format!("{at}: a card is in two places"));
        }
        for c in &h.kitty[1..] {
            if owned_set.contains(c) {
                bad.push(format!("{at}: buried kitty card {c} is in play"));
            }
        }
        let mut deck = owned_set.clone();
        deck.extend(h.kitty.iter().copied());
        if deck.len() != 24 {
            bad.push(format!("{at}: the deck does not close"));
        }
        if h.up_card != Some(h.kitty[0]) {
            bad.push(format!("{at}: upCard is not kitty[0]"));
        }
    }

    // V10 - five tricks, and every trick has the right width.
    let width = if h.sitting_seat.is_none() { 4 } else { 3 };
    for t in &h.trick_log {
        if t.plays.len() != width {
            bad.push(format!(
                "{at}: trick {} has {} plays",
                t.index,
                t.plays.len()
            ));
        }
        if t.winner_seat.is_none() {
            bad.push(format!("{at}: logged trick {} is unresolved", t.index));
        }
    }
    if h.phase == GamePhase::HandScore && h.trick_log.len() != 5 {
        bad.push(format!("{at}: scored with {} tricks", h.trick_log.len()));
    }
    if usize::from(h.tricks_won[0] + h.tricks_won[1]) != h.trick_log.len() {
        bad.push(format!("{at}: tricksWon mismatch"));
    }

    bad
}

/// The section 5.1 row a finished hand must have scored.
struct Row {
    result: HandResult,
    raw: u8,
    to_makers: bool,
}

fn expected_row(alone: bool, made: u8) -> Row {
    if made <= 2 {
        return Row {
            result: HandResult::Euchre,
            raw: 2,
            to_makers: false,
        };
    }
    if made <= 4 {
        return Row {
            result: if alone {
                HandResult::LonePoint
            } else {
                HandResult::Point
            },
            raw: 1,
            to_makers: true,
        };
    }
    Row {
        result: if alone {
            HandResult::LoneMarch
        } else {
            HandResult::March
        },
        raw: if alone { 4 } else { 2 },
        to_makers: true,
    }
}

/// What one played-out match produced, for the fuzz assertions.
struct MatchReport {
    fin: GameState,
    hands: usize,
    throw_ins: usize,
    loners: usize,
    tricks: usize,
    clamps: usize,
    cards_played: Vec<usize>,
    results: Vec<HandResult>,
    violations: Vec<String>,
    score_deltas: Vec<[i32; 2]>,
}

fn play_match<P>(initial: GameState, pick: P, deep: bool) -> Result<MatchReport, String>
where
    P: FnMut(&GameState, Seat, &[euchre_engine::LegalMove]) -> euchre_engine::LegalMove,
{
    use std::cell::RefCell;
    let report = RefCell::new(MatchReport {
        fin: initial.clone(),
        hands: 0,
        throw_ins: 0,
        loners: 0,
        tricks: 0,
        clamps: 0,
        cards_played: Vec::new(),
        results: Vec::new(),
        violations: Vec::new(),
        score_deltas: Vec::new(),
    });
    let note = |r: &mut MatchReport, m: String| {
        if r.violations.len() < 25 {
            r.violations.push(m);
        }
    };

    let fin = try_run_game(
        initial,
        pick,
        RunHooks {
            on_state: Some(Box::new(|s| {
                if deep {
                    let mut r = report.borrow_mut();
                    for v in state_violations(s) {
                        note(&mut r, v);
                    }
                    // Beyond TS: the engine's own validators on every state.
                    if let Err(e) = assert_invariants(s) {
                        note(&mut r, e.to_string());
                    }
                    if let Err(e) = assert_legal_non_empty(s, legal_moves) {
                        note(&mut r, e.to_string());
                    }
                }
            })),
            on_transition: Some(Box::new(|before, res, _| {
                let mut r = report.borrow_mut();
                // Scores move only when a hand scores, and only by a table amount.
                let d = [
                    i32::from(res.state.score[0]) - i32::from(before.score[0]),
                    i32::from(res.state.score[1]) - i32::from(before.score[1]),
                ];
                if d != [0, 0] {
                    r.score_deltas.push(d);
                }
                if d[0] < 0 || d[1] < 0 {
                    note(&mut r, format!("score decreased by {},{}", d[0], d[1]));
                }

                for step in &res.steps {
                    if matches!(step, Step::TrickWon { .. }) {
                        r.tricks += 1;
                    }
                    if matches!(step, Step::ThrowIn) {
                        r.throw_ins += 1;
                        if d != [0, 0] {
                            note(&mut r, "a throw-in moved the score".to_owned());
                        }
                    }
                    let Step::HandScored { result, delta, .. } = step else {
                        continue;
                    };

                    r.hands += 1;
                    r.results.push(*result);
                    let h = &before.hand;
                    // Collected like every other violation (TS pushes to the
                    // report and asserts it afterwards) instead of panicking.
                    let Some(maker) = h.maker_seat else {
                        note(&mut r, "a hand scored with no maker".to_owned());
                        continue;
                    };
                    let alone = h.alone_seat.is_some();
                    if alone {
                        r.loners += 1;
                    }

                    let cards: usize = h.trick_log.iter().map(|t| t.plays.len()).sum();
                    r.cards_played.push(cards);
                    if cards != if alone { 15 } else { 20 } {
                        note(&mut r, format!("hand played {cards} cards, alone={alone}"));
                    }

                    let makers = team_of(maker);
                    let row = expected_row(alone, h.tricks_won[makers.index()]);
                    if *result != row.result {
                        note(&mut r, format!("result {result}, want {}", row.result));
                    }

                    let credited = if row.to_makers {
                        makers.index()
                    } else {
                        1 - makers.index()
                    };
                    let mut want = [0i32; 2];
                    let cur = i32::from(before.score[credited]);
                    want[credited] =
                        (i32::from(before.cfg.game_to)).min(cur + i32::from(row.raw)) - cur;
                    if want[credited] < i32::from(row.raw) {
                        r.clamps += 1;
                    }
                    let got = [i32::from(delta[0]), i32::from(delta[1])];
                    if got != want {
                        note(
                            &mut r,
                            format!("delta {},{}, want {},{}", got[0], got[1], want[0], want[1]),
                        );
                    }
                    if d != want {
                        note(
                            &mut r,
                            format!("applied {},{}, want {},{}", d[0], d[1], want[0], want[1]),
                        );
                    }
                    if delta[0] != 0 && delta[1] != 0 {
                        note(&mut r, "delta has two non-zero components".to_owned());
                    }

                    // Cross-check against the standalone table implementation.
                    let direct = score_hand(ScoreHandParams {
                        maker_seat: maker,
                        alone_seat: h.alone_seat,
                        tricks_won: h.tricks_won,
                        score: before.score,
                        game_to: before.cfg.game_to,
                    });
                    if direct.result != *result {
                        note(&mut r, "scoreHand disagrees on the result".to_owned());
                    }
                    if direct.delta != *delta {
                        note(&mut r, "scoreHand disagrees on the delta".to_owned());
                    }
                }
            })),
            budget: None,
        },
    )?;

    let mut out = report.into_inner();
    out.fin = fin;
    Ok(out)
}

fn must_play(initial: GameState, pick: Picker, deep: bool) -> MatchReport {
    play_match(initial, pick, deep).unwrap_or_else(|e| panic!("{e}"))
}

/* -------------------------------------------------------------------------- */
/* Going alone, end to end                                                     */
/* -------------------------------------------------------------------------- */

struct LonerCase {
    skip: usize,
    maker: u8,
    sitting: u8,
    lead: u8,
    dealer_sits_out: bool,
}

// Dealer is seat 0 throughout, so eldest is 1 and the cutter is 3.
const LONER_ELDEST: LonerCase = LonerCase {
    skip: 0,
    maker: 1,
    sitting: 3,
    lead: 1,
    dealer_sits_out: false,
};
const LONER_DEALERS_PARTNER: LonerCase = LonerCase {
    skip: 1,
    maker: 2,
    sitting: 0,
    lead: 1,
    dealer_sits_out: true,
};
const LONER_THIRD_SEAT: LonerCase = LonerCase {
    skip: 2,
    maker: 3,
    sitting: 1,
    lead: 2,
    dealer_sits_out: false,
};
const LONER_DEALER: LonerCase = LonerCase {
    skip: 3,
    maker: 0,
    sitting: 2,
    lead: 1,
    dealer_sits_out: false,
};

fn loner_seats_out_and_leads(c: &LonerCase) {
    let seed = format!("alone-{}", c.maker);
    let called = order_up_after(&seed, seat(0), c.skip, true);
    let (maker, sitting, lead) = (seat(c.maker), seat(c.sitting), seat(c.lead));

    assert_eq!(called.hand.maker_seat, Some(maker));
    assert_eq!(called.hand.alone_seat, Some(maker));
    assert_eq!(called.hand.sitting_seat, Some(sitting));
    assert_eq!(called.hand.sitting_seat, Some(partner_of(maker)));
    assert_eq!(lead, first_active_from(eldest_of(seat(0)), Some(sitting)));

    // The dealer's discard is only skipped when the dealer is the one sitting.
    let at_lead = if c.dealer_sits_out {
        assert_eq!(called.hand.phase, GamePhase::TrickPlay);
        assert_eq!(called.hand.dealer_discard, called.hand.up_card);
        assert_eq!(called.hand.hands[0].len(), 5);
        called.clone()
    } else {
        assert_eq!(called.hand.phase, GamePhase::DealerDiscard);
        assert_eq!(called.hand.turn_seat, Some(seat(0)));
        assert_eq!(called.hand.hands[0].len(), 6);
        let first = called.hand.hands[0][0];
        let t = ok(&called, seat(0), Move::Discard { card: first }).state;
        assert_eq!(t.hand.dealer_discard, Some(first));
        assert_eq!(t.hand.hands[0].len(), 5);
        t
    };

    assert_eq!(at_lead.hand.phase, GamePhase::TrickPlay);
    assert_eq!(at_lead.hand.turn_seat, Some(lead));
    assert_ne!(at_lead.hand.turn_seat, Some(sitting));
    assert_eq!(at_lead.hand.trick.index, 0);
    assert_eq!(at_lead.hand.hands[sitting.index()].len(), 5);
}

fn loner_three_plays_a_trick(c: &LonerCase) {
    let seed = format!("alone-play-{}", c.maker);
    let called = order_up_after(&seed, seat(0), c.skip, true);
    let sitting = seat(c.sitting);
    let start = if c.dealer_sits_out {
        called
    } else {
        let first = called.hand.hands[0][0];
        ok(&called, seat(0), Move::Discard { card: first }).state
    };

    let mut seen_leaders: Vec<Seat> = Vec::new();
    let mut violations: Vec<String> = Vec::new();
    let scored = run_until(
        start,
        random_picker(&seed),
        |s| s.hand.phase == GamePhase::HandScore,
        RunHooks::on_state(|s| {
            // The sitting partner is offered nothing, in every phase.
            if !legal_moves(s, sitting).is_empty() {
                violations.push(format!("{}: legal set", s.hand.phase));
            }
            if s.hand.turn_seat == Some(sitting) {
                violations.push(format!("{}: on turn", s.hand.phase));
            }
            let view = project(s, sitting);
            if !view.hand.is_empty() {
                violations.push(format!("{}: own hand shown", s.hand.phase));
            }
            if view.hand_counts[sitting.index()] != 0 {
                violations.push(format!("{}: count shown", s.hand.phase));
            }
            if s.hand.phase == GamePhase::TrickPlay && s.hand.trick.plays.is_empty() {
                seen_leaders.push(turn(s));
            }
        }),
    );

    assert!(violations.is_empty(), "{violations:?}");
    assert_eq!(scored.hand.trick_log.len(), 5);
    for t in &scored.hand.trick_log {
        assert_eq!(t.plays.len(), 3);
        assert!(!t.plays.iter().any(|p| p.seat == sitting));
        assert_ne!(t.winner_seat, Some(sitting));
        let seats: HashSet<Seat> = t.plays.iter().map(|p| p.seat).collect();
        assert_eq!(seats.len(), 3);
    }
    let cards: Vec<Card> = scored
        .hand
        .trick_log
        .iter()
        .flat_map(|t| t.plays.iter().map(|p| p.card))
        .collect();
    assert_eq!(cards.len(), 15);
    assert_eq!(cards.iter().collect::<HashSet<_>>().len(), 15);

    // Every active hand empties simultaneously; the dead hand is untouched.
    for seat in SEATS_ALL {
        let want = if seat == sitting { 5 } else { 0 };
        assert_eq!(scored.hand.hands[seat.index()].len(), want);
    }
    assert_eq!(scored.hand.tricks_won[0] + scored.hand.tricks_won[1], 5);

    // The lead: trick 1 by rule, thereafter always the previous winner (V9).
    assert_eq!(seen_leaders[0], seat(c.lead));
    assert_eq!(seen_leaders.len(), 5);
    for (i, &leader) in seen_leaders.iter().enumerate().skip(1) {
        assert_eq!(Some(leader), scored.hand.trick_log[i - 1].winner_seat);
        assert_ne!(leader, sitting);
    }

    // The section 5.1 loner rows.
    let makers = team_of(seat(c.maker)).index();
    let made = scored.hand.tricks_won[makers];
    let row = expected_row(true, made);
    assert_eq!(scored.hand.result, Some(row.result));
    assert!(matches!(
        scored.hand.result,
        Some(HandResult::LonePoint | HandResult::LoneMarch | HandResult::Euchre)
    ));
    let credited = if row.to_makers { makers } else { 1 - makers };
    assert_eq!(scored.hand.delta.map(|d| d[credited]), Some(row.raw));
}

fn loner_rejects_the_sitting_seat(c: &LonerCase) {
    let seed = format!("alone-reject-{}", c.maker);
    let called = order_up_after(&seed, seat(0), c.skip, true);
    let sit = seat(c.sitting);
    let any_card = called.hand.hands[sit.index()][0];
    let before = called.clone();

    for mv in [
        Move::Pass,
        Move::OrderUp { alone: false },
        Move::Call {
            suit: Suit::Spades,
            alone: false,
        },
        Move::Cut { cut: true },
        Move::Discard { card: any_card },
        Move::Play { card: any_card },
    ] {
        expect_rule(RuleCode::SittingOut, apply(&called, sit, mv, None));
    }
    assert_eq!(called, before);
}

#[test]
fn loner_is_eldest_seats_out_3_and_leads_trick_1_from_1() {
    loner_seats_out_and_leads(&LONER_ELDEST);
}

#[test]
fn loner_is_eldest_three_plays_a_trick_fifteen_cards_and_a_dead_partner() {
    loner_three_plays_a_trick(&LONER_ELDEST);
}

#[test]
fn loner_is_eldest_rejects_every_action_from_the_sitting_seat_with_sitting_out() {
    loner_rejects_the_sitting_seat(&LONER_ELDEST);
}

#[test]
fn loner_is_dealers_partner_seats_out_0_and_leads_trick_1_from_1() {
    loner_seats_out_and_leads(&LONER_DEALERS_PARTNER);
}

#[test]
fn loner_is_dealers_partner_three_plays_a_trick_fifteen_cards_and_a_dead_partner() {
    loner_three_plays_a_trick(&LONER_DEALERS_PARTNER);
}

#[test]
fn loner_is_dealers_partner_rejects_every_action_from_the_sitting_seat_with_sitting_out() {
    loner_rejects_the_sitting_seat(&LONER_DEALERS_PARTNER);
}

#[test]
fn loner_is_third_seat_seats_out_1_and_leads_trick_1_from_2() {
    loner_seats_out_and_leads(&LONER_THIRD_SEAT);
}

#[test]
fn loner_is_third_seat_three_plays_a_trick_fifteen_cards_and_a_dead_partner() {
    loner_three_plays_a_trick(&LONER_THIRD_SEAT);
}

#[test]
fn loner_is_third_seat_rejects_every_action_from_the_sitting_seat_with_sitting_out() {
    loner_rejects_the_sitting_seat(&LONER_THIRD_SEAT);
}

#[test]
fn loner_is_the_dealer_seats_out_2_and_leads_trick_1_from_1() {
    loner_seats_out_and_leads(&LONER_DEALER);
}

#[test]
fn loner_is_the_dealer_three_plays_a_trick_fifteen_cards_and_a_dead_partner() {
    loner_three_plays_a_trick(&LONER_DEALER);
}

#[test]
fn loner_is_the_dealer_rejects_every_action_from_the_sitting_seat_with_sitting_out() {
    loner_rejects_the_sitting_seat(&LONER_DEALER);
}

#[test]
fn never_lets_the_loner_take_the_lead_from_their_own_sitting_partner() {
    // The sitting partner is eldest, so the opening lead passes clockwise to
    // the dealer's partner: a defender.
    let s = order_up_after("folk-rule", seat(0), 2, true);
    assert_eq!(s.hand.alone_seat, Some(seat(3)));
    assert_eq!(s.hand.sitting_seat, Some(seat(1)));
    let lead = ok(
        &s,
        seat(0),
        Move::Discard {
            card: s.hand.hands[0][0],
        },
    )
    .state;
    assert_eq!(lead.hand.turn_seat, Some(seat(2)));
    assert_ne!(lead.hand.turn_seat, s.hand.alone_seat);
    assert_ne!(team_of(turn(&lead)), team_of(seat(3)));
}

#[test]
fn an_ordinary_call_keeps_all_four_seats_in_with_four_plays_a_trick() {
    let called = order_up_after("not-alone", seat(0), 0, false);
    assert_eq!(called.hand.alone_seat, None);
    assert_eq!(called.hand.sitting_seat, None);
    let start = ok(
        &called,
        seat(0),
        Move::Discard {
            card: called.hand.hands[0][0],
        },
    )
    .state;
    let scored = run_until(
        start,
        random_picker("not-alone"),
        |s| s.hand.phase == GamePhase::HandScore,
        RunHooks::default(),
    );
    assert_eq!(scored.hand.trick_log.len(), 5);
    for t in &scored.hand.trick_log {
        assert_eq!(t.plays.len(), 4);
    }
    let cards: Vec<Card> = scored
        .hand
        .trick_log
        .iter()
        .flat_map(|t| t.plays.iter().map(|p| p.card))
        .collect();
    assert_eq!(cards.len(), 20);
    assert_eq!(cards.iter().collect::<HashSet<_>>().len(), 20);
    for seat in SEATS_ALL {
        assert!(scored.hand.hands[seat.index()].is_empty());
    }
    assert!(matches!(
        scored.hand.result,
        Some(HandResult::Point | HandResult::March | HandResult::Euchre)
    ));
}

/* -------------------------------------------------------------------------- */
/* A seeded match, and the score table                                         */
/* -------------------------------------------------------------------------- */

fn winner_and_loser(s: &GameState) -> (u8, u8) {
    let w = s.winner_team.unwrap_or_else(|| panic!("no winner")).index();
    (s.score[w], s.score[1 - w])
}

#[test]
fn ends_with_exactly_one_team_on_the_target_score() {
    let report = must_play(new_game("full-10", None), random_picker("full-10"), true);
    let s = &report.fin;

    assert!(report.violations.is_empty(), "{:?}", report.violations);
    assert_eq!(s.status, GameStatus::Complete);
    assert_eq!(s.hand.phase, GamePhase::GameOver);
    assert_eq!(s.hand.turn_seat, None);
    assert!(s.winner_team.is_some());
    let (w, l) = winner_and_loser(s);
    assert_eq!(w, 10);
    assert!(l < 10);
    assert!(report.hands >= 3);
    assert_eq!(report.tricks, report.hands * 5);
    assert!(report.cards_played.iter().all(|&n| n == 20 || n == 15));
    for seat in SEATS_ALL {
        assert!(legal_moves(s, seat).is_empty());
    }
}

#[test]
fn emits_game_won_exactly_once_as_the_last_step_of_the_match() {
    let mut wins: Vec<Step> = Vec::new();
    let mut last_steps: Vec<Step> = Vec::new();
    let fin = run_game(
        new_game("gamewon", None),
        random_picker("gamewon"),
        RunHooks::on_transition(|_b, res, _| {
            for st in &res.steps {
                if matches!(st, Step::GameWon { .. }) {
                    wins.push(st.clone());
                }
            }
            if !res.steps.is_empty() {
                last_steps = res.steps.clone();
            }
        }),
    );
    assert_eq!(wins.len(), 1);
    let Step::GameWon { team, score } = &wins[0] else {
        panic!("expected gameWon")
    };
    assert_eq!(Some(*team), fin.winner_team);
    assert_eq!(*score, fin.score);
    assert_eq!(last_steps.last(), Some(&wins[0]));
}

#[test]
fn rotates_the_deal_one_seat_left_after_every_scored_hand() {
    let mut dealers: Vec<Seat> = Vec::new();
    run_game(
        create_game(GameInit {
            game_id: "g".to_owned(),
            seed: "rotate".to_owned(),
            first_dealer: Some(seat(0)),
            ..GameInit::default()
        }),
        random_picker("rotate"),
        RunHooks::on_transition(|before, res, _| {
            if res
                .steps
                .iter()
                .any(|st| matches!(st, Step::HandScored { .. } | Step::ThrowIn))
            {
                dealers.push(before.hand.dealer_seat);
            }
        }),
    );
    assert!(dealers.len() >= 3);
    assert_eq!(dealers[0], seat(0));
    for i in 1..dealers.len() {
        assert_eq!(dealers[i], next_seat(dealers[i - 1]));
    }
}

#[test]
fn clamps_the_winning_score_never_overshooting_the_target_v11() {
    // gameTo 5 makes the 4-point lone march overshoot often, so the clamp is
    // genuinely exercised rather than merely asserted.
    let mut clamps = 0;
    for i in 0..60 {
        let seed = format!("clamp-{i}");
        let report = must_play(new_game(&seed, cfg_game_to(5)), random_picker(&seed), false);
        assert_eq!(assert_invariants(&report.fin), Ok(()));
        assert!(report.violations.is_empty(), "{:?}", report.violations);
        let (w, l) = winner_and_loser(&report.fin);
        assert_eq!(w, 5);
        assert!(l < 5);
        for d in &report.score_deltas {
            assert!([1, 2, 3, 4].contains(&(d[0] + d[1])));
            assert!(d[0] == 0 || d[1] == 0);
        }
        clamps += report.clamps;
    }
    // A clamp is `delta < rawPoints(result)`.
    assert!(clamps > 0);
}

#[test]
fn never_awards_more_than_the_table_row_clamped_on_any_hand() {
    let report = must_play(
        new_game("raw-vs-delta", cfg_game_to(7)),
        random_picker("raw-vs-delta"),
        true,
    );
    assert!(report.violations.is_empty(), "{:?}", report.violations);
    assert_eq!(report.results.len(), report.score_deltas.len());
    for (i, result) in report.results.iter().enumerate() {
        let d = report.score_deltas[i];
        assert!(d[0] + d[1] <= i32::from(raw_points(*result)));
        assert!(d[0] + d[1] > 0);
    }
}

fn reaches_a_target_score_of(game_to: u8) {
    let seed = format!("to-{game_to}");
    let report = must_play(
        new_game(&seed, cfg_game_to(game_to)),
        random_picker(&seed),
        true,
    );
    assert!(report.violations.is_empty(), "{:?}", report.violations);
    assert_eq!(report.fin.cfg.game_to, game_to);
    let (w, l) = winner_and_loser(&report.fin);
    assert_eq!(w, game_to);
    assert!(l < game_to);
    assert!(report.hands <= 2 * usize::from(game_to));
}

#[test]
fn reaches_a_target_score_of_5_exactly() {
    reaches_a_target_score_of(5);
}

#[test]
fn reaches_a_target_score_of_7_exactly() {
    reaches_a_target_score_of(7);
}

#[test]
fn reaches_a_target_score_of_10_exactly() {
    reaches_a_target_score_of(10);
}

#[test]
fn reaches_a_target_score_of_11_exactly() {
    reaches_a_target_score_of(11);
}

#[test]
fn every_hand_scores_under_stick_the_dealer_so_no_hand_is_ever_thrown_in() {
    let report = must_play(
        new_game("no-throwin", None),
        random_picker("no-throwin"),
        false,
    );
    assert_eq!(report.throw_ins, 0);
    assert!(!report.results.contains(&HandResult::ThrowIn));
    assert_eq!(report.results.len(), report.hands);
}

/* -------------------------------------------------------------------------- */
/* The fuzz harness                                                            */
/* -------------------------------------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThrowIns {
    None,
    Some,
}

struct Batch {
    label: &'static str,
    games: usize,
    cfg: Option<EngineConfigOverrides>,
    max_hands: usize,
    pass_heavy: bool,
    throw_ins: ThrowIns,
}

fn run_batch(b: &Batch) {
    let mut failures: Vec<String> = Vec::new();
    let mut total_hands = 0;
    let mut total_tricks = 0;
    let mut total_loners = 0;
    let mut total_throw_ins = 0;
    let mut seen_results: BTreeSet<&'static str> = BTreeSet::new();
    let mut seen_deltas: HashSet<[i32; 2]> = HashSet::new();

    for i in 0..b.games {
        let seed = format!("fuzz-{}-{i}", b.label);
        let start = create_game(GameInit {
            game_id: format!("g-{seed}"),
            seed: seed.clone(),
            cfg: b.cfg,
            internal_token: None,
            first_dealer: Some(Seat::wrapping((i % 4) as u8)),
        });
        let pick = if b.pass_heavy {
            pass_heavy_picker(&seed)
        } else {
            random_picker(&seed)
        };
        let report = match play_match(start, pick, true) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{seed}: threw {e}"));
                continue;
            }
        };

        for v in &report.violations {
            failures.push(format!("{seed}: {v}"));
        }

        let s = &report.fin;
        let game_to = s.cfg.game_to;
        if s.status != GameStatus::Complete {
            failures.push(format!("{seed}: status {}", s.status));
        }
        if s.hand.phase != GamePhase::GameOver {
            failures.push(format!("{seed}: phase {}", s.hand.phase));
        }
        match s.winner_team {
            None => failures.push(format!("{seed}: no winner")),
            Some(w) => {
                if s.score[w.index()] != game_to {
                    failures.push(format!("{seed}: winner at {}", s.score[w.index()]));
                }
                if s.score[1 - w.index()] >= game_to {
                    failures.push(format!("{seed}: both teams at {game_to}"));
                }
            }
        }
        if report.hands < 1 {
            failures.push(format!("{seed}: no hand scored"));
        }
        if report.hands > b.max_hands {
            failures.push(format!("{seed}: {} scored hands", report.hands));
        }
        if report.tricks != report.hands * 5 {
            failures.push(format!(
                "{seed}: {} tricks over {} hands",
                report.tricks, report.hands
            ));
        }
        let bound = (b.max_hands + report.throw_ins) * 5;
        if report.tricks > bound {
            failures.push(format!("{seed}: {} tricks exceeds {bound}", report.tricks));
        }
        for &n in &report.cards_played {
            if n != 20 && n != 15 {
                failures.push(format!("{seed}: {n} cards in a hand"));
            }
        }
        for d in &report.score_deltas {
            let total = d[0] + d[1];
            if !(1..=4).contains(&total) {
                failures.push(format!("{seed}: score moved by {},{}", d[0], d[1]));
            }
            if d[0] != 0 && d[1] != 0 {
                failures.push(format!("{seed}: both scores moved"));
            }
            seen_deltas.insert(*d);
        }
        if report.score_deltas.len() != report.hands {
            failures.push(format!(
                "{seed}: {} score moves, {} hands",
                report.score_deltas.len(),
                report.hands
            ));
        }

        total_hands += report.hands;
        total_tricks += report.tricks;
        total_loners += report.loners;
        total_throw_ins += report.throw_ins;
        for r in &report.results {
            seen_results.insert(r.as_str());
        }
    }

    let rows = ["euchre", "lone_march", "lone_point", "march", "point"];

    assert!(failures.is_empty(), "{failures:#?}");
    assert!(total_hands > b.games);
    assert_eq!(total_tricks, total_hands * 5);
    assert!(total_loners > 0);
    assert!(seen_deltas.len() > 1);
    // No row outside the section 5.1 table is ever produced...
    for r in &seen_results {
        assert!(rows.contains(r), "{r}");
    }
    // ...and the big batch must actually reach all five of them.
    if b.games >= 200 {
        let seen: Vec<&str> = seen_results.iter().copied().collect();
        assert_eq!(seen, rows);
    }
    if b.throw_ins == ThrowIns::None {
        assert_eq!(total_throw_ins, 0);
    } else {
        assert!(total_throw_ins > 0);
    }
}

#[test]
fn plays_200_matches_to_completion_default_with_no_invariant_broken() {
    run_batch(&Batch {
        label: "default",
        games: 200,
        cfg: None,
        max_hands: 25,
        pass_heavy: false,
        throw_ins: ThrowIns::None,
    });
}

#[test]
fn plays_40_matches_to_completion_to_5_with_no_invariant_broken() {
    run_batch(&Batch {
        label: "to-5",
        games: 40,
        cfg: cfg_game_to(5),
        max_hands: 15,
        pass_heavy: false,
        throw_ins: ThrowIns::None,
    });
}

#[test]
fn plays_30_matches_to_completion_to_11_with_no_invariant_broken() {
    run_batch(&Batch {
        label: "to-11",
        games: 30,
        cfg: cfg_game_to(11),
        max_hands: 30,
        pass_heavy: false,
        throw_ins: ThrowIns::None,
    });
}

#[test]
fn plays_40_matches_to_completion_no_stick_with_no_invariant_broken() {
    run_batch(&Batch {
        label: "no-stick",
        games: 40,
        cfg: cfg_stick(false),
        max_hands: 60,
        pass_heavy: false,
        throw_ins: ThrowIns::None,
    });
}

#[test]
fn plays_40_matches_to_completion_pass_heavy_stick_with_no_invariant_broken() {
    run_batch(&Batch {
        label: "pass-heavy-stick",
        games: 40,
        cfg: None,
        max_hands: 30,
        pass_heavy: true,
        throw_ins: ThrowIns::None,
    });
}

#[test]
fn plays_40_matches_to_completion_pass_heavy_throwin_with_no_invariant_broken() {
    run_batch(&Batch {
        label: "pass-heavy-throwin",
        games: 40,
        cfg: cfg_stick(false),
        max_hands: 40,
        pass_heavy: true,
        throw_ins: ThrowIns::Some,
    });
}

#[test]
fn never_deals_a_card_twice_in_any_of_200_opening_deals() {
    let want = sorted(&DECK);
    for i in 0..200u32 {
        let seed = format!("deal-fuzz-{i}");
        let cut = advance(&create_game(GameInit {
            game_id: "g".to_owned(),
            seed,
            first_dealer: Some(Seat::wrapping((i % 4) as u8)),
            ..GameInit::default()
        }))
        .state;
        let dealt = ok(&cut, turn(&cut), Move::Cut { cut: i % 2 == 0 }).state;
        let h = &dealt.hand;
        let mut all: Vec<Card> = h.hands.iter().flatten().copied().collect();
        all.extend(h.kitty.iter().copied());
        assert_eq!(all.len(), 24);
        assert_eq!(sorted(&all), want);
        assert_eq!(h.up_card, Some(h.kitty[0]));
        for seat in SEATS_ALL {
            assert_eq!(h.hands[seat.index()].len(), 5);
        }
    }
}
