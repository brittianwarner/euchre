//! Port of the pure parts of `src/lib/ai/partnership.test.ts`: "partnership
//! decision evidence" and "conservation prior regressions". The two "Jev
//! integration" tests need a scripted Jev model and belong to the LLM stage.

mod common;

use common::*;
use euchre_ai::partnership::{PartnershipAnalysis, PartnershipParams, analyze_partnership};
use euchre_ai::{RankParams, rank_moves};
use euchre_engine::{GamePhase, Play, PublicGameView, Team, Trick};

fn analyze(view: &PublicGameView) -> PartnershipAnalysis {
    analyze_partnership(PartnershipParams {
        view,
        legal: &view.legal,
    })
    .unwrap()
}

fn option<'a>(
    a: &'a PartnershipAnalysis,
    card: &str,
) -> &'a euchre_ai::partnership::PartnershipOption {
    a.options.iter().find(|o| o.card == c(card)).unwrap()
}

fn top(view: &PublicGameView) -> String {
    rank_moves(RankParams::new(view)).remove(0).id
}

fn trick(index: u8, led: char, winner: u8, plays: Vec<Play>) -> Trick {
    Trick {
        index,
        led_suit: Some(suit(led)),
        plays,
        winner_seat: Some(seat(winner)),
    }
}

#[test]
fn marks_a_last_seat_partner_winner_secure_and_distinguishes_overtaking_from_laying_off() {
    let a = analyze(&partnership_cases()[0].view);
    assert_eq!(a.context.partner_seat, seat(0));
    assert!(a.context.partner_winning);
    assert!(a.context.partner_trick_secured);
    assert!(a.context.opponents_after_you.is_empty());
    assert!(a.context.possible_opponent_beaters.is_empty());
    let ts = option(&a, "TS");
    assert_eq!(ts.winner_after_play, play(0, "KS"));
    assert!(!ts.overtakes_partner);
    assert_eq!(ts.remaining_hand, cards(&["AS", "9C", "KC", "AD"]));
    assert!(option(&a, "AS").overtakes_partner);
}

#[test]
fn does_not_mistake_a_vulnerable_lead_for_a_won_trick_shows_how_covering_helps() {
    let a = analyze(&partnership_cases()[4].view);
    assert!(!a.context.partner_trick_secured);
    assert_eq!(a.context.opponents_after_you, vec![seat(3)]);
    for card in ["AH", "JD"] {
        assert!(a.context.possible_opponent_beaters.contains(&c(card)));
    }
    let jh = option(&a, "JH");
    assert!(jh.overtakes_partner);
    assert!(jh.possible_opponent_beaters_after_play.is_empty());
}

#[test]
fn recognizes_an_unbeatable_partner_even_with_opponents_left_to_play() {
    let a = analyze(&partnership_cases()[3].view);
    assert_eq!(a.context.opponents_after_you, vec![seat(3)]);
    assert!(a.context.partner_trick_secured);
    assert!(a.context.possible_opponent_beaters.is_empty());
}

#[test]
fn saves_the_right_bower_when_a_nine_already_wins_from_the_last_seat() {
    let view = play_view(
        &["JH", "AH", "9H"],
        &[play(3, "KS"), play(0, "QS"), play(1, "AS")],
        None,
        |_| {},
    );
    let a = analyze(&view);
    assert!(a.context.opponents_after_you.is_empty());
    let jh = option(&a, "JH");
    for card in ["9H", "AH"] {
        assert!(jh.cheaper_equivalent_cards.contains(&c(card)));
    }
}

#[test]
fn identifies_a_cheaper_equivalent_without_treating_necessary_cover_as_equivalent() {
    let cases = partnership_cases();
    let a3 = analyze(&cases[3].view);
    let jd = option(&a3, "JD");
    assert_eq!(jd.resource, "left bower, second-highest trump");
    assert_eq!(jd.cheaper_equivalent_cards, cards(&["9H"]));
    let a5 = analyze(&cases[5].view);
    assert_eq!(option(&a5, "AS").cheaper_equivalent_cards, cards(&["KS"]));
    let a4 = analyze(&cases[4].view);
    assert!(option(&a4, "JH").cheaper_equivalent_cards.is_empty());
}

fn recognizes_partners_relative_to(s: u8) {
    let base = &partnership_cases()[0].view;
    let rotate = |x: u8| seat((x + s + 2) % 4);
    let mut view = base.clone();
    view.you = seat(s);
    view.turn_seat = Some(seat(s));
    for p in &mut view.trick.plays {
        p.seat = rotate(p.seat.get());
    }
    let a = analyze(&view);
    assert_eq!(a.context.partner_seat, rotate(0));
    assert_eq!(a.context.your_team, Team::new(s % 2).unwrap());
    assert!(a.context.partner_trick_secured);
}

#[test]
fn recognizes_partners_relative_to_seat_0() {
    recognizes_partners_relative_to(0);
}
#[test]
fn recognizes_partners_relative_to_seat_1() {
    recognizes_partners_relative_to(1);
}
#[test]
fn recognizes_partners_relative_to_seat_2() {
    recognizes_partners_relative_to(2);
}
#[test]
fn recognizes_partners_relative_to_seat_3() {
    recognizes_partners_relative_to(3);
}

#[test]
fn uses_the_left_bower_as_trump_when_determining_winners_and_following_suit() {
    let view = play_view(
        &["9H", "AD", "9C"],
        &[play(0, "JD"), play(1, "AH")],
        None,
        |_| {},
    );
    assert_eq!(ids(&view), vec!["play:9H"]);
    let a = analyze(&view);
    assert_eq!(a.context.current_winner, Some(play(0, "JD")));
    assert_eq!(a.context.possible_opponent_beaters, cards(&["JH"]));
    assert!(!a.context.partner_trick_secured);
}

#[test]
fn does_not_count_an_already_played_or_turned_down_bower_as_a_threat() {
    let view = play_view(&["9H", "AD"], &[play(0, "JD"), play(1, "AH")], None, |v| {
        v.up_card = Some(c("JH"));
        v.up_card_turned_down = true;
    });
    assert!(analyze(&view).context.partner_trick_secured);
    let mut up = view.clone();
    up.up_card_turned_down = false;
    assert!(!analyze(&up).context.partner_trick_secured);
    let mut played = up.clone();
    played.trick_log = vec![trick(1, 'H', 0, vec![play(0, "JH")])];
    assert!(analyze(&played).context.partner_trick_secured);
}

#[test]
fn uses_public_void_evidence_conservatively_instead_of_inventing_enemy_holdings() {
    let view = play_view(&["9H", "AD"], &[play(0, "JD"), play(1, "AH")], None, |v| {
        v.trick_log = vec![trick(1, 'H', 0, vec![play(0, "KH"), play(3, "9C")])];
    });
    let a = analyze(&view);
    assert!(a.context.partner_trick_secured);
    assert!(a.context.unseen_cards_are_possibilities_not_known_holdings);
}

#[test]
fn skips_a_sitting_seat_and_uses_the_acting_team_rather_than_always_the_human_team() {
    let view = play_view(&["AH", "9C"], &[play(0, "KH")], None, |v| {
        v.you = seat(1);
        v.turn_seat = Some(seat(1));
        v.alone_seat = Some(seat(0));
        v.sitting_seat = Some(seat(2));
        v.maker_seat = Some(seat(0));
        v.tricks_won = [2, 1];
    });
    let ctx = analyze(&view).context;
    assert_eq!(ctx.your_team, Team::EW);
    assert_eq!(ctx.partner_seat, seat(3));
    assert_eq!(ctx.seats_after_you, vec![seat(3)]);
    assert!(ctx.opponents_after_you.is_empty());
    assert_eq!(ctx.your_team_tricks, 1);
    assert_eq!(ctx.opponent_tricks, 2);
    assert!(!ctx.partner_trick_secured);
    assert!(ctx.loner_sweep_stopped);
}

#[test]
fn keeps_the_sweep_objective_after_makers_have_already_made_the_hand() {
    let mut view = partnership_cases()[0].view.clone();
    view.tricks_won = [3, 0];
    let ctx = analyze(&view).context;
    assert!(ctx.your_team_has_won_hand);
    assert!(ctx.makers_have_three_tricks);
    assert!(ctx.makers_can_still_sweep);
    view.tricks_won = [3, 1];
    assert!(!analyze(&view).context.makers_can_still_sweep);
}

#[test]
fn does_not_attach_trick_advice_during_bidding() {
    let mut view = partnership_cases()[0].view.clone();
    view.phase = GamePhase::BidRound1;
    assert!(
        analyze_partnership(PartnershipParams {
            view: &view,
            legal: &view.legal,
        })
        .is_none()
    );
}

#[test]
fn conservation_prior_regressions_rank_the_expected_card_first() {
    for (i, case) in partnership_cases().iter().enumerate() {
        if i == 4 {
            continue;
        }
        assert_eq!(
            top(&case.view),
            case.expected.as_str().unwrap(),
            "{}",
            case.name
        );
    }
}

#[test]
fn does_not_spend_low_trump_when_a_plain_loser_can_be_discarded() {
    let view = play_view(
        &["9H", "QC", "AC"],
        &[play(3, "9S"), play(0, "AS"), play(1, "KS")],
        None,
        |_| {},
    );
    assert_eq!(top(&view), "play:QC");
}

#[test]
fn does_not_overtake_a_partner_to_stop_a_loner_sweep_the_partner_already_stopped() {
    let view = play_view(
        &["KH", "JD", "9C"],
        &[play(3, "9H"), play(0, "AH")],
        None,
        |v| {
            v.alone_seat = Some(seat(3));
            v.sitting_seat = Some(seat(1));
            v.maker_seat = Some(seat(3));
        },
    );
    assert_eq!(top(&view), "play:KH");
    assert!(analyze(&view).context.partner_trick_secured);
}
