//! The dense `KEY=value` line notation the model reads (`ai/notation.ts`).

use euchre_engine::{Card, LegalMove, PublicGameView, Trick};
use euchre_protocol::RankedMove;
use regex::Regex;
use std::sync::LazyLock;

use crate::js::{JS_WS_CLASS, js_trim, to_fixed, utf16_len, utf16_prefix};

/// `NOTES=` lines kept.
pub const MAX_NOTE_LINES: usize = 6;
/// Max `NOTES=` line length (UTF-16 units).
pub const MAX_NOTE_CHARS: usize = 80;

static WS_RUN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(&format!("{JS_WS_CLASS}+")).ok());

/// `text.replace(/\s+/g, ' ')` with the JS whitespace set.
pub fn collapse_ws(text: &str) -> String {
    match WS_RUN.as_ref() {
        Some(re) => re.replace_all(text, " ").into_owned(),
        None => text.to_owned(),
    }
}

/// Parameters of [`encode_for_llm`] (`EncodeOptions` plus the view and legal set).
#[derive(Clone, Copy, Debug)]
pub struct EncodeParams<'a> {
    /// The redacted view.
    pub view: &'a PublicGameView,
    /// The candidate set the model may choose from.
    pub legal: &'a [LegalMove],
    /// The engine's ranking over the same moves.
    pub ranking: Option<&'a [RankedMove]>,
    /// Per-hand notes derived from the public view.
    pub notes: Option<&'a [String]>,
}

fn plays_text(trick: &Trick, sep: &str) -> String {
    trick
        .plays
        .iter()
        .map(|p| format!("{}:{}", p.seat.get(), p.card))
        .collect::<Vec<_>>()
        .join(sep)
}

fn opt_text<T: std::fmt::Display>(v: Option<T>, none: &str) -> String {
    v.map_or_else(|| none.to_owned(), |x| x.to_string())
}

fn header_lines(view: &PublicGameView, with_sitting: bool) -> Vec<String> {
    let mut l = Vec::with_capacity(12);
    l.push(format!(
        "YOU={} DEALER={} HAND#{} SCORE={}-{}",
        view.you.get(),
        view.dealer_seat.get(),
        view.hand_no,
        view.score[0],
        view.score[1]
    ));
    let mut phase = format!("PHASE={}", view.phase.as_str());
    if let Some(t) = view.trump {
        phase.push_str(&format!(" TRUMP={}", t.as_char()));
    }
    if let Some(m) = view.maker_seat {
        phase.push_str(&format!(" MAKER={}", m.get()));
    }
    if let Some(a) = view.alone_seat {
        phase.push_str(&format!(" ALONE={}", a.get()));
    }
    if with_sitting && let Some(s) = view.sitting_seat {
        phase.push_str(&format!(" SITTING={}", s.get()));
    }
    l.push(phase);
    l.push(format!(
        "UP={}{}",
        opt_text(view.up_card, "-"),
        if view.up_card_turned_down {
            " (turned down)"
        } else {
            ""
        }
    ));
    l
}

fn bids_line(view: &PublicGameView) -> Option<String> {
    if view.bids.is_empty() {
        return None;
    }
    Some(format!(
        "BIDS={}",
        view.bids
            .iter()
            .map(|b| format!("{}:{}", b.seat.get(), b.say))
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

/// `encodeForLlm(view, legal, opts)`.
pub fn encode_for_llm(params: EncodeParams<'_>) -> String {
    let view = params.view;
    let mut l = header_lines(view, true);
    if let Some(s) = view.turned_down_suit {
        l.push(format!("BARRED={}", s.as_char()));
    }
    l.push(format!(
        "HAND={}",
        view.hand
            .iter()
            .map(Card::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ));

    let (current, past) = split_tricks(view);
    if let Some(cur) = current.filter(|c| !c.plays.is_empty()) {
        l.push(format!(
            "TRICK={} LED={} ON={}",
            u32::from(cur.index) + 1,
            opt_text(cur.led_suit.map(|s| s.as_char()), "-"),
            plays_text(cur, ",")
        ));
    }
    if !past.is_empty() {
        let parts: Vec<String> = past
            .iter()
            .map(|t| {
                format!(
                    "trick{}:led={},winner={}<{}",
                    u32::from(t.index) + 1,
                    opt_text(t.led_suit.map(|s| s.as_char()), "null"),
                    opt_text(t.winner_seat.map(|s| s.get()), "?"),
                    plays_text(t, " ")
                )
            })
            .collect();
        l.push(format!("PAST={}", parts.join(" | ")));
    }

    l.push(format!(
        "TRICKS={}-{} COUNTS={}",
        view.tricks_won[0],
        view.tricks_won[1],
        view.hand_counts
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ));
    if let Some(b) = bids_line(view) {
        l.push(b);
    }

    for note in params.notes.unwrap_or(&[]).iter().take(MAX_NOTE_LINES) {
        let collapsed = collapse_ws(note);
        let clean = utf16_prefix(js_trim(&collapsed), MAX_NOTE_CHARS);
        if !clean.is_empty() {
            l.push(format!("NOTES={clean}"));
        }
    }

    l.push("LEGAL:".to_owned());
    for m in params.legal {
        let r = params
            .ranking
            .and_then(|rs| rs.iter().find(|x| x.id == m.id));
        let score = r.map_or_else(String::new, |r| {
            format!(" [{} {}]", to_fixed(r.score, 3), r.why)
        });
        l.push(format!("  {} = {}{}", m.id, m.label, score));
    }
    l.join("\n")
}

/// `publicStateOnly(view)`: the narrower slice banter is generated from.
pub fn public_state_only(view: &PublicGameView) -> String {
    let mut l = header_lines(view, false);
    let (current, past) = split_tricks(view);
    if let Some(cur) = current.filter(|c| !c.plays.is_empty()) {
        l.push(format!(
            "TRICK={} ON={}",
            u32::from(cur.index) + 1,
            plays_text(cur, ",")
        ));
    }
    if !past.is_empty() {
        l.push(format!(
            "PAST={}",
            past.iter()
                .map(|t| opt_text(t.winner_seat.map(|s| s.get()), "?"))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    l.push(format!(
        "TRICKS={}-{}",
        view.tricks_won[0], view.tricks_won[1]
    ));
    if let Some(b) = bids_line(view) {
        l.push(b);
    }
    l.join("\n")
}

/// `splitTricks(view)`: the live trick (if unfinished) and the completed ones, never double-counted.
pub fn split_tricks(view: &PublicGameView) -> (Option<&Trick>, Vec<&Trick>) {
    let live = if view.trick.winner_seat.is_none() {
        Some(&view.trick)
    } else {
        None
    };
    let past = view
        .trick_log
        .iter()
        .filter(|t| live.is_none_or(|l| t.index != l.index))
        .collect();
    (live, past)
}

/// `publicCards(view)`: every card already public, in first-seen order.
pub fn public_cards(view: &PublicGameView) -> Vec<Card> {
    let mut seen: Vec<Card> = Vec::with_capacity(21);
    let mut add = |c: Card| {
        if !seen.contains(&c) {
            seen.push(c);
        }
    };
    let (current, past) = split_tricks(view);
    for t in past {
        for p in &t.plays {
            add(p.card);
        }
    }
    if let Some(cur) = current {
        for p in &cur.plays {
            add(p.card);
        }
    }
    if let Some(up) = view.up_card {
        add(up);
    }
    seen
}

/// `estimateTokens(text)`: `ceil(text.length / 3.7)`.
pub fn estimate_tokens(text: &str) -> u64 {
    (utf16_len(text) as f64 / 3.7).ceil() as u64
}
