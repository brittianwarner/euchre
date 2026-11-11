//! Output screening: the last line of defence before generated text reaches a
//! human. Two screens exist in the TS tree and both are ported, as distinct
//! modules, because they differ in detail:
//!
//! - [`ai`] is `src/lib/ai/screen.ts` (`screenText`, `screenBanter`,
//!   `cleanRationale`, the phrasebook), screened against `publicCards(view)`;
//! - [`seat`] is `src/lib/actors/ai-seat/screen.ts` (`screenBanter`,
//!   `cleanRationale`, `sanitizePersona`, `publicOnlyView`), screened against an
//!   explicit allow-list.
//!
//! Regex parity (D27): JS regexes without the `u` flag use an ASCII `\b` and ASCII
//! case folding, so those parts are written `(?-u:\b)` and `(?i-u:...)`; JS `\s`
//! is [`JS_WS_CLASS`]. A regex that fails to compile (never, in practice; a unit
//! test pins that) makes the screen reject, never pass.

use std::sync::LazyLock;

use euchre_engine::{Card, PublicGameView, Rank, Suit};
use regex::Regex;

use crate::js::{JS_WS_CLASS, js_trim, js_trim_end, utf16_prefix};

/// The C0/C1 control characters both screens strip
/// (`[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F]`).
fn is_ctrl(c: char) -> bool {
    matches!(c, '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}' | '\u{007F}'..='\u{009F}')
}

fn strip_ctrl(s: &str) -> String {
    s.chars().filter(|c| !is_ctrl(*c)).collect()
}

fn compile(src: &str) -> Option<Regex> {
    Regex::new(src).ok()
}

/// `<[^>]*>`.
static MARKUP: LazyLock<Option<Regex>> = LazyLock::new(|| compile("<[^>]*>"));
/// `\s+`.
static WS_RUN: LazyLock<Option<Regex>> = LazyLock::new(|| compile(&format!("{JS_WS_CLASS}+")));
/// `\b([9TJQKA])([SHDC])\b` (case-sensitive; shared by both screens).
static CODE: LazyLock<Option<Regex>> =
    LazyLock::new(|| compile(r"(?-u:\b)([9TJQKA])([SHDC])(?-u:\b)"));

fn replace_all(re: &LazyLock<Option<Regex>>, text: &str, with: &str) -> Option<String> {
    re.as_ref().map(|r| r.replace_all(text, with).into_owned())
}

fn rank_word(word: &str) -> Option<Rank> {
    match word.to_ascii_lowercase().as_str() {
        "nine" => Some(Rank::Nine),
        "ten" => Some(Rank::Ten),
        "jack" => Some(Rank::Jack),
        "queen" => Some(Rank::Queen),
        "king" => Some(Rank::King),
        "ace" => Some(Rank::Ace),
        _ => None,
    }
}

fn suit_word(word: &str) -> Option<Suit> {
    match word.to_ascii_lowercase().as_str() {
        "spade" | "spades" => Some(Suit::Spades),
        "heart" | "hearts" => Some(Suit::Hearts),
        "diamond" | "diamonds" => Some(Suit::Diamonds),
        "club" | "clubs" => Some(Suit::Clubs),
        _ => None,
    }
}

fn suit_glyph(glyph: &str) -> Option<Suit> {
    match glyph {
        "♠" => Some(Suit::Spades),
        "♥" => Some(Suit::Hearts),
        "♦" => Some(Suit::Diamonds),
        "♣" => Some(Suit::Clubs),
        _ => None,
    }
}

/// A rank word or a one-character rank code in either case.
fn rank_token(token: &str) -> Option<Rank> {
    if let Some(r) = rank_word(token) {
        return Some(r);
    }
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Rank::from_char(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// The table screen (`src/lib/ai/screen.ts`).
pub mod ai {
    use super::*;
    use crate::notation::public_cards;
    use euchre_protocol::{BANTER_MAX_CHARS, RATIONALE_MAX_CHARS};

    /// `WORDS_RE`: "ace of spades", "nine of clubs", "jack diamonds".
    static WORDS: LazyLock<Option<Regex>> = LazyLock::new(|| {
        compile(&format!(
            r"(?i-u:\b(nine|ten|jack|queen|king|ace)s?\b)(?:{JS_WS_CLASS}+(?i-u:of))?{JS_WS_CLASS}+(?i-u:\b(spades?|hearts?|diamonds?|clubs?)\b)"
        ))
    });
    /// `GLYPH_RE`: "A♠", "ace ♥", "J of ♦".
    static GLYPH: LazyLock<Option<Regex>> = LazyLock::new(|| {
        compile(&format!(
            r"(?i-u:\b(nine|ten|jack|queen|king|ace|[9TJQKA])\b){JS_WS_CLASS}*(?:(?i-u:of){JS_WS_CLASS}*)?([♠♥♦♣])"
        ))
    });
    /// `BOWER_RE`: "the right bower" / "the left bower".
    static BOWER: LazyLock<Option<Regex>> = LazyLock::new(|| {
        compile(&format!(
            r"(?i-u:\b(right|left)){JS_WS_CLASS}+(?i-u:bower\b)"
        ))
    });
    /// The table-advice trigger in `screenBanter`.
    static ADVICE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        compile(
            r"(?i-u:\b(partner|your (?:call|choice)|you should|play (?:your|the)|lead (?:your|the)|save (?:your|the)|trump (?:it|in)|take (?:it|this)|go on|go alone)\b)",
        )
    });

    /// `cardsNamedIn(text, trump)`'s result.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct NamedCards {
        /// Every card named, in detection order (codes, words, glyphs, bowers).
        pub cards: Vec<Card>,
        /// A bower was named before trump is known.
        pub unresolved_bower: bool,
    }

    /// `cardsNamedIn(text, trump)`. `None` only when a pattern failed to compile.
    pub fn cards_named_in(text: &str, trump: Option<Suit>) -> Option<NamedCards> {
        let mut cards = Vec::new();
        let mut unresolved_bower = false;
        for m in CODE.as_ref()?.captures_iter(text) {
            let rank = m
                .get(1)
                .and_then(|g| g.as_str().chars().next())
                .and_then(Rank::from_char);
            let suit = m
                .get(2)
                .and_then(|g| g.as_str().chars().next())
                .and_then(Suit::from_char);
            if let (Some(r), Some(s)) = (rank, suit) {
                cards.push(Card::new(r, s));
            }
        }
        for m in WORDS.as_ref()?.captures_iter(text) {
            let rank = m.get(1).and_then(|g| rank_word(g.as_str()));
            let suit = m.get(2).and_then(|g| suit_word(g.as_str()));
            if let (Some(r), Some(s)) = (rank, suit) {
                cards.push(Card::new(r, s));
            }
        }
        for m in GLYPH.as_ref()?.captures_iter(text) {
            let rank = m.get(1).and_then(|g| rank_token(g.as_str()));
            let suit = m.get(2).and_then(|g| suit_glyph(g.as_str()));
            if let (Some(r), Some(s)) = (rank, suit) {
                cards.push(Card::new(r, s));
            }
        }
        for m in BOWER.as_ref()?.captures_iter(text) {
            let Some(t) = trump else {
                unresolved_bower = true;
                continue;
            };
            let right = m
                .get(1)
                .is_some_and(|g| g.as_str().eq_ignore_ascii_case("right"));
            let suit = if right {
                t
            } else {
                euchre_engine::color_mate(t)
            };
            cards.push(Card::new(Rank::Jack, suit));
        }
        Some(NamedCards {
            cards,
            unresolved_bower,
        })
    }

    /// `ScreenReject`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ScreenReject {
        /// Names a card that is not public.
        PrivateCard,
        /// Names a bower before trump exists.
        UnresolvedBower,
        /// Only markup.
        Markup,
        /// Nothing left.
        Empty,
        /// Reserved by the TS union; never produced.
        TooLongAfterClamp,
        /// Tells partner what to do.
        TableAdvice,
    }

    impl ScreenReject {
        /// The wire string.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::PrivateCard => "private_card",
                Self::UnresolvedBower => "unresolved_bower",
                Self::Markup => "markup",
                Self::Empty => "empty",
                Self::TooLongAfterClamp => "too_long_after_clamp",
                Self::TableAdvice => "table_advice",
            }
        }
    }

    /// `ScreenResult`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ScreenResult {
        /// The text that ships.
        Ok(String),
        /// Rejected; `offending` is the private card, when one was found.
        Rejected {
            /// Why.
            reason: ScreenReject,
            /// The offending card id.
            offending: Option<Card>,
        },
    }

    const fn reject(reason: ScreenReject) -> ScreenResult {
        ScreenResult::Rejected {
            reason,
            offending: None,
        }
    }

    /// Parameters of [`screen_text`] (`ScreenOptions` plus the text and view).
    #[derive(Clone, Copy, Debug)]
    pub struct ScreenTextParams<'a> {
        /// The generated text.
        pub raw: &'a str,
        /// The public view.
        pub view: &'a PublicGameView,
        /// The clamp, in UTF-16 units.
        pub max_chars: usize,
        /// Cards that may be named; `publicCards(view)` when `None`.
        pub allow: Option<&'a [Card]>,
    }

    /// `screenText(raw, view, opts)`: strip markup and control characters,
    /// collapse whitespace, clamp, then check card references on the clamped text.
    pub fn screen_text(params: ScreenTextParams<'_>) -> ScreenResult {
        let ScreenTextParams {
            raw,
            view,
            max_chars,
            allow,
        } = params;
        let Some(markup) = MARKUP.as_ref() else {
            return reject(ScreenReject::Empty);
        };
        let had_markup = markup.is_match(raw);
        let Some(text) = replace_all(&MARKUP, raw, " ")
            .map(|t| strip_ctrl(&t))
            .and_then(|t| replace_all(&WS_RUN, &t, " "))
        else {
            return reject(ScreenReject::Empty);
        };
        let text = js_trim(utf16_prefix(js_trim(&text), max_chars)).to_owned();
        if text.is_empty() {
            return reject(if had_markup {
                ScreenReject::Markup
            } else {
                ScreenReject::Empty
            });
        }
        let public;
        let allow = match allow {
            Some(a) => a,
            None => {
                public = public_cards(view);
                &public
            }
        };
        let Some(named) = cards_named_in(&text, view.trump) else {
            return reject(ScreenReject::Empty);
        };
        if named.unresolved_bower {
            return reject(ScreenReject::UnresolvedBower);
        }
        if let Some(c) = named.cards.iter().find(|c| !allow.contains(c)) {
            return ScreenResult::Rejected {
                reason: ScreenReject::PrivateCard,
                offending: Some(*c),
            };
        }
        ScreenResult::Ok(text)
    }

    /// `screenBanter(raw, view)`: public state only, clamped to
    /// [`BANTER_MAX_CHARS`]; table advice is rejected first.
    pub fn screen_banter(raw: &str, view: &PublicGameView) -> ScreenResult {
        match ADVICE.as_ref() {
            Some(re) if !re.is_match(raw) => {}
            _ => return reject(ScreenReject::TableAdvice),
        }
        screen_text(ScreenTextParams {
            raw,
            view,
            max_chars: BANTER_MAX_CHARS,
            allow: None,
        })
    }

    /// Parameters of [`clean_rationale`].
    #[derive(Clone, Copy, Debug)]
    pub struct CleanRationaleParams<'a> {
        /// The model's rationale.
        pub raw: &'a str,
        /// The engine's own `RankedMove.why`.
        pub fallback: &'a str,
        /// The public view.
        pub view: &'a PublicGameView,
    }

    /// `cleanRationale(raw, fallback, view)`.
    pub fn clean_rationale(params: CleanRationaleParams<'_>) -> String {
        if let ScreenResult::Ok(text) = screen_text(ScreenTextParams {
            raw: params.raw,
            view: params.view,
            max_chars: RATIONALE_MAX_CHARS,
            allow: None,
        }) {
            return text;
        }
        let cleaned = replace_all(&MARKUP, params.fallback, " ")
            .and_then(|t| replace_all(&WS_RUN, &t, " "))
            .unwrap_or_default();
        utf16_prefix(js_trim(&cleaned), RATIONALE_MAX_CHARS).to_owned()
    }

    /// `PHRASEBOOK`, in TS key order.
    pub const PHRASEBOOK: [(&str, &[&str]); 9] = [
        (
            "generic",
            &[
                "Hm.",
                "All right then.",
                "We go on.",
                "Fair enough.",
                "That's the way of it.",
            ],
        ),
        (
            "bid",
            &[
                "I like my chances.",
                "Somebody has to.",
                "I'll take a look at it.",
                "Worth a try.",
            ],
        ),
        (
            "pass",
            &[
                "Not from here.",
                "Pass.",
                "It's not for me.",
                "Someone else can have it.",
            ],
        ),
        (
            "loner",
            &["On my own, then.", "Going alone.", "I'll handle this one."],
        ),
        (
            "win_trick",
            &["Mine.", "That one travels.", "I'll take it."],
        ),
        ("lose_trick", &["Yours.", "Well played.", "Nice trick."]),
        (
            "euchre",
            &[
                "Well. That happened.",
                "Didn't hold up.",
                "We'll get the next one.",
            ],
        ),
        (
            "march",
            &["All five.", "That'll do nicely.", "Clean sweep."],
        ),
        ("game", &["Good game.", "That's the game.", "Well played."]),
    ];

    /// `phrasebookLine(situation, n)`: deterministic selection.
    pub fn phrasebook_line(situation: &str, n: i64) -> &'static str {
        let generic: &[&str] = PHRASEBOOK[0].1;
        let lines = PHRASEBOOK
            .iter()
            .find(|(k, _)| *k == situation)
            .map_or(generic, |(_, v)| *v);
        let safe = if lines.is_empty() { generic } else { lines };
        let len = safe.len() as u64;
        let i = n.unsigned_abs() % len.max(1);
        safe.get(i as usize).copied().unwrap_or("Hm.")
    }
}

/// The seat screen (`src/lib/actors/ai-seat/screen.ts`).
pub mod seat {
    use super::*;
    use euchre_protocol::{BANTER_MAX_CHARS, RATIONALE_MAX_CHARS};

    /// `WORD_CARD`: "ace of spades", "nine of hearts", "jack diamonds".
    static WORD_CARD: LazyLock<Option<Regex>> = LazyLock::new(|| {
        compile(&format!(
            r"(?i-u:\b(nine|ten|jack|queen|king|ace)){JS_WS_CLASS}+(?:(?i-u:of){JS_WS_CLASS}+)?(?i-u:(spades?|hearts?|diamonds?|clubs?)\b)"
        ))
    });
    /// `GLYPH_CARD`: "A♠", "ace ♥", "9 ♦".
    static GLYPH_CARD: LazyLock<Option<Regex>> = LazyLock::new(|| {
        compile(&format!(
            r"(?i-u:\b(nine|ten|jack|queen|king|ace|[9TJQKA])){JS_WS_CLASS}?([♠♥♦♣])"
        ))
    });

    /// `cardsNamedIn(text)`: every card the text names, as a set in insertion
    /// order. `None` only when a pattern failed to compile.
    pub fn cards_named_in(text: &str) -> Option<Vec<Card>> {
        let mut found: Vec<Card> = Vec::new();
        let mut push = |rank: Option<Rank>, suit: Option<Suit>| {
            if let (Some(r), Some(s)) = (rank, suit) {
                let card = Card::new(r, s);
                if !found.contains(&card) {
                    found.push(card);
                }
            }
        };
        for m in WORD_CARD.as_ref()?.captures_iter(text) {
            push(
                m.get(1).and_then(|g| rank_token(g.as_str())),
                m.get(2).and_then(|g| suit_word(g.as_str())),
            );
        }
        for m in GLYPH_CARD.as_ref()?.captures_iter(text) {
            push(
                m.get(1).and_then(|g| rank_token(g.as_str())),
                m.get(2).and_then(|g| suit_glyph(g.as_str())),
            );
        }
        for m in CODE.as_ref()?.captures_iter(text) {
            push(
                m.get(1).and_then(|g| rank_token(g.as_str())),
                m.get(2)
                    .and_then(|g| g.as_str().chars().next())
                    .and_then(Suit::from_char),
            );
        }
        Some(found)
    }

    /// `publiclyNamedCards(view)`: the up-card plus every card face up this
    /// hand, never `view.hand`.
    pub fn publicly_named_cards(view: &PublicGameView) -> Vec<Card> {
        let mut allowed: Vec<Card> = Vec::new();
        let mut add = |c: Card| {
            if !allowed.contains(&c) {
                allowed.push(c);
            }
        };
        if let Some(up) = view.up_card {
            add(up);
        }
        let live = view.trick.index;
        for t in &view.trick_log {
            if t.index == live {
                continue;
            }
            for p in &t.plays {
                add(p.card);
            }
        }
        for p in &view.trick.plays {
            add(p.card);
        }
        allowed
    }

    /// `scrub(raw)`: control characters, then markup, then whitespace, trimmed.
    /// `None` only when a pattern failed to compile.
    fn scrub(raw: &str) -> Option<String> {
        let t = strip_ctrl(raw);
        let t = replace_all(&MARKUP, &t, "")?;
        let t = replace_all(&WS_RUN, &t, " ")?;
        Some(js_trim(&t).to_owned())
    }

    fn names_only(text: &str, allowed: &[Card]) -> bool {
        cards_named_in(text).is_some_and(|cards| cards.iter().all(|c| allowed.contains(c)))
    }

    /// `screenBanter(raw, allowed)`: the cleaned line, or `None` when it must not
    /// be shown (terminal: ship a phrasebook line, never regenerate).
    pub fn screen_banter(raw: &str, allowed: &[Card]) -> Option<String> {
        let text = scrub(raw)?;
        if text.is_empty() || !names_only(&text, allowed) {
            return None;
        }
        Some(js_trim_end(utf16_prefix(&text, BANTER_MAX_CHARS)).to_owned())
    }

    /// Parameters of [`clean_rationale`].
    #[derive(Clone, Copy, Debug)]
    pub struct SeatRationaleParams<'a> {
        /// The model's rationale (`undefined` in TS when absent).
        pub raw: Option<&'a str>,
        /// The engine's own `RankedMove.why`.
        pub fallback: &'a str,
        /// The allow-list.
        pub allowed: &'a [Card],
    }

    /// `cleanRationale(raw, fallback, allowed)`.
    pub fn clean_rationale(params: SeatRationaleParams<'_>) -> String {
        let fb = scrub(params.fallback)
            .map(|s| utf16_prefix(&s, RATIONALE_MAX_CHARS).to_owned())
            .unwrap_or_default();
        let Some(raw) = params.raw else { return fb };
        let Some(text) = scrub(raw) else { return fb };
        if text.is_empty() || !names_only(&text, params.allowed) {
            return fb;
        }
        utf16_prefix(&text, RATIONALE_MAX_CHARS).to_owned()
    }

    /// Parameters of [`sanitize_persona`].
    #[derive(Clone, Copy, Debug)]
    pub struct SanitizeParams<'a> {
        /// Untrusted persona text.
        pub raw: &'a str,
        /// The per-game fence nonce.
        pub nonce: &'a str,
        /// The hard clamp, in UTF-16 units.
        pub cap: usize,
    }

    /// `sanitizePersona(raw, nonce, cap)`: strip the nonce, control characters
    /// and newline runs of three or more, clamp, trim.
    pub fn sanitize_persona(params: SanitizeParams<'_>) -> String {
        let mut out = params.raw.to_owned();
        if !params.nonce.is_empty() {
            out = out.replace(params.nonce, "");
        }
        let out = strip_ctrl(&out);
        let mut collapsed = String::with_capacity(out.len());
        let mut run = 0usize;
        for c in out.chars() {
            if c == '\n' {
                run += 1;
                if run <= 2 {
                    collapsed.push('\n');
                }
            } else {
                run = 0;
                collapsed.push(c);
            }
        }
        js_trim(utf16_prefix(&collapsed, params.cap)).to_owned()
    }

    /// `publicOnlyView(view)`: the view with this seat's own cards removed.
    pub fn public_only_view(view: &PublicGameView) -> PublicGameView {
        PublicGameView {
            hand: Vec::new(),
            legal: Vec::new(),
            ..view.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(MARKUP.is_some() && WS_RUN.is_some() && CODE.is_some());
        assert!(ai::cards_named_in("x", None).is_some());
        assert!(seat::cards_named_in("x").is_some());
    }

    #[test]
    fn ascii_word_boundary_and_case() {
        let named = ai::cards_named_in("I hold AS and the Jack of Hearts, ten ♦", None)
            .unwrap_or_else(|| unreachable!());
        let ids: Vec<String> = named.cards.iter().map(ToString::to_string).collect();
        assert_eq!(ids, ["AS", "JH", "TD"]);
        // `é` is not an ASCII word char, so `\b` sits between it and `AS`.
        let named = ai::cards_named_in("éAS", None).unwrap_or_else(|| unreachable!());
        assert_eq!(named.cards.len(), 1);
        // JS `i` without `u` does not fold the Kelvin sign onto `k`.
        let named =
            ai::cards_named_in("\u{212A}ing of spades", None).unwrap_or_else(|| unreachable!());
        assert!(named.cards.is_empty());
    }

    #[test]
    fn phrasebook_is_deterministic() {
        assert_eq!(ai::phrasebook_line("pass", 1), "Pass.");
        assert_eq!(ai::phrasebook_line("nope", -5), "Hm.");
    }
}
