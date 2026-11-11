//! The screen-reader announcer (06 §10): every game step, as one plain sentence, spoken
//! politely. app.rs `poll` calls [`take`] every frame with the table's hooks and, when it
//! returns a line, hands it to `Presentation::announce_line` (which bumps an id and keeps the
//! text in `Presentation.announced`); table/mod.rs mounts [`layer`] with that pair.
//!
//! How Rune (rev 85cd8c6) speaks it, read from `crates/rune/src`:
//! - There is no imperative "speak this string" call. A screen reader hears a node marked
//!   live ([`El::live_region`] `LiveRegion::Polite`, ARIA `aria-live=polite`) appear with a
//!   name, or change its name. AccessKit's platform adapters (VoiceOver through the macOS
//!   adapter, Narrator/NVDA through UI Automation) do the speaking. So the native port has the
//!   06 §10 region itself, not a fallback.
//! - A node that is `a11y_hidden`, at opacity ≤ 0.01, exiting, or outside an open modal is
//!   left out of the tree, so the region stays at full opacity, 1×1 points, pinned out of
//!   flow with no fill: present to the tree, invisible on screen.
//! - Saying the same line twice in a row needs a fresh node: the line's child is keyed by the
//!   bumped id (`announcer-{id}`) and mounts with no fade, so it is in the tree on its first
//!   frame and the old one leaves at once.
//! - The gap: while the first-run tour (a modal) is open, the table behind it is inert and
//!   steps that land then are not spoken. The tour is the player's focus at that moment.

use euchre_engine::cards::bowers_of;
use euchre_engine::score::{is_game_over, is_in_the_barn};
use euchre_engine::seats::team_of;
use euchre_engine::{DEFAULT_ENGINE_CONFIG, card_name_lower, suit_name, suit_noun};
use euchre_protocol::{HandResult, PublicGameView, Seat, Step};
use rune::{El, LiveRegion, Transition, el};

use super::copy::seat_name;
use super::present::Hooks;

/// `RESULT_LABEL`: the hand result's opening word (06 §10).
pub fn result_label(result: HandResult) -> &'static str {
    match result {
        HandResult::Point => "Point.",
        HandResult::LonePoint => "Lone point.",
        HandResult::March => "March!",
        HandResult::LoneMarch => "Lone march!",
        HandResult::Euchre => "Euchre!",
        HandResult::ThrowIn => "Thrown in.",
    }
}

/// The sentence for one step, or `None` for a silent one (a bid: the table talk already
/// carries its call line; a deal with nothing in your hand).
pub fn sentence(step: &Step, view: &PublicGameView) -> Option<String> {
    let you = view.you;
    let mine = team_of(you).index();
    Some(match step {
        Step::Cut { seat, cut } => {
            format!(
                "{}: {}",
                seat_name(view, *seat),
                if *cut { "Bump." } else { "Run 'em." }
            )
        }
        Step::Dealt { .. } => {
            if view.hand.is_empty() {
                return None;
            }
            let hand: Vec<String> = view.hand.iter().map(|&c| card_name_lower(c)).collect();
            format!("Cards dealt. Your hand: {}.", hand.join(", "))
        }
        Step::UpCardTurned { card } => format!("Up-card is the {}.", card_name_lower(*card)),
        Step::TurnedDown { suit } => {
            let name = suit_name(*suit);
            format!("{name} turned down. {name} may not be named.")
        }
        Step::TrumpSet {
            suit,
            maker_seat,
            alone_seat,
            ..
        } => {
            let name = suit_name(*suit);
            let made_by = if *maker_seat == you {
                format!("{name} are trump. You called it.")
            } else {
                format!(
                    "{name} are trump, made by {}.",
                    seat_name(view, *maker_seat)
                )
            };
            let alone = match alone_seat {
                None => String::new(),
                Some(s) if *s == you => " You are going alone.".to_owned(),
                Some(s) => format!(" {} is going alone.", seat_name(view, *s)),
            };
            let [_, left] = bowers_of(*suit);
            format!(
                "{made_by}{alone} The {} is now a {}.",
                card_name_lower(left),
                suit_noun(*suit)
            )
        }
        Step::DealerDiscarded { seat } => {
            if *seat == you {
                "You discard face down.".to_owned()
            } else {
                format!("{} discards face down.", seat_name(view, *seat))
            }
        }
        Step::CardPlayed { seat, card } => {
            if *seat == you {
                format!("You play the {}.", card_name_lower(*card))
            } else {
                format!(
                    "{} plays the {}.",
                    seat_name(view, *seat),
                    card_name_lower(*card)
                )
            }
        }
        Step::TrickWon {
            index, winner_seat, ..
        } => {
            let n = u32::from(*index) + 1;
            let lead = if n >= 5 {
                String::new()
            } else if *winner_seat == you {
                " Your lead.".to_owned()
            } else if team_of(*winner_seat) == team_of(you) {
                " Partner leads.".to_owned()
            } else {
                format!(" {} leads.", seat_name(view, *winner_seat))
            };
            let took = if *winner_seat == you {
                "You take".to_owned()
            } else {
                format!("{} takes", seat_name(view, *winner_seat))
            };
            format!("{took} the trick. {n} of five.{lead}")
        }
        Step::HandScored {
            result,
            delta,
            score,
            ..
        } => {
            let theirs = 1 - mine;
            let scored = if delta[mine] > 0 {
                format!("You score {}.", delta[mine])
            } else if delta[theirs] > 0 {
                format!("They score {}.", delta[theirs])
            } else {
                "Nobody scores.".to_owned()
            };
            // The view carries no game length; every table plays the default (first to 10),
            // as the web announcer assumes.
            let game_to = DEFAULT_ENGINE_CONFIG.game_to;
            let barn = if is_game_over(*score, game_to) {
                ""
            } else if is_in_the_barn(*score, team_of(you), game_to) {
                " You are in the barn."
            } else if is_in_the_barn(*score, team_of(other(you)), game_to) {
                " They are in the barn."
            } else {
                ""
            };
            format!(
                "{} {scored} {} to {}.{barn}",
                result_label(*result),
                score[mine],
                score[theirs]
            )
        }
        Step::ThrowIn => "All passed twice. Thrown in — the deal moves left.".to_owned(),
        Step::GameWon { team, score } => {
            format!(
                "{}, {} to {}.",
                if *team == team_of(you) {
                    "You win"
                } else {
                    "They win"
                },
                score[mine],
                score[1 - mine]
            )
        }
        Step::Bid { .. } => return None,
    })
}

/// A seat on the other team.
fn other(seat: Seat) -> Seat {
    Seat::wrapping(seat.get() + 1)
}

/// The line for `steps` (one sync's worth, oldest first): each step's sentence, a space apart,
/// with repeated deal packets said once.
pub fn narrate(steps: &[Step], view: &PublicGameView) -> String {
    let mut dealt = false;
    let mut lines = Vec::new();
    for step in steps {
        if matches!(step, Step::Dealt { .. }) {
            if dealt {
                continue;
            }
            dealt = true;
        }
        if let Some(line) = sentence(step, view) {
            lines.push(line);
        }
    }
    lines.join(" ")
}

/// Drain the steps waiting in `hooks.announce` and return the line to say for them, if any.
pub fn take(hooks: &mut Hooks, view: &PublicGameView) -> Option<String> {
    if hooks.announce.is_empty() {
        return None;
    }
    let steps: Vec<Step> = hooks.announce.drain(..).collect();
    let line = narrate(&steps, view);
    (!line.is_empty()).then_some(line)
}

/// The announcer's live region, saying `said` (the bumped id and the line).
pub fn layer(said: Option<&(u64, String)>) -> El {
    let mut region = el()
        .key("announcer")
        .pin(Some(0.0), Some(0.0), None, None)
        .size(1.0, 1.0)
        .transition(Transition::none());
    if let Some((id, line)) = said {
        region = region.child(
            el().key(format!("announcer-{id}"))
                .size(1.0, 1.0)
                .role("status")
                .live_region(LiveRegion::Polite)
                .label(line.clone())
                .transition(Transition::none()),
        );
    }
    region
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::TableConfig;
    use crate::net::sim::fixture_view;

    #[test]
    fn take_drains_the_queue() {
        let v = fixture_view("announcer-tests", TableConfig::default()).expect("fixture");
        let mut hooks = Hooks::default();
        assert_eq!(take(&mut hooks, &v), None);
        hooks.announce.push_back(Step::ThrowIn);
        assert_eq!(
            take(&mut hooks, &v).as_deref(),
            Some("All passed twice. Thrown in — the deal moves left.")
        );
        assert!(hooks.announce.is_empty());
        hooks.announce.push_back(Step::Bid {
            seat: Seat::WEST,
            id: "pass".to_owned(),
            say: "Pass.".to_owned(),
        });
        assert_eq!(take(&mut hooks, &v), None, "a bid is silent");
    }
}
