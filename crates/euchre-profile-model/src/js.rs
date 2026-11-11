//! The JavaScript semantics the TS profile code leans on, made explicit.
//!
//! `typeof`, `Number.isFinite`, `Number.isInteger`, `Number(string)`, `'k' in obj`,
//! `String.prototype.trim`, UTF-16 `.length` and `JSON.stringify` all behave in
//! ways a Rust default does not. Each one the profile logic needs lives here.

use euchre_protocol::serde_util::js_number_string;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// `Number.MAX_SAFE_INTEGER` (`2^53 - 1`).
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// True for a character JS `\s` and `String.prototype.trim` treat as whitespace
/// (ECMAScript WhiteSpace plus LineTerminator).
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000a}'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `String.prototype.length`: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(0, max)` in UTF-16 units, except that a surrogate pair straddling
/// the cut is dropped whole rather than split (Rust strings cannot hold a lone
/// surrogate; TS would keep the high half).
pub fn truncate_utf16(s: &str, max: usize) -> &str {
    let mut units = 0usize;
    for (index, c) in s.char_indices() {
        let width = c.len_utf16();
        if units + width > max {
            return &s[..index];
        }
        units += width;
    }
    s
}

/// `isRecord`: a non-null, non-array object.
pub fn is_record(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Object(_)))
}

/// `pick(value, key)`: `value[key]` when `value` is a record, else `undefined`.
pub fn pick<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match value {
        Some(Value::Object(map)) => map.get(key),
        _ => None,
    }
}

/// `value === null || value === undefined`.
pub fn is_nullish(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

/// `value ?? fallback`.
pub fn or_default<'a>(value: Option<&'a Value>, fallback: &'a Value) -> &'a Value {
    match value {
        None | Some(Value::Null) => fallback,
        Some(v) => v,
    }
}

/// `typeof value === 'number' ? value : undefined`, as an `f64`.
pub fn as_number(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }
}

/// `Number(text)` for a JS string: `NaN` when the text is not a number.
pub fn js_string_to_number(text: &str) -> f64 {
    let t = js_trim(text);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = match t.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &t[2..];
        if digits.is_empty() {
            return f64::NAN;
        }
        let mut acc = 0f64;
        for c in digits.chars() {
            match c.to_digit(radix) {
                Some(d) => acc = acc * f64::from(radix) + f64::from(d),
                None => return f64::NAN,
            }
        }
        return acc;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Math.trunc(v)` saturated into an `i64` (finite input only matters).
pub fn trunc_i64(v: f64) -> i64 {
    // `as` saturates and maps NaN to 0; callers pass finite values.
    v.trunc() as i64
}

/// `JSON.stringify(value)` for a decoded JSON value.
///
/// Numbers print as JS numbers (`1`, not `1.0`; `0.000001`, `1e-7`), and object
/// keys follow JS own-property order: array-index keys ascending first, then
/// the rest in insertion order.
pub fn js_json_string(value: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&js_number_text(n)),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, item)) in js_key_order(map).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_json(out, item);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    // serde_json escapes exactly what JSON.stringify escapes for well-formed
    // strings: `"`, `\`, and controls below U+0020 (short forms where JSON has
    // them, lowercase `\u00xx` otherwise).
    match serde_json::to_string(s) {
        Ok(text) => out.push_str(&text),
        Err(_) => out.push_str("\"\""),
    }
}

fn js_number_text(n: &serde_json::Number) -> String {
    const SAFE: u64 = 1 << 53;
    if let Some(i) = n.as_i64()
        && i.unsigned_abs() <= SAFE
    {
        return i.to_string();
    }
    match n.as_f64() {
        Some(f) => js_number_string(f),
        None => "null".to_owned(),
    }
}

/// A JS array index: canonical decimal, below `2^32 - 1`.
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u32>().ok().filter(|n| *n != u32::MAX)
}

fn js_key_order(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indexed: Vec<(u32, &String, &Value)> = Vec::new();
    let mut named: Vec<(&String, &Value)> = Vec::new();
    for (key, value) in map {
        match array_index(key) {
            Some(n) => indexed.push((n, key, value)),
            None => named.push((key, value)),
        }
    }
    indexed.sort_by_key(|(n, _, _)| *n);
    indexed
        .into_iter()
        .map(|(_, k, v)| (k, v))
        .chain(named)
        .collect()
}

/// Opaque JSON carried through verbatim (a stored `cfg`, a move, a persona
/// assignment). Serializes exactly as `JSON.stringify` would on JSON
/// serializers, and as the plain value on binary ones such as CBOR.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct JsJson(pub Value);

impl JsJson {
    /// The wrapped value.
    pub fn value(&self) -> &Value {
        &self.0
    }

    /// `JSON.stringify` of the wrapped value.
    pub fn to_js_string(&self) -> String {
        js_json_string(&self.0)
    }
}

impl From<Value> for JsJson {
    fn from(value: Value) -> JsJson {
        JsJson(value)
    }
}

impl Serialize for JsJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        if !serializer.is_human_readable() {
            return self.0.serialize(serializer);
        }
        let raw = serde_json::value::RawValue::from_string(js_json_string(&self.0))
            .map_err(S::Error::custom)?;
        raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<JsJson, D::Error> {
        Value::deserialize(deserializer).map(JsJson)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whitespace_set_matches_js() {
        assert_eq!(js_trim("\u{feff}\u{3000} a \u{2029}"), "a");
        assert!(!is_js_whitespace('\u{0085}'));
        assert!(!is_js_whitespace('\u{180e}'));
    }

    #[test]
    fn utf16_helpers() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(truncate_utf16("a😀b", 2), "a");
        assert_eq!(truncate_utf16("a😀b", 3), "a😀");
    }

    #[test]
    fn number_parsing() {
        assert_eq!(js_string_to_number(" 12 "), 12.0);
        assert_eq!(js_string_to_number("0x10"), 16.0);
        assert!(js_string_to_number("inf").is_nan());
        assert!(js_string_to_number("1_0").is_nan());
        assert_eq!(js_string_to_number(".5"), 0.5);
    }

    #[test]
    fn stringify_matches_js() {
        let v: Value =
            serde_json::from_str(r#"{"b":1.0,"2":0.000001,"a":1e-7,"1":[true,null,"\u0001"]}"#)
                .unwrap_or(Value::Null);
        assert_eq!(
            js_json_string(&v),
            r#"{"1":[true,null,"\u0001"],"2":0.000001,"b":1,"a":1e-7}"#
        );
        assert_eq!(js_json_string(&json!(1e21)), "1e+21");
        assert_eq!(
            js_json_string(&json!({"01": 1, "0": 2})),
            r#"{"0":2,"01":1}"#
        );
    }
}
