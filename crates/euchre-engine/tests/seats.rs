//! Port of `__tests__/seats.test.ts`: seat arithmetic, teams, and loner-aware turn order.

mod common;

use common::seat;
use euchre_engine::{
    HUMAN_SEAT, SEATS, Seat, SeatRole, Team, active_seat_count, bidding_order, cutter_of,
    eldest_of, first_active_from, is_active_seat, next_active_seat, next_seat, opponent_team,
    partner_of, seat_role, team_of,
};

const ALL: [Seat; 4] = Seat::ALL;
/// Every `sitting_seat` value the engine can present, including "no loner".
const SITTINGS: [Option<Seat>; 5] = [
    None,
    Some(Seat::SOUTH),
    Some(Seat::WEST),
    Some(Seat::NORTH),
    Some(Seat::EAST),
];

fn nums(xs: &[Seat]) -> Vec<u8> {
    xs.iter().map(|s| s.get()).collect()
}

fn sorted_nums(xs: &[Seat]) -> Vec<u8> {
    let mut v = nums(xs);
    v.sort_unstable();
    v
}

/* seat constants */

#[test]
fn the_human_always_sits_at_seat_0_south() {
    assert_eq!(HUMAN_SEAT.get(), 0);
}

#[test]
fn seats_is_0_1_2_3_clockwise() {
    // The TS `Object.isFrozen` half has no Rust equivalent: a `const` array is immutable.
    assert_eq!(nums(&SEATS), vec![0, 1, 2, 3]);
}

/* nextSeat */

#[test]
fn next_seat_goes_0_1_2_3_0() {
    assert_eq!(nums(&ALL.map(next_seat)), vec![1, 2, 3, 0]);
}

#[test]
fn returns_to_the_start_after_exactly_four_steps_from_every_seat() {
    for s in ALL {
        let mut cur = s;
        let mut visited = Vec::new();
        for _ in 0..4 {
            cur = next_seat(cur);
            visited.push(cur);
        }
        assert_eq!(cur, s);
        assert_eq!(sorted_nums(&visited), vec![0, 1, 2, 3]);
    }
}

/* partnerOf */

#[test]
fn partner_of_pairs_0_2_and_1_3() {
    assert_eq!(nums(&ALL.map(partner_of)), vec![2, 3, 0, 1]);
}

#[test]
fn partner_of_is_an_involution_and_never_self_partners() {
    for s in ALL {
        assert_eq!(partner_of(partner_of(s)), s);
        assert_ne!(partner_of(s), s);
    }
}

#[test]
fn a_seat_and_its_partner_are_always_the_same_team() {
    for s in ALL {
        assert_eq!(team_of(partner_of(s)), team_of(s));
    }
}

/* teamOf / opponentTeam */

#[test]
fn puts_0_2_on_team_0_and_1_3_on_team_1() {
    let teams: Vec<u8> = ALL.iter().map(|&s| team_of(s).get()).collect();
    assert_eq!(teams, vec![0, 1, 0, 1]);
    assert_eq!(team_of(HUMAN_SEAT).get(), 0);
}

#[test]
fn opponent_team_flips_0_1_and_is_an_involution() {
    assert_eq!(opponent_team(Team::NS).get(), 1);
    assert_eq!(opponent_team(Team::EW).get(), 0);
    for t in Team::ALL {
        assert_eq!(opponent_team(opponent_team(t)), t);
    }
}

#[test]
fn adjacent_seats_are_always_opponents() {
    for s in ALL {
        assert_eq!(team_of(next_seat(s)), opponent_team(team_of(s)));
    }
}

/* seatRole */

#[test]
fn seat_role_is_dealer_eldest_partner_third_for_every_dealer() {
    for dealer in ALL {
        assert_eq!((dealer, seat_role(dealer, dealer).get()), (dealer, 0));
        assert_eq!(
            (dealer, seat_role(eldest_of(dealer), dealer).get()),
            (dealer, 1)
        );
        assert_eq!(
            (dealer, seat_role(partner_of(dealer), dealer).get()),
            (dealer, 2)
        );
        assert_eq!(
            (dealer, seat_role(cutter_of(dealer), dealer).get()),
            (dealer, 3)
        );
    }
}

#[test]
fn matches_the_spec_example_seat_1_is_eldest_when_seat_0_deals() {
    assert_eq!(seat_role(seat(1), seat(0)), SeatRole::Eldest);
    assert_eq!(seat_role(seat(1), seat(0)).get(), 1);
}

#[test]
fn seat_role_is_a_bijection_from_seats_onto_roles_for_every_dealer() {
    for dealer in ALL {
        let mut roles: Vec<u8> = ALL.iter().map(|&s| seat_role(s, dealer).get()).collect();
        roles.sort_unstable();
        assert_eq!(roles, vec![0, 1, 2, 3]);
    }
}

/* eldestOf / cutterOf */

#[test]
fn eldest_is_left_of_the_dealer() {
    assert_eq!(nums(&ALL.map(eldest_of)), vec![1, 2, 3, 0]);
}

#[test]
fn the_cutter_is_right_of_the_dealer() {
    assert_eq!(nums(&ALL.map(cutter_of)), vec![3, 0, 1, 2]);
}

#[test]
fn the_cutter_is_an_opponent_of_the_dealer_eldest_is_too() {
    for d in ALL {
        assert_eq!(team_of(cutter_of(d)), opponent_team(team_of(d)));
        assert_eq!(team_of(eldest_of(d)), opponent_team(team_of(d)));
    }
}

/* biddingOrder */

#[test]
fn bidding_order_is_eldest_partner_third_dealer_for_every_dealer() {
    assert_eq!(nums(&bidding_order(seat(0))), vec![1, 2, 3, 0]);
    assert_eq!(nums(&bidding_order(seat(1))), vec![2, 3, 0, 1]);
    assert_eq!(nums(&bidding_order(seat(2))), vec![3, 0, 1, 2]);
    assert_eq!(nums(&bidding_order(seat(3))), vec![0, 1, 2, 3]);
}

#[test]
fn always_seats_the_dealer_last_and_eldest_first() {
    for d in ALL {
        let order = bidding_order(d);
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], eldest_of(d));
        assert_eq!(order[3], d);
        assert_eq!(sorted_nums(&order), vec![0, 1, 2, 3]);
    }
}

#[test]
fn bidding_order_returns_a_fresh_array_each_call() {
    // TS also checks `not.toBe` (reference identity); a Rust array is returned by value.
    let mut a = bidding_order(seat(0));
    let b = bidding_order(seat(0));
    assert_eq!(a, b);
    a[0] = seat(3);
    assert_eq!(a[0].get(), 3);
    assert_eq!(bidding_order(seat(0))[0].get(), 1);
}

/* firstActiveFrom: the trick-1 lead under a loner */

/// `sitting_seat` is always `partner_of(alone_seat)` (V4).
fn lead(dealer: Seat, alone: Seat) -> Seat {
    first_active_from(eldest_of(dealer), Some(partner_of(alone)))
}

#[test]
fn loner_is_eldest_seat_1_leads() {
    assert_eq!(partner_of(seat(1)).get(), 3);
    assert_eq!(lead(seat(0), seat(1)).get(), 1);
}

#[test]
fn loner_is_dealers_partner_seat_1_leads() {
    assert_eq!(partner_of(seat(2)).get(), 0);
    assert_eq!(lead(seat(0), seat(2)).get(), 1);
}

#[test]
fn loner_is_third_seat_seat_2_leads_a_defender() {
    assert_eq!(partner_of(seat(3)).get(), 1);
    let l = lead(seat(0), seat(3));
    assert_eq!(l.get(), 2);
    assert_eq!(l, partner_of(seat(0)));
    assert_ne!(l.get(), 3);
    assert_eq!(team_of(l), opponent_team(team_of(seat(3))));
}

#[test]
fn loner_is_the_dealer_seat_1_leads() {
    assert_eq!(partner_of(seat(0)).get(), 2);
    assert_eq!(lead(seat(0), seat(0)).get(), 1);
}

#[test]
fn first_active_from_never_returns_the_sitting_seat_for_every_pair() {
    for s in ALL {
        for sitting in SITTINGS {
            assert_ne!(
                Some(first_active_from(s, sitting)),
                sitting,
                "{s:?} {sitting:?}"
            );
        }
    }
}

#[test]
fn first_active_from_is_the_identity_whenever_the_seat_is_active() {
    for s in ALL {
        assert_eq!(first_active_from(s, None), s);
        for sitting in ALL {
            if sitting == s {
                continue;
            }
            assert_eq!(
                (s, sitting, first_active_from(s, Some(sitting))),
                (s, sitting, s)
            );
        }
    }
}

#[test]
fn first_active_from_skips_exactly_one_seat_clockwise_when_the_seat_is_sitting_out() {
    for s in ALL {
        assert_eq!(first_active_from(s, Some(s)), next_seat(s));
    }
}

#[test]
fn holds_for_every_dealer_the_lead_is_never_the_sitting_seat() {
    for dealer in ALL {
        for alone in ALL {
            let sitting = partner_of(alone);
            let l = lead(dealer, alone);
            assert_eq!((dealer, alone, l == sitting), (dealer, alone, false));
            let want = if eldest_of(dealer) == sitting {
                next_seat(eldest_of(dealer))
            } else {
                eldest_of(dealer)
            };
            assert_eq!((dealer, alone, l), (dealer, alone, want));
        }
    }
}

/* nextActiveSeat */

#[test]
fn next_active_seat_is_plain_next_seat_when_nobody_is_sitting_out() {
    for s in ALL {
        assert_eq!(next_active_seat(s, None), next_seat(s));
    }
}

#[test]
fn next_active_seat_never_returns_the_sitting_seat_v8() {
    for s in ALL {
        for sitting in SITTINGS {
            let got = next_active_seat(s, sitting);
            assert_eq!((s, sitting, Some(got) == sitting), (s, sitting, false));
        }
    }
}

#[test]
fn skips_exactly_one_seat_when_the_next_seat_is_sitting_out() {
    assert_eq!(next_active_seat(seat(2), Some(seat(3))).get(), 0);
    assert_eq!(next_active_seat(seat(0), Some(seat(1))).get(), 2);
    assert_eq!(next_active_seat(seat(1), Some(seat(2))).get(), 3);
    assert_eq!(next_active_seat(seat(3), Some(seat(0))).get(), 1);
}

#[test]
fn never_skips_when_the_next_seat_is_active() {
    for s in ALL {
        for sitting in ALL {
            if next_seat(s) == sitting {
                continue;
            }
            assert_eq!(
                (s, sitting, next_active_seat(s, Some(sitting))),
                (s, sitting, next_seat(s))
            );
        }
    }
}

#[test]
fn is_total_even_when_called_from_the_sitting_seat_itself() {
    for s in ALL {
        assert_eq!(next_active_seat(s, Some(s)), next_seat(s));
    }
}

#[test]
fn cycles_through_exactly_the_3_active_seats_under_a_loner_and_returns_to_the_lead() {
    for sitting in ALL {
        let lead = next_seat(sitting);
        let mut cur = lead;
        let mut visited = vec![lead];
        for _ in 0..2 {
            cur = next_active_seat(cur, Some(sitting));
            visited.push(cur);
        }
        assert_eq!(visited.len(), 3);
        let mut uniq = sorted_nums(&visited);
        uniq.dedup();
        assert_eq!(uniq.len(), 3);
        assert!(!visited.contains(&sitting));
        assert_eq!(next_active_seat(cur, Some(sitting)), lead);
    }
}

#[test]
fn cycles_through_all_4_seats_with_no_loner() {
    let mut cur = seat(0);
    let mut visited = vec![cur];
    for _ in 0..3 {
        cur = next_active_seat(cur, None);
        visited.push(cur);
    }
    assert_eq!(nums(&visited), vec![0, 1, 2, 3]);
    assert_eq!(next_active_seat(cur, None).get(), 0);
}

/* activeSeatCount / isActiveSeat */

#[test]
fn active_seat_count_is_4_with_no_loner_and_3_with_one() {
    assert_eq!(active_seat_count(None), 4);
    for s in ALL {
        assert_eq!(active_seat_count(Some(s)), 3);
    }
}

#[test]
fn active_seat_count_equals_the_number_of_seats_is_active_seat_admits() {
    for sitting in SITTINGS {
        let n = ALL.iter().filter(|&&s| is_active_seat(s, sitting)).count();
        assert_eq!(
            (sitting, n),
            (sitting, usize::from(active_seat_count(sitting)))
        );
    }
}

#[test]
fn implies_20_cards_played_per_hand_normally_and_15_under_a_loner_v10() {
    assert_eq!(u32::from(active_seat_count(None)) * 5, 20);
    assert_eq!(u32::from(active_seat_count(Some(seat(2)))) * 5, 15);
}

#[test]
fn is_active_seat_is_false_only_for_the_sitting_seat() {
    for s in ALL {
        assert!(is_active_seat(s, None));
        for sitting in ALL {
            assert_eq!(
                (s, sitting, is_active_seat(s, Some(sitting))),
                (s, sitting, s != sitting)
            );
        }
    }
}

/* Rust-only: the newtype rejects out-of-range numbers on the wire. */

#[test]
fn seat_and_team_serde_are_plain_numbers_and_validate_range() {
    assert_eq!(serde_json::to_string(&seat(3)).ok(), Some("3".to_owned()));
    assert_eq!(serde_json::from_str::<Seat>("2").ok(), Some(seat(2)));
    assert!(serde_json::from_str::<Seat>("4").is_err());
    assert!(serde_json::from_str::<Team>("2").is_err());
    assert_eq!(
        serde_json::from_str::<SeatRole>("3").ok(),
        Some(SeatRole::Third)
    );
    assert!(Seat::new(4).is_none());
}
