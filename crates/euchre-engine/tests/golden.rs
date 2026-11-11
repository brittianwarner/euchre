//! Golden vectors: spec 01 section 15 (hardcoded) plus the differential RNG and
//! deal fixtures in `fixtures/deal.json`, generated from the real TS engine by
//! `scripts/gen-fixtures.ts`.

mod common;

use common::{c, cards, seat};
use euchre_engine::{
    Card, DECK, DealError, DealHandParams, DealPacket, DealtPacket, Hands, Mulberry32, Seat,
    assert_deck_order, cut_at, cut_for_hand, deal_from, deal_hand, deal_plan, dealt_packets, fnv1a,
    hand_rng, hash_seed, mulberry32, new_deck, pick_first_dealer, shuffle, shuffle_for_hand,
    starts_with_three, to_base36,
};
use serde_json::Value;

const DEAL_JSON: &str = include_str!("fixtures/deal.json");

fn deal_fixture() -> Value {
    serde_json::from_str(DEAL_JSON).unwrap_or(Value::Null)
}

fn split(ids: &str) -> Vec<Card> {
    ids.split_whitespace().map(c).collect()
}

fn packet(n: u8, count: u8) -> DealPacket {
    DealPacket {
        seat: seat(n),
        count,
    }
}

fn hands_of(h: [&[&str]; 4]) -> Hands {
    h.map(cards)
}

/// Exact numerator of a `[0, 1)` draw on the `2^-32` grid every mulberry32 draw lives on.
fn numerator(x: f64) -> u64 {
    (x * 4_294_967_296.0).round() as u64
}

/* -------------------------------------------------------------------------- */
/* Spec 01 section 15, hardcoded                                               */
/* -------------------------------------------------------------------------- */

#[test]
fn fnv1a_golden_vectors() {
    assert_eq!(fnv1a(""), 2_166_136_261);
    assert_eq!(fnv1a("a"), 3_826_002_220);
    assert_eq!(fnv1a("foobar"), 3_214_735_720);
    assert_eq!(fnv1a("seed#0"), 3_952_383_447);
    assert_eq!(fnv1a("h\u{e9}llo"), 4_058_363_231);
    // A surrogate pair: hashed as two UTF-16 code units, like `charCodeAt`.
    assert_eq!(fnv1a("\u{1F600}"), 3_409_036_472);
    assert_eq!(fnv1a("abc#0#cut"), 4_023_222_183);
}

#[test]
fn base36_golden_vectors() {
    assert_eq!(to_base36(4_294_967_295), "1z141z3");
    assert_eq!(to_base36(0), "0");
}

#[test]
fn derived_token_composes_fnv1a_and_base36_like_create_game() {
    // `deriveToken("abc", "g-abc")` = createGame's default internalToken (spec 01 section 15).
    let token: String = (0..4)
        .map(|i| to_base36(fnv1a(&format!("abc#g-abc#token#{i}"))))
        .collect();
    assert_eq!(token, "26xgdh1wxuoi1my8zj1cynak");
}

#[test]
fn mulberry32_golden_vectors() {
    let mut r1 = mulberry32(1);
    let got: Vec<f64> = (0..5).map(|_| r1()).collect();
    assert_eq!(
        got,
        vec![
            0.6270739405881613,
            0.002735721180215478,
            0.5274470399599522,
            0.9810509674716741,
            0.9683778982143849
        ]
    );
    let got0: Vec<f64> = Mulberry32::new(0).take(3).collect();
    assert_eq!(
        got0,
        vec![
            0.26642920868471265,
            0.0003297457005828619,
            0.2232720274478197
        ]
    );
}

#[test]
fn hash_seed_golden_vectors() {
    assert_eq!(hash_seed("abc", 0), 2_666_227_160);
    assert_eq!(hash_seed("abc", 1), 2_683_004_779);
    assert_eq!(hash_seed("seed-1", 3), 1_258_440_548);
}

#[test]
fn shuffle_for_hand_golden_vectors() {
    assert_eq!(
        shuffle_for_hand("abc", 0),
        split("KC KD AD 9H QD AH 9D TD TC QH JD AS TH TS JS QS 9S JC 9C KS JH QC KH AC")
    );
    assert_eq!(
        shuffle_for_hand("abc", 1),
        split("JS TH QD KD AS JH JD JC 9S QC 9C AC 9H KS KC 9D TD KH TC AH QH TS AD QS")
    );
}

#[test]
fn cut_for_hand_golden_vector_rotates_at_index_11() {
    let shuffled = shuffle_for_hand("abc", 0);
    let cut = cut_for_hand(&shuffled, "abc", 0);
    assert_eq!(
        cut,
        split("AS TH TS JS QS 9S JC 9C KS JH QC KH AC KC KD AD 9H QD AH 9D TD TC QH JD")
    );
    assert_eq!(4 + 4_023_222_183u32 % 16, 11);
    assert_eq!(cut[0], shuffled[11]);
}

#[test]
fn starts_with_three_golden_vectors() {
    let got: Vec<bool> = (0..4).map(|n| starts_with_three("abc", n)).collect();
    assert_eq!(got, vec![false, true, false, true]);
}

#[test]
fn first_dealer_golden_vectors() {
    let first =
        |s: &str| pick_first_dealer(&mut mulberry32(fnv1a(&format!("{s}#firstDealer")))).get();
    assert_eq!(first("abc"), 1);
    assert_eq!(first("seed-1"), 0);
}

#[test]
fn run_em_deal_of_abc_hand_0_dealer_1() {
    let deck = shuffle_for_hand("abc", 0);
    let d = deal_hand(DealHandParams {
        deck_order: &deck,
        dealer_seat: seat(1),
        seed: "abc",
        hand_no: 0,
    });
    let Ok(d) = d else {
        panic!("deal failed: {d:?}")
    };
    assert_eq!(
        d.hands,
        hands_of([
            &["AH", "9D", "QS", "9S", "JC"],
            &["TD", "TC", "QH", "9C", "KS"],
            &["KC", "KD", "JD", "AS", "TH"],
            &["AD", "9H", "QD", "TS", "JS"],
        ])
    );
    assert_eq!(d.up_card, c("JH"));
    assert_eq!(d.kitty.to_vec(), cards(&["JH", "QC", "KH", "AC"]));
    assert_eq!(
        d.plan,
        vec![
            packet(2, 2),
            packet(3, 3),
            packet(0, 2),
            packet(1, 3),
            packet(2, 3),
            packet(3, 2),
            packet(0, 3),
            packet(1, 2)
        ]
    );
    assert_eq!(d.deck_order, deck);
    let want: Vec<(u8, Vec<Card>)> = vec![
        (2, cards(&["KC", "KD"])),
        (3, cards(&["AD", "9H", "QD"])),
        (0, cards(&["AH", "9D"])),
        (1, cards(&["TD", "TC", "QH"])),
        (2, cards(&["JD", "AS", "TH"])),
        (3, cards(&["TS", "JS"])),
        (0, cards(&["QS", "9S", "JC"])),
        (1, cards(&["9C", "KS"])),
    ];
    let want: Vec<DealtPacket> = want
        .into_iter()
        .map(|(n, cs)| DealtPacket {
            seat: seat(n),
            count: cs.len() as u8,
            cards: Some(cs),
        })
        .collect();
    assert_eq!(dealt_packets(&deck, &d.plan), want);
}

#[test]
fn bump_deal_of_abc_hand_0_dealer_1() {
    let deck = cut_for_hand(&shuffle_for_hand("abc", 0), "abc", 0);
    let d = deal_hand(DealHandParams {
        deck_order: &deck,
        dealer_seat: seat(1),
        seed: "abc",
        hand_no: 0,
    });
    let Ok(d) = d else {
        panic!("deal failed: {d:?}")
    };
    assert_eq!(
        d.hands,
        hands_of([
            &["9S", "JC", "AD", "9H", "QD"],
            &["9C", "KS", "JH", "AH", "9D"],
            &["AS", "TH", "QC", "KH", "AC"],
            &["TS", "JS", "QS", "KC", "KD"],
        ])
    );
    assert_eq!(d.up_card, c("TD"));
}

/* -------------------------------------------------------------------------- */
/* Deal errors and edge cases (Rust-side contract of the TS throws)            */
/* -------------------------------------------------------------------------- */

#[test]
fn deal_errors_carry_the_ts_messages() {
    let short = &DECK[..23];
    assert_eq!(
        assert_deck_order(short),
        Err(DealError::WrongLength { got: 23 })
    );
    assert_eq!(
        DealError::WrongLength { got: 23 }.to_string(),
        "deckOrder must hold 24 cards, got 23"
    );
    let mut dup = DECK.to_vec();
    dup[1] = dup[0];
    assert_eq!(assert_deck_order(&dup), Err(DealError::Duplicate));
    assert_eq!(
        DealError::Duplicate.to_string(),
        "deckOrder contains duplicate cards"
    );
    assert_eq!(
        DealError::Missing { card: c("AC") }.to_string(),
        "deckOrder is missing AC"
    );
    assert_eq!(assert_deck_order(&DECK), Ok(()));
    let plan = vec![packet(0, 5), packet(1, 5), packet(2, 5)];
    assert_eq!(
        deal_from(&DECK, &plan),
        Err(DealError::WrongPlanTotal { got: 15 })
    );
    assert_eq!(
        DealError::WrongPlanTotal { got: 15 }.to_string(),
        "deal plan must account for 20 cards, got 15"
    );
}

#[test]
fn shuffle_consumes_len_minus_1_draws_and_never_mutates_its_input() {
    let mut draws = 0;
    let mut inner = mulberry32(9);
    let mut rnd = || {
        draws += 1;
        inner()
    };
    let deck = new_deck();
    let out = shuffle(&deck, &mut rnd);
    assert_eq!(draws, 23);
    assert_eq!(deck, DECK.to_vec());
    let mut a = out.clone();
    a.sort();
    let mut b = DECK.to_vec();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(shuffle::<Card>(&[], &mut mulberry32(1)), Vec::<Card>::new());
}

#[test]
fn hand_rng_is_mulberry32_of_hash_seed() {
    let mut a = hand_rng("abc", 2);
    let mut b = mulberry32(hash_seed("abc", 2));
    for _ in 0..10 {
        assert_eq!(a(), b());
    }
}

/* -------------------------------------------------------------------------- */
/* Differential fixtures from the TS engine                                    */
/* -------------------------------------------------------------------------- */

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn as_u32(v: &Value) -> u32 {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or_else(|| panic!("not a u32: {v}"))
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("bad fixture value {v}: {e}"))
}

#[test]
fn fixture_fnv1a_matches_ts_over_utf16() {
    let d = deal_fixture();
    let rows = arr(&d["fnv1a"]);
    assert!(rows.len() >= 10);
    for row in rows {
        let s = row["s"].as_str().unwrap_or_default();
        assert_eq!((s, fnv1a(s)), (s, as_u32(&row["h"])));
    }
}

#[test]
fn fixture_base36_matches_js_to_string_36() {
    let d = deal_fixture();
    for row in arr(&d["base36"]) {
        assert_eq!(
            to_base36(as_u32(&row["n"])),
            row["s"].as_str().unwrap_or_default()
        );
    }
}

#[test]
fn fixture_mulberry32_draws_are_bit_identical() {
    let d = deal_fixture();
    let rows = arr(&d["mulberry32"]);
    assert_eq!(rows.len(), 7);
    for row in rows {
        let seed = as_u32(&row["seed"]);
        let mut rng = Mulberry32::new(seed);
        for want in arr(&row["draws"]) {
            let got = rng.next_f64();
            let want = want.as_f64().unwrap_or(-1.0);
            // Every draw is k / 2^32; compare the exact numerator, and the value to 1 ulp.
            assert_eq!((seed, numerator(got)), (seed, numerator(want)));
            assert!(
                (got - want).abs() <= f64::EPSILON * want.max(f64::MIN_POSITIVE),
                "{seed}: {got} vs {want}"
            );
        }
    }
}

#[test]
fn fixture_deal_plans_match_for_every_dealer_and_alternation() {
    let d = deal_fixture();
    let rows = arr(&d["dealPlans"]);
    assert_eq!(rows.len(), 8);
    for row in rows {
        let dealer: Seat = parse(&row["dealer"]);
        let three_first = row["threeFirst"].as_bool().unwrap_or_default();
        let want: Vec<DealPacket> = parse(&row["plan"]);
        assert_eq!(
            (dealer, three_first, deal_plan(dealer, three_first)),
            (dealer, three_first, want)
        );
    }
}

#[test]
fn fixture_cut_at_matches_for_seeded_streams() {
    let d = deal_fixture();
    for row in arr(&d["cutAt"]) {
        let seed = as_u32(&row["seed"]);
        let want: Vec<Card> = parse(&row["deck"]);
        assert_eq!((seed, cut_at(&DECK, &mut mulberry32(seed))), (seed, want));
    }
}

#[test]
fn fixture_seeds_shuffle_cut_and_deal_identically_for_four_hands_each() {
    let d = deal_fixture();
    let seeds = arr(&d["seeds"]);
    assert_eq!(seeds.len(), 10);
    let mut hands_checked = 0;
    for row in seeds {
        let seed = row["seed"].as_str().unwrap_or_default();
        let first: Seat = parse(&row["firstDealer"]);
        assert_eq!(
            pick_first_dealer(&mut mulberry32(fnv1a(&format!("{seed}#firstDealer")))),
            first,
            "{seed}"
        );
        for h in arr(&row["hands"]) {
            let hand_no = as_u32(&h["handNo"]);
            let ctx = format!("{seed}#{hand_no}");
            assert_eq!(hash_seed(seed, hand_no), as_u32(&h["hashSeed"]), "{ctx}");
            assert_eq!(
                starts_with_three(seed, hand_no),
                h["startsWithThree"].as_bool().unwrap_or_default(),
                "{ctx}"
            );
            let shuffled = shuffle_for_hand(seed, hand_no);
            assert_eq!(shuffled, parse::<Vec<Card>>(&h["shuffled"]), "{ctx}");
            let cut = cut_for_hand(&shuffled, seed, hand_no);
            assert_eq!(cut, parse::<Vec<Card>>(&h["cut"]), "{ctx}");
            let dealer: Seat = parse(&h["dealer"]);

            for (key, deck) in [("run", &shuffled), ("bump", &cut)] {
                let want = &h[key];
                let got = deal_hand(DealHandParams {
                    deck_order: deck,
                    dealer_seat: dealer,
                    seed,
                    hand_no,
                });
                let Ok(got) = got else {
                    panic!("{ctx} {key}: {got:?}")
                };
                let want_hands: [Vec<Card>; 4] =
                    [0, 1, 2, 3].map(|i| parse(&want["hands"][i.to_string()]));
                assert_eq!(got.hands, want_hands, "{ctx} {key}");
                assert_eq!(
                    got.kitty.to_vec(),
                    parse::<Vec<Card>>(&want["kitty"]),
                    "{ctx} {key}"
                );
                assert_eq!(got.up_card, parse::<Card>(&want["upCard"]), "{ctx} {key}");
                if key == "run" {
                    assert_eq!(got.plan, parse::<Vec<DealPacket>>(&want["plan"]), "{ctx}");
                }
                assert_eq!(
                    dealt_packets(deck, &got.plan),
                    parse::<Vec<DealtPacket>>(&want["packets"]),
                    "{ctx} {key}"
                );
                // And the packets serialize to the TS JSON byte for byte.
                assert_eq!(
                    serde_json::to_string(&dealt_packets(deck, &got.plan)).ok(),
                    serde_json::to_string(&want["packets"]).ok(),
                    "{ctx} {key}"
                );
            }
            hands_checked += 1;
        }
    }
    assert_eq!(hands_checked, 40);
}

#[test]
fn degenerate_cut_lengths_follow_js_slice_semantics() {
    // Not reachable with a 24-card deck; pinned so the port stays total.
    let eight = &DECK[..8];
    assert_eq!(cut_for_hand(eight, "abc", 0), eight.to_vec());
    // JS: span = -2, `4023222183 % -2 === 1` (dividend's sign), so i = 5.
    let six = &DECK[..6];
    assert_eq!(
        cut_for_hand(six, "abc", 0),
        cards(&["AS", "9S", "TS", "JS", "QS", "KS"])
    );
    // JS `cutAt` on a 6-card deck: i = 4 + floor(r * -2), so i is 2 or 3.
    let mut lo = || 0.9;
    assert_eq!(
        cut_at(six, &mut lo),
        cards(&["JS", "QS", "KS", "AS", "9S", "TS"])
    );
}
