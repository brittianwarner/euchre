//! Port of `__tests__/project.test.ts`: the security boundary. No card a seat
//! is not entitled to can appear in `project` or `project_steps` output, in any
//! state reachable in a real match (V8, V12, V13, V14, V19).

mod common;

use std::collections::{BTreeSet, HashSet};

use common::*;
use euchre_engine::{
    Card, DECK, GameInit, GamePhase, GameState, GameStatus, Move, PublicGameView, Seat, Step,
    advance, create_game, cutter_of, eldest_of, legal_moves, new_game, partner_of, project,
    project_steps,
};
use serde_json::{Value, json as j};

/* -------------------------------------------------------------------------- */
/* Helpers                                                                     */
/* -------------------------------------------------------------------------- */

/// Every `[9TJQKA][SHDC]` token in a serialised payload, wherever it sits. The
/// two character classes are disjoint, so a sliding 2-byte window finds
/// exactly the non-overlapping regex matches.
fn card_tokens_in(json: &str) -> BTreeSet<Card> {
    let b = json.as_bytes();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i + 1 < b.len() {
        if b"9TJQKA".contains(&b[i]) && b"SHDC".contains(&b[i + 1]) {
            if let Some(card) = std::str::from_utf8(&b[i..i + 2]).ok().and_then(Card::parse) {
                out.insert(card);
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// The cards `seat` is legitimately entitled to know at this instant.
fn visible_to(state: &GameState, seat: Seat) -> HashSet<Card> {
    let h = &state.hand;
    let mut vis = HashSet::new();
    // Own cards, but the loner's sitting partner is entitled to nothing (V8).
    if Some(seat) != h.sitting_seat {
        vis.extend(h.hands[seat.index()].iter().copied());
    }
    // The up-card is public for the whole hand (V14).
    vis.extend(h.up_card);
    // Everything laid face up.
    vis.extend(h.trick.plays.iter().map(|p| p.card));
    for t in &h.trick_log {
        vis.extend(t.plays.iter().map(|p| p.card));
    }
    vis
}

fn hidden_from(state: &GameState, seat: Seat) -> Vec<Card> {
    let vis = visible_to(state, seat);
    DECK.iter().copied().filter(|c| !vis.contains(c)).collect()
}

fn has(json: &str, c: Card) -> bool {
    json.contains(&c.to_string())
}

fn cut_no(seed: &str) -> GameState {
    ok(
        &opened(seed, Some(seat(0)), None),
        cutter_of(seat(0)),
        Move::Cut { cut: false },
    )
    .state
}

/// `JSON.stringify({ ...view, ...patch })`.
fn widened(view: &PublicGameView, patch: Value) -> String {
    let mut v = serde_json::to_value(view).unwrap();
    let obj = v.as_object_mut().unwrap();
    for (k, x) in patch.as_object().unwrap() {
        obj.insert(k.clone(), x.clone());
    }
    v.to_string()
}

/* -------------------------------------------------------------------------- */
/* The scanner itself must be trustworthy                                      */
/* -------------------------------------------------------------------------- */

#[test]
fn finds_every_card_id_wherever_it_is_embedded() {
    let got: Vec<Card> = card_tokens_in(r#"{"hand":["9C","JD"],"legal":[{"id":"play:AS"}]}"#)
        .into_iter()
        .collect();
    assert_eq!(got, cards(&["9C", "AS", "JD"]));
    let got: Vec<Card> = card_tokens_in(r#""discard:TH""#).into_iter().collect();
    assert_eq!(got, cards(&["TH"]));
}

#[test]
fn does_not_fire_on_any_string_the_engine_can_generate() {
    let copy = [
        "Bump",
        "Run 'em",
        "Pass",
        "Pass.",
        "Turn it down.",
        "Order it up",
        "Order it up, alone",
        "Order it up. Alone.",
        "I assist",
        "I assist.",
        "I take it",
        "I take it.",
        "Next.",
        "Next. Alone.",
        "Crossing the creek.",
        "Spades",
        "Hearts",
        "Diamonds",
        "Clubs",
        "Spades, alone",
        "Spades, I guess.",
        "Nine of spades",
        "Ten of hearts",
        "Jack of diamonds",
        "Queen of clubs",
        "King of spades",
        "Ace of diamonds",
        "lobby",
        "cutting",
        "bid_round_1",
        "dealer_discard",
        "bid_round_2",
        "trick_play",
        "trick_resolve",
        "hand_score",
        "game_over",
        "lone_march",
        "throw_in",
        "active",
        "complete",
    ];
    for s in copy {
        assert!(card_tokens_in(&json(&s)).is_empty(), "{s}");
    }
}

#[test]
fn does_not_fire_on_a_turn_nonce_which_is_lowercase_base_36() {
    let mut s = new_game("nonce-scan", None);
    let mut nonces: Vec<String> = Vec::new();
    let mut i = 0;
    while i < 40 && s.status == GameStatus::Active {
        nonces.push(s.turn_id.clone());
        s = match s.hand.turn_seat {
            None => advance(&s).state,
            Some(seat) => ok(&s, seat, legal_moves(&s, seat)[0].mv).state,
        };
        i += 1;
    }
    assert!(nonces.len() > 20);
    for id in &nonces {
        assert!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
        );
        assert!(card_tokens_in(id).is_empty());
    }
}

/* -------------------------------------------------------------------------- */
/* Shape and basic contract                                                    */
/* -------------------------------------------------------------------------- */

#[test]
fn reports_the_seat_it_was_cut_for_and_mirrors_the_public_state() {
    let s = cut_no("shape");
    for seat in SEATS_ALL {
        let v = project(&s, seat);
        assert_eq!(v.you, seat);
        assert_eq!(v.v, s.v);
        assert_eq!(v.turn_id, s.turn_id);
        assert_eq!(v.status, GameStatus::Active);
        assert_eq!(v.phase, GamePhase::BidRound1);
        assert_eq!(v.hand_no, 0);
        assert_eq!(v.dealer_seat, euchre_engine::Seat::SOUTH);
        assert_eq!(v.turn_seat, Some(eldest_of(euchre_engine::Seat::SOUTH)));
        assert_eq!(v.score, [0, 0]);
        assert_eq!(v.hand, s.hand.hands[seat.index()]);
        assert_eq!(v.hand_counts, [5, 5, 5, 5]);
        assert_eq!(v.kitty_count, 3);
        assert_eq!(v.up_card, s.hand.up_card);
        assert_eq!(v.trump, None);
        assert_eq!(v.result, None);
        assert_eq!(v.delta, None);
        assert_eq!(v.winner_team, None);
    }
}

#[test]
fn offers_a_legal_set_only_to_the_seat_on_turn_and_never_an_empty_one_v19() {
    let mut checked_on_turn = 0;
    let mut checked_off = 0;
    run_game(
        new_game("legal-field", cfg_game_to(10)),
        random_picker("legal-field"),
        RunHooks::on_state(|s| {
            for seat in SEATS_ALL {
                let v = project(s, seat);
                if s.hand.turn_seat == Some(seat) && s.status == GameStatus::Active {
                    assert!(!v.legal.is_empty());
                    let got: Vec<&str> = v.legal.iter().map(|m| m.id.as_str()).collect();
                    let want: Vec<String> =
                        legal_moves(s, seat).into_iter().map(|m| m.id).collect();
                    assert_eq!(got, want);
                    checked_on_turn += 1;
                } else {
                    assert!(v.legal.is_empty());
                    checked_off += 1;
                }
            }
        }),
    );
    assert!(checked_on_turn > 200);
    assert!(checked_off > 600);
}

#[test]
fn never_mutates_a_deep_frozen_state() {
    // `deepFreeze`/`Object.isFrozen` have no Rust equivalent: `project` borrows
    // the state immutably and returns an owned, freely mutable view.
    let s = cut_no("frozen-proj");
    let before = s.clone();
    for seat in SEATS_ALL {
        let mut v = project(&s, seat);
        assert_eq!(v.hand.len(), 5);
        v.hand.pop();
        assert_eq!(v.hand.len(), 4);
    }
    assert_eq!(s, before);
}

/* -------------------------------------------------------------------------- */
/* V14 - the up-card is never nulled                                           */
/* -------------------------------------------------------------------------- */

#[test]
fn stays_identified_from_the_turn_until_the_hand_ends_in_both_branches() {
    let live = |p: GamePhase| {
        matches!(
            p,
            GamePhase::BidRound1
                | GamePhase::DealerDiscard
                | GamePhase::BidRound2
                | GamePhase::TrickPlay
                | GamePhase::TrickResolve
                | GamePhase::HandScore
        )
    };
    let mut checked = 0;
    for pick in [random_picker("up-a"), alone_picker("up-b")] {
        run_game(
            new_game("upcard", cfg_game_to(10)),
            pick,
            RunHooks::on_state(|s| {
                if !live(s.hand.phase) {
                    return;
                }
                for seat in SEATS_ALL {
                    let v = project(s, seat);
                    assert!(v.up_card.is_some());
                    assert_eq!(v.up_card, s.hand.up_card);
                    checked += 1;
                }
            }),
        );
    }
    assert!(checked > 1000);
}

#[test]
fn survives_the_dealer_discarding_the_up_card_itself() {
    let base = cut_no("up-discard");
    let up_card = base.hand.up_card.unwrap();
    let picked = ok(&base, seat(1), Move::OrderUp { alone: false }).state;
    let done = ok(&picked, seat(0), Move::Discard { card: up_card }).state;

    assert_eq!(done.hand.dealer_discard, Some(up_card));
    for seat in SEATS_ALL {
        let v = project(&done, seat);
        assert_eq!(v.up_card, Some(up_card));
        assert!(!v.up_card_turned_down);
        assert_eq!(v.kitty_count, 4);
    }
    // And the discard is not inferable from the dealer's own hand either.
    assert!(!project(&done, seat(0)).hand.contains(&up_card));
}

/* -------------------------------------------------------------------------- */
/* kittyCount and V13                                                          */
/* -------------------------------------------------------------------------- */

#[test]
fn is_3_before_the_up_card_leaves_the_table_and_4_after_in_both_branches() {
    let base = cut_no("kitty");
    assert_eq!(project(&base, seat(1)).kitty_count, 3);

    // Branch A: turned down. The up-card is buried.
    let mut turned_down = base.clone();
    for _ in 0..4 {
        turned_down = ok(&turned_down, turn(&turned_down), Move::Pass).state;
    }
    assert!(turned_down.hand.up_card_turned_down);
    assert_eq!(turned_down.hand.dealer_discard, None);
    assert_eq!(project(&turned_down, seat(1)).kitty_count, 4);

    // Branch B: picked up. Still 3 until the discard actually lands.
    let picked = ok(&base, seat(1), Move::OrderUp { alone: false }).state;
    assert_eq!(picked.hand.dealer_discard, None);
    assert_eq!(project(&picked, seat(1)).kitty_count, 3);
    let discarded = ok(
        &picked,
        seat(0),
        Move::Discard {
            card: picked.hand.hands[0][0],
        },
    )
    .state;
    assert_eq!(project(&discarded, seat(1)).kitty_count, 4);
}

#[test]
fn two_states_differing_only_in_the_discard_project_identically_v13() {
    let base = cut_no("v13");
    let picked = ok(&base, seat(1), Move::OrderUp { alone: false }).state;
    let six = picked.hand.hands[0].clone();
    assert_eq!(six.len(), 6);

    let outcomes: Vec<GameState> = six
        .iter()
        .map(|&card| ok(&picked, seat(0), Move::Discard { card }).state)
        .collect();
    let reference = &outcomes[0];

    for other in &outcomes[1..] {
        assert_ne!(other.hand.dealer_discard, reference.hand.dealer_discard);
        // Every seat but the dealer sees a byte-identical view.
        for n in [1, 2, 3] {
            assert_eq!(
                json(&project(other, seat(n))),
                json(&project(reference, seat(n)))
            );
        }
        // The dealer's view differs only in their own five cards.
        let a = project(reference, seat(0));
        let b = project(other, seat(0));
        let blank = j!({ "hand": null, "legal": null });
        assert_eq!(widened(&a, blank.clone()), widened(&b, blank));
        assert_ne!(a.hand, b.hand);
        assert_eq!(a.hand_counts, b.hand_counts);
        assert_eq!(a.kitty_count, b.kitty_count);
    }
    assert_eq!(outcomes.len(), 6);
}

/* -------------------------------------------------------------------------- */
/* V8 - the loner's sitting partner                                            */
/* -------------------------------------------------------------------------- */

#[test]
fn is_blanked_from_the_moment_the_call_lands_but_kept_in_state_for_the_journal() {
    // Eldest orders up alone: dealer 0, maker 1, sitting 3.
    let base = cut_no("sit");
    let dead = base.hand.hands[3].clone();
    let s = ok(&base, seat(1), Move::OrderUp { alone: true }).state;

    assert_eq!(s.hand.alone_seat, Some(seat(1)));
    assert_eq!(s.hand.sitting_seat, Some(partner_of(seat(1))));
    assert_eq!(s.hand.sitting_seat, Some(seat(3)));
    assert_eq!(s.hand.hands[3], dead);
    assert_eq!(s.hand.hands[3].len(), 5);

    let own = project(&s, seat(3));
    assert!(own.hand.is_empty());
    assert!(own.legal.is_empty());
    assert_eq!(own.sitting_seat, Some(seat(3)));
    assert_eq!(own.alone_seat, Some(seat(1)));
    for seat in SEATS_ALL {
        assert_eq!(project(&s, seat).hand_counts[3], 0);
    }
    assert_eq!(project(&s, seat(0)).hand_counts, [6, 5, 5, 0]);
}

#[test]
fn hides_the_dead_hand_from_its_own_owner_for_the_whole_hand() {
    let mut checked = 0;
    let mut leaks: Vec<String> = Vec::new();
    run_game(
        new_game("sit-sweep", cfg_game_to(10)),
        alone_picker("sit-sweep"),
        RunHooks::on_state(|s| {
            let Some(sitting) = s.hand.sitting_seat else {
                return;
            };
            let view = project(s, sitting);
            assert!(view.hand.is_empty());
            assert_eq!(view.hand_counts[sitting.index()], 0);
            let payload = json(&view);
            for &c in &s.hand.hands[sitting.index()] {
                if has(&payload, c) && Some(c) != s.hand.up_card {
                    leaks.push(format!(
                        "{}: seat {sitting} saw its own dead card {c}",
                        s.hand.phase
                    ));
                }
            }
            checked += 1;
        }),
    );
    assert!(leaks.is_empty(), "{leaks:?}");
    assert!(checked > 100);
}

/* -------------------------------------------------------------------------- */
/* V12 - the redaction sweep                                                   */
/* -------------------------------------------------------------------------- */

#[derive(Default)]
struct SweepTally {
    states: usize,
    views: usize,
    foreign_checks: usize,
    kitty_checks: usize,
    discard_checks: usize,
    step_scans: usize,
    sitting_states: usize,
    leaks: Vec<String>,
}

/// Every way `json`, a payload addressed to `seat`, betrays a card that seat is
/// not entitled to in the state `s`. Empty means clean.
fn payload_leaks(
    s: &GameState,
    seat: Seat,
    json: &str,
    mut tally: Option<&mut SweepTally>,
) -> Vec<String> {
    let h = &s.hand;
    let mut found = Vec::new();
    let place = format!("h{} {} seat{}", s.hand_no, h.phase, seat);

    // 1. The literal requirement: not one hidden card id appears in the bytes.
    for c in hidden_from(s, seat) {
        if has(json, c) {
            found.push(format!("{place}: hidden {c} in payload"));
        }
    }

    // 2. The same fact, stated by category, so a failure names the leak.
    for other in SEATS_ALL {
        if other == seat {
            continue;
        }
        for &c in &h.hands[other.index()] {
            if let Some(t) = tally.as_deref_mut() {
                t.foreign_checks += 1;
            }
            if Some(c) == h.up_card {
                continue; // public: the dealer's picked-up up-card
            }
            if has(json, c) {
                found.push(format!("{place}: hand of {other} ({c})"));
            }
        }
    }
    for &c in &h.kitty[1..] {
        if let Some(t) = tally.as_deref_mut() {
            t.kitty_checks += 1;
        }
        if has(json, c) {
            found.push(format!("{place}: buried kitty {c}"));
        }
    }
    if let Some(d) = h.dealer_discard
        && Some(d) != h.up_card
    {
        if let Some(t) = tally {
            t.discard_checks += 1;
        }
        if has(json, d) {
            found.push(format!("{place}: dealer discard {d}"));
        }
    }

    // 3. And the complement, which cannot be weakened by adding a field.
    let vis = visible_to(s, seat);
    for c in card_tokens_in(json) {
        if !vis.contains(&c) {
            found.push(format!("{place}: unentitled token {c}"));
        }
    }
    found
}

fn sweep(initial: GameState, pick: Picker, tally: &mut SweepTally) -> GameState {
    let tally = std::cell::RefCell::new(tally);
    let note = |t: &mut SweepTally, m: String| {
        if t.leaks.len() < 20 {
            t.leaks.push(m);
        }
    };

    run_game(
        initial,
        pick,
        RunHooks {
            on_state: Some(Box::new(|s| {
                let mut t = tally.borrow_mut();
                t.states += 1;
                if s.hand.sitting_seat.is_some() {
                    t.sitting_states += 1;
                }
                for seat in SEATS_ALL {
                    t.views += 1;
                    let payload = json(&project(s, seat));
                    for m in payload_leaks(s, seat, &payload, Some(&mut t)) {
                        note(&mut t, m);
                    }
                }
            })),
            on_transition: Some(Box::new(|_before, res, _| {
                if res.steps.is_empty() {
                    return;
                }
                let mut t = tally.borrow_mut();
                let after = &res.state;
                for seat in SEATS_ALL {
                    let redacted = project_steps(&res.steps, seat);
                    assert_eq!(redacted.len(), res.steps.len());
                    let payload = json(&redacted);
                    t.step_scans += 1;
                    let vis = visible_to(after, seat);
                    for c in card_tokens_in(&payload) {
                        if !vis.contains(&c) {
                            note(
                                &mut t,
                                format!(
                                    "h{} steps for seat{seat}: unentitled token {c}",
                                    after.hand_no
                                ),
                            );
                        }
                    }
                }
            })),
            budget: None,
        },
    )
}

fn contains_line(found: &[String], needle: &str) -> bool {
    found.iter().any(|m| m.contains(needle))
}

#[test]
fn negative_control_the_detector_fires_on_every_category_it_claims_to_cover() {
    // A real mid-hand state with a loner, a discard, and a buried kitty.
    let base = cut_no("control");
    let called = ok(&base, seat(1), Move::OrderUp { alone: true }).state;
    let drop = *called.hand.hands[0]
        .iter()
        .find(|&&c| Some(c) != called.hand.up_card)
        .unwrap();
    let s = ok(&called, seat(0), Move::Discard { card: drop }).state;

    assert_eq!(s.hand.sitting_seat, Some(seat(3)));
    assert_eq!(s.hand.dealer_discard, Some(drop));
    let view0 = project(&s, seat(0));
    assert!(payload_leaks(&s, seat(0), &json(&view0), None).is_empty());

    let clean = json(&view0);
    let widen = |extra: Value| widened(&view0, j!({ "extra": extra }));

    // A foreign hand.
    let stolen = s.hand.hands[2][0];
    assert!(!has(&clean, stolen));
    assert!(has(&widen(j!({ "opponent": stolen })), stolen));
    assert!(contains_line(
        &payload_leaks(&s, seat(0), &widen(j!({ "opponent": stolen })), None),
        &format!("hand of 2 ({stolen})")
    ));

    // A buried kitty card.
    let buried = s.hand.kitty[2];
    assert!(contains_line(
        &payload_leaks(&s, seat(0), &widen(j!({ "kitty": buried })), None),
        &format!("buried kitty {buried}")
    ));

    // The dealer's own discard, projected back to the dealer.
    assert!(contains_line(
        &payload_leaks(&s, seat(0), &widen(j!({ "discarded": drop })), None),
        &format!("dealer discard {drop}")
    ));

    // The sitting partner's dead hand, projected back to its owner.
    let dead = s.hand.hands[3][0];
    let view3 = project(&s, seat(3));
    assert!(payload_leaks(&s, seat(3), &json(&view3), None).is_empty());
    assert!(!payload_leaks(&s, seat(3), &widened(&view3, j!({ "hand": [dead] })), None).is_empty());

    // And an id-shaped leak, not just a bare card field.
    let id_leak = widen(j!({ "legal": [{ "id": format!("play:{stolen}") }] }));
    assert!(!payload_leaks(&s, seat(0), &id_leak, None).is_empty());
}

#[test]
fn leaks_nothing_across_a_full_match_every_state_every_seat() {
    let mut tally = SweepTally::default();
    let fin = sweep(
        new_game("redact-1", None),
        random_picker("redact-1"),
        &mut tally,
    );

    assert!(tally.leaks.is_empty(), "{:?}", tally.leaks);
    assert_eq!(fin.status, GameStatus::Complete);
    // The sweep must actually have exercised each hidden category.
    assert!(tally.states > 200);
    assert_eq!(tally.views, tally.states * 4);
    assert!(tally.foreign_checks > 1000);
    assert!(tally.kitty_checks > 1000);
    assert!(tally.discard_checks > 50);
    assert!(tally.step_scans > 200);
}

#[test]
fn leaks_nothing_when_every_hand_is_played_as_a_loner() {
    let mut tally = SweepTally::default();
    let fin = sweep(
        new_game("redact-loner", None),
        alone_picker("redact-loner"),
        &mut tally,
    );
    assert!(tally.leaks.is_empty(), "{:?}", tally.leaks);
    assert_eq!(fin.status, GameStatus::Complete);
    assert!(tally.sitting_states > 100);
}

#[test]
fn leaks_nothing_for_any_of_the_four_sitting_seats_or_any_first_dealer() {
    let mut tally = SweepTally::default();
    for dealer in SEATS_ALL {
        for skip in 0..4u8 {
            let seed = format!("redact-d{dealer}-s{skip}");
            sweep(
                create_game(GameInit {
                    game_id: format!("g-{seed}"),
                    seed: seed.clone(),
                    first_dealer: Some(dealer),
                    cfg: cfg_game_to(5),
                    internal_token: None,
                }),
                alone_at_picker(&seed, skip),
                &mut tally,
            );
        }
    }
    assert!(tally.leaks.is_empty(), "{:?}", tally.leaks);
    assert!(tally.states > 1000);
    assert!(tally.sitting_states > 500);
}

#[test]
fn leaks_nothing_with_stick_the_dealer_off_where_hands_can_be_thrown_in() {
    let mut tally = SweepTally::default();
    let fin = sweep(
        new_game(
            "redact-throwin",
            Some(euchre_engine::EngineConfigOverrides {
                stick_the_dealer: Some(false),
                game_to: Some(7),
                ..Default::default()
            }),
        ),
        random_picker("redact-throwin"),
        &mut tally,
    );
    assert!(tally.leaks.is_empty(), "{:?}", tally.leaks);
    assert_eq!(fin.status, GameStatus::Complete);
    assert!(tally.states > 100);
}

/* -------------------------------------------------------------------------- */
/* projectSteps                                                                */
/* -------------------------------------------------------------------------- */

#[test]
fn blanks_every_deal_packet_not_addressed_to_the_receiving_seat() {
    let s = opened("steps", Some(seat(0)), None);
    let res = ok(&s, cutter_of(seat(0)), Move::Cut { cut: false });
    let Some(Step::Dealt { packets: dealt }) =
        res.steps.iter().find(|x| matches!(x, Step::Dealt { .. }))
    else {
        panic!("expected a dealt step")
    };
    assert!(dealt.iter().all(|p| p.cards.is_some()));

    for seat in SEATS_ALL {
        let redacted = project_steps(&res.steps, seat);
        let Some(Step::Dealt { packets: mine }) =
            redacted.iter().find(|x| matches!(x, Step::Dealt { .. }))
        else {
            panic!("expected a dealt step")
        };
        assert_eq!(mine.len(), 8);
        for (i, p) in mine.iter().enumerate() {
            let original = &dealt[i];
            // Seat and count always survive, so the deal still animates faithfully.
            assert_eq!(p.seat, original.seat);
            assert_eq!(p.count, original.count);
            if p.seat == seat {
                assert_eq!(p.cards, original.cards);
                assert_eq!(p.cards.as_ref().map(Vec::len), Some(usize::from(p.count)));
            } else {
                assert_eq!(p.cards, None);
            }
        }
        let own: Vec<Card> = mine
            .iter()
            .filter(|p| p.seat == seat)
            .flat_map(|p| p.cards.clone().unwrap_or_default())
            .collect();
        assert_eq!(own, res.state.hand.hands[seat.index()]);
        // Nothing but this seat's own five cards is anywhere in the payload.
        let tokens: Vec<Card> = card_tokens_in(&json(&Step::Dealt {
            packets: mine.clone(),
        }))
        .into_iter()
        .collect();
        assert_eq!(tokens, sorted(&own));
    }
}

#[test]
fn passes_every_non_dealt_step_through_by_reference() {
    // TS also asserts reference identity (`toBe`); Rust asserts value identity.
    let s = cut_no("steps-2");
    let bid = ok(&s, seat(1), Move::OrderUp { alone: true });
    let tags: Vec<&str> = bid.steps.iter().map(Step::tag).collect();
    assert_eq!(tags, ["bid", "trumpSet"]);
    for seat in SEATS_ALL {
        let redacted = project_steps(&bid.steps, seat);
        assert_eq!(redacted, bid.steps);
        assert_eq!(redacted[0], bid.steps[0]);
        assert_eq!(redacted[1], bid.steps[1]);
    }
}

#[test]
fn cannot_name_the_dealer_discard_the_step_has_no_card_field() {
    let base = cut_no("steps-3");
    let picked = ok(&base, seat(1), Move::OrderUp { alone: false }).state;
    let drop = picked.hand.hands[0][2];
    let res = ok(&picked, seat(0), Move::Discard { card: drop });

    assert_eq!(res.steps, vec![Step::DealerDiscarded { seat: seat(0) }]);
    for seat in SEATS_ALL {
        let payload = json(&project_steps(&res.steps, seat));
        assert!(!has(&payload, drop));
        assert!(card_tokens_in(&payload).is_empty());
    }
}

#[test]
fn never_mutates_the_steps_it_is_given() {
    let s = opened("steps-4", Some(seat(0)), None);
    let res = ok(&s, cutter_of(seat(0)), Move::Cut { cut: false });
    let before = json(&res.steps);
    project_steps(&res.steps, seat(0));
    project_steps(&res.steps, seat(2));
    assert_eq!(json(&res.steps), before);
}
