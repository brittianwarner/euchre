//! Port of `__tests__/cards.test.ts`: the bower-aware ranking primitives.
//!
//! Ground truth is written out by hand rather than re-derived from the colour-mate table.

mod common;

use common::{ALL_SUITS, c, cards, s, sorted};
use euchre_engine::{
    CARD_IDS, Card, DECK, RANKS, Rank, SUITS, Suit, bowers_of, effective_suit, is_left_bower,
    plain_rank, rank_of, same_color, suit_of, trump_rank,
};

/// The one card per trump whose printed suit is a lie. Written out, not derived.
fn left_bower(trump: Suit) -> Card {
    match trump {
        Suit::Spades => c("JC"),
        Suit::Clubs => c("JS"),
        Suit::Hearts => c("JD"),
        Suit::Diamonds => c("JH"),
    }
}

/// The seven trump for each suit, strictly high to low.
fn trump_order(trump: Suit) -> Vec<Card> {
    match trump {
        Suit::Spades => cards(&["JS", "JC", "AS", "KS", "QS", "TS", "9S"]),
        Suit::Hearts => cards(&["JH", "JD", "AH", "KH", "QH", "TH", "9H"]),
        Suit::Diamonds => cards(&["JD", "JH", "AD", "KD", "QD", "TD", "9D"]),
        Suit::Clubs => cards(&["JC", "JS", "AC", "KC", "QC", "TC", "9C"]),
    }
}

const TRUMP_RANKS: [u8; 7] = [8, 7, 6, 5, 4, 3, 2];

/// The colour-mate suit. Written out, not derived.
fn mate(trump: Suit) -> Suit {
    match trump {
        Suit::Spades => Suit::Clubs,
        Suit::Clubs => Suit::Spades,
        Suit::Hearts => Suit::Diamonds,
        Suit::Diamonds => Suit::Hearts,
    }
}

fn card_of(rank: Rank, suit: Suit) -> Card {
    Card::new(rank, suit)
}

/* deck constants */

#[test]
fn suits_is_exactly_s_h_d_c_in_that_order() {
    assert_eq!(SUITS, [s('S'), s('H'), s('D'), s('C')]);
}

#[test]
fn ranks_is_exactly_9_t_j_q_k_a_low_to_high() {
    let got: String = RANKS.iter().map(|r| r.as_char()).collect();
    assert_eq!(got, "9TJQKA");
}

#[test]
fn deck_is_the_24_cards_in_canonical_suit_major_rank_ascending_order() {
    assert_eq!(
        DECK.to_vec(),
        cards(&[
            "9S", "TS", "JS", "QS", "KS", "AS", "9H", "TH", "JH", "QH", "KH", "AH", "9D", "TD",
            "JD", "QD", "KD", "AD", "9C", "TC", "JC", "QC", "KC", "AC",
        ])
    );
}

#[test]
fn deck_has_24_distinct_cards_no_2_8_no_joker() {
    assert_eq!(DECK.len(), 24);
    let mut d = DECK.to_vec();
    d.sort();
    d.dedup();
    assert_eq!(d.len(), 24);
}

#[test]
fn card_ids_is_the_same_array_as_deck() {
    // TS checks reference identity (`toBe`); in Rust both are the same const value.
    assert_eq!(CARD_IDS, DECK);
}

/* rankOf / suitOf */

#[test]
fn splits_every_one_of_the_24_card_ids_into_its_printed_rank_and_suit() {
    for suit in ALL_SUITS {
        for rank in RANKS {
            let id = format!("{}{}", rank.as_char(), suit.as_char());
            let card = c(&id);
            assert_eq!(rank_of(card), rank);
            assert_eq!(suit_of(card), suit);
        }
    }
}

#[test]
fn suit_of_reports_the_printed_suit_even_for_the_left_bower() {
    assert_eq!(suit_of(c("JD")), s('D'));
    assert_eq!(rank_of(c("JD")), Rank::Jack);
}

#[test]
fn round_trips_rank_of_plus_suit_of_equals_c_for_all_24() {
    for card in DECK {
        assert_eq!(
            format!("{}{}", rank_of(card).as_char(), suit_of(card).as_char()),
            card.to_string()
        );
    }
}

/* sameColor */

#[test]
fn same_color_matches_the_full_4x4_truth_table_and_is_reflexive() {
    let expected = |a: char, b: char| -> bool {
        match a {
            'S' | 'C' => matches!(b, 'S' | 'C'),
            _ => matches!(b, 'H' | 'D'),
        }
    };
    for a in ALL_SUITS {
        for b in ALL_SUITS {
            assert_eq!(
                (a, b, same_color(a, b)),
                (a, b, expected(a.as_char(), b.as_char()))
            );
        }
    }
}

#[test]
fn same_color_is_symmetric() {
    for a in ALL_SUITS {
        for b in ALL_SUITS {
            assert_eq!(same_color(a, b), same_color(b, a));
        }
    }
}

/* effectiveSuit */

#[test]
fn reports_the_left_bower_as_trump() {
    assert_eq!(effective_suit(c("JD"), Some(s('H'))), s('H'));
    assert_eq!(effective_suit(c("JH"), Some(s('D'))), s('D'));
    assert_eq!(effective_suit(c("JC"), Some(s('S'))), s('S'));
    assert_eq!(effective_suit(c("JS"), Some(s('C'))), s('C'));
}

#[test]
fn reports_the_right_bower_as_trump() {
    assert_eq!(effective_suit(c("JH"), Some(s('H'))), s('H'));
    assert_eq!(effective_suit(c("JS"), Some(s('S'))), s('S'));
    assert_eq!(effective_suit(c("JD"), Some(s('D'))), s('D'));
    assert_eq!(effective_suit(c("JC"), Some(s('C'))), s('C'));
}

#[test]
fn leaves_a_jack_of_the_other_colour_alone() {
    assert_eq!(effective_suit(c("JD"), Some(s('S'))), s('D'));
    assert_eq!(effective_suit(c("JD"), Some(s('C'))), s('D'));
    assert_eq!(effective_suit(c("JS"), Some(s('H'))), s('S'));
    assert_eq!(effective_suit(c("JS"), Some(s('D'))), s('S'));
}

#[test]
fn never_promotes_a_non_jack_however_suggestive_its_suit() {
    assert_eq!(effective_suit(c("AD"), Some(s('H'))), s('D'));
    assert_eq!(effective_suit(c("9D"), Some(s('H'))), s('D'));
    assert_eq!(effective_suit(c("QC"), Some(s('S'))), s('C'));
}

#[test]
fn is_the_identity_when_trump_is_null_for_all_24_cards() {
    for card in DECK {
        assert_eq!(effective_suit(card, None), suit_of(card));
    }
}

#[test]
fn differs_from_suit_of_for_exactly_one_card_per_trump_the_left_bower() {
    for trump in ALL_SUITS {
        let moved: Vec<Card> = DECK
            .iter()
            .copied()
            .filter(|&x| effective_suit(x, Some(trump)) != suit_of(x))
            .collect();
        assert_eq!((trump, moved), (trump, vec![left_bower(trump)]));
    }
}

#[test]
fn matches_the_hand_written_bower_table_over_all_96_card_trump_pairs() {
    for trump in ALL_SUITS {
        for card in DECK {
            let want = if card == left_bower(trump) {
                trump
            } else {
                suit_of(card)
            };
            assert_eq!(
                (card, trump, effective_suit(card, Some(trump))),
                (card, trump, want)
            );
        }
    }
}

#[test]
fn only_ever_returns_a_suit_the_card_could_plausibly_be_printed_suit_or_trump() {
    for trump in ALL_SUITS {
        for card in DECK {
            let e = effective_suit(card, Some(trump));
            assert_eq!(
                (card, trump, e == suit_of(card) || e == trump),
                (card, trump, true)
            );
        }
    }
}

#[test]
fn makes_jd_9c_tc_kc_ac_void_in_diamonds_when_hearts_are_trump() {
    let hand = cards(&["JD", "9C", "TC", "KC", "AC"]);
    let diamonds: Vec<Card> = hand
        .iter()
        .copied()
        .filter(|&x| effective_suit(x, Some(s('H'))) == s('D'))
        .collect();
    assert_eq!(diamonds, Vec::<Card>::new());
    let hearts: Vec<Card> = hand
        .iter()
        .copied()
        .filter(|&x| effective_suit(x, Some(s('H'))) == s('H'))
        .collect();
    assert_eq!(hearts, cards(&["JD"]));
}

/* trump census (V16) */

#[test]
fn gives_trump_7_cards_the_same_colour_suit_5_and_each_off_colour_suit_6() {
    for trump in ALL_SUITS {
        let mut counts = [0u32; 4];
        for card in DECK {
            counts[effective_suit(card, Some(trump)).index()] += 1;
        }
        let m = mate(trump);
        assert_eq!((trump, "trump", counts[trump.index()]), (trump, "trump", 7));
        assert_eq!((trump, "mate", counts[m.index()]), (trump, "mate", 5));
        for other in ALL_SUITS {
            if other == trump || other == m {
                continue;
            }
            assert_eq!((trump, other, counts[other.index()]), (trump, other, 6));
        }
        assert_eq!(counts.iter().sum::<u32>(), 24);
    }
}

#[test]
fn names_the_seven_trump_exactly_for_every_trump_suit() {
    for trump in ALL_SUITS {
        let trumps: Vec<Card> = DECK
            .iter()
            .copied()
            .filter(|&x| effective_suit(x, Some(trump)) == trump)
            .collect();
        assert_eq!(
            (trump, sorted(&trumps)),
            (trump, sorted(&trump_order(trump)))
        );
    }
}

/* trumpRank */

#[test]
fn trump_rank_matches_the_spec_sample_values() {
    assert_eq!(trump_rank(c("JH"), s('H')), 8);
    assert_eq!(trump_rank(c("JD"), s('H')), 7);
    assert_eq!(trump_rank(c("AH"), s('H')), 6);
    assert_eq!(trump_rank(c("9H"), s('H')), 2);
    assert_eq!(trump_rank(c("AS"), s('H')), 0);
}

#[test]
fn orders_trump_right_left_a_k_q_t_9_for_every_trump_suit() {
    for trump in ALL_SUITS {
        let got: Vec<u8> = trump_order(trump)
            .iter()
            .map(|&x| trump_rank(x, trump))
            .collect();
        assert_eq!((trump, got.clone()), (trump, TRUMP_RANKS.to_vec()));
        for i in 1..got.len() {
            assert!(got[i - 1] > got[i]);
        }
    }
}

#[test]
fn scores_exactly_7_of_the_24_cards_non_zero_for_every_trump_v16() {
    for trump in ALL_SUITS {
        let non_zero = DECK.iter().filter(|&&x| trump_rank(x, trump) > 0).count();
        assert_eq!((trump, non_zero), (trump, 7));
    }
}

#[test]
fn trump_rank_is_positive_exactly_when_effective_suit_is_trump_over_all_96_pairs() {
    for trump in ALL_SUITS {
        for card in DECK {
            let is_trump = effective_suit(card, Some(trump)) == trump;
            assert_eq!(
                (card, trump, trump_rank(card, trump) > 0),
                (card, trump, is_trump)
            );
        }
    }
}

#[test]
fn returns_0_for_every_off_suit_card_including_the_jack_of_the_wrong_colour() {
    assert_eq!(trump_rank(c("JD"), s('S')), 0);
    assert_eq!(trump_rank(c("JS"), s('H')), 0);
    assert_eq!(trump_rank(c("AD"), s('S')), 0);
    assert_eq!(trump_rank(c("9C"), s('D')), 0);
}

#[test]
fn the_right_bower_is_the_single_highest_card_at_8_the_left_the_single_second_at_7() {
    for trump in ALL_SUITS {
        let eights: Vec<Card> = DECK
            .iter()
            .copied()
            .filter(|&x| trump_rank(x, trump) == 8)
            .collect();
        let sevens: Vec<Card> = DECK
            .iter()
            .copied()
            .filter(|&x| trump_rank(x, trump) == 7)
            .collect();
        assert_eq!((trump, eights), (trump, vec![card_of(Rank::Jack, trump)]));
        assert_eq!((trump, sevens), (trump, vec![left_bower(trump)]));
    }
}

/* plainRank */

#[test]
fn puts_the_jack_below_the_queen() {
    assert_eq!(plain_rank(c("JS")), 3);
    assert_eq!(plain_rank(c("QS")), 4);
    assert!(plain_rank(c("JS")) < plain_rank(c("QS")));
}

#[test]
fn matches_the_full_a_k_q_j_t_9_table() {
    let want = |r: Rank| -> u8 {
        match r {
            Rank::Ace => 6,
            Rank::King => 5,
            Rank::Queen => 4,
            Rank::Jack => 3,
            Rank::Ten => 2,
            Rank::Nine => 1,
        }
    };
    for suit in ALL_SUITS {
        for rank in RANKS {
            let card = card_of(rank, suit);
            assert_eq!((card, plain_rank(card)), (card, want(rank)));
        }
    }
}

#[test]
fn depends_on_rank_alone_the_same_rank_scores_the_same_in_all_four_suits() {
    for rank in RANKS {
        let mut vals: Vec<u8> = ALL_SUITS
            .iter()
            .map(|&su| plain_rank(card_of(rank, su)))
            .collect();
        vals.sort_unstable();
        vals.dedup();
        assert_eq!(vals.len(), 1);
    }
}

#[test]
fn is_strictly_ascending_over_ranks() {
    let vals: Vec<u8> = RANKS
        .iter()
        .map(|&r| plain_rank(card_of(r, s('S'))))
        .collect();
    assert_eq!(vals, vec![1, 2, 3, 4, 5, 6]);
}

/* bowersOf / isLeftBower */

#[test]
fn bowers_of_returns_right_left_for_every_trump() {
    assert_eq!(bowers_of(s('H')).to_vec(), cards(&["JH", "JD"]));
    assert_eq!(bowers_of(s('D')).to_vec(), cards(&["JD", "JH"]));
    assert_eq!(bowers_of(s('S')).to_vec(), cards(&["JS", "JC"]));
    assert_eq!(bowers_of(s('C')).to_vec(), cards(&["JC", "JS"]));
}

#[test]
fn bowers_of_agrees_with_trump_rank_8_7() {
    for trump in ALL_SUITS {
        let [right, left] = bowers_of(trump);
        assert_eq!(
            (trump, trump_rank(right, trump), trump_rank(left, trump)),
            (trump, 8, 7)
        );
    }
}

#[test]
fn is_left_bower_is_true_for_exactly_one_card_per_trump_over_all_96_pairs() {
    for trump in ALL_SUITS {
        for card in DECK {
            assert_eq!(
                (card, trump, is_left_bower(card, Some(trump))),
                (card, trump, card == left_bower(trump))
            );
        }
    }
}

#[test]
fn is_left_bower_is_false_for_the_right_bower() {
    for trump in ALL_SUITS {
        assert!(!is_left_bower(card_of(Rank::Jack, trump), Some(trump)));
    }
}

#[test]
fn is_left_bower_is_false_for_every_card_when_trump_is_null() {
    for card in DECK {
        assert!(!is_left_bower(card, None));
    }
}

/* Rust-only: the card id string form is a total, lossless round trip. */

#[test]
fn card_id_parse_display_and_serde_round_trip() {
    for card in DECK {
        let id = card.to_string();
        assert_eq!(Card::parse(&id), Some(card));
        assert_eq!(serde_json::to_string(&card).ok(), Some(format!("\"{id}\"")));
        assert_eq!(
            serde_json::from_str::<Card>(&format!("\"{id}\"")).ok(),
            Some(card)
        );
    }
    for bad in ["", "J", "JX", "1S", "JSS", "js"] {
        assert_eq!(Card::parse(bad), None, "{bad:?}");
    }
}
