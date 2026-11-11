//! Port of `__tests__/legal.test.ts`: follow-suit legality and the move-level API.
//!
//! Like the TS suite, every `GameState` fixture is built by hand rather than
//! driven through the reducer, so a `legal.rs` regression fails here.

mod common;

use common::{ALL_SUITS, c, cards, s, seat};
use euchre_engine::{
    Card, DECK, DEFAULT_ENGINE_CONFIG, GamePhase, GameState, GameStatus, HandState, LegalMove,
    Move, Play, RANK_NAME, RANKS, SUIT_NAME, SUIT_NOUN, SUITS, SchemaV1, Seat, Suit, Trick,
    WhyIllegalParams, card_name, card_name_lower, effective_suit, find_legal_move, get_legal_plays,
    is_stuck_dealer, legal_move_ids, legal_moves, order_label, partner_of, rank_name, suit_name,
    suit_noun, why_illegal,
};

const ALL_SEATS: [Seat; 4] = Seat::ALL;
/// `None` means "on lead".
const LEDS: [Option<Suit>; 5] = [
    None,
    Some(Suit::Spades),
    Some(Suit::Hearts),
    Some(Suit::Diamonds),
    Some(Suit::Clubs),
];

/* Fixtures */

/// The TS `makeState()` with no overrides; tests mutate the returned value.
fn make_state() -> GameState {
    GameState {
        schema: SchemaV1,
        v: 1,
        game_id: "g-test".to_owned(),
        internal_token: "tok".to_owned(),
        seed: "seed".to_owned(),
        cfg: DEFAULT_ENGINE_CONFIG,
        first_dealer: Seat::SOUTH,
        status: GameStatus::Active,
        score: [0, 0],
        hand_no: 0,
        misdeal_streak: 0,
        winner_team: None,
        hand: HandState {
            hand_no: 0,
            phase: GamePhase::TrickPlay,
            dealer_seat: Seat::SOUTH,
            turn_seat: Some(Seat::SOUTH),
            deck_order: DECK.to_vec(),
            cut: Some(false),
            hands: Default::default(),
            kitty: [c("9S"), c("TS"), c("JS"), c("QS")],
            up_card: Some(c("9S")),
            up_card_turned_down: false,
            turned_down_suit: None,
            trump: None,
            maker_seat: None,
            alone_seat: None,
            sitting_seat: None,
            dealer_discard: None,
            passes: 0,
            bids: Vec::new(),
            trick: Trick::empty(0),
            trick_log: Vec::new(),
            tricks_won: [0, 0],
            result: None,
            delta: None,
        },
        turn_id: "t0".to_owned(),
        turn_deadline_at: None,
        timer_id: None,
        watchdog_id: None,
        reveal_id: None,
        pending: None,
        recent_move_ids: Vec::new(),
        applied_seq: 0,
    }
}

/// `makeState` with a hand-level override closure.
fn state_with(f: impl FnOnce(&mut HandState)) -> GameState {
    let mut st = make_state();
    f(&mut st.hand);
    st
}

/// A small deterministic PRNG (the TS `lcg`).
fn lcg(seed: u32) -> impl FnMut() -> f64 {
    let mut st = seed;
    move || {
        st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f64::from(st) / 4_294_967_296.0
    }
}

/// The TS `randomHand`: a Fisher-Yates of the deck, first `size` cards.
fn random_hand(rnd: &mut impl FnMut() -> f64, size: usize) -> Vec<Card> {
    let mut pool = DECK.to_vec();
    for i in (1..pool.len()).rev() {
        let j = (rnd() * (i + 1) as f64).floor() as usize;
        pool.swap(i, j);
    }
    pool.truncate(size);
    pool
}

fn lm(mv: Move, label: &str) -> LegalMove {
    LegalMove {
        id: mv.id(),
        mv,
        label: label.to_owned(),
    }
}

fn ids(moves: &[LegalMove]) -> Vec<&str> {
    moves.iter().map(|m| m.id.as_str()).collect()
}

fn why(hand: &[&str], card: &str, led: Option<char>, trump: Option<char>) -> Option<String> {
    why_illegal(WhyIllegalParams {
        hand: &cards(hand),
        card: c(card),
        led_suit: led.map(s),
        trump: trump.map(s),
    })
}

fn glp(hand: &[&str], led: Option<char>, trump: Option<char>) -> Vec<Card> {
    get_legal_plays(&cards(hand), led.map(s), trump.map(s))
}

const VOID_HAND: [&str; 5] = ["JD", "9C", "TC", "KC", "AC"];

/* getLegalPlays */

#[test]
fn case_8_jd_9c_tc_kc_ac_with_hearts_trump_is_void_in_diamonds_all_five_legal() {
    assert_eq!(
        glp(&VOID_HAND, Some('D'), Some('H')),
        cards(&["JD", "9C", "TC", "KC", "AC"])
    );
}

#[test]
fn case_9_to_a_heart_lead_that_same_hand_has_exactly_one_legal_play_jd() {
    assert_eq!(glp(&VOID_HAND, Some('H'), Some('H')), cards(&["JD"]));
}

#[test]
fn case_10_on_lead_everything_is_legal() {
    assert_eq!(
        glp(&VOID_HAND, None, Some('H')),
        cards(&["JD", "9C", "TC", "KC", "AC"])
    );
}

#[test]
fn to_a_club_lead_that_hand_must_follow_with_its_four_clubs() {
    assert_eq!(
        glp(&VOID_HAND, Some('C'), Some('H')),
        cards(&["9C", "TC", "KC", "AC"])
    );
}

#[test]
fn case_11_as_ks_9h_td_qc_with_spades_trump_spades_led_as_ks() {
    assert_eq!(
        glp(&["AS", "KS", "9H", "TD", "QC"], Some('S'), Some('S')),
        cards(&["AS", "KS"])
    );
}

#[test]
fn case_12_same_hand_with_clubs_trump_one_legal_card_per_lead() {
    let hand = ["AS", "KS", "9H", "TD", "QC"];
    assert_eq!(glp(&hand, Some('C'), Some('C')), cards(&["QC"]));
    assert_eq!(glp(&hand, Some('H'), Some('C')), cards(&["9H"]));
    assert_eq!(glp(&hand, Some('D'), Some('C')), cards(&["TD"]));
    assert_eq!(glp(&hand, Some('S'), Some('C')), cards(&["AS", "KS"]));
}

#[test]
fn case_13_spades_trump_spades_led_hand_jc_9d_jc_is_the_only_legal_play() {
    assert_eq!(glp(&["JC", "9D"], Some('S'), Some('S')), cards(&["JC"]));
}

#[test]
fn case_14_spades_trump_clubs_led_hand_jc_ah_void_in_clubs_both_legal() {
    assert_eq!(
        glp(&["JC", "AH"], Some('C'), Some('S')),
        cards(&["JC", "AH"])
    );
}

#[test]
fn is_measured_by_effective_suit_never_suit_of_jd_follows_hearts_not_diamonds() {
    assert_eq!(glp(&["JD", "AD"], Some('H'), Some('H')), cards(&["JD"]));
    assert_eq!(glp(&["JD", "AD"], Some('D'), Some('H')), cards(&["AD"]));
}

#[test]
fn there_is_no_obligation_to_trump_and_none_to_overtrump() {
    assert_eq!(
        glp(&["AH", "9C", "AD"], Some('S'), Some('H')),
        cards(&["AH", "9C", "AD"])
    );
}

#[test]
fn preserves_hand_order_in_the_returned_subset() {
    assert_eq!(
        glp(&["9S", "AS", "KS", "TH"], Some('S'), Some('H')),
        cards(&["9S", "AS", "KS"])
    );
}

#[test]
fn returns_a_fresh_array_and_never_mutates_the_hand() {
    // TS also checks `not.toBe(hand)`; a returned `Vec` is always a distinct allocation.
    let hand = cards(&VOID_HAND);
    let snapshot = hand.clone();
    let mut on_lead = get_legal_plays(&hand, None, Some(s('H')));
    on_lead.push(c("AS"));
    let mut void_case = get_legal_plays(&hand, Some(s('D')), Some(s('H')));
    void_case.clear();
    assert_eq!(hand, snapshot);
}

#[test]
fn works_before_trump_exists_on_printed_suits() {
    assert_eq!(
        glp(&["JD", "AD", "AH"], Some('D'), None),
        cards(&["JD", "AD"])
    );
    assert_eq!(glp(&["JD", "AD", "AH"], Some('H'), None), cards(&["AH"]));
}

#[test]
fn returns_empty_only_for_an_empty_hand() {
    assert_eq!(glp(&[], Some('S'), Some('H')), Vec::<Card>::new());
    assert_eq!(glp(&[], None, Some('H')), Vec::<Card>::new());
}

#[test]
fn case_19_never_empty_always_a_subset_and_never_admits_a_renege_v7_v19() {
    let mut rnd = lcg(0x00c0_ffee);
    let mut checks = 0u32;
    for _ in 0..2000 {
        let size = 1 + (rnd() * 5.0).floor() as usize;
        let hand = random_hand(&mut rnd, size);
        for trump in ALL_SUITS {
            for led in LEDS {
                let legal = get_legal_plays(&hand, led, Some(trump));
                checks += 1;
                assert_eq!(
                    (&hand, led, trump, !legal.is_empty()),
                    (&hand, led, trump, true)
                );
                assert_eq!(
                    (&hand, led, trump, legal.iter().all(|x| hand.contains(x))),
                    (&hand, led, trump, true)
                );
                let mut uniq = legal.clone();
                uniq.sort();
                uniq.dedup();
                assert_eq!(uniq.len(), legal.len());

                let Some(led) = led else {
                    assert_eq!((&hand, trump, &legal), (&hand, trump, &hand));
                    continue;
                };
                let following: Vec<Card> = hand
                    .iter()
                    .copied()
                    .filter(|&x| effective_suit(x, Some(trump)) == led)
                    .collect();
                if following.is_empty() {
                    assert_eq!((&hand, led, trump, &legal), (&hand, led, trump, &hand));
                } else {
                    assert_eq!((&hand, led, trump, &legal), (&hand, led, trump, &following));
                }
            }
        }
    }
    assert_eq!(checks, 2000 * 4 * 5);
}

/* whyIllegal */

#[test]
fn why_illegal_reproduces_the_spec_example_verbatim() {
    assert_eq!(
        why(&["JD", "9C"], "9C", Some('H'), Some('H')).as_deref(),
        Some(
            "Hearts were led. Your Jack of diamonds is a heart right now \u{2014} you have to follow."
        )
    );
}

#[test]
fn why_illegal_is_null_for_a_legal_play() {
    assert_eq!(why(&["JD", "9C"], "JD", Some('H'), Some('H')), None);
    assert_eq!(why(&["JD", "9C"], "9C", None, Some('H')), None);
    assert_eq!(why(&["9C", "TC"], "9C", Some('H'), Some('H')), None);
}

#[test]
fn why_illegal_rejects_a_card_that_is_not_in_the_hand() {
    assert_eq!(
        why(&["JD", "9C"], "AS", Some('H'), Some('H')).as_deref(),
        Some("That card isn't in your hand.")
    );
    assert_eq!(
        why(&["JD", "9C"], "AS", None, Some('H')).as_deref(),
        Some("That card isn't in your hand.")
    );
}

#[test]
fn lists_the_cards_the_seat_still_holds_joined_with_a_comma_and_and() {
    assert_eq!(
        why(&["AS", "KS", "TH"], "TH", Some('S'), Some('H')).as_deref(),
        Some(
            "Spades were led. You still hold the ace of spades and the king of spades \u{2014} you have to follow suit."
        )
    );
    assert_eq!(
        why(&["AS", "KS", "QS", "TH"], "TH", Some('S'), Some('H')).as_deref(),
        Some(
            "Spades were led. You still hold the ace of spades, the king of spades and the queen of spades \u{2014} you have to follow suit."
        )
    );
}

#[test]
fn uses_the_singular_form_for_a_single_held_card() {
    assert_eq!(
        why(&["AS", "TH"], "TH", Some('S'), Some('H')).as_deref(),
        Some("Spades were led. You still hold the ace of spades \u{2014} you have to follow suit.")
    );
}

#[test]
fn prefers_the_bower_explanation_over_the_list_when_the_left_bower_is_held() {
    assert_eq!(
        why(&["JC", "AS", "9H"], "9H", Some('S'), Some('S')).as_deref(),
        Some(
            "Spades were led. Your Jack of clubs is a spade right now \u{2014} you have to follow."
        )
    );
}

#[test]
fn why_illegal_agrees_with_get_legal_plays_on_every_card_over_every_lead_and_trump() {
    let hand = cards(&["JD", "AD", "AS", "9C", "TH"]);
    for trump in ALL_SUITS {
        for led in LEDS {
            let legal = get_legal_plays(&hand, led, Some(trump));
            for &x in &hand {
                let w = why_illegal(WhyIllegalParams {
                    hand: &hand,
                    card: x,
                    led_suit: led,
                    trump: Some(trump),
                });
                assert_eq!(
                    (x, led, trump, w.is_none()),
                    (x, led, trump, legal.contains(&x))
                );
            }
        }
    }
}

/* naming */

#[test]
fn suit_name_suit_noun_rank_name_are_exact() {
    // The TS `Object.isFrozen(SUIT_NAME)` half has no Rust equivalent: these are consts.
    let by_suit = |f: fn(Suit) -> &'static str| -> Vec<(char, &'static str)> {
        SUITS.iter().map(|&x| (x.as_char(), f(x))).collect()
    };
    assert_eq!(
        by_suit(suit_name),
        vec![
            ('S', "Spades"),
            ('H', "Hearts"),
            ('D', "Diamonds"),
            ('C', "Clubs")
        ]
    );
    assert_eq!(
        by_suit(suit_noun),
        vec![
            ('S', "spade"),
            ('H', "heart"),
            ('D', "diamond"),
            ('C', "club")
        ]
    );
    let ranks: Vec<(char, &str)> = RANKS.iter().map(|&r| (r.as_char(), rank_name(r))).collect();
    assert_eq!(
        ranks,
        vec![
            ('9', "Nine"),
            ('T', "Ten"),
            ('J', "Jack"),
            ('Q', "Queen"),
            ('K', "King"),
            ('A', "Ace")
        ]
    );
    assert_eq!(SUIT_NAME.len(), 4);
    assert_eq!(SUIT_NOUN.len(), 4);
    assert_eq!(RANK_NAME.len(), 6);
}

#[test]
fn card_name_names_the_printed_card_never_the_effective_one() {
    assert_eq!(card_name(c("JD")), "Jack of diamonds");
    assert_eq!(card_name(c("9C")), "Nine of clubs");
    assert_eq!(card_name(c("AS")), "Ace of spades");
    assert_eq!(card_name_lower(c("JD")), "jack of diamonds");
}

#[test]
fn names_all_24_cards_distinctly() {
    let mut names: Vec<String> = DECK.iter().map(|&x| card_name(x)).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 24);
}

#[test]
fn order_label_is_i_take_it_for_the_dealer_i_assist_for_their_partner_else_order_it_up() {
    for dealer in ALL_SEATS {
        assert_eq!((dealer, order_label(dealer, dealer)), (dealer, "I take it"));
        assert_eq!(
            (dealer, order_label(partner_of(dealer), dealer)),
            (dealer, "I assist")
        );
        for st in ALL_SEATS {
            if st == dealer || st == partner_of(dealer) {
                continue;
            }
            assert_eq!(
                (dealer, st, order_label(st, dealer)),
                (dealer, st, "Order it up")
            );
        }
    }
}

/* isStuckDealer (V3) */

fn stuck_state(f: impl FnOnce(&mut GameState)) -> GameState {
    let mut st = state_with(|h| {
        h.phase = GamePhase::BidRound2;
        h.dealer_seat = seat(0);
        h.turn_seat = Some(seat(0));
        h.passes = 3;
    });
    f(&mut st);
    st
}

#[test]
fn holds_only_for_the_dealer_in_bid_round_2_at_passes_3_with_the_flag_on() {
    assert!(is_stuck_dealer(&stuck_state(|_| {}), seat(0)));
}

#[test]
fn is_stuck_dealer_is_false_for_any_seat_that_is_not_the_dealer() {
    for n in [1, 2, 3] {
        assert!(!is_stuck_dealer(&stuck_state(|_| {}), seat(n)));
    }
}

#[test]
fn is_stuck_dealer_is_false_at_fewer_than_three_passes() {
    for passes in [0u8, 1, 2] {
        let st = stuck_state(|g| g.hand.passes = passes);
        assert_eq!((passes, is_stuck_dealer(&st, seat(0))), (passes, false));
    }
}

#[test]
fn is_stuck_dealer_is_false_in_round_1_the_dealer_may_always_pass_the_up_card() {
    assert!(!is_stuck_dealer(
        &stuck_state(|g| g.hand.phase = GamePhase::BidRound1),
        seat(0)
    ));
}

#[test]
fn is_stuck_dealer_is_false_with_stick_the_dealer_off() {
    assert!(!is_stuck_dealer(
        &stuck_state(|g| g.cfg.stick_the_dealer = false),
        seat(0)
    ));
}

/* legalMoves: guards */

#[test]
fn legal_moves_is_empty_for_any_seat_that_is_not_to_act() {
    let st = state_with(|h| {
        h.phase = GamePhase::BidRound1;
        h.turn_seat = Some(seat(1));
        h.dealer_seat = seat(0);
    });
    assert_eq!(legal_moves(&st, seat(0)), vec![]);
    assert_eq!(legal_moves(&st, seat(2)), vec![]);
    assert_eq!(legal_moves(&st, seat(3)), vec![]);
    assert!(!legal_moves(&st, seat(1)).is_empty());
}

#[test]
fn legal_moves_is_empty_unless_status_is_active() {
    for status in [GameStatus::Complete, GameStatus::Abandoned] {
        let mut st = state_with(|h| {
            h.phase = GamePhase::BidRound1;
            h.turn_seat = Some(seat(1));
        });
        st.status = status;
        assert_eq!((status, legal_moves(&st, seat(1))), (status, vec![]));
    }
}

#[test]
fn case_18_is_empty_for_the_sitting_seat_in_every_phase_even_on_its_turn() {
    for &phase in GamePhase::ALL {
        let st = state_with(|h| {
            h.phase = phase;
            h.turn_seat = Some(seat(3));
            h.sitting_seat = Some(seat(3));
            h.alone_seat = Some(seat(1));
            h.maker_seat = Some(seat(1));
            h.trump = Some(s('H'));
            h.hands = [
                cards(&["AS"]),
                cards(&["KS"]),
                cards(&["QS"]),
                cards(&["9S", "TS", "JS", "QH", "KH"]),
            ];
        });
        assert_eq!((phase, legal_moves(&st, seat(3))), (phase, vec![]));
    }
    assert_eq!(GamePhase::ALL.len(), 10);
}

#[test]
fn returns_empty_in_every_engine_only_phase() {
    for phase in [
        GamePhase::Lobby,
        GamePhase::Deal,
        GamePhase::TrickResolve,
        GamePhase::HandScore,
        GamePhase::GameOver,
    ] {
        let st = state_with(|h| {
            h.phase = phase;
            h.turn_seat = Some(seat(0));
        });
        assert_eq!((phase, legal_moves(&st, seat(0))), (phase, vec![]));
    }
}

/* legalMoves: cutting */

#[test]
fn offers_exactly_bump_and_run_em() {
    let st = state_with(|h| {
        h.phase = GamePhase::Cutting;
        h.turn_seat = Some(seat(3));
        h.dealer_seat = seat(0);
    });
    assert_eq!(
        legal_moves(&st, seat(3)),
        vec![
            lm(Move::Cut { cut: true }, "Bump"),
            lm(Move::Cut { cut: false }, "Run 'em")
        ]
    );
    assert_eq!(ids(&legal_moves(&st, seat(3))), vec!["cut:yes", "cut:no"]);
}

/* legalMoves: bid_round_1 */

fn round1(turn: u8) -> GameState {
    state_with(|h| {
        h.phase = GamePhase::BidRound1;
        h.turn_seat = Some(seat(turn));
        h.dealer_seat = seat(0);
    })
}

#[test]
fn offers_pass_order_up_order_up_alone_with_the_role_correct_label() {
    assert_eq!(
        legal_moves(&round1(2), seat(2)),
        vec![
            lm(Move::Pass, "Pass"),
            lm(Move::OrderUp { alone: false }, "I assist"),
            lm(Move::OrderUp { alone: true }, "I assist, alone"),
        ]
    );
    assert_eq!(
        ids(&legal_moves(&round1(2), seat(2))),
        vec!["pass", "orderUp", "orderUp+alone"]
    );
}

#[test]
fn labels_the_dealers_own_acceptance_i_take_it() {
    let labels: Vec<String> = legal_moves(&round1(0), seat(0))
        .into_iter()
        .map(|m| m.label)
        .collect();
    assert_eq!(labels, vec!["Pass", "I take it", "I take it, alone"]);
}

#[test]
fn gives_all_four_seats_the_same_three_move_ids() {
    for st in ALL_SEATS {
        let g = round1(st.get());
        assert_eq!(
            (st, legal_move_ids(&g, st)),
            (
                st,
                vec!["pass".to_owned(), "orderUp".into(), "orderUp+alone".into()]
            )
        );
    }
}

/* legalMoves: dealer_discard */

#[test]
fn offers_all_six_cards_including_the_up_card_itself() {
    let six = cards(&["9S", "TS", "JS", "QS", "KS", "AS"]);
    let st = state_with(|h| {
        h.phase = GamePhase::DealerDiscard;
        h.turn_seat = Some(seat(0));
        h.dealer_seat = seat(0);
        h.up_card = Some(c("AS"));
        h.trump = Some(s('S'));
        h.hands = [six, Vec::new(), Vec::new(), Vec::new()];
    });
    let moves = legal_moves(&st, seat(0));
    assert_eq!(moves.len(), 6);
    assert_eq!(
        ids(&moves),
        vec![
            "discard:9S",
            "discard:TS",
            "discard:JS",
            "discard:QS",
            "discard:KS",
            "discard:AS"
        ]
    );
    assert!(ids(&moves).contains(&"discard:AS"));
    assert_eq!(
        moves[0],
        lm(Move::Discard { card: c("9S") }, "Nine of spades")
    );
}

/* legalMoves: bid_round_2 */

fn round2(f: impl FnOnce(&mut GameState)) -> GameState {
    let mut st = state_with(|h| {
        h.phase = GamePhase::BidRound2;
        h.turn_seat = Some(seat(0));
        h.dealer_seat = seat(0);
        h.up_card = Some(c("9H"));
        h.up_card_turned_down = true;
        h.turned_down_suit = Some(s('H'));
        h.passes = 0;
    });
    f(&mut st);
    st
}

#[test]
fn case_15_7_entries_with_turned_down_suit_h_and_no_call_h_anywhere() {
    let moves = legal_moves(&round2(|g| g.hand.turn_seat = Some(seat(1))), seat(1));
    assert_eq!(moves.len(), 7);
    assert_eq!(
        ids(&moves),
        vec![
            "pass",
            "call:S",
            "call:S+alone",
            "call:D",
            "call:D+alone",
            "call:C",
            "call:C+alone"
        ]
    );
    assert!(!moves.iter().any(|m| m.id.starts_with("call:H")));
}

#[test]
fn labels_the_calls_by_suit_name_with_alone_on_the_loner_variant() {
    let moves = legal_moves(&round2(|g| g.hand.turn_seat = Some(seat(1))), seat(1));
    let labels: Vec<&str> = moves.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "Pass",
            "Spades",
            "Spades, alone",
            "Diamonds",
            "Diamonds, alone",
            "Clubs",
            "Clubs, alone"
        ]
    );
    assert_eq!(
        moves[1].mv,
        Move::Call {
            suit: s('S'),
            alone: false
        }
    );
    assert_eq!(
        moves[2].mv,
        Move::Call {
            suit: s('S'),
            alone: true
        }
    );
}

#[test]
fn excludes_the_turned_down_suit_whichever_suit_it_was() {
    for down in ALL_SUITS {
        let moves = legal_moves(
            &round2(|g| {
                g.hand.turn_seat = Some(seat(1));
                g.hand.turned_down_suit = Some(down);
            }),
            seat(1),
        );
        assert_eq!((down, moves.len()), (down, 7));
        let prefix = format!("call:{}", down.as_char());
        assert_eq!(
            (down, moves.iter().any(|m| m.id.starts_with(&prefix))),
            (down, false)
        );
    }
}

#[test]
fn case_16_the_stuck_dealer_gets_6_entries_and_no_pass() {
    let st = round2(|g| {
        g.hand.turn_seat = Some(seat(0));
        g.hand.dealer_seat = seat(0);
        g.hand.passes = 3;
        g.cfg.stick_the_dealer = true;
    });
    let moves = legal_moves(&st, seat(0));
    assert_eq!(moves.len(), 6);
    assert_eq!(
        ids(&moves),
        vec![
            "call:S",
            "call:S+alone",
            "call:D",
            "call:D+alone",
            "call:C",
            "call:C+alone"
        ]
    );
    assert!(!moves.iter().any(|m| m.id == "pass"));
    assert!(!moves.is_empty());
}

#[test]
fn case_17_with_stick_the_dealer_off_the_same_state_keeps_pass_at_7_entries() {
    let st = round2(|g| {
        g.hand.turn_seat = Some(seat(0));
        g.hand.dealer_seat = seat(0);
        g.hand.passes = 3;
        g.cfg.stick_the_dealer = false;
    });
    let moves = legal_moves(&st, seat(0));
    assert_eq!(moves.len(), 7);
    assert_eq!(moves[0], lm(Move::Pass, "Pass"));
}

#[test]
fn a_non_dealer_at_passes_3_still_gets_pass() {
    let st = round2(|g| {
        g.hand.turn_seat = Some(seat(2));
        g.hand.dealer_seat = seat(0);
        g.hand.passes = 3;
        g.cfg.stick_the_dealer = true;
    });
    assert!(legal_move_ids(&st, seat(2)).contains(&"pass".to_owned()));
    assert_eq!(legal_moves(&st, seat(2)).len(), 7);
}

#[test]
fn offers_all_four_suits_when_nothing_has_been_turned_down() {
    let st = round2(|g| {
        g.hand.turn_seat = Some(seat(1));
        g.hand.turned_down_suit = None;
    });
    assert_eq!(legal_moves(&st, seat(1)).len(), 9);
}

/* legalMoves: trick_play */

fn trick(led: Option<char>, plays: &[(u8, &str)]) -> Trick {
    Trick {
        index: 0,
        led_suit: led.map(s),
        plays: plays
            .iter()
            .map(|&(n, id)| Play {
                seat: seat(n),
                card: c(id),
            })
            .collect(),
        winner_seat: None,
    }
}

#[test]
fn wraps_get_legal_plays_and_prefixes_ids_with_play() {
    let st = state_with(|h| {
        h.phase = GamePhase::TrickPlay;
        h.turn_seat = Some(seat(1));
        h.trump = Some(s('H'));
        h.trick = trick(Some('H'), &[(0, "AH")]);
        h.hands = [Vec::new(), cards(&VOID_HAND), Vec::new(), Vec::new()];
    });
    assert_eq!(
        legal_moves(&st, seat(1)),
        vec![lm(Move::Play { card: c("JD") }, "Jack of diamonds")]
    );
    assert_eq!(ids(&legal_moves(&st, seat(1))), vec!["play:JD"]);
}

#[test]
fn offers_the_whole_hand_on_lead() {
    let st = state_with(|h| {
        h.phase = GamePhase::TrickPlay;
        h.turn_seat = Some(seat(1));
        h.trump = Some(s('H'));
        h.trick = Trick::empty(0);
        h.hands = [
            Vec::new(),
            cards(&["JD", "9C", "TC"]),
            Vec::new(),
            Vec::new(),
        ];
    });
    assert_eq!(
        legal_move_ids(&st, seat(1)),
        vec!["play:JD", "play:9C", "play:TC"]
    );
}

#[test]
fn offers_the_whole_hand_when_void_the_left_bower_does_not_follow_its_printed_suit() {
    let st = state_with(|h| {
        h.phase = GamePhase::TrickPlay;
        h.turn_seat = Some(seat(1));
        h.trump = Some(s('H'));
        h.trick = trick(Some('D'), &[(0, "AD")]);
        h.hands = [Vec::new(), cards(&VOID_HAND), Vec::new(), Vec::new()];
    });
    assert_eq!(
        legal_move_ids(&st, seat(1)),
        vec!["play:JD", "play:9C", "play:TC", "play:KC", "play:AC"]
    );
}

/* V19, V21 and the lookup helpers */

const ACTIONABLE: [GamePhase; 5] = [
    GamePhase::Cutting,
    GamePhase::BidRound1,
    GamePhase::DealerDiscard,
    GamePhase::BidRound2,
    GamePhase::TrickPlay,
];

#[test]
fn v19_holds_in_every_phase_that_has_an_acting_seat_across_all_four_seats() {
    let mut rnd = lcg(0x5eed);
    for phase in ACTIONABLE {
        for st in ALL_SEATS {
            for dealer in ALL_SEATS {
                for passes in [0u8, 1, 2, 3] {
                    for stick in [true, false] {
                        let hand = random_hand(
                            &mut rnd,
                            if phase == GamePhase::DealerDiscard {
                                6
                            } else {
                                5
                            },
                        );
                        let mut g = state_with(|h| {
                            h.phase = phase;
                            h.turn_seat = Some(st);
                            h.dealer_seat = dealer;
                            h.passes = passes;
                            h.trump = Some(s('H'));
                            h.turned_down_suit = (phase == GamePhase::BidRound2).then(|| s('H'));
                            h.trick = trick(Some('S'), &[(3, "AS")]);
                            h.hands = [hand.clone(), hand.clone(), hand.clone(), hand.clone()];
                        });
                        g.cfg.stick_the_dealer = stick;
                        let n = legal_moves(&g, st).len();
                        assert_eq!(
                            (phase, st, dealer, passes, stick, n > 0),
                            (phase, st, dealer, passes, stick, true)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn v21_does_not_mutate_the_state_in_any_actionable_phase() {
    // TS deep-freezes the state; Rust takes `&GameState`, so mutation is a compile
    // error. Kept as a clone-and-compare plus determinism check.
    for phase in ACTIONABLE {
        let g = state_with(|h| {
            h.phase = phase;
            h.turn_seat = Some(seat(0));
            h.dealer_seat = seat(0);
            h.trump = Some(s('H'));
            h.turned_down_suit = Some(s('H'));
            h.trick = trick(Some('S'), &[(3, "AS")]);
            h.hands = [
                cards(&["AS", "KS", "9H", "TD", "QC"]),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ];
        });
        let before = g.clone();
        let first = legal_moves(&g, seat(0));
        assert_eq!(g, before);
        assert_eq!((phase, !first.is_empty()), (phase, true));
        assert_eq!(
            (phase, legal_moves(&g, seat(0))),
            (phase, legal_moves(&g, seat(0)))
        );
    }
}

#[test]
fn v21_does_not_mutate_the_hand_array_in_get_legal_plays() {
    let hand = cards(&VOID_HAND);
    assert_eq!(
        get_legal_plays(&hand, Some(s('H')), Some(s('H'))),
        cards(&["JD"])
    );
    assert_eq!(hand, cards(&VOID_HAND));
}

#[test]
fn returns_fresh_arrays_and_objects_on_every_call() {
    // TS checks `not.toBe`; each call returns a distinct owned `Vec`. Mutating one
    // result must not affect the next.
    let g = state_with(|h| {
        h.phase = GamePhase::Cutting;
        h.turn_seat = Some(seat(3));
    });
    let mut a = legal_moves(&g, seat(3));
    let b = legal_moves(&g, seat(3));
    assert_eq!(a, b);
    a[0].label.push('!');
    a.clear();
    assert_eq!(legal_moves(&g, seat(3)), b);
}

fn lookup_state() -> GameState {
    state_with(|h| {
        h.phase = GamePhase::BidRound2;
        h.turn_seat = Some(seat(1));
        h.dealer_seat = seat(0);
        h.turned_down_suit = Some(s('H'));
    })
}

#[test]
fn legal_move_ids_is_legal_moves_map_id_in_the_same_order() {
    let g = lookup_state();
    let want: Vec<String> = legal_moves(&g, seat(1)).into_iter().map(|m| m.id).collect();
    assert_eq!(legal_move_ids(&g, seat(1)), want);
}

#[test]
fn find_legal_move_returns_the_offered_move_or_null_when_it_is_not_on_offer() {
    let g = lookup_state();
    assert_eq!(
        find_legal_move(&g, seat(1), "call:S+alone"),
        Some(lm(
            Move::Call {
                suit: s('S'),
                alone: true
            },
            "Spades, alone"
        ))
    );
    assert_eq!(find_legal_move(&g, seat(1), "call:H"), None);
    assert_eq!(find_legal_move(&g, seat(1), "play:AS"), None);
    assert_eq!(find_legal_move(&g, seat(0), "pass"), None);
}

#[test]
fn finds_every_id_legal_move_ids_reports_and_nothing_else() {
    let g = state_with(|h| {
        h.phase = GamePhase::TrickPlay;
        h.turn_seat = Some(seat(2));
        h.trump = Some(s('S'));
        h.trick = Trick::empty(0);
        h.hands = [
            Vec::new(),
            Vec::new(),
            cards(&["AS", "KH", "9D"]),
            Vec::new(),
        ];
    });
    for id in legal_move_ids(&g, seat(2)) {
        assert!(find_legal_move(&g, seat(2), &id).is_some());
    }
    assert_eq!(find_legal_move(&g, seat(2), "play:QC"), None);
}

/* Rust-only: the natural-trump house rule filters on the printed suit. */

#[test]
fn natural_trump_filters_round_1_and_round_2_on_the_printed_suit() {
    // Round 1, hearts up-card, non-dealer holding only the left bower (JD): pass only.
    let mut g = state_with(|h| {
        h.phase = GamePhase::BidRound1;
        h.turn_seat = Some(seat(1));
        h.dealer_seat = seat(0);
        h.up_card = Some(c("9H"));
        h.hands = [
            Vec::new(),
            cards(&["JD", "9C", "TC", "KC", "AC"]),
            Vec::new(),
            Vec::new(),
        ];
    });
    g.cfg.require_natural_trump = Some(true);
    assert_eq!(legal_move_ids(&g, seat(1)), vec!["pass"]);
    // The dealer is exempt in round 1.
    g.hand.turn_seat = Some(seat(0));
    g.hand.hands[0] = cards(&["9C"]);
    assert_eq!(
        legal_move_ids(&g, seat(0)),
        vec!["pass", "orderUp", "orderUp+alone"]
    );
    // Round 2: only printed suits held, minus the turned-down suit; the stuck dealer is exempt.
    g.hand.phase = GamePhase::BidRound2;
    g.hand.turned_down_suit = Some(s('H'));
    g.hand.turn_seat = Some(seat(1));
    assert_eq!(
        legal_move_ids(&g, seat(1)),
        vec!["pass", "call:D", "call:D+alone", "call:C", "call:C+alone"]
    );
    g.hand.turn_seat = Some(seat(0));
    g.hand.passes = 3;
    assert_eq!(legal_moves(&g, seat(0)).len(), 6);
}
