//! Runtime guards and text hygiene (`player-profile/guards.ts`).
//!
//! Every argument reaches the actor as decoded data from outside it, so each
//! value is narrowed here first and a value that will not narrow becomes a
//! [`ProfileError`] with the exact TS message. `None` stands for JS `undefined`
//! (an absent key).

use euchre_protocol::ProtocolErrorCode;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::error::{ProfileError, ProfileResult};
use crate::js::{as_number, is_js_whitespace, js_json_string, js_trim, trunc_i64};

/// `String(ID_PATTERN)`, as it appears in `requireId` messages.
pub const ID_PATTERN_TEXT: &str = "/^[A-Za-z0-9_-]{1,128}$/";

/// `ID_PATTERN.test(value)`: `^[A-Za-z0-9_-]{1,128}$`.
pub fn is_valid_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `requireId(value, field)`: an id matching [`ID_PATTERN_TEXT`], else
/// `internal_error` "`<field> must match /^[A-Za-z0-9_-]{1,128}$/`".
pub fn require_id(value: Option<&Value>, field: &str) -> ProfileResult<String> {
    match value {
        Some(Value::String(s)) if is_valid_id(s) => Ok(s.clone()),
        _ => Err(ProfileError::internal(format!(
            "{field} must match {ID_PATTERN_TEXT}"
        ))),
    }
}

/// The shape of a stored episode id, as it appears in [`require_episode_id`] messages.
pub const EPISODE_ID_PATTERN_TEXT: &str = "<role>.<matchId>.<handNo>.<kind>";

/// Whether `value` is an id that [`crate::records::episode_id`] can produce (D30):
/// exactly four `.`-separated parts, where `role` and `matchId` match
/// [`ID_PATTERN_TEXT`], `handNo` is a canonical decimal in
/// `0..=`[`crate::records::MAX_EPISODE_HAND_NO`] (no sign, no leading zero) and
/// `kind` is one of [`crate::records::EPISODE_KINDS`].
///
/// Deliberate deviation from TS, which checked `forgetEpisode`'s id with
/// `requireId` and so refused every stored id (they all contain `.`). The check
/// stays strict: no `/`, `%`, whitespace, empty part or `..` gets through, and an
/// id that passes names one row of the owner's own `episodes` table.
pub fn is_valid_episode_id(value: &str) -> bool {
    let mut parts = value.split('.');
    let (Some(role), Some(match_id), Some(hand_no), Some(kind), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    is_valid_id(role)
        && is_valid_id(match_id)
        && is_canonical_hand_no(hand_no)
        && crate::records::EPISODE_KINDS
            .iter()
            .any(|(_, text)| *text == kind)
}

/// `0`, or `[1-9][0-9]*` no larger than [`crate::records::MAX_EPISODE_HAND_NO`]:
/// exactly the text `format!("{hand_no}")` writes for an accepted `handNo`.
fn is_canonical_hand_no(text: &str) -> bool {
    let digits = text.bytes().all(|b| b.is_ascii_digit());
    let canonical = text == "0" || !text.starts_with('0');
    digits
        && canonical
        && (1..=6).contains(&text.len())
        && text
            .parse::<i64>()
            .is_ok_and(|n| n <= crate::records::MAX_EPISODE_HAND_NO)
}

/// `forgetEpisode`'s id guard (D30): an id [`is_valid_episode_id`] accepts, else
/// `internal_error` "`episodeId must match <role>.<matchId>.<handNo>.<kind>`".
pub fn require_episode_id(value: Option<&Value>) -> ProfileResult<String> {
    match value {
        Some(Value::String(s)) if is_valid_episode_id(s) => Ok(s.clone()),
        _ => Err(ProfileError::internal(format!(
            "episodeId must match {EPISODE_ID_PATTERN_TEXT}"
        ))),
    }
}

/// `requireClientId(value, field)`: as [`require_id`] but `game_not_found`
/// "`<field> is not a valid id`".
pub fn require_client_id(value: Option<&Value>, field: &str) -> ProfileResult<String> {
    match value {
        Some(Value::String(s)) if is_valid_id(s) => Ok(s.clone()),
        _ => Err(ProfileError::new(
            ProtocolErrorCode::GameNotFound,
            format!("{field} is not a valid id"),
        )),
    }
}

/// `requireOneOf(value, allowed, field, code)` over a string-backed enum:
/// "`<field> must be one of: a, b`".
pub fn require_one_of<T: Copy>(
    value: Option<&Value>,
    allowed: &[(T, &str)],
    field: &str,
    code: ProtocolErrorCode,
) -> ProfileResult<T> {
    if let Some(Value::String(s)) = value
        && let Some((v, _)) = allowed.iter().find(|(_, text)| *text == s)
    {
        return Ok(*v);
    }
    Err(one_of_error(allowed, field, code))
}

/// As [`require_one_of`] for a plain string (a column already read as text).
pub fn require_one_of_str<T: Copy>(
    value: &str,
    allowed: &[(T, &str)],
    field: &str,
    code: ProtocolErrorCode,
) -> ProfileResult<T> {
    match allowed.iter().find(|(_, text)| *text == value) {
        Some((v, _)) => Ok(*v),
        None => Err(one_of_error(allowed, field, code)),
    }
}

fn one_of_error<T>(allowed: &[(T, &str)], field: &str, code: ProtocolErrorCode) -> ProfileError {
    let list: Vec<&str> = allowed.iter().map(|(_, s)| *s).collect();
    ProfileError::new(code, format!("{field} must be one of: {}", list.join(", ")))
}

/// `requireInt(value, field, min, max)`: a finite integer number in range, else
/// `internal_error` "`<field> must be an integer in [min, max]`".
pub fn require_int(value: Option<&Value>, field: &str, min: i64, max: i64) -> ProfileResult<i64> {
    if let Some(v) = as_number(value)
        && v.is_finite()
        && v.fract() == 0.0
        && v >= min as f64
        && v <= max as f64
    {
        return Ok(v as i64);
    }
    Err(ProfileError::internal(format!(
        "{field} must be an integer in [{min}, {max}]"
    )))
}

/// `clampNumber(value, min, max, field, code)`: clamps a finite number, else
/// "`<field> must be a finite number`".
pub fn clamp_number(
    value: Option<&Value>,
    min: f64,
    max: f64,
    field: &str,
    code: ProtocolErrorCode,
) -> ProfileResult<f64> {
    match as_number(value) {
        Some(v) if v.is_finite() => Ok(js_clamp(v, min, max)),
        _ => Err(ProfileError::new(
            code,
            format!("{field} must be a finite number"),
        )),
    }
}

/// `Math.min(max, Math.max(min, v))`, with JS `-0` handling (`Math.max(0, -0)`
/// is `+0`).
pub fn js_clamp(v: f64, min: f64, max: f64) -> f64 {
    let lo = if v > min || (v == min && v.is_sign_positive()) {
        v
    } else {
        min
    };
    if lo < max { lo } else { max }
}

/// `requireBoolean(value, field)`: `invalid_persona` "`<field> must be a boolean`".
pub fn require_boolean(value: Option<&Value>, field: &str) -> ProfileResult<bool> {
    match value {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(ProfileError::invalid_persona(format!(
            "{field} must be a boolean"
        ))),
    }
}

/// `toInt(value, fallback)` over a decoded value: a finite number truncates, a
/// numeric non-blank string parses, null/undefined is `fallback`, anything else
/// is `internal_error` "unexpected non-integer column value".
pub fn value_to_int(value: Option<&Value>, fallback: i64) -> ProfileResult<i64> {
    match value {
        Some(Value::Number(n)) => match n.as_i64() {
            Some(i) => Ok(i),
            None => Ok(trunc_i64(n.as_f64().unwrap_or(0.0))),
        },
        Some(Value::String(s)) if !js_trim(s).is_empty() => {
            let parsed = crate::js::js_string_to_number(s);
            if parsed.is_finite() {
                Ok(trunc_i64(parsed))
            } else {
                Err(non_integer())
            }
        }
        None | Some(Value::Null) => Ok(fallback),
        _ => Err(non_integer()),
    }
}

/// `toFloat(value, fallback)` over a decoded value; never fails.
pub fn value_to_float(value: Option<&Value>, fallback: f64) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite()).unwrap_or(fallback),
        Some(Value::String(s)) if !js_trim(s).is_empty() => {
            let parsed = crate::js::js_string_to_number(s);
            if parsed.is_finite() { parsed } else { fallback }
        }
        _ => fallback,
    }
}

pub(crate) fn non_integer() -> ProfileError {
    ProfileError::internal("unexpected non-integer column value")
}

/// Options for [`sanitize_text`] (`SanitizeOptions`).
#[derive(Clone, Copy, Debug)]
pub struct SanitizeOptions<'a> {
    /// `true` keeps `\n` (and collapses blank-line runs); `false` folds all
    /// whitespace to single spaces.
    pub multiline: bool,
    /// The cap, in code points.
    pub max_chars: usize,
    /// The field name used in the error message.
    pub field: &'a str,
    /// The code for a non-string value (TS default `invalid_persona`).
    pub code: ProtocolErrorCode,
}

impl<'a> SanitizeOptions<'a> {
    /// Options with the TS default code, `invalid_persona`.
    pub fn new(multiline: bool, max_chars: usize, field: &'a str) -> SanitizeOptions<'a> {
        SanitizeOptions {
            multiline,
            max_chars,
            field,
            code: ProtocolErrorCode::InvalidPersona,
        }
    }

    /// The same options reporting `code` instead.
    pub fn with_code(self, code: ProtocolErrorCode) -> SanitizeOptions<'a> {
        SanitizeOptions { code, ..self }
    }
}

/// `sanitizeText(value, opts)` for a value that may not be a string:
/// "`<field> must be a string`" when it is not.
pub fn sanitize_value(value: Option<&Value>, opts: SanitizeOptions<'_>) -> ProfileResult<String> {
    match value {
        Some(Value::String(s)) => Ok(sanitize_text(s, opts)),
        _ => Err(ProfileError::new(
            opts.code,
            format!("{} must be a string", opts.field),
        )),
    }
}

fn is_exotic_break(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}' | '\u{0085}')
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{061c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
    )
}

fn is_stripped_control(c: char) -> bool {
    matches!(
        c,
        '\u{0000}'..='\u{0008}' | '\u{000b}' | '\u{000c}' | '\u{000e}'..='\u{001f}' | '\u{007f}'..='\u{009f}'
    )
}

/// `sanitizeText` on a string: NFC, exotic breaks, CRLF, invisible and control
/// stripping, whitespace collapse, trim, then a clamp to `max_chars` **code
/// points** (and trim again).
pub fn sanitize_text(value: &str, opts: SanitizeOptions<'_>) -> String {
    let nfc: String = value.nfc().collect();

    // EXOTIC_BREAKS, then /\r\n?/g -> '\n', then INVISIBLE_CHARS, then CONTROL_CHARS.
    let mut text = String::with_capacity(nfc.len());
    // The exotic-break mapping runs per char before the CR fold sees it, so a
    // `\r` followed by U+2028 (multiline) folds to one `\n`, as TS does.
    let break_to = if opts.multiline { '\n' } else { ' ' };
    let map_break = |c: char| if is_exotic_break(c) { break_to } else { c };
    let mut chars = nfc.chars().map(map_break).peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            text.push('\n');
            continue;
        }
        if is_invisible(c) || is_stripped_control(c) {
            continue;
        }
        text.push(c);
    }

    let text = if opts.multiline {
        collapse_multiline(&text)
    } else {
        collapse_runs(&text, is_js_whitespace)
    };

    let trimmed = js_trim(&text);
    if trimmed.chars().count() > opts.max_chars {
        let cut: String = trimmed.chars().take(opts.max_chars).collect();
        js_trim(&cut).to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Replaces each run of characters matching `pred` with one space.
fn collapse_runs(text: &str, pred: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if pred(c) {
            if !in_run {
                out.push(' ');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

/// `[^\S\n]+` -> ' ', then ` *\n *` -> '\n', then `\n{3,}` -> '\n\n'.
fn collapse_multiline(text: &str) -> String {
    let spaced = collapse_runs(text, |c| c != '\n' && is_js_whitespace(c));
    let segments: Vec<&str> = spaced.split('\n').collect();
    let last = segments.len().saturating_sub(1);
    let mut joined = String::with_capacity(spaced.len());
    for (i, segment) in segments.iter().enumerate() {
        let mut s: &str = segment;
        if i > 0 {
            s = s.trim_start_matches(' ');
        }
        if i < last {
            s = s.trim_end_matches(' ');
        }
        if i > 0 {
            joined.push('\n');
        }
        joined.push_str(s);
    }
    let mut out = String::with_capacity(joined.len());
    let mut newlines = 0usize;
    for c in joined.chars() {
        if c == '\n' {
            newlines += 1;
            continue;
        }
        if newlines > 0 {
            out.push_str(if newlines >= 3 {
                "\n\n"
            } else {
                &"\n\n"[..newlines]
            });
            newlines = 0;
        }
        out.push(c);
    }
    if newlines > 0 {
        out.push_str(if newlines >= 3 {
            "\n\n"
        } else {
            &"\n\n"[..newlines]
        });
    }
    out
}

/// `encodeJson(value)`: `JSON.stringify(value ?? null)`.
pub fn encode_json(value: Option<&Value>) -> String {
    match value {
        None => "null".to_owned(),
        Some(v) => js_json_string(v),
    }
}

/// `decodeJson(text, fallback)`: the parsed value, or `None` (use the fallback)
/// for a non-string, an empty string, a parse error, or `null`.
pub fn decode_json(text: Option<&str>) -> Option<Value> {
    let text = text?;
    if text.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Null) | Err(_) => None,
        Ok(v) => Some(v),
    }
}

/// `asArray(value)`: the array, or an empty slice.
pub fn as_array(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(s: &str, max: usize) -> String {
        sanitize_text(s, SanitizeOptions::new(false, max, "f"))
    }

    fn multi(s: &str, max: usize) -> String {
        sanitize_text(s, SanitizeOptions::new(true, max, "f"))
    }

    #[test]
    fn single_line_folds_whitespace() {
        assert_eq!(single("  a\n\tb\u{2028}c  ", 32), "a b c");
        assert_eq!(single("a\u{200b}b\u{0007}c", 32), "abc");
    }

    #[test]
    fn multiline_collapses_blank_lines() {
        assert_eq!(multi("a  \r\n\r\n\r\n  b \n c", 100), "a\n\nb\nc");
        assert_eq!(multi("a\u{2029}b", 100), "a\nb");
        assert_eq!(multi("x \n \n y", 100), "x\n\ny");
    }

    #[test]
    fn cr_then_exotic_break_folds_like_crlf() {
        // TS maps EXOTIC_BREAKS first, then /\r\n?/g: one newline, not two.
        assert_eq!(multi("a\r\u{2028}b", 100), "a\nb");
        assert_eq!(multi("a\r\u{2029}b", 100), "a\nb");
        assert_eq!(multi("a\r\u{0085}b", 100), "a\nb");
        assert_eq!(multi("a\r\r\u{2028}b", 100), "a\n\nb");
        assert_eq!(single("a\r\u{2028}b", 100), "a b");
    }

    #[test]
    fn clamp_counts_code_points() {
        assert_eq!(single("😀😀😀", 2), "😀😀");
        assert_eq!(single("ab c", 3), "ab");
    }

    #[test]
    fn js_clamp_zero() {
        assert!(js_clamp(-0.0, 0.0, 1.0).is_sign_positive());
        assert_eq!(js_clamp(2.0, 0.0, 1.0), 1.0);
    }

    // ---- D30: forgetEpisode takes the stored id ---------------------------------

    use crate::records::{
        EPISODE_KINDS, MAX_EPISODE_HAND_NO, StoredEpisode, parse_record_episodes,
    };
    use crate::sql::{Column, EPISODE_DELETE_SQL, episode_params};
    use serde_json::json;

    const NOW: i64 = 1_760_000_000_000;

    fn record(episodes: Value) -> Vec<StoredEpisode> {
        match parse_record_episodes(&json!({ "internalToken": "t", "episodes": episodes }), NOW) {
            Ok(rows) => rows,
            Err(e) => panic!("recordEpisodes: {e:?}"),
        }
    }

    fn forget_arg(id: &str) -> ProfileResult<String> {
        require_episode_id(Some(&json!(id)))
    }

    #[test]
    fn an_episode_is_forgotten_by_the_id_it_was_stored_under() {
        let stored = record(json!([{
            "role": "verna",
            "matchId": "m_01HZX-9c",
            "handNo": 3,
            "kind": "loner",
            "ts": NOW - 1_000,
            "summary": "Went alone on a black jack and made it.",
            "salience": 0.8,
        }]));
        let [episode] = stored.as_slice() else {
            panic!("one episode, got {stored:?}");
        };
        assert_eq!(episode.episode_id, "verna.m_01HZX-9c.3.loner");

        // listEpisodes hands this id to the client; forgetEpisode takes it back.
        let arg = match forget_arg(&episode.episode_id) {
            Ok(arg) => arg,
            Err(e) => panic!("forgetEpisode({:?}): {e:?}", episode.episode_id),
        };
        assert_eq!(arg, episode.episode_id);
        // The delete binds exactly the primary key the insert wrote.
        assert_eq!(
            EPISODE_DELETE_SQL,
            "DELETE FROM episodes WHERE episode_id = ?"
        );
        assert_eq!(episode_params(episode)[0], Column::from(arg.as_str()));
        let mut table = stored.clone();
        table.retain(|row| row.episode_id != arg);
        assert!(table.is_empty(), "the recorded episode is gone");

        // TS checked the id with requireId, which no stored id can pass (the bug).
        let ts_guard = require_id(Some(&json!(episode.episode_id)), "episodeId");
        assert!(ts_guard.is_err());
    }

    #[test]
    fn every_storable_episode_id_is_forgettable() {
        let long = "x".repeat(128);
        let mut entries = Vec::new();
        for (_, kind) in EPISODE_KINDS {
            for hand_no in [0, 1, 9, 10, 99, 1_000, MAX_EPISODE_HAND_NO] {
                entries.push(json!({
                    "role": "role_-AZaz09", "matchId": "m", "handNo": hand_no, "kind": kind, "summary": "s",
                }));
            }
        }
        entries.push(json!({
            "role": long, "matchId": long, "handNo": 7.0, "kind": "read", "summary": "s",
        }));
        for chunk in entries.chunks(crate::records::MAX_EPISODES_PER_CALL) {
            for episode in record(Value::Array(chunk.to_vec())) {
                assert!(
                    is_valid_episode_id(&episode.episode_id),
                    "{}",
                    episode.episode_id
                );
                assert_eq!(
                    forget_arg(&episode.episode_id).ok(),
                    Some(episode.episode_id)
                );
            }
        }
    }

    #[test]
    fn malformed_episode_ids_are_still_refused() {
        let long = "x".repeat(129);
        let refused = [
            String::new(),
            ".".into(),
            "...".into(),
            "....".into(),
            "verna".into(),
            "verna.m".into(),
            "verna.m.3".into(),
            "verna.m.3.bid.extra".into(),
            ".m.3.bid".into(),
            "verna..3.bid".into(),
            "verna.m..bid".into(),
            "verna.m.3.".into(),
            "verna.m.03.bid".into(),
            "verna.m.00.bid".into(),
            "verna.m.-1.bid".into(),
            "verna.m.+1.bid".into(),
            "verna.m.1e2.bid".into(),
            "verna.m.3.0.bid".into(),
            "verna.m.10001.bid".into(),
            "verna.m.9999999999999999999999.bid".into(),
            "verna.m.\u{0663}.bid".into(),
            "verna.m.3.BID".into(),
            "verna.m.3.unknown".into(),
            "verna.m.3.bid ".into(),
            " verna.m.3.bid".into(),
            "verna.m.3.bid\n".into(),
            "ver na.m.3.bid".into(),
            "verna/x.m.3.bid".into(),
            "../m.3.bid".into(),
            "verna.m%2e.3.bid".into(),
            "verna.m;DROP.3.bid".into(),
            "verna.m*.3.bid".into(),
            "v\u{e9}rna.m.3.bid".into(),
            format!("{long}.m.3.bid"),
            format!("verna.{long}.3.bid"),
        ];
        for id in &refused {
            assert!(!is_valid_episode_id(id), "{id:?}");
            let err = forget_arg(id).err();
            assert_eq!(
                err.as_ref().map(|e| (e.code, e.message.as_str())),
                Some((
                    ProtocolErrorCode::InternalError,
                    "episodeId must match <role>.<matchId>.<handNo>.<kind>"
                )),
                "{id:?}"
            );
        }
        for value in [json!(3), json!(null), json!(["verna.m.3.bid"]), json!({})] {
            assert!(require_episode_id(Some(&value)).is_err(), "{value}");
        }
        assert!(require_episode_id(None).is_err());
    }
}
