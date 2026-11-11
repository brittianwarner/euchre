//! Port of `__tests__/reduce.test.ts`: the phase state machine, the rejection
//! codes, determinism (V20) and purity (V21).

mod common;

use common::*;
use euchre_engine::{
    ApplyOptions, Bid, Card, DECK, EngineConfigOverrides, GameInit, GamePhase, GameState,
    GameStatus, Move, RuleCode, SUITS, Seat, Step, Suit, Team, Trick, advance, apply, create_game,
    cut_for_hand, cutter_of, effective_suit, eldest_of, is_stuck_dealer, legal_move_ids,
    legal_moves, move_id, new_game, next_seat, project, say_for_bid, shuffle_for_hand, start_hand,
    suit_of, team_of,
};

fn ids(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| (*s).to_owned()).collect()
}

fn step_tags(steps: &[Step]) -> Vec<&'static str> {
    steps.iter().map(Step::tag).collect()
}

fn mate_of(s: Suit) -> Suit {
    match s {
        Suit::Spades => Suit::Clubs,
        Suit::Clubs => Suit::Spades,
        Suit::Hearts => Suit::Diamonds,
        Suit::Diamonds => Suit::Hearts,
    }
}

fn pass_record(seat_n: u8, say: &str) -> euchre_engine::BidRecord {
    euchre_engine::BidRecord {
        seat: seat(seat_n),
        mv: Bid::Pass,
        say: say.to_owned(),
    }
}

/* -------------------------------------------------------------------------- */
/* createGame                                                                  */
/* -------------------------------------------------------------------------- */

#[test]
fn produces_a_lobby_state_with_nothing_dealt_and_no_seat_to_act() {
    let g = new_game("lobby-seed", None);
    assert_eq!(json(&g.schema), "1");
    assert_eq!(g.status, GameStatus::Active);
    assert_eq!(g.hand.phase, GamePhase::Lobby);
    assert_eq!(g.hand.turn_seat, None);
    assert_eq!(g.score, [0, 0]);
    assert_eq!(g.hand_no, 0);
    assert_eq!(g.hand.hand_no, 0);
    assert_eq!(g.misdeal_streak, 0);
    assert_eq!(g.winner_team, None);
    assert_eq!(g.hand.up_card, None);
    assert_eq!(g.hand.trump, None);
    assert_eq!(g.hand.maker_seat, None);
    assert_eq!(g.hand.dealer_discard, None);
    assert_eq!(g.hand.trick, Trick::empty(0));
    for s in SEATS_ALL {
        assert!(g.hand.hands[s.index()].is_empty());
        assert!(legal_moves(&g, s).is_empty());
    }
}

#[test]
fn persists_a_genuine_24_card_permutation_derived_from_the_seed_alone() {
    let g = new_game("perm-seed", None);
    assert_eq!(g.hand.deck_order.len(), 24);
    let set: std::collections::HashSet<Card> = g.hand.deck_order.iter().copied().collect();
    assert_eq!(set.len(), 24);
    assert_eq!(sorted(&g.hand.deck_order), sorted(&DECK));
    assert_eq!(g.hand.deck_order, shuffle_for_hand("perm-seed", 0));
    assert_eq!(g.hand.kitty.to_vec(), g.hand.deck_order[20..].to_vec());
}

#[test]
fn honours_an_explicit_first_dealer_and_otherwise_draws_one_from_the_seed() {
    for d in SEATS_ALL {
        let g = create_game(GameInit {
            game_id: "g".to_owned(),
            seed: "fd".to_owned(),
            first_dealer: Some(d),
            ..GameInit::default()
        });
        assert_eq!(g.first_dealer, d);
        assert_eq!(g.hand.dealer_seat, d);
    }
    let drawn = new_game("fd-drawn", None);
    assert!(SEATS_ALL.contains(&drawn.first_dealer));
    assert_eq!(drawn.hand.dealer_seat, drawn.first_dealer);
}

#[test]
fn applies_cfg_overrides_on_top_of_the_v1_defaults() {
    let g = new_game(
        "cfg",
        Some(EngineConfigOverrides {
            game_to: Some(5),
            stick_the_dealer: Some(false),
            ..EngineConfigOverrides::default()
        }),
    );
    assert_eq!(g.cfg.game_to, 5);
    assert!(!g.cfg.stick_the_dealer);
    assert_eq!(json(&g.cfg.deck_variant), "\"24\"");
    assert_eq!(g.cfg.misdeal_limit, 3);
}

/* -------------------------------------------------------------------------- */
/* lobby -> cutting                                                            */
/* -------------------------------------------------------------------------- */

#[test]
fn advance_opens_the_first_hand_and_offers_the_cut_to_third_seat() {
    let g = new_game("cut-open", None);
    let res = advance(&g);
    assert!(res.steps.is_empty());
    let s = res.state;
    assert_eq!(s.hand.phase, GamePhase::Cutting);
    assert_eq!(s.hand.turn_seat, Some(cutter_of(g.hand.dealer_seat)));
    assert_eq!(s.hand.dealer_seat, g.hand.dealer_seat);
    assert_eq!(s.hand.deck_order, shuffle_for_hand("cut-open", 0));
    assert_ne!(s.turn_id, g.turn_id);
    assert_eq!(legal_move_ids(&s, turn(&s)), ids(&["cut:yes", "cut:no"]));
}

#[test]
fn offers_nothing_to_the_three_seats_that_are_not_the_cutter() {
    let s = opened("cut-open-2", None, None);
    let cutter = cutter_of(s.hand.dealer_seat);
    for seat in SEATS_ALL {
        if seat == cutter {
            continue;
        }
        assert!(legal_moves(&s, seat).is_empty());
    }
}

#[test]
fn start_hand_is_a_pure_re_open_of_the_same_hand_number_and_dealer() {
    let g = new_game("sh", None);
    let a = start_hand(&g).state;
    let b = start_hand(&g).state;
    assert_eq!(a.hand.phase, GamePhase::Cutting);
    assert_eq!(a.hand.hand_no, 0);
    assert_eq!(a.hand.dealer_seat, g.hand.dealer_seat);
    assert_eq!(a.hand, b.hand);
}

/* -------------------------------------------------------------------------- */
/* cutting -> deal -> bid_round_1                                              */
/* -------------------------------------------------------------------------- */

#[test]
fn run_em_deals_from_the_untouched_permutation_and_turns_the_up_card() {
    let s0 = opened("deal-a", Some(seat(0)), None);
    let before = s0.hand.deck_order.clone();
    let res = ok(&s0, cutter_of(seat(0)), Move::Cut { cut: false });
    let s = res.state;

    assert_eq!(s.hand.cut, Some(false));
    assert_eq!(s.hand.deck_order, before);
    assert_eq!(s.hand.phase, GamePhase::BidRound1);
    assert_eq!(s.hand.turn_seat, Some(eldest_of(seat(0))));
    assert_eq!(s.hand.passes, 0);
    assert!(s.hand.bids.is_empty());
    assert_eq!(s.hand.up_card, Some(before[20]));
    assert!(!s.hand.up_card_turned_down);
    assert_eq!(s.hand.turned_down_suit, None);
    assert_eq!(s.hand.kitty.to_vec(), before[20..].to_vec());
    for seat in SEATS_ALL {
        assert_eq!(s.hand.hands[seat.index()].len(), 5);
    }

    let dealt: Vec<Card> = s.hand.hands.iter().flatten().copied().collect();
    assert_eq!(dealt.len(), 20);
    let all: std::collections::HashSet<Card> =
        dealt.iter().chain(s.hand.kitty.iter()).copied().collect();
    assert_eq!(all.len(), 24);
}

#[test]
fn bump_rotates_the_permutation_deterministically_never_reshuffles() {
    let s0 = opened("deal-b", Some(seat(2)), None);
    let before = s0.hand.deck_order.clone();
    let s = ok(&s0, cutter_of(seat(2)), Move::Cut { cut: true }).state;
    assert_eq!(s.hand.cut, Some(true));
    assert_eq!(s.hand.deck_order, cut_for_hand(&before, "deal-b", 0));
    assert_ne!(s.hand.deck_order, before);
    assert_eq!(sorted(&s.hand.deck_order), sorted(&before));
    assert_eq!(s.hand.up_card, Some(s.hand.deck_order[20]));
}

#[test]
fn emits_cut_dealt_and_up_card_turned_in_that_order_with_eight_faithful_packets() {
    let s0 = opened("deal-c", Some(seat(1)), None);
    let res = ok(&s0, cutter_of(seat(1)), Move::Cut { cut: false });
    assert_eq!(step_tags(&res.steps), ["cut", "dealt", "upCardTurned"]);

    assert_eq!(
        res.steps[0],
        Step::Cut {
            seat: cutter_of(seat(1)),
            cut: false
        }
    );

    let Step::Dealt { packets } = &res.steps[1] else {
        panic!("expected dealt")
    };
    assert_eq!(packets.len(), 8);
    assert_eq!(packets.iter().map(|p| p.count as usize).sum::<usize>(), 20);
    for seat in SEATS_ALL {
        let mine: Vec<_> = packets.iter().filter(|p| p.seat == seat).collect();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine.iter().map(|p| p.count as usize).sum::<usize>(), 5);
        let cards: Vec<Card> = mine
            .iter()
            .flat_map(|p| p.cards.clone().unwrap_or_default())
            .collect();
        assert_eq!(cards, res.state.hand.hands[seat.index()]);
    }
    // Packet order is dealing order: eldest first, two passes clockwise.
    let order: Vec<u8> = packets.iter().map(|p| p.seat.get()).collect();
    assert_eq!(order, [2, 3, 0, 1, 2, 3, 0, 1]);

    let Step::UpCardTurned { card } = &res.steps[2] else {
        panic!("expected upCardTurned")
    };
    assert_eq!(Some(*card), res.state.hand.up_card);
}

#[test]
fn rejects_a_cut_from_a_seat_that_is_not_the_cutter_and_any_non_cut_move() {
    let s = opened("deal-d", Some(seat(0)), None);
    let cutter = cutter_of(seat(0));
    for seat in SEATS_ALL {
        if seat == cutter {
            continue;
        }
        expect_rule(
            RuleCode::NotYourTurn,
            apply(&s, seat, Move::Cut { cut: true }, None),
        );
    }
    expect_rule(RuleCode::WrongPhase, apply(&s, cutter, Move::Pass, None));
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, cutter, Move::OrderUp { alone: false }, None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(
            &s,
            cutter,
            Move::Play {
                card: s.hand.deck_order[0],
            },
            None,
        ),
    );
}

/* -------------------------------------------------------------------------- */
/* bid_round_1                                                                 */
/* -------------------------------------------------------------------------- */

#[test]
fn offers_exactly_pass_order_up_order_up_alone_with_role_correct_labels() {
    let s = after_cut(&opened("b1-a", Some(seat(0)), None), false);
    assert_eq!(
        legal_move_ids(&s, eldest_of(seat(0))),
        ids(&["pass", "orderUp", "orderUp+alone"])
    );
    let labels: Vec<String> = legal_moves(&s, seat(1))
        .into_iter()
        .map(|m| m.label)
        .collect();
    assert_eq!(labels, ids(&["Pass", "Order it up", "Order it up, alone"]));

    let at_partner = passes(&s, 1);
    let labels: Vec<String> = legal_moves(&at_partner, seat(2))
        .into_iter()
        .map(|m| m.label)
        .collect();
    assert_eq!(labels, ids(&["Pass", "I assist", "I assist, alone"]));
    let at_dealer = passes(&s, 3);
    let labels: Vec<String> = legal_moves(&at_dealer, seat(0))
        .into_iter()
        .map(|m| m.label)
        .collect();
    assert_eq!(labels, ids(&["Pass", "I take it", "I take it, alone"]));
}

#[test]
fn rotates_one_seat_left_per_pass_and_counts_them() {
    let mut s = after_cut(&opened("b1-b", Some(seat(0)), None), false);
    let order = [1u8, 2, 3, 0];
    for (i, &who) in order.iter().take(3).enumerate() {
        assert_eq!(s.hand.turn_seat, Some(seat(who)));
        assert_eq!(s.hand.passes as usize, i);
        s = ok(&s, seat(who), Move::Pass).state;
        assert_eq!(s.hand.phase, GamePhase::BidRound1);
        assert_eq!(s.hand.passes as usize, i + 1);
        assert_eq!(s.hand.bids.len(), i + 1);
        assert_eq!(s.hand.bids[i], pass_record(who, "Pass."));
    }
    assert_eq!(s.hand.turn_seat, Some(seat(0)));
}

#[test]
fn four_passes_turn_the_up_card_down_and_open_round_2_at_eldest() {
    let s3 = passes(&after_cut(&opened("b1-c", Some(seat(0)), None), false), 3);
    let up_card = s3.hand.up_card.unwrap();
    let res = ok(&s3, seat(0), Move::Pass);
    let s = &res.state;

    assert_eq!(s.hand.phase, GamePhase::BidRound2);
    assert_eq!(s.hand.turn_seat, Some(eldest_of(seat(0))));
    assert!(s.hand.up_card_turned_down);
    assert_eq!(s.hand.turned_down_suit, Some(suit_of(up_card)));
    assert_eq!(s.hand.up_card, Some(up_card));
    assert_eq!(s.hand.passes, 0);
    assert_eq!(s.hand.bids.len(), 4);
    assert_eq!(s.hand.bids[3], pass_record(0, "Turn it down."));
    assert_eq!(s.hand.trump, None);
    assert_eq!(s.hand.maker_seat, None);
    assert_eq!(s.hand.dealer_discard, None);

    assert_eq!(step_tags(&res.steps), ["bid", "turnedDown"]);
    assert_eq!(
        res.steps[1],
        Step::TurnedDown {
            suit: suit_of(up_card)
        }
    );
}

#[test]
fn routes_every_successful_order_up_to_dealer_discard_whoever_called() {
    let base = after_cut(&opened("b1-d", Some(seat(0)), None), false);
    let up_card = base.hand.up_card.unwrap();
    let order = [1u8, 2, 3, 0];

    for (k, &caller_n) in order.iter().enumerate() {
        let caller = seat(caller_n);
        let s0 = passes(&base, k);
        let before = s0.hand.hands[0].clone();
        let res = ok(&s0, caller, Move::OrderUp { alone: false });
        let s = &res.state;

        assert_eq!(s.hand.phase, GamePhase::DealerDiscard);
        assert_eq!(s.hand.turn_seat, Some(seat(0)));
        assert_eq!(s.hand.trump, Some(suit_of(up_card)));
        assert_eq!(s.hand.maker_seat, Some(caller));
        assert_eq!(s.hand.alone_seat, None);
        assert_eq!(s.hand.sitting_seat, None);
        assert_eq!(s.hand.up_card, Some(up_card));
        assert!(!s.hand.up_card_turned_down);
        assert_eq!(s.hand.turned_down_suit, None);
        assert_eq!(s.hand.dealer_discard, None);
        let mut want = before.clone();
        want.push(up_card);
        assert_eq!(s.hand.hands[0], want);
        assert_eq!(s.hand.hands[0].len(), 6);
        for other in [1, 2, 3] {
            assert_eq!(s.hand.hands[other].len(), 5);
        }

        assert_eq!(step_tags(&res.steps), ["bid", "trumpSet"]);
        assert_eq!(
            res.steps[1],
            Step::TrumpSet {
                suit: suit_of(up_card),
                maker_seat: caller,
                alone_seat: None,
                sitting_seat: None,
                via_order_up: true,
            }
        );
    }
}

#[test]
fn records_the_role_correct_spoken_line_for_each_order_up() {
    let base = after_cut(&opened("b1-e", Some(seat(0)), None), false);
    let said = |k: usize, alone: bool| -> String {
        let s0 = passes(&base, k);
        let seat = seat([1, 2, 3, 0][k]);
        ok(&s0, seat, Move::OrderUp { alone }).state.hand.bids[k]
            .say
            .clone()
    };
    assert_eq!(said(0, false), "Order it up.");
    assert_eq!(said(1, false), "I assist.");
    assert_eq!(said(2, false), "Order it up.");
    assert_eq!(said(3, false), "I take it.");
    assert_eq!(said(0, true), "Order it up. Alone.");
    assert_eq!(said(3, true), "I take it. Alone.");
}

#[test]
fn refuses_round_2_vocabulary_in_round_1_v2() {
    let s = after_cut(&opened("b1-f", Some(seat(0)), None), false);
    let first = s.hand.hands[1][0];
    expect_rule(
        RuleCode::WrongPhase,
        apply(
            &s,
            seat(1),
            Move::Call {
                suit: Suit::Spades,
                alone: false,
            },
            None,
        ),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(1), Move::Cut { cut: true }, None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(1), Move::Discard { card: first }, None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(1), Move::Play { card: first }, None),
    );
}

/* -------------------------------------------------------------------------- */
/* dealer_discard                                                              */
/* -------------------------------------------------------------------------- */

struct DiscardSetup {
    s: GameState,
    dealt: Vec<Card>,
    up_card: Card,
}

fn discard_setup(seed: &str) -> DiscardSetup {
    let base = after_cut(&opened(seed, Some(seat(0)), None), false);
    let dealt = base.hand.hands[0].clone();
    let up_card = base.hand.up_card.unwrap();
    let s = ok(&base, seat(1), Move::OrderUp { alone: false }).state;
    DiscardSetup { s, dealt, up_card }
}

#[test]
fn offers_exactly_the_dealer_six_including_the_up_card_itself() {
    let DiscardSetup { s, dealt, up_card } = discard_setup("dd-a");
    let got = legal_move_ids(&s, seat(0));
    assert_eq!(got.len(), 6);
    let mut want: Vec<String> = dealt.iter().map(|c| format!("discard:{c}")).collect();
    want.push(format!("discard:{up_card}"));
    assert_eq!(got, want);
    assert!(got.contains(&format!("discard:{up_card}")));
    for seat_n in [1, 2, 3] {
        assert!(legal_moves(&s, seat(seat_n)).is_empty());
    }
}

#[test]
fn discarding_the_up_card_leaves_five_cards_and_the_original_deal_intact() {
    let DiscardSetup { s, dealt, up_card } = discard_setup("dd-a");
    let res = ok(&s, seat(0), Move::Discard { card: up_card });
    let t = &res.state;

    assert_eq!(t.hand.hands[0], dealt);
    assert_eq!(t.hand.hands[0].len(), 5);
    assert_eq!(t.hand.dealer_discard, Some(up_card));
    assert_eq!(t.hand.up_card, Some(up_card));
    assert!(!t.hand.up_card_turned_down);
    assert_eq!(t.hand.phase, GamePhase::TrickPlay);
    assert_eq!(t.hand.turn_seat, Some(eldest_of(seat(0))));
    assert_eq!(t.hand.trick, Trick::empty(0));
    assert_eq!(res.steps, vec![Step::DealerDiscarded { seat: seat(0) }]);
    // V12/V13: the step is structurally incapable of naming the card.
    let v: serde_json::Value = serde_json::from_str(&json(&res.steps[0])).unwrap();
    let mut keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["seat", "t"]);
    assert_eq!(project(t, seat(0)).up_card, Some(up_card));
    assert_eq!(project(t, seat(1)).kitty_count, 4);
}

#[test]
fn discarding_a_dealt_card_keeps_the_up_card_in_the_dealer_hand() {
    let DiscardSetup { s, dealt, up_card } = discard_setup("dd-b");
    let drop = dealt[2];
    let t = ok(&s, seat(0), Move::Discard { card: drop }).state;
    let mut want: Vec<Card> = dealt.iter().copied().filter(|&c| c != drop).collect();
    want.push(up_card);
    assert_eq!(t.hand.hands[0], want);
    assert_eq!(t.hand.hands[0].len(), 5);
    assert!(t.hand.hands[0].contains(&up_card));
    assert_eq!(t.hand.dealer_discard, Some(drop));
}

#[test]
fn rejects_a_discard_of_a_card_the_dealer_does_not_hold() {
    let DiscardSetup { s, .. } = discard_setup("dd-c");
    let foreign = *DECK.iter().find(|c| !s.hand.hands[0].contains(c)).unwrap();
    expect_rule(
        RuleCode::CardNotInHand,
        apply(&s, seat(0), Move::Discard { card: foreign }, None),
    );
}

#[test]
fn rejects_bidding_after_trump_is_set_with_bidding_closed_not_wrong_phase() {
    let DiscardSetup { s, .. } = discard_setup("dd-d");
    expect_rule(
        RuleCode::BiddingClosed,
        apply(&s, seat(0), Move::Pass, None),
    );
    expect_rule(
        RuleCode::BiddingClosed,
        apply(&s, seat(0), Move::OrderUp { alone: false }, None),
    );
    expect_rule(
        RuleCode::BiddingClosed,
        apply(
            &s,
            seat(0),
            Move::Call {
                suit: Suit::Spades,
                alone: false,
            },
            None,
        ),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(0), Move::Cut { cut: false }, None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(
            &s,
            seat(0),
            Move::Play {
                card: s.hand.hands[0][0],
            },
            None,
        ),
    );
}

#[test]
fn rejects_a_discard_from_anyone_but_the_dealer() {
    let DiscardSetup { s, .. } = discard_setup("dd-e");
    for n in [1u8, 2, 3] {
        let err = expect_rule(
            RuleCode::NotYourTurn,
            apply(
                &s,
                seat(n),
                Move::Discard {
                    card: s.hand.hands[n as usize][0],
                },
                None,
            ),
        );
        assert!(err.legal.is_empty());
    }
}

/* -------------------------------------------------------------------------- */
/* bid_round_2                                                                 */
/* -------------------------------------------------------------------------- */

/// Four round-1 passes, then `n` round-2 passes.
fn round2(seed: &str, n: usize, cfg: Option<EngineConfigOverrides>) -> GameState {
    passes(&after_cut(&opened(seed, Some(seat(0)), cfg), false), 4 + n)
}

fn call(suit: Suit, alone: bool) -> Move {
    Move::Call { suit, alone }
}

#[test]
fn offers_pass_plus_three_suits_x_plain_alone_and_never_the_turned_down_suit() {
    let s = round2("b2-a", 0, None);
    let down = s.hand.turned_down_suit.unwrap();
    let got = legal_move_ids(&s, eldest_of(seat(0)));
    assert_eq!(got.len(), 7);
    assert_eq!(got[0], "pass");
    assert!(!got.contains(&format!("call:{down}")));
    assert!(!got.contains(&format!("call:{down}+alone")));
    for suit in SUITS {
        if suit == down {
            continue;
        }
        assert!(got.contains(&format!("call:{suit}")));
        assert!(got.contains(&format!("call:{suit}+alone")));
    }
}

#[test]
fn rejects_naming_the_turned_down_suit_as_illegal_move() {
    let s = round2("b2-b", 0, None);
    let down = s.hand.turned_down_suit.unwrap();
    let before = s.clone();
    let err = expect_rule(
        RuleCode::IllegalMove,
        apply(&s, seat(1), call(down, false), None),
    );
    expect_rule(
        RuleCode::IllegalMove,
        apply(&s, seat(1), call(down, true), None),
    );
    assert_eq!(s, before);
    let got: Vec<String> = err.legal.into_iter().map(|m| m.id).collect();
    assert_eq!(got, legal_move_ids(&s, seat(1)));
}

#[test]
fn refuses_round_1_vocabulary_once_the_up_card_is_buried_v2() {
    let s = round2("b2-c", 0, None);
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(1), Move::OrderUp { alone: false }, None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(1), Move::Cut { cut: false }, None),
    );
}

#[test]
fn a_successful_call_skips_the_discard_and_leads_straight_into_trick_play() {
    let s = round2("b2-d", 0, None);
    let down = s.hand.turned_down_suit.unwrap();
    let suit = *SUITS.iter().find(|&&x| x != down).unwrap();
    let res = ok(&s, seat(1), call(suit, false));
    let t = &res.state;

    assert_eq!(t.hand.phase, GamePhase::TrickPlay);
    assert_eq!(t.hand.turn_seat, Some(eldest_of(seat(0))));
    assert_eq!(t.hand.trump, Some(suit));
    assert_eq!(t.hand.maker_seat, Some(seat(1)));
    assert_eq!(t.hand.alone_seat, None);
    assert_eq!(t.hand.dealer_discard, None);
    assert_eq!(t.hand.up_card, s.hand.up_card);
    assert!(t.hand.up_card_turned_down);
    for seat in SEATS_ALL {
        assert_eq!(t.hand.hands[seat.index()].len(), 5);
    }

    assert_eq!(step_tags(&res.steps), ["bid", "trumpSet"]);
    assert_eq!(
        res.steps[1],
        Step::TrumpSet {
            suit,
            maker_seat: seat(1),
            alone_seat: None,
            sitting_seat: None,
            via_order_up: false,
        }
    );
}

#[test]
fn says_next_for_the_colour_mate_and_crossing_the_creek_otherwise() {
    let s = round2("b2-e", 0, None);
    let down = s.hand.turned_down_suit.unwrap();
    let mate = mate_of(down);
    let cross: Vec<Suit> = SUITS
        .iter()
        .copied()
        .filter(|&x| x != down && x != mate)
        .collect();

    let bid = |suit, alone| Bid::Call { suit, alone };
    assert_eq!(say_for_bid(&s, seat(1), &bid(mate, false)), "Next.");
    assert_eq!(say_for_bid(&s, seat(1), &bid(mate, true)), "Next. Alone.");
    for c in cross {
        assert_eq!(
            say_for_bid(&s, seat(1), &bid(c, false)),
            "Crossing the creek."
        );
    }
    let t = ok(&s, seat(1), call(mate, false)).state;
    assert_eq!(
        t.hand.bids[4],
        euchre_engine::BidRecord {
            seat: seat(1),
            mv: bid(mate, false),
            say: "Next.".to_owned(),
        }
    );
    assert_eq!(t.hand.bids.len(), 5);
}

#[test]
fn bounds_the_auction_at_eight_records_over_both_rounds_v18() {
    let s = round2("b2-f", 3, None);
    let down = s.hand.turned_down_suit.unwrap();
    let suit = *SUITS.iter().find(|&&x| x != down).unwrap();
    let t = ok(&s, seat(0), call(suit, false)).state;
    assert_eq!(t.hand.bids.len(), 8);
    assert_eq!(t.hand.bids.iter().filter(|b| b.mv == Bid::Pass).count(), 7);
}

/* -------------------------------------------------------------------------- */
/* stick the dealer                                                            */
/* -------------------------------------------------------------------------- */

fn stuck(seed: &str) -> GameState {
    passes(&after_cut(&opened(seed, Some(seat(0)), None), false), 7)
}

#[test]
fn removes_pass_from_the_dealer_legal_set_at_the_third_round_2_pass_v3() {
    let s = stuck("std-a");
    assert_eq!(s.hand.phase, GamePhase::BidRound2);
    assert_eq!(s.hand.turn_seat, Some(seat(0)));
    assert_eq!(s.hand.passes, 3);
    assert!(is_stuck_dealer(&s, seat(0)));
    assert!(!is_stuck_dealer(&s, seat(1)));

    let got = legal_move_ids(&s, seat(0));
    assert_eq!(got.len(), 6);
    assert!(!got.contains(&"pass".to_owned()));
    assert!(got.iter().all(|id| id.starts_with("call:")));
}

#[test]
fn rejects_the_dealer_pass_with_dealer_must_call_never_bidding_closed() {
    let s = stuck("std-b");
    let before = s.clone();
    let err = expect_rule(
        RuleCode::DealerMustCall,
        apply(&s, seat(0), Move::Pass, None),
    );
    assert_ne!(err.code, RuleCode::BiddingClosed);
    let got: Vec<String> = err.legal.iter().map(|m| m.id.clone()).collect();
    assert_eq!(got, legal_move_ids(&s, seat(0)));
    assert_eq!(err.legal.len(), 6);
    assert_eq!(s, before);
}

#[test]
fn accepts_the_forced_call_and_speaks_the_stuck_line() {
    let s = stuck("std-c");
    let down = s.hand.turned_down_suit.unwrap();
    let mate = mate_of(down);
    // Even the colour mate is "I guess" from a stuck dealer, never "Next."
    assert!(
        say_for_bid(
            &s,
            seat(0),
            &Bid::Call {
                suit: mate,
                alone: false
            }
        )
        .ends_with(", I guess.")
    );

    let suit = *SUITS.iter().find(|&&x| x != down).unwrap();
    let res = ok(&s, seat(0), call(suit, false));
    assert_eq!(res.state.hand.phase, GamePhase::TrickPlay);
    assert_eq!(res.state.hand.trump, Some(suit));
    assert_eq!(res.state.hand.maker_seat, Some(seat(0)));
    assert_eq!(res.state.hand.turn_seat, Some(eldest_of(seat(0))));
    assert_eq!(
        res.state.hand.bids[7].say,
        say_for_bid(&s, seat(0), &Bid::Call { suit, alone: false })
    );
}

#[test]
fn with_stick_the_dealer_off_the_dealer_may_pass_and_the_hand_is_thrown_in() {
    let s = passes(
        &after_cut(&opened("std-d", Some(seat(0)), cfg_stick(false)), false),
        7,
    );
    assert!(!is_stuck_dealer(&s, seat(0)));
    assert_eq!(legal_move_ids(&s, seat(0)).len(), 7);
    assert!(legal_move_ids(&s, seat(0)).contains(&"pass".to_owned()));

    let res = ok(&s, seat(0), Move::Pass);
    let t = &res.state;
    assert_eq!(step_tags(&res.steps), ["bid", "throwIn"]);
    assert_eq!(t.score, [0, 0]);
    assert_eq!(t.hand_no, 1);
    assert_eq!(t.hand.hand_no, 1);
    assert_eq!(t.hand.dealer_seat, next_seat(seat(0)));
    assert_eq!(t.misdeal_streak, 1);
    assert_eq!(t.hand.phase, GamePhase::Cutting);
    assert_eq!(t.hand.turn_seat, Some(cutter_of(next_seat(seat(0)))));
    assert_eq!(t.hand.deck_order, shuffle_for_hand("std-d", 1));
    assert!(t.hand.bids.is_empty());
    assert_eq!(t.hand.trump, None);
    assert_eq!(t.hand.up_card, None);
    assert_eq!(t.winner_team, None);
    assert_eq!(t.status, GameStatus::Active);
}

#[test]
fn caps_consecutive_throw_ins_at_misdeal_limit_while_still_moving_the_deal_left() {
    let cfg = Some(EngineConfigOverrides {
        stick_the_dealer: Some(false),
        misdeal_limit: Some(3),
        ..EngineConfigOverrides::default()
    });
    let mut s = opened("std-e", Some(seat(0)), cfg);
    let mut streaks = Vec::new();
    let mut dealers = Vec::new();
    for _ in 0..5 {
        s = passes(&after_cut(&s, false), 8);
        streaks.push(s.misdeal_streak);
        dealers.push(s.hand.dealer_seat.get());
    }
    assert_eq!(streaks, [1, 2, 3, 3, 3]);
    assert_eq!(dealers, [1, 2, 3, 0, 1]);
    assert_eq!(s.hand_no, 5);
    assert_eq!(s.score, [0, 0]);
}

/* -------------------------------------------------------------------------- */
/* trick_play                                                                  */
/* -------------------------------------------------------------------------- */

/// Hearts trump, seat 1 has led the ace of hearts, seat 2 holds the left bower
/// `JD` and four clubs.
fn bower_fixture() -> GameState {
    let base = after_cut(&opened("bower", Some(seat(0)), None), false);
    patch_hand(&base, |h| {
        h.phase = GamePhase::TrickPlay;
        h.turn_seat = Some(seat(2));
        h.trump = Some(Suit::Hearts);
        h.maker_seat = Some(seat(1));
        h.alone_seat = None;
        h.sitting_seat = None;
        h.hands = [
            cards(&["9S", "TS", "QS", "KS", "AS"]),
            cards(&["9H", "TH", "QH", "KH"]),
            cards(&["JD", "9C", "TC", "KC", "AC"]),
            cards(&["9D", "TD", "QD", "KD", "AD"]),
        ];
        h.trick = Trick {
            index: 0,
            led_suit: Some(Suit::Hearts),
            plays: vec![euchre_engine::Play {
                seat: seat(1),
                card: c("AH"),
            }],
            winner_seat: None,
        };
        h.trick_log = Vec::new();
        h.tricks_won = [0, 0];
    })
}

fn play(card: &str) -> Move {
    Move::Play { card: c(card) }
}

#[test]
fn makes_the_left_bower_the_only_legal_follow_to_a_trump_lead() {
    let s = bower_fixture();
    assert_eq!(effective_suit(c("JD"), Some(Suit::Hearts)), Suit::Hearts);
    assert_eq!(legal_move_ids(&s, seat(2)), ids(&["play:JD"]));
    assert_eq!(legal_moves(&s, seat(2))[0].label, "Jack of diamonds");
}

#[test]
fn rejects_a_renege_with_must_follow_suit_and_returns_the_real_legal_set() {
    let s = bower_fixture();
    let before = s.clone();
    let err = expect_rule(
        RuleCode::MustFollowSuit,
        apply(&s, seat(2), play("9C"), None),
    );
    let got: Vec<String> = err.legal.into_iter().map(|m| m.id).collect();
    assert_eq!(got, ids(&["play:JD"]));
    assert_eq!(s, before);
}

#[test]
fn rejects_a_card_the_seat_does_not_hold_before_it_checks_follow_suit() {
    let s = bower_fixture();
    expect_rule(
        RuleCode::CardNotInHand,
        apply(&s, seat(2), play("AS"), None),
    );
    expect_rule(
        RuleCode::CardNotInHand,
        apply(&s, seat(2), play("AH"), None),
    );
}

#[test]
fn rejects_bids_as_bidding_closed_and_other_kinds_as_wrong_phase() {
    let s = bower_fixture();
    expect_rule(
        RuleCode::BiddingClosed,
        apply(&s, seat(2), Move::Pass, None),
    );
    expect_rule(
        RuleCode::BiddingClosed,
        apply(&s, seat(2), call(Suit::Spades, false), None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(2), Move::Discard { card: c("JD") }, None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(2), Move::Cut { cut: true }, None),
    );
}

#[test]
fn resolves_the_trick_to_the_left_bower_and_parks_in_trick_resolve() {
    let mut s = bower_fixture();
    s = ok(&s, seat(2), play("JD")).state;
    assert_eq!(s.hand.turn_seat, Some(seat(3)));
    assert_eq!(s.hand.trick.plays.len(), 2);
    assert_eq!(s.hand.trick.led_suit, Some(Suit::Hearts));
    assert_eq!(s.hand.hands[2], cards(&["9C", "TC", "KC", "AC"]));

    // Seat 3 is void in hearts (no jack of clubs), so anything goes.
    assert_eq!(legal_move_ids(&s, seat(3)).len(), 5);
    s = ok(&s, seat(3), play("AD")).state;
    s = ok(&s, seat(0), play("AS")).state;

    assert_eq!(s.hand.phase, GamePhase::TrickResolve);
    assert_eq!(s.hand.turn_seat, None);
    assert_eq!(s.hand.trick.winner_seat, Some(seat(2)));
    let played: Vec<Card> = s.hand.trick.plays.iter().map(|p| p.card).collect();
    assert_eq!(played, cards(&["AH", "JD", "AD", "AS"]));
    assert_eq!(s.hand.trick_log.len(), 1);
    assert_eq!(s.hand.tricks_won, [1, 0]);
    assert!(legal_moves(&s, seat(2)).is_empty());
}

#[test]
fn emits_trick_won_with_seals_euchre_only_when_the_defenders_reach_three() {
    let mut s = bower_fixture();
    s = ok(&s, seat(2), play("JD")).state;
    s = ok(&s, seat(3), play("AD")).state;
    let res = ok(&s, seat(0), play("AS"));
    assert_eq!(step_tags(&res.steps), ["cardPlayed", "trickWon"]);
    assert_eq!(
        res.steps[1],
        Step::TrickWon {
            index: 0,
            winner_seat: seat(2),
            plays: res.state.hand.trick.plays.clone(),
            seals_euchre: false,
        }
    );

    // Same trick, but the defenders already hold two: this one seals the euchre.
    let primed = patch_hand(&s, |h| h.tricks_won = [2, 0]);
    let sealed = ok(&primed, seat(0), play("AS"));
    let Step::TrickWon { seals_euchre, .. } = &sealed.steps[1] else {
        panic!("expected trickWon")
    };
    assert_eq!(sealed.state.hand.tricks_won, [3, 0]);
    assert_eq!(team_of(sealed.state.hand.maker_seat.unwrap()), Team::EW);
    assert!(*seals_euchre);
}

#[test]
fn advance_releases_the_hold_and_gives_the_lead_to_the_trick_winner() {
    let mut s = bower_fixture();
    s = ok(&s, seat(2), play("JD")).state;
    s = ok(&s, seat(3), play("AD")).state;
    s = ok(&s, seat(0), play("AS")).state;

    let res = advance(&s);
    assert!(res.steps.is_empty());
    assert_eq!(res.state.hand.phase, GamePhase::TrickPlay);
    assert_eq!(res.state.hand.turn_seat, Some(seat(2)));
    assert_eq!(res.state.hand.trick, Trick::empty(1));
    assert_eq!(res.state.hand.trick_log.len(), 1);
    assert_eq!(
        legal_move_ids(&res.state, seat(2)),
        ids(&["play:9C", "play:TC", "play:KC", "play:AC"])
    );
}

/* -------------------------------------------------------------------------- */
/* advance() totality                                                          */
/* -------------------------------------------------------------------------- */

#[test]
fn is_the_identity_in_every_phase_that_has_a_seat_to_act() {
    let cutting = opened("adv-a", Some(seat(0)), None);
    let bidding = after_cut(&cutting, false);
    let discard = ok(&bidding, seat(1), Move::OrderUp { alone: false }).state;
    let round2 = passes(&bidding, 4);
    let play = ok(
        &discard,
        seat(0),
        Move::Discard {
            card: discard.hand.hands[0][0],
        },
    )
    .state;

    for s in [cutting, bidding, discard, round2, play] {
        let res = advance(&s);
        // TS asserts reference identity; Rust asserts value identity.
        assert_eq!(res.state, s);
        assert!(res.steps.is_empty());
    }
}

#[test]
fn is_the_identity_once_the_match_is_complete() {
    let fin = run_game(
        new_game("adv-b", cfg_game_to(5)),
        random_picker("adv-b"),
        RunHooks::default(),
    );
    assert_eq!(fin.status, GameStatus::Complete);
    assert_eq!(fin.hand.phase, GamePhase::GameOver);
    let res = advance(&fin);
    assert_eq!(res.state, fin);
    assert!(res.steps.is_empty());
}

#[test]
fn scores_the_hand_after_the_fifth_trick_then_opens_the_next_one() {
    let mut s = ok(
        &after_cut(&opened("adv-score", Some(seat(0)), None), false),
        seat(1),
        Move::OrderUp { alone: false },
    )
    .state;
    s = ok(
        &s,
        seat(0),
        Move::Discard {
            card: s.hand.hands[0][0],
        },
    )
    .state;
    let mut pick = random_picker("adv-score");
    let mut releases: Vec<(u8, GamePhase, Vec<&'static str>)> = Vec::new();
    while s.hand.phase != GamePhase::HandScore {
        match s.hand.turn_seat {
            None => {
                assert_eq!(s.hand.phase, GamePhase::TrickResolve);
                assert!(s.hand.trick.winner_seat.is_some());
                let res = advance(&s);
                releases.push((
                    s.hand.trick.index,
                    res.state.hand.phase,
                    step_tags(&res.steps),
                ));
                s = res.state;
            }
            Some(t) => {
                let m = pick(&s, t, &legal_moves(&s, t));
                s = ok(&s, t, m.mv).state;
            }
        }
    }
    // Four resolves hand the lead to the winner; the fifth scores the hand.
    let froms: Vec<u8> = releases.iter().map(|r| r.0).collect();
    assert_eq!(froms, [0, 1, 2, 3, 4]);
    let tos: Vec<GamePhase> = releases.iter().map(|r| r.1).collect();
    assert_eq!(
        tos,
        [
            GamePhase::TrickPlay,
            GamePhase::TrickPlay,
            GamePhase::TrickPlay,
            GamePhase::TrickPlay,
            GamePhase::HandScore
        ]
    );
    assert!(releases[..4].iter().all(|r| r.2.is_empty()));
    assert_eq!(releases[4].2, ["handScored"]);

    // hand_score: the recap is parked, the fifth trick is still in `trick`.
    assert_eq!(s.hand.turn_seat, None);
    assert_eq!(s.hand.trick_log.len(), 5);
    assert_eq!(s.hand.trick, s.hand.trick_log[4]);
    assert!(s.hand.result.is_some());
    assert!(s.hand.delta.is_some());
    let delta = s.hand.delta.unwrap();
    assert_eq!(s.score, [delta[0], delta[1]]);
    assert_eq!(s.hand.tricks_won[0] + s.hand.tricks_won[1], 5);
    for seat in SEATS_ALL {
        assert!(legal_moves(&s, seat).is_empty());
    }

    // The recap releases into the next hand: deal left, streak cleared.
    let opened2 = advance(&s);
    assert!(opened2.steps.is_empty());
    let t = &opened2.state;
    assert_eq!(t.hand.phase, GamePhase::Cutting);
    assert_eq!(t.hand_no, 1);
    assert_eq!(t.hand.hand_no, 1);
    assert_eq!(t.hand.dealer_seat, next_seat(seat(0)));
    assert_eq!(t.hand.turn_seat, Some(cutter_of(next_seat(seat(0)))));
    assert_eq!(t.misdeal_streak, 0);
    assert_eq!(t.score, s.score);
    assert_eq!(t.hand.result, None);
    assert!(t.hand.trick_log.is_empty());
    assert_eq!(t.hand.trump, None);
    assert_eq!(t.hand.deck_order, shuffle_for_hand("adv-score", 1));
}

#[test]
fn emits_hand_scored_with_the_table_row_and_game_won_when_the_score_is_reached() {
    let mut scored: Option<Step> = None;
    let mut won: Option<Step> = None;
    let mut hand_score_seen = 0;
    let fin = run_game(
        new_game("adv-win", cfg_game_to(5)),
        random_picker("adv-win"),
        RunHooks::on_transition(|before, res, _| {
            for st in &res.steps {
                if let Step::HandScored {
                    delta,
                    score,
                    maker_seat,
                    alone_seat,
                    tricks_won,
                    ..
                } = st
                {
                    scored = Some(st.clone());
                    assert_eq!(*maker_seat, before.hand.maker_seat);
                    assert_eq!(*alone_seat, before.hand.alone_seat);
                    assert_eq!(*tricks_won, before.hand.tricks_won);
                    assert_eq!(*score, res.state.score);
                    assert_eq!(Some(*delta), res.state.hand.delta);
                    hand_score_seen += 1;
                }
                if matches!(st, Step::GameWon { .. }) {
                    won = Some(st.clone());
                }
            }
        }),
    );
    assert!(hand_score_seen >= 2);
    assert!(scored.is_some());
    let Some(Step::GameWon { team, score }) = won else {
        panic!("expected gameWon")
    };
    assert_eq!(Some(team), fin.winner_team);
    assert_eq!(score, fin.score);
    assert_eq!(fin.status, GameStatus::Complete);
    assert_eq!(fin.hand.phase, GamePhase::GameOver);
}

#[test]
fn re_deals_defensively_from_a_bare_deal_phase() {
    let cut = opened("adv-c", Some(seat(0)), None);
    let forced = patch_hand(&cut, |h| {
        h.phase = GamePhase::Deal;
        h.turn_seat = None;
        h.cut = Some(false);
    });
    let res = advance(&forced);
    assert_eq!(res.state.hand.phase, GamePhase::BidRound1);
    assert_eq!(res.state.hand.turn_seat, Some(eldest_of(seat(0))));
    assert_eq!(step_tags(&res.steps), ["dealt", "upCardTurned"]);
    assert_eq!(res.state.hand.up_card, Some(cut.hand.deck_order[20]));
}

/* -------------------------------------------------------------------------- */
/* Seat mismatch and rejection atomicity                                       */
/* -------------------------------------------------------------------------- */

#[test]
fn rejects_every_seat_that_is_not_turn_seat_in_every_phase_with_a_turn() {
    let mut states: Vec<GameState> = Vec::new();
    let cutting = opened("mismatch", Some(seat(0)), None);
    states.push(cutting.clone());
    let bidding = after_cut(&cutting, false);
    states.push(bidding.clone());
    let discard = ok(&bidding, seat(1), Move::OrderUp { alone: false }).state;
    states.push(discard.clone());
    states.push(passes(&bidding, 4));
    states.push(
        ok(
            &discard,
            seat(0),
            Move::Discard {
                card: discard.hand.hands[0][0],
            },
        )
        .state,
    );

    for s in &states {
        let on = turn(s);
        let before = s.clone();
        for seat in SEATS_ALL {
            if seat == on {
                continue;
            }
            // The legal move *for the seat on turn*, offered to the wrong seat.
            let mv = legal_moves(s, on)[0].mv;
            let err = expect_rule(RuleCode::NotYourTurn, apply(s, seat, mv, None));
            assert!(err.legal.is_empty());
        }
        assert_eq!(*s, before);
        // And the correct seat still works: nothing was half-applied.
        assert!(apply(s, on, legal_moves(s, on)[0].mv, None).is_ok());
    }
}

#[test]
fn rejects_a_stale_turn_nonce_without_touching_the_state() {
    let s = after_cut(&opened("stale", Some(seat(0)), None), false);
    let before = s.clone();
    let stale = |id: String| ApplyOptions { turn_id: Some(id) };
    expect_rule(
        RuleCode::StaleTurn,
        apply(
            &s,
            seat(1),
            Move::Pass,
            Some(&stale(format!("{}-not-current", s.turn_id))),
        ),
    );
    assert_eq!(s, before);
    // The current nonce is accepted, and the next state carries a fresh one.
    let next = apply(&s, seat(1), Move::Pass, Some(&stale(s.turn_id.clone())))
        .unwrap()
        .state;
    assert_eq!(next.hand.passes, 1);
    assert_ne!(next.turn_id, s.turn_id);
    expect_rule(
        RuleCode::StaleTurn,
        apply(&next, seat(2), Move::Pass, Some(&stale(s.turn_id.clone()))),
    );
}

#[test]
fn rejects_everything_once_the_match_is_complete() {
    let fin = run_game(
        new_game("done", cfg_game_to(5)),
        random_picker("done"),
        RunHooks::default(),
    );
    let before = fin.clone();
    for seat in SEATS_ALL {
        expect_rule(RuleCode::WrongPhase, apply(&fin, seat, Move::Pass, None));
        assert!(legal_moves(&fin, seat).is_empty());
    }
    assert_eq!(fin, before);
}

#[test]
fn leaves_the_state_byte_identical_after_a_whole_battery_of_rejections() {
    let s = passes(&after_cut(&opened("atomic", Some(seat(0)), None), false), 7);
    let before = json(&s);
    let down = s.hand.turned_down_suit.unwrap();
    expect_rule(
        RuleCode::DealerMustCall,
        apply(&s, seat(0), Move::Pass, None),
    );
    expect_rule(
        RuleCode::IllegalMove,
        apply(&s, seat(0), call(down, false), None),
    );
    expect_rule(
        RuleCode::WrongPhase,
        apply(&s, seat(0), Move::OrderUp { alone: false }, None),
    );
    expect_rule(
        RuleCode::NotYourTurn,
        apply(&s, seat(2), call(down, false), None),
    );
    expect_rule(
        RuleCode::StaleTurn,
        apply(
            &s,
            seat(0),
            Move::Pass,
            Some(&ApplyOptions {
                turn_id: Some("x".to_owned()),
            }),
        ),
    );
    assert_eq!(json(&s), before);
}

/* -------------------------------------------------------------------------- */
/* moveId                                                                      */
/* -------------------------------------------------------------------------- */

#[test]
fn round_trips_every_offered_move_over_twenty_whole_matches() {
    let mut checked = 0;
    let mut kinds: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    let mut mismatches: Vec<String> = Vec::new();
    for i in 0..20 {
        let seed = format!("moveid-{i}");
        run_game(
            new_game(&seed, None),
            random_picker(&seed),
            RunHooks::on_state(|s| {
                let Some(t) = s.hand.turn_seat else { return };
                for m in legal_moves(s, t) {
                    if move_id(&m.mv) != m.id {
                        mismatches.push(format!("{} !== {}", m.id, move_id(&m.mv)));
                    }
                    checked += 1;
                    kinds.insert(m.mv.tag());
                }
            }),
        );
    }
    assert!(mismatches.is_empty(), "{mismatches:?}");
    assert!(checked > 5000);
    let kinds: Vec<&str> = kinds.into_iter().collect();
    assert_eq!(kinds, ["call", "cut", "discard", "orderUp", "pass", "play"]);
}

#[test]
fn is_total_over_the_move_union() {
    assert_eq!(move_id(&Move::Pass), "pass");
    assert_eq!(move_id(&Move::OrderUp { alone: false }), "orderUp");
    assert_eq!(move_id(&Move::OrderUp { alone: true }), "orderUp+alone");
    assert_eq!(move_id(&call(Suit::Hearts, false)), "call:H");
    assert_eq!(move_id(&call(Suit::Hearts, true)), "call:H+alone");
    assert_eq!(move_id(&Move::Cut { cut: true }), "cut:yes");
    assert_eq!(move_id(&Move::Cut { cut: false }), "cut:no");
    assert_eq!(move_id(&Move::Discard { card: c("9C") }), "discard:9C");
    assert_eq!(move_id(&play("JS")), "play:JS");
}

/* -------------------------------------------------------------------------- */
/* V20 - determinism                                                           */
/* -------------------------------------------------------------------------- */

#[test]
fn the_same_seed_produces_an_identical_deal() {
    let a = new_game("same", None);
    let b = new_game("same", None);
    assert_eq!(a, b);
    assert_eq!(a.hand.deck_order, b.hand.deck_order);
    assert_eq!(a.internal_token, b.internal_token);

    let dealt_a = after_cut(&advance(&a).state, true);
    let dealt_b = after_cut(&advance(&b).state, true);
    assert_eq!(dealt_a.hand.hands, dealt_b.hand.hands);
    assert_eq!(dealt_a.hand.kitty, dealt_b.hand.kitty);
    assert_eq!(dealt_a.hand.up_card, dealt_b.hand.up_card);

    let other = new_game("different", None);
    assert_ne!(other.hand.deck_order, a.hand.deck_order);
}

#[test]
fn replays_an_identical_full_game_from_the_same_seed_and_move_sequence() {
    let seed = "replay-seed";
    let mut recorded: Vec<(Seat, Move)> = Vec::new();
    let mut trace_a: Vec<String> = Vec::new();

    let final_a = run_game(
        new_game(seed, None),
        random_picker(seed),
        RunHooks::on_state(|s| trace_a.push(json(s))),
    );

    // A second, independent run: same seed, a freshly rebuilt picker stream.
    let mut s = new_game(seed, None);
    let mut pick_a = random_picker(seed);
    while s.status == GameStatus::Active {
        let Some(seat) = s.hand.turn_seat else {
            s = advance(&s).state;
            continue;
        };
        let mv = pick_a(&s, seat, &legal_moves(&s, seat)).mv;
        recorded.push((seat, mv));
        s = ok(&s, seat, mv).state;
    }
    assert!(recorded.len() > 200);

    // Replay from the journal alone: { seed, moves[] }.
    let mut trace_b: Vec<String> = Vec::new();
    let mut t = new_game(seed, None);
    let mut i = 0;
    trace_b.push(json(&t));
    while t.status == GameStatus::Active {
        match t.hand.turn_seat {
            None => t = advance(&t).state,
            Some(seat) => {
                let (rec_seat, mv) = recorded[i];
                i += 1;
                assert_eq!(rec_seat, seat);
                assert!(legal_move_ids(&t, seat).contains(&move_id(&mv)));
                t = ok(&t, seat, mv).state;
            }
        }
        trace_b.push(json(&t));
    }

    assert_eq!(i, recorded.len());
    assert_eq!(trace_b, trace_a);
    assert_eq!(t, final_a);
    assert_eq!(t.status, GameStatus::Complete);
}

#[test]
fn mints_turn_nonces_from_a_hash_chain_so_every_state_is_reproducible() {
    let a = run_game(
        new_game("nonce", cfg_game_to(5)),
        random_picker("nonce"),
        RunHooks::default(),
    );
    let b = run_game(
        new_game("nonce", cfg_game_to(5)),
        random_picker("nonce"),
        RunHooks::default(),
    );
    assert_eq!(a.turn_id, b.turn_id);
    assert!(
        !a.turn_id.is_empty()
            && a.turn_id
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
    );
    assert_eq!(a, b);
}

/* -------------------------------------------------------------------------- */
/* V21 - purity                                                                */
/* -------------------------------------------------------------------------- */

#[test]
fn never_mutates_a_deep_frozen_state_across_a_whole_match() {
    // `deepFreeze` has no Rust equivalent: the reducer borrows `&GameState`, so
    // writing through the input is impossible by type. The observable half of
    // the TS test is kept: `on_state` snapshots the JSON of the live state
    // *before* it is handed to `apply`/`advance`, and `on_transition` (which
    // runs after that call, with the same input state) re-serialises that very
    // input and compares it with the snapshot.
    use std::cell::{Cell, RefCell};
    let states = Cell::new(0usize);
    let checked = Cell::new(0usize);
    let snapshot: RefCell<Option<String>> = RefCell::new(None);
    let fin = run_game(
        new_game("frozen", None),
        random_picker("frozen"),
        RunHooks {
            on_state: Some(Box::new(|s| {
                *snapshot.borrow_mut() = Some(json(s));
                states.set(states.get() + 1);
            })),
            on_transition: Some(Box::new(|input, res, _| {
                let before = snapshot
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| panic!("no snapshot of the input state"));
                assert_eq!(json(input), before, "the input state changed");
                // The result shares nothing with the input: it is a new state.
                assert_ne!(json(&res.state), before);
                checked.set(checked.get() + 1);
            })),
            budget: None,
        },
    );
    assert!(states.get() > 200);
    assert_eq!(checked.get(), states.get() - 1);
    assert_eq!(fin.status, GameStatus::Complete);
}

#[test]
fn legal_moves_and_project_hand_back_fresh_arrays_every_call() {
    let s = after_cut(&opened("fresh", Some(seat(0)), None), false);
    let mut a = legal_moves(&s, seat(1));
    let b = legal_moves(&s, seat(1));
    assert_eq!(a, b);
    // TS `not.toBe`: owned values are independent; mutating one leaves the other.
    a.clear();
    assert_eq!(b.len(), 3);

    let mut va = project(&s, seat(1));
    let vb = project(&s, seat(1));
    assert_eq!(va, vb);
    va.hand.clear();
    va.trick_log.push(Trick::empty(9));
    assert_eq!(vb.hand, s.hand.hands[1]);
    assert_eq!(vb.trick_log, s.hand.trick_log);
}
