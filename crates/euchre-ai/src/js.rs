//! JavaScript semantics the TS AI relies on wherever a number or a string length
//! reaches a string, a JSON body or an RNG (ARCHITECTURE D27).
//!
//! Everything here is small and total: UTF-16 lengths and slices, `Math.round`,
//! `Number.prototype.toFixed`, the ECMAScript whitespace set behind `trim`, and a
//! `JSON.stringify` writer for [`serde_json::Value`] that prints every number the
//! way JS does.

use euchre_protocol::serde_util::js_number_string;
use serde_json::Value;

/// `s.length`: the number of UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(0, max)` counted in UTF-16 code units, except that a surrogate pair
/// is never split: a character that would straddle `max` is dropped whole, so
/// the result may be one unit shorter than JS would return (D27).
pub fn utf16_prefix(s: &str, max: usize) -> &str {
    let mut units = 0usize;
    for (i, c) in s.char_indices() {
        let next = units + c.len_utf16();
        if next > max {
            return &s[..i];
        }
        units = next;
    }
    s
}

/// `Math.round(x)`: rounds half toward `+Infinity` (unlike [`f64::round`]).
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let r = x.floor();
    if x - r >= 0.5 { r + 1.0 } else { r }
}

/// `Math.round(x * 100) / 100`, the TS "two decimals" idiom.
pub fn round2(x: f64) -> f64 {
    js_round(x * 100.0) / 100.0
}

/// `Number.prototype.toFixed(digits)` for `digits <= 20`.
///
/// JS rounds the **exact** binary value and breaks an exact tie upward; Rust's
/// `{:.N}` breaks it to even, so this rounds the exact expansion by hand.
pub fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() {
        return js_number_string(x);
    }
    if x.abs() >= 1e21 {
        return js_number_string(x);
    }
    let negative = x < 0.0;
    // 1100 digits is enough for the exact expansion of any f64 below 1e21.
    let exact = format!("{:.1100}", x.abs());
    let (int_part, frac_part) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let mut digits_vec: Vec<u8> = int_part.bytes().collect();
    let frac = frac_part.as_bytes();
    for i in 0..digits {
        digits_vec.push(frac.get(i).copied().unwrap_or(b'0'));
    }
    let round_up = frac.get(digits).is_some_and(|d| *d >= b'5');
    if round_up {
        let mut i = digits_vec.len();
        loop {
            if i == 0 {
                digits_vec.insert(0, b'1');
                break;
            }
            i -= 1;
            if digits_vec[i] == b'9' {
                digits_vec[i] = b'0';
            } else {
                digits_vec[i] += 1;
                break;
            }
        }
    }
    let split = digits_vec.len() - digits;
    let int_str: String = digits_vec[..split].iter().map(|b| *b as char).collect();
    let frac_str: String = digits_vec[split..].iter().map(|b| *b as char).collect();
    let body = if digits == 0 {
        int_str
    } else {
        format!("{int_str}.{frac_str}")
    };
    // JS prints `(-0.0001).toFixed(2)` as "-0.00": the sign follows `x < 0`.
    if negative { format!("-{body}") } else { body }
}

/// ECMAScript `WhiteSpace` plus `LineTerminator`: the set `trim()` strips and the
/// regex class `\s` matches.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `s.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `s.trimEnd()`.
pub fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_whitespace)
}

/// The regex source for JS `\s` (use inside a character class or alone).
pub const JS_WS_CLASS: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

/// `JSON.stringify(value)` for a [`Value`] tree: numbers print as JS numbers
/// (`2`, not `2.0`; `0.0000015`, not `1.5e-6`) and object keys keep insertion
/// order (`serde_json` is built with `preserve_order`).
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

/// A JS-formatted number as a [`Value`]: integral values become integers so they
/// print without a fraction; a non-finite value becomes `null`, as in JSON.
pub fn num(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    if x.fract() == 0.0 && x.abs() < 9_007_199_254_740_992.0 {
        // Exact for every safe integer; `-0` prints as `0` in JS too.
        return Value::from(x as i64);
    }
    serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else if let Some(f) = n.as_f64() {
                if f.is_finite() {
                    out.push_str(&js_number_string(f));
                } else {
                    out.push_str("null");
                }
            } else {
                out.push_str("null");
            }
        }
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify` string quoting (`QuoteJSONString`).
pub fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.stringify(s)` for one string.
pub fn quote(s: &str) -> String {
    let mut out = String::new();
    write_string(&mut out, s);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_matches_js() {
        assert_eq!(to_fixed(0.0625, 3), "0.063");
        assert_eq!(to_fixed(1.0005, 3), "1.000");
        assert_eq!(to_fixed(0.5, 0), "1");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(0.1, 3), "0.100");
        assert_eq!(to_fixed(-0.0625, 3), "-0.063");
        assert_eq!(to_fixed(9.9996, 3), "10.000");
        assert_eq!(to_fixed(0.0, 3), "0.000");
    }

    #[test]
    fn js_round_rounds_half_up() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(-0.4), 0.0);
        assert_eq!(round2(1.005), 1.0);
    }

    #[test]
    fn utf16_prefix_never_splits_a_pair() {
        assert_eq!(utf16_prefix("ab\u{1F600}c", 3), "ab");
        assert_eq!(utf16_prefix("ab\u{1F600}c", 4), "ab\u{1F600}");
        assert_eq!(utf16_len("ab\u{1F600}c"), 5);
    }

    #[test]
    fn stringify_prints_js_numbers() {
        let v = serde_json::json!({"a": 2.0, "b": 0.0000015, "c": [1, -0.5], "d": "x\u{1}"});
        assert_eq!(
            stringify(&v),
            r#"{"a":2,"b":0.0000015,"c":[1,-0.5],"d":"x\u0001"}"#
        );
    }
}
