//! Port of `src/lib/ai/banter-safety.test.ts`.

mod common;

use common::*;
use euchre_ai::screen::ai::{ScreenResult, screen_banter};
use euchre_engine::project;

fn view() -> euchre_engine::PublicGameView {
    project(&game("banter", "banter", None), seat(0))
}

fn ok(text: &str) -> bool {
    matches!(screen_banter(text, &view()), ScreenResult::Ok(_))
}

#[test]
fn blocks_advice_your_call_partner() {
    assert!(!ok("Your call, partner."));
}
#[test]
fn blocks_advice_your_choice_partner() {
    assert!(!ok("Your choice, partner."));
}
#[test]
fn blocks_advice_you_should_take_it() {
    assert!(!ok("You should take it."));
}
#[test]
fn blocks_advice_play_your_trump() {
    assert!(!ok("Play your trump."));
}
#[test]
fn blocks_advice_save_the_ace() {
    assert!(!ok("Save the ace."));
}
#[test]
fn blocks_advice_go_alone() {
    assert!(!ok("Go alone!"));
}
#[test]
fn allows_a_public_social_reaction() {
    assert!(ok("Good game. Well played."));
}
