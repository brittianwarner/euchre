//! Port of `__tests__/trick.test.ts`: the total order inside one trick, and who takes it.

mod common;

use common::{ALL_SUITS, c, s, seat};
use euchre_engine::{
    Card, DECK, Play, Suit, card_value, currently_winning, effective_suit, is_trick_complete,
    led_suit_of, plain_rank, plays_per_trick, suit_of, trick_winner, trump_rank,
};

/// Builds a trick from `(seat, card)` pairs, laid in array order.
fn plays(ps: &[(u8, &str)]) -> Vec<Play> {
    ps.iter()
        .map(|&(n, id)| Play {
            seat: seat(n),
            card: c(id),
        })
        .collect()
}

fn play(n: u8, id: &str) -> Play {
    Play {
        seat: seat(n),
        card: c(id),
    }
}

fn h() -> Option<Suit> {
    Some(s('H'))
}

fn trump_cards(trump: Suit) -> impl Iterator<Item = Card> {
    DECK.into_iter()
        .filter(move |&x| effective_suit(x, Some(trump)) == trump)
}

fn winner(t: &[Play], led: char, trump: char) -> u8 {
    trick_winner(t, s(led), s(trump))
        .map(|x| x.get())
        .unwrap_or(99)
}

/* cardValue: trump band 102..108 */

#[test]
fn scores_the_seven_hearts_trump_exactly_108_to_102_right_bower_first() {
    assert_eq!(card_value(c("JH"), s('S'), h()), 108);
    assert_eq!(card_value(c("JD"), s('S'), h()), 107);
    assert_eq!(card_value(c("AH"), s('S'), h()), 106);
    assert_eq!(card_value(c("KH"), s('S'), h()), 105);
    assert_eq!(card_value(c("QH"), s('S'), h()), 104);
    assert_eq!(card_value(c("TH"), s('S'), h()), 103);
    assert_eq!(card_value(c("9H"), s('S'), h()), 102);
}

#[test]
fn is_100_plus_trump_rank_for_every_trump_card_for_every_trump_suit() {
    for trump in ALL_SUITS {
        for led in ALL_SUITS {
            for x in trump_cards(trump) {
                assert_eq!(
                    (x, led, trump, card_value(x, led, Some(trump))),
                    (x, led, trump, 100 + trump_rank(x, trump))
                );
            }
        }
    }
}

#[test]
fn lands_every_trump_card_strictly_inside_102_108() {
    for trump in ALL_SUITS {
        for x in trump_cards(trump) {
            let v = card_value(x, s('S'), Some(trump));
            assert_eq!((x, trump, (102..=108).contains(&v)), (x, trump, true));
        }
    }
}

#[test]
fn scores_trump_the_same_whether_or_not_trump_happens_to_be_the_led_suit() {
    for trump in ALL_SUITS {
        for x in trump_cards(trump) {
            let other_led = if trump == s('S') { s('H') } else { s('S') };
            assert_eq!(
                (x, trump, card_value(x, trump, Some(trump))),
                (x, trump, card_value(x, other_led, Some(trump)))
            );
        }
    }
}

/* cardValue: led-suit band 51..56 */

#[test]
fn scores_the_six_spades_56_to_51_when_spades_are_led_and_hearts_are_trump() {
    assert_eq!(card_value(c("AS"), s('S'), h()), 56);
    assert_eq!(card_value(c("KS"), s('S'), h()), 55);
    assert_eq!(card_value(c("QS"), s('S'), h()), 54);
    assert_eq!(card_value(c("JS"), s('S'), h()), 53);
    assert_eq!(card_value(c("TS"), s('S'), h()), 52);
    assert_eq!(card_value(c("9S"), s('S'), h()), 51);
}

#[test]
fn is_50_plus_plain_rank_for_every_non_trump_card_of_the_led_suit() {
    for trump in ALL_SUITS {
        for led in ALL_SUITS {
            for x in DECK {
                if effective_suit(x, Some(trump)) != led || led == trump {
                    continue;
                }
                assert_eq!(
                    (x, led, trump, card_value(x, led, Some(trump))),
                    (x, led, trump, 50 + plain_rank(x))
                );
            }
        }
    }
}

#[test]
fn the_whole_trump_band_outranks_the_whole_led_band() {
    for trump in ALL_SUITS {
        for led in ALL_SUITS {
            if led == trump {
                continue;
            }
            let lowest_trump = trump_cards(trump)
                .map(|x| card_value(x, led, Some(trump)))
                .min()
                .unwrap_or(0);
            let highest_led = DECK
                .into_iter()
                .filter(|&x| effective_suit(x, Some(trump)) == led)
                .map(|x| card_value(x, led, Some(trump)))
                .max()
                .unwrap_or(u8::MAX);
            assert_eq!((led, trump, lowest_trump > highest_led), (led, trump, true));
        }
    }
}

/* cardValue: the zero band */

#[test]
fn makes_9c_beat_ah_when_clubs_are_led_and_diamonds_are_trump() {
    assert_eq!(card_value(c("9C"), s('C'), Some(s('D'))), 51);
    assert_eq!(card_value(c("AH"), s('C'), Some(s('D'))), 0);
    assert!(card_value(c("9C"), s('C'), Some(s('D'))) > card_value(c("AH"), s('C'), Some(s('D'))));
}

#[test]
fn scores_0_for_every_card_that_is_neither_trump_nor_the_led_suit() {
    for trump in ALL_SUITS {
        for led in ALL_SUITS {
            for x in DECK {
                let eff = effective_suit(x, Some(trump));
                if eff == trump || eff == led {
                    continue;
                }
                assert_eq!(
                    (x, led, trump, card_value(x, led, Some(trump))),
                    (x, led, trump, 0)
                );
            }
        }
    }
}

#[test]
fn scores_the_ace_of_an_off_suit_0() {
    assert_eq!(card_value(c("AS"), s('C'), Some(s('D'))), 0);
    assert_eq!(card_value(c("AH"), s('S'), Some(s('D'))), 0);
    assert_eq!(card_value(c("AD"), s('S'), h()), 0);
}

#[test]
fn scores_the_left_bower_in_the_trump_band_not_its_printed_suit() {
    assert_eq!(effective_suit(c("JD"), h()), s('H'));
    assert_eq!(card_value(c("JD"), s('D'), h()), 107);
    assert_eq!(card_value(c("AD"), s('D'), h()), 56);
}

/* cardValue: trump null */

#[test]
fn falls_back_to_the_led_suit_band_with_no_trump_band_at_all() {
    assert_eq!(card_value(c("AS"), s('S'), None), 56);
    assert_eq!(card_value(c("9S"), s('S'), None), 51);
    assert_eq!(card_value(c("JS"), s('S'), None), 53);
    assert_eq!(card_value(c("AH"), s('S'), None), 0);
}

#[test]
fn never_produces_a_value_above_56_when_trump_is_null_over_all_96_pairs() {
    for led in ALL_SUITS {
        for x in DECK {
            let v = card_value(x, led, None);
            assert_eq!((x, led, v <= 56), (x, led, true));
        }
    }
}

#[test]
fn treats_the_would_be_left_bower_as_an_ordinary_card_when_trump_is_null() {
    assert_eq!(card_value(c("JD"), s('H'), None), 0);
    assert_eq!(card_value(c("JD"), s('D'), None), 53);
}

/* trickWinner */

#[test]
fn case_20_a_nine_of_the_led_suit_beats_an_ace_of_anything_else() {
    let t = plays(&[(0, "9C"), (1, "AH"), (2, "KH"), (3, "QH")]);
    assert_eq!(winner(&t, 'C', 'D'), 0);
}

#[test]
fn case_21_any_trump_beats_any_plain_card() {
    let t = plays(&[(0, "AS"), (1, "9H"), (2, "KS"), (3, "QS")]);
    assert_eq!(winner(&t, 'S', 'H'), 1);
}

#[test]
fn case_22_the_left_bower_is_trump_and_outranks_the_nine_of_trump() {
    let t = plays(&[(0, "AS"), (1, "9H"), (2, "JD"), (3, "KS")]);
    assert_eq!(winner(&t, 'S', 'H'), 2);
}

#[test]
fn case_23_right_beats_left() {
    let t = plays(&[(0, "AS"), (1, "JD"), (2, "JH"), (3, "KS")]);
    assert_eq!(winner(&t, 'S', 'H'), 2);
}

#[test]
fn case_24_leading_the_left_bower_leads_trump_and_the_left_bower_wins_it() {
    let t = plays(&[(0, "JD"), (1, "AH"), (2, "KH"), (3, "9H")]);
    let led = led_suit_of(&t, h());
    assert_eq!(led, h());
    let led = led.unwrap_or(s('D'));
    assert_eq!(trick_winner(&t, led, s('H')).map(|x| x.get()), Ok(0));
}

#[test]
fn case_25_with_spades_trump_jd_is_an_ordinary_diamond_jack_below_the_queen() {
    let t = plays(&[(0, "9D"), (1, "TD"), (2, "JD"), (3, "AD")]);
    assert_eq!(winner(&t, 'D', 'S'), 3);
    assert_eq!(card_value(c("JD"), s('D'), Some(s('S'))), 53);
    assert_eq!(card_value(c("QD"), s('D'), Some(s('S'))), 54);
}

#[test]
fn case_26_three_play_loner_trick_trump_c_led_h_9c_takes_it_for_seat_0() {
    let t = plays(&[(1, "KH"), (3, "AH"), (0, "9C")]);
    assert_eq!(t.len(), 3);
    assert_eq!(winner(&t, 'H', 'C'), 0);
}

#[test]
fn a_single_play_trick_is_won_by_that_play() {
    assert_eq!(winner(&plays(&[(2, "9S")]), 'S', 'H'), 2);
}

#[test]
fn errors_on_an_empty_trick_a_programmer_error_not_a_rules_event() {
    // TS throws a plain Error matching /trickWinner/; Rust returns a typed error with that message.
    let err = trick_winner(&[], s('S'), s('H'));
    assert!(err.is_err());
    assert!(
        err.err()
            .map(|e| e.to_string())
            .unwrap_or_default()
            .contains("trickWinner")
    );
}

#[test]
fn does_not_mutate_the_plays_array_or_its_members() {
    // The borrow checker already forbids mutation through `&[Play]`; asserted for parity.
    let t = plays(&[(0, "AS"), (1, "JD"), (2, "JH"), (3, "KS")]);
    let before = serde_json::to_string(&t).unwrap_or_default();
    let _ = trick_winner(&t, s('S'), s('H'));
    assert_eq!(serde_json::to_string(&t).unwrap_or_default(), before);
}

#[test]
fn always_picks_the_play_with_the_maximum_card_value() {
    let t = plays(&[(0, "AS"), (1, "9H"), (2, "JD"), (3, "KS")]);
    let w = trick_winner(&t, s('S'), s('H')).ok();
    let max = t.iter().map(|p| card_value(p.card, s('S'), h())).max();
    let winning = t
        .iter()
        .find(|p| Some(p.seat) == w)
        .map(|p| card_value(p.card, s('S'), h()));
    assert_eq!(winning, max);
}

#[test]
fn the_winning_card_always_scores_above_the_zero_band() {
    for trump in ALL_SUITS {
        for first in DECK {
            let led = effective_suit(first, Some(trump));
            let second = if first == c("AS") { c("KS") } else { c("AS") };
            let t = vec![
                Play {
                    seat: seat(0),
                    card: first,
                },
                Play {
                    seat: seat(1),
                    card: second,
                },
            ];
            let w = trick_winner(&t, led, trump).ok();
            let card = t.iter().find(|p| Some(p.seat) == w).map(|p| p.card);
            let ok = card.is_some_and(|x| card_value(x, led, Some(trump)) > 0);
            assert_eq!((first, trump, ok), (first, trump, true));
        }
    }
}

#[test]
fn is_independent_of_play_order_for_the_trump_led_bands() {
    let t = plays(&[(0, "AS"), (1, "JD"), (2, "JH"), (3, "KS")]);
    let mut reversed = t.clone();
    reversed.reverse();
    assert_eq!(
        trick_winner(&reversed, s('S'), s('H')),
        trick_winner(&t, s('S'), s('H'))
    );
}

#[test]
fn resolves_a_zero_value_tie_deterministically_and_never_to_a_discard() {
    assert_eq!(card_value(c("TC"), s('S'), h()), 0);
    assert_eq!(card_value(c("9D"), s('S'), h()), 0);
    let t = plays(&[(0, "9S"), (1, "TC"), (2, "9D"), (3, "TS")]);
    assert_eq!(winner(&t, 'S', 'H'), 3);
}

#[test]
fn is_total_when_every_play_scores_0_the_first_play_holds() {
    let t = plays(&[(2, "TC"), (3, "9D"), (0, "AC")]);
    for p in &t {
        assert_eq!(card_value(p.card, s('S'), h()), 0);
    }
    assert_eq!(winner(&t, 'S', 'H'), 2);
}

/* ledSuitOf */

#[test]
fn led_suit_of_returns_null_for_an_empty_trick() {
    assert_eq!(led_suit_of(&[], h()), None);
    assert_eq!(led_suit_of(&[], None), None);
}

#[test]
fn reports_the_effective_suit_leading_the_left_bower_leads_trump() {
    assert_eq!(led_suit_of(&plays(&[(0, "JD")]), h()), h());
    assert_eq!(
        led_suit_of(&plays(&[(0, "JC")]), Some(s('S'))),
        Some(s('S'))
    );
    assert_eq!(
        led_suit_of(&plays(&[(0, "JS")]), Some(s('C'))),
        Some(s('C'))
    );
    assert_eq!(
        led_suit_of(&plays(&[(0, "JH")]), Some(s('D'))),
        Some(s('D'))
    );
}

#[test]
fn reports_the_printed_suit_when_the_jack_is_the_wrong_colour() {
    assert_eq!(
        led_suit_of(&plays(&[(0, "JD")]), Some(s('S'))),
        Some(s('D'))
    );
    assert_eq!(
        led_suit_of(&plays(&[(0, "JD")]), Some(s('C'))),
        Some(s('D'))
    );
}

#[test]
fn reports_the_printed_suit_when_there_is_no_trump_yet() {
    for x in DECK {
        assert_eq!(
            led_suit_of(
                &[Play {
                    seat: seat(0),
                    card: x
                }],
                None
            ),
            Some(suit_of(x))
        );
    }
}

#[test]
fn is_fixed_by_the_first_play_and_ignores_everything_after_it() {
    let t = plays(&[(0, "9S"), (1, "JH"), (2, "AD")]);
    assert_eq!(led_suit_of(&t, h()), Some(s('S')));
}

#[test]
fn led_suit_of_agrees_with_effective_suit_over_all_96_pairs() {
    for trump in ALL_SUITS {
        for x in DECK {
            assert_eq!(
                (
                    x,
                    trump,
                    led_suit_of(
                        &[Play {
                            seat: seat(0),
                            card: x
                        }],
                        Some(trump)
                    )
                ),
                (x, trump, Some(effective_suit(x, Some(trump))))
            );
        }
    }
}

/* playsPerTrick / isTrickComplete */

#[test]
fn needs_4_plays_normally_and_3_under_a_loner() {
    assert_eq!(plays_per_trick(None), 4);
    assert_eq!(plays_per_trick(Some(seat(0))), 3);
    assert_eq!(plays_per_trick(Some(seat(3))), 3);
}

#[test]
fn is_incomplete_until_the_last_active_seat_has_played() {
    let four = plays(&[(0, "9S"), (1, "TS"), (2, "JS"), (3, "QS")]);
    assert!(!is_trick_complete(&[], None));
    assert!(!is_trick_complete(&four[..1], None));
    assert!(!is_trick_complete(&four[..3], None));
    assert!(is_trick_complete(&four, None));
}

#[test]
fn completes_at_three_plays_under_a_loner() {
    let three = plays(&[(1, "9S"), (2, "TS"), (3, "JS")]);
    assert!(!is_trick_complete(&three[..2], Some(seat(0))));
    assert!(is_trick_complete(&three, Some(seat(0))));
}

/* currentlyWinning */

#[test]
fn currently_winning_is_null_on_an_empty_trick() {
    assert_eq!(currently_winning(&[], h()), None);
    assert_eq!(currently_winning(&[], None), None);
}

#[test]
fn currently_winning_is_the_leader_after_one_card() {
    assert_eq!(
        currently_winning(&plays(&[(2, "9S")]), h()),
        Some(play(2, "9S"))
    );
}

#[test]
fn tracks_the_takeover_card_by_card() {
    let t = plays(&[(0, "AS"), (1, "9H"), (2, "JD"), (3, "KS")]);
    assert_eq!(currently_winning(&t[..1], h()), Some(play(0, "AS")));
    assert_eq!(currently_winning(&t[..2], h()), Some(play(1, "9H")));
    assert_eq!(currently_winning(&t[..3], h()), Some(play(2, "JD")));
    assert_eq!(currently_winning(&t, h()), Some(play(2, "JD")));
}

#[test]
fn never_disagrees_with_trick_winner_on_any_prefix_of_any_trick() {
    let cases = [
        ('H', plays(&[(0, "AS"), (1, "9H"), (2, "JD"), (3, "KS")])),
        ('D', plays(&[(0, "9C"), (1, "AH"), (2, "KH"), (3, "QH")])),
        ('S', plays(&[(0, "9D"), (1, "TD"), (2, "JD"), (3, "AD")])),
        ('C', plays(&[(1, "KH"), (3, "AH"), (0, "9C")])),
        ('H', plays(&[(0, "JD"), (1, "AH"), (2, "KH"), (3, "9H")])),
    ];
    for (trump, t) in &cases {
        let trump = s(*trump);
        let led = led_suit_of(t, Some(trump)).unwrap_or(trump);
        for n in 1..=t.len() {
            let prefix = &t[..n];
            assert_eq!(
                (
                    trump,
                    n,
                    currently_winning(prefix, Some(trump)).map(|p| p.seat)
                ),
                (trump, n, trick_winner(prefix, led, trump).ok())
            );
        }
    }
}

#[test]
fn derives_the_led_suit_itself_so_a_left_bower_lead_is_handled() {
    let t = plays(&[(0, "JD"), (1, "AD")]);
    assert_eq!(currently_winning(&t, h()), Some(play(0, "JD")));
}
