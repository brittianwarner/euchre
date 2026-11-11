//! Small serde helpers that keep JSON byte-compatible with the TS app.
//!
//! - [`string_enum!`] declares a fieldless enum with one exact wire string per variant.
//! - [`True`] is the TS literal type `true` (`ChatEvent.final`, `{ queued: true }`).
//! - [`js_f64`] writes an `f64` the way `JSON.stringify` writes a JS number
//!   (`1` rather than `1.0`, `0.0000015` rather than `1.5e-6`).
//! - [`double_option`] distinguishes an absent key from an explicit `null`
//!   for TS fields typed `T | null` *and* optional (`upCard?: CardId | null`).
//! - [`lenient`] reads untrusted client fields the way a TS handler does,
//!   without failing before the handler's own checks run.

use std::fmt;

use serde::de::{self, Deserializer, Unexpected};
use serde::{Deserialize, Serialize, Serializer};

/// Error returned when a wire string does not name a variant of a protocol string enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownVariant {
    /// The Rust type that was being parsed.
    pub type_name: &'static str,
    /// The rejected input.
    pub value: String,
}

impl fmt::Display for UnknownVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not a valid {}", self.value, self.type_name)
    }
}

impl std::error::Error for UnknownVariant {}

/// Declares a fieldless enum whose serde form, `as_str`, `Display` and `FromStr`
/// all use one exact wire string per variant, plus an `ALL` list in declaration order.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $s:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $s)] $variant ),+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The exact wire string of this variant.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $s),+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::serde_util::UnknownVariant;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($s => Ok($name::$variant),)+
                    _ => Err($crate::serde_util::UnknownVariant {
                        type_name: stringify!($name),
                        value: s.to_owned(),
                    }),
                }
            }
        }
    };
}

pub(crate) use string_enum;

/// The TS literal type `true`: serializes as `true` and rejects `false` on read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct True;

impl Serialize for True {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for True {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(True)
        } else {
            Err(de::Error::invalid_value(
                Unexpected::Bool(false),
                &"the literal true",
            ))
        }
    }
}

/// `Number.prototype.toString()` for a JS number (ECMAScript `Number::toString`,
/// radix 10): the shortest round-trip digits, laid out as a plain decimal when
/// `1e-6 <= |v| < 1e21` and as `d.ddde±n` otherwise. `JSON.stringify` prints
/// every finite number this way.
pub fn js_number_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_owned();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if v == 0.0 {
        // Both zeros print as `0`.
        return "0".to_owned();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    // Rust's `{:e}` with no precision is the shortest round-trip form, e.g.
    // `1.5e-6` or `1e20`: the same digits ECMAScript asks for.
    let sci = format!("{:e}", v.abs());
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return format!("{v}");
    };
    let Ok(exp) = exp.parse::<i32>() else {
        return format!("{v}");
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    // ECMAScript names: `s` has `k` digits and the value is `s * 10^(n - k)`.
    let k = digits.len() as i32;
    let n = exp + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        format!("{int}.{frac}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let e = if e >= 0 {
            format!("+{e}")
        } else {
            e.to_string()
        };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{e}")
        } else {
            format!("{first}.{rest}e{e}")
        }
    };
    format!("{sign}{body}")
}

/// Exclusive bound of the integers an `i64` holds (`2^63`).
const I64_BOUND: f64 = 9_223_372_036_854_775_808.0;

/// `#[serde(with = "js_f64")]` for an `f64` that must print like a JS number.
pub mod js_f64 {
    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::value::RawValue;

    /// Writes `v` exactly as `JSON.stringify` would.
    ///
    /// Integral values that fit an `i64` are written as integers (`12`, and `0`
    /// for `-0`) on every serializer. Other finite values are written as raw
    /// JSON number text from [`super::js_number_string`] on human-readable
    /// serializers (`serde_json`; `serde_json::to_value` keeps the numeric value
    /// but not the text) and as a plain `f64` on binary ones such as CBOR.
    /// Non-finite values go to `serialize_f64`, which `serde_json` writes as
    /// `null`, as `JSON.stringify` does.
    pub fn serialize<S: Serializer>(v: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        let v = *v;
        if !v.is_finite() {
            return serializer.serialize_f64(v);
        }
        if v.fract() == 0.0 && v.abs() < super::I64_BOUND {
            // In range by the check above, so the cast is exact.
            return serializer.serialize_i64(v as i64);
        }
        if !serializer.is_human_readable() {
            return serializer.serialize_f64(v);
        }
        let raw = RawValue::from_string(super::js_number_string(v)).map_err(S::Error::custom)?;
        raw.serialize(serializer)
    }

    /// Reads any JSON number as an `f64`.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        f64::deserialize(deserializer)
    }
}

/// Field readers for untrusted client input that mirror a TS handler's own
/// checks instead of failing the whole payload (`#[serde(deserialize_with = ...)]`).
pub mod lenient {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// A JS string field read as `typeof v === 'string' ? v : ''`.
    pub fn js_string(v: Option<&Value>) -> String {
        match v {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        }
    }

    /// Reads a string, or `""` for any other JSON value (`null`, a number, ...).
    pub fn string_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        Ok(js_string(Some(&Value::deserialize(d)?)))
    }

    /// Reads a string as `Some`, and any other JSON value as `None`.
    pub fn string_or_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) => Some(s),
            _ => None,
        })
    }

    /// Keeps a present value as is, `null` included (`Some(Value::Null)`); pair
    /// it with `#[serde(default)]` so an absent key reads as `None`.
    pub fn present_value<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
        Value::deserialize(d).map(Some)
    }
}

/// `#[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]`
/// for a TS field typed `field?: T | null`: `None` is an absent key,
/// `Some(None)` an explicit `null`, `Some(Some(v))` a value.
pub mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Writes `Some(None)` as `null` and `Some(Some(v))` as `v`.
    pub fn serialize<T: Serialize, S: Serializer>(
        v: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Reads a present key: `null` becomes `Some(None)`.
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct N {
        #[serde(with = "js_f64")]
        n: f64,
    }

    #[test]
    fn js_f64_prints_like_json_stringify() {
        let cases = [
            (12.0, "12"),
            (0.5, "0.5"),
            (-0.0, "0"),
            (2.25, "2.25"),
            (1.0, "1"),
            (0.3, "0.3"),
            (0.000001, "0.000001"),
            (0.0000015, "0.0000015"),
            (0.00000999, "0.00000999"),
            (0.0000001, "1e-7"),
            (9_007_199_254_740_992.0, "9007199254740992"),
            (1e19, "10000000000000000000"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.5e300, "1.5e+300"),
            (-0.0000015, "-0.0000015"),
            (123.456, "123.456"),
        ];
        for (n, want) in cases {
            let got = serde_json::to_string(&N { n }).expect("serialize");
            assert_eq!(got, format!("{{\"n\":{want}}}"), "{n:e}");
        }
        let back: N = serde_json::from_str("{\"n\":12}").expect("integer reads as f64");
        assert_eq!(back.n, 12.0);
        let back: N = serde_json::from_str("{\"n\":0.0000015}").expect("decimal reads");
        assert_eq!(back.n, 0.0000015);
    }

    #[test]
    fn js_f64_keeps_the_value_through_to_value_and_writes_null_for_nan() {
        let v = serde_json::to_value(N { n: 0.0000015 }).expect("to_value");
        assert_eq!(v["n"].as_f64(), Some(0.0000015));
        let v = serde_json::to_value(N { n: 1e20 }).expect("to_value");
        assert_eq!(v["n"].as_f64(), Some(1e20));
        assert_eq!(
            serde_json::to_string(&N { n: f64::NAN }).expect("serialize"),
            "{\"n\":null}"
        );
    }

    #[test]
    fn js_number_string_matches_ecmascript_layout() {
        assert_eq!(js_number_string(f64::NAN), "NaN");
        assert_eq!(js_number_string(f64::INFINITY), "Infinity");
        assert_eq!(js_number_string(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(js_number_string(5e-324), "5e-324");
        assert_eq!(
            js_number_string(1.7976931348623157e308),
            "1.7976931348623157e+308"
        );
        assert_eq!(js_number_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(js_number_string(1e21 + 1e6), "1.000000000000001e+21");
    }

    #[test]
    fn true_literal_rejects_false() {
        assert_eq!(serde_json::to_string(&True).expect("serialize"), "true");
        assert!(serde_json::from_str::<True>("true").is_ok());
        assert!(serde_json::from_str::<True>("false").is_err());
    }
}
