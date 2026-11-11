//! Coach hints (06 §13). When `prefs.hints` is on and the decision is yours, the coach names
//! the move the AI's own heuristic ranks first (`euchre_ai::rank_moves`, read only), in the
//! heuristic's own words:
//! - [`for_view`]: the suggestion, when one shows: `status == active`, your turn, more than one
//!   legal move, and the [`Situation`] not retired (streak ≥ [`RETIRE_AFTER`] in
//!   `prefs.hint_streaks`).
//! - [`badge`]: the small `Try` mark on the suggested card in the rack (hand.rs calls it per
//!   rack button); [`move_badge`] is the same mark for a bid or cut button.
//! - [`panel`]: the suggestion box in the settings rail, `Try: <strong>{label}</strong> —
//!   {gloss(why)}` (role status; 06 §13 colours; panels.rs mounts it under the Hints toggle).
//! - [`record_move`]: the streak update after each of your decisions. app.rs calls it with the
//!   view the move was made from just before it sends the move and saves what it returns.
//!
//! Hidden information: the ranking reads only the player's own projection (`PublicGameView`:
//! your hand, the public table, your legal set), the same redaction boundary an AI seat sees.
//! Nothing here looks at another seat's cards.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use euchre_ai::{RankParams, rank_moves};
use euchre_protocol::{Card, GamePhase, GameStatus, Move, PublicGameView};
use rune::tok::TextStyle;
use rune::{Color, El, Span, SpringPair, Transition, col, row, text};

use crate::prefs::Prefs;
use crate::theme::{color, hexes, motion, radius, space};

/// A situation retires after this many decisions in a row that match the suggestion.
pub const RETIRE_AFTER: u32 = 3;

/// The badge's word.
pub const BADGE: &str = "Try";

/// The jargon the heuristic's reasons use, and what each means (06 §13, verbatim).
pub const JARGON: [(&str, &str); 5] = [
    ("ruff", "trump a card in a suit that wasn’t led"),
    ("lay off", "play a low card since you can’t win this one"),
    (
        "third hand high",
        "you’re third to play, so play your best card",
    ),
    ("second hand", "you’re the second to play this trick"),
    ("bower", "one of the two top trump jacks"),
];

/// The colours (06 §13 for the box; the badge takes the table's gold).
mod tokens {
    /// The box fill.
    pub const BOX_BG: &str = "#e6eadc";
    /// The badge fill (the table's gold, `theme::motion::GOLD`).
    pub const BADGE_BG: &str = super::motion::GOLD;
    /// The badge's edge (`theme::motion::GOLD_DEEP`).
    pub const BADGE_EDGE: &str = super::motion::GOLD_DEEP;
    /// The badge's word: the deep felt green.
    pub const BADGE_TEXT: &str = "#12362b";
    /// Every token, for the parse test.
    #[cfg(test)]
    pub const ALL: [&str; 4] = [BOX_BG, BADGE_BG, BADGE_EDGE, BADGE_TEXT];
}

struct Colors {
    box_bg: Color,
    box_border: Color,
    box_text: Color,
    strong: Color,
    badge_bg: Color,
    badge_edge: Color,
    badge_text: Color,
}

static COLORS: LazyLock<Colors> = LazyLock::new(|| Colors {
    box_bg: color(tokens::BOX_BG),
    box_border: color(hexes::HAIRLINE_STRONG),
    box_text: color(hexes::INK),
    strong: color(hexes::GREEN_BUTTON),
    badge_bg: color(tokens::BADGE_BG),
    badge_edge: color(tokens::BADGE_EDGE),
    badge_text: color(tokens::BADGE_TEXT),
});

/// The box's type: 16/1.5 (06 §13).
const BOX_TYPE: TextStyle = TextStyle::new(16.0, 400.0).lh(24.0);
/// The badge's type.
const BADGE_TYPE: TextStyle = TextStyle::new(11.0, 700.0).lh(14.0).tracking(0.4);
/// The badge's height; it sits half over the card's top edge.
const BADGE_H: f32 = 20.0;

/// A situation class (06 §13); its [`Situation::key`] is the streak map's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Situation {
    /// `cut`: the cut decision.
    Cut,
    /// `bid1`: bidding round one.
    Bid1,
    /// `bid2`: bidding round two.
    Bid2,
    /// `discard`: the dealer's discard.
    Discard,
    /// `lead`: leading a trick.
    Lead,
    /// `follow`: following a led suit.
    Follow,
}

impl Situation {
    /// The class for `view`'s decision, if it has one.
    pub fn of(view: &PublicGameView) -> Option<Situation> {
        Some(match view.phase {
            GamePhase::Cutting => Situation::Cut,
            GamePhase::BidRound1 => Situation::Bid1,
            GamePhase::BidRound2 => Situation::Bid2,
            GamePhase::DealerDiscard => Situation::Discard,
            GamePhase::TrickPlay if view.trick.led_suit.is_none() => Situation::Lead,
            GamePhase::TrickPlay => Situation::Follow,
            _ => return None,
        })
    }

    /// The streak key (`euchre:hints:v1:streaks`).
    pub fn key(self) -> &'static str {
        match self {
            Situation::Cut => "cut",
            Situation::Bid1 => "bid1",
            Situation::Bid2 => "bid2",
            Situation::Discard => "discard",
            Situation::Lead => "lead",
            Situation::Follow => "follow",
        }
    }
}

/// A suggestion to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintSpec {
    /// The situation it answers.
    pub situation: Situation,
    /// The suggested move's legal id.
    pub move_id: String,
    /// The card it plays or discards, for a card move.
    pub card: Option<Card>,
    /// The move's label (fallback: its id).
    pub label: String,
    /// The heuristic's reason, raw (the panel glosses it).
    pub why: String,
    /// The decision's turn nonce: one suggestion is one turn's.
    pub turn_id: String,
}

/// Whether `situation` is retired: the player matched the suggestion [`RETIRE_AFTER`] times in
/// a row.
pub fn retired(prefs: &Prefs, situation: Situation) -> bool {
    prefs
        .hint_streaks
        .get(situation.key())
        .is_some_and(|&n| n >= RETIRE_AFTER)
}

/// Whether `view` is a real choice of yours: the match is on, it is your turn, and more than
/// one move is legal.
fn your_choice(view: &PublicGameView) -> bool {
    view.status == GameStatus::Active && view.turn_seat == Some(view.you) && view.legal.len() > 1
}

/// The suggestion for `view`, if one shows.
pub fn for_view(view: &PublicGameView, prefs: &Prefs) -> Option<HintSpec> {
    if !prefs.hints || !your_choice(view) {
        return None;
    }
    let situation = Situation::of(view)?;
    if retired(prefs, situation) {
        return None;
    }
    let top = rank_moves(RankParams::new(view)).into_iter().next()?;
    let legal = view.legal.iter().find(|m| m.id == top.id);
    let card = legal.and_then(|m| match m.mv {
        Move::Play { card } | Move::Discard { card } => Some(card),
        _ => None,
    });
    Some(HintSpec {
        situation,
        label: legal.map_or_else(|| top.id.clone(), |m| m.label.clone()),
        move_id: top.id,
        card,
        why: top.why,
        turn_id: view.turn_id.clone(),
    })
}

/// The hint streaks after you chose `move_id` from `view`, or `None` when they do not change.
///
/// 06 §13: a decision that matches the AI's top move adds one to its situation's streak, any
/// other resets it to 0 (which brings a retired hint back). It counts whether hints are on or
/// not, as the web app does. A forced move (one legal choice) is not a decision and counts for
/// nothing: following suit with your only card teaches nothing.
pub fn record_move(
    view: &PublicGameView,
    move_id: &str,
    prefs: &Prefs,
) -> Option<BTreeMap<String, u32>> {
    if !your_choice(view) {
        return None;
    }
    let situation = Situation::of(view)?;
    let top = rank_moves(RankParams::new(view)).into_iter().next()?;
    let key = situation.key();
    let current = prefs.hint_streaks.get(key).copied().unwrap_or(0);
    let next = if top.id == move_id {
        current.saturating_add(1)
    } else {
        0
    };
    if next == current {
        return None;
    }
    let mut streaks = prefs.hint_streaks.clone();
    streaks.insert(key.to_owned(), next);
    Some(streaks)
}

thread_local! {
    /// The jargon terms explained this session, each with the suggestion that explained it
    /// (`{turn_id}:{move_id}`), so the meaning stays on that suggestion while it shows and
    /// never repeats on another (06 §13: once per term per session).
    static EXPLAINED: RefCell<BTreeMap<&'static str, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Whether `hay` holds `word` as whole words, ignoring case (the web's `\bword\b`).
fn has_word(hay: &str, word: &str) -> bool {
    let hay = hay.to_lowercase();
    let bytes = hay.as_bytes();
    let boundary = |i: Option<&u8>| i.is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_');
    hay.match_indices(word).any(|(at, _)| {
        let before = at.checked_sub(1).and_then(|i| bytes.get(i));
        let after = bytes.get(at + word.len());
        (at == 0 || boundary(before)) && boundary(after)
    })
}

/// `gloss(why)`: the reason as a sentence (capitalised, a full stop), plus ` — meaning …` the
/// first time this session one of the [`JARGON`] terms shows. `claim` names the suggestion
/// asking, so asking again for the same suggestion keeps its meaning.
pub fn gloss(why: &str, claim: &str) -> String {
    let why = why.trim();
    if why.is_empty() {
        return String::new();
    }
    let mut chars = why.chars();
    let sentence = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => String::new(),
    };
    for (term, meaning) in JARGON {
        if !has_word(why, term) {
            continue;
        }
        let first = EXPLAINED.with(|seen| {
            let mut seen = seen.borrow_mut();
            match seen.get(term) {
                Some(owner) => owner == claim,
                None => {
                    seen.insert(term, claim.to_owned());
                    true
                }
            }
        });
        return if first {
            format!("{sentence} — meaning {meaning}.")
        } else {
            format!("{sentence}.")
        };
    }
    format!("{sentence}.")
}

/// The suggestion as one line: `Try: {label} — {gloss(why)}`, without the reason when it is
/// empty or only repeats the label. Also returns where the label sits, for the bold span.
pub fn line(hint: &HintSpec) -> (String, std::ops::Range<usize>) {
    let mut out = String::from("Try: ");
    let start = out.len();
    out.push_str(&hint.label);
    let label = start..out.len();
    let why = hint.why.trim();
    if !why.is_empty() && !why.eq_ignore_ascii_case(hint.label.trim()) {
        let claim = format!("{}:{}", hint.turn_id, hint.move_id);
        out.push_str(" — ");
        out.push_str(&gloss(why, &claim));
    }
    (out, label)
}

/// The base of a bid id: `orderUp+alone` and `orderUp` are one button with a Go alone box.
fn base_id(id: &str) -> &str {
    id.strip_suffix("+alone").unwrap_or(id)
}

fn mark(key: String, hint: &HintSpec) -> El {
    let (said, _) = line(hint);
    let pill = row()
        .h(BADGE_H)
        .px(space::S)
        .center()
        .radius(radius::PILL)
        .fill(COLORS.badge_bg)
        .border(1.0, COLORS.badge_edge)
        .child(text(BADGE).font(BADGE_TYPE).color(COLORS.badge_text))
        .help(said)
        .transition(
            Transition::scale(0.7)
                .combined(Transition::opacity())
                .springs(SpringPair::new(motion::CHIP_IN, motion::CHIP_OUT)),
        );
    // Pinned across the top edge, centred, half over the card: out of flow, so the rack
    // never shifts when a suggestion comes or goes.
    row()
        .key(key)
        .pin(Some(0.0), Some(-BADGE_H / 2.0), Some(0.0), None)
        .h(BADGE_H)
        .justify_center()
        .a11y_hidden()
        .child(pill)
}

/// The suggestion's mark on rack card `card`, when `hint` suggests it.
pub fn badge(card: Card, hint: &HintSpec) -> Option<El> {
    (hint.card == Some(card)).then(|| mark(format!("hint-{card}"), hint))
}

/// The suggestion's mark on the bid or cut button for `move_id` (its base id, without
/// `+alone`), when `hint` suggests that move. hand.rs `bid_panel` mounts it.
pub fn move_badge(move_id: &str, hint: &HintSpec) -> Option<El> {
    (hint.card.is_none() && base_id(&hint.move_id) == base_id(move_id))
        .then(|| mark(format!("hint-{}", base_id(move_id)), hint))
}

/// The suggestion box for the settings rail (06 §13: padding 12, `#e6eadc` on a `#cad3c1`
/// hairline, `#233e32` text, the move in `#285641` bold, role status).
pub fn panel(hint: &HintSpec) -> Option<El> {
    let (said, label) = line(hint);
    Some(
        col()
            .key("hint-panel")
            .w_full()
            .padding(space::SM)
            .radius(radius::BUTTON)
            .fill(COLORS.box_bg)
            .border(1.0, COLORS.box_border)
            .role("status")
            .label(said.clone())
            .child(
                text(said)
                    .font(BOX_TYPE)
                    .color(COLORS.box_text)
                    .spans(vec![
                        Span::new(label.start, label.end)
                            .weight(700.0)
                            .color(COLORS.strong),
                    ])
                    .wrap()
                    .a11y_hidden(),
            )
            .transition(Transition::opacity()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::TableConfig;
    use crate::net::sim::fixture_view;

    #[test]
    fn situations_follow_the_phase() {
        let mut v = fixture_view("hint-tests", TableConfig::default()).expect("fixture");
        v.phase = GamePhase::BidRound2;
        assert_eq!(Situation::of(&v).map(Situation::key), Some("bid2"));
        v.phase = GamePhase::HandScore;
        assert_eq!(Situation::of(&v), None);
        assert_eq!(for_view(&v, &Prefs::default()), None);
        assert_eq!(record_move(&v, "pass", &Prefs::default()), None);
    }

    #[test]
    fn every_token_parses() {
        for t in tokens::ALL {
            assert!(rune::color::hex(t).is_some(), "{t}");
        }
    }

    #[test]
    fn jargon_matches_whole_words_only() {
        assert!(has_word("cash it before the ruff", "ruff"));
        assert!(has_word("Ask for a bower", "bower"));
        assert!(!has_word("ask for bowers", "bower"));
        assert!(!has_word("gruff", "ruff"));
        assert!(has_word("lay off, it is theirs", "lay off"));
    }

    #[test]
    fn a_term_is_explained_once_per_session() {
        assert_eq!(
            gloss("cash it before the ruff", "t1:play:AS"),
            "Cash it before the ruff — meaning trump a card in a suit that wasn’t led."
        );
        assert_eq!(
            gloss("cash it before the ruff", "t1:play:AS"),
            "Cash it before the ruff — meaning trump a card in a suit that wasn’t led.",
            "the same suggestion keeps its meaning while it shows"
        );
        assert_eq!(
            gloss("cash it before the ruff", "t2:play:KS"),
            "Cash it before the ruff."
        );
        assert_eq!(gloss("pull their trump", "t3:play:JH"), "Pull their trump.");
        assert_eq!(gloss("", "t4:pass"), "");
    }

    #[test]
    fn the_line_drops_a_reason_that_repeats_the_label() {
        let hint = HintSpec {
            situation: Situation::Cut,
            move_id: "cut:no".to_owned(),
            card: None,
            label: "Run 'em".to_owned(),
            why: "run 'em".to_owned(),
            turn_id: "t".to_owned(),
        };
        assert_eq!(line(&hint).0, "Try: Run 'em");
        assert_eq!(line(&hint).1, 5..12);
        assert!(move_badge("cut:no", &hint).is_some());
        assert!(move_badge("cut:yes", &hint).is_none());
        assert!(badge(Card::parse("AS").expect("a card"), &hint).is_none());
    }
}
