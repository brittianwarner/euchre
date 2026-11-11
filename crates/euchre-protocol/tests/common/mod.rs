//! Shared helpers for the protocol integration tests: golden fixtures written
//! by `scripts/gen-fixtures.ts` from the real TS engine.

#![allow(dead_code)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A whole fixture file, parsed with key order preserved.
pub fn fixture(name: &str) -> Value {
    let text = match name {
        "actions" => include_str!("../fixtures/actions.json"),
        "ai" => include_str!("../fixtures/ai.json"),
        "events" => include_str!("../fixtures/events.json"),
        "numbers" => include_str!("../fixtures/numbers.json"),
        "pacing" => include_str!("../fixtures/pacing.json"),
        "records" => include_str!("../fixtures/records.json"),
        "review" => include_str!("../fixtures/review.json"),
        "steps" => include_str!("../fixtures/steps.json"),
        "sync" => include_str!("../fixtures/sync.json"),
        "view" => include_str!("../fixtures/view.json"),
        other => panic!("no fixture {other}"),
    };
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}.json: {e}"))
}

/// One entry of a fixture file.
pub fn entry(file: &str, key: &str) -> Value {
    fixture(file)
        .get(key)
        .cloned()
        .unwrap_or_else(|| panic!("{file}.json has no {key}"))
}

/// The compact JSON TS `JSON.stringify` would print for `v`.
pub fn compact(v: &Value) -> String {
    serde_json::to_string(v).expect("compact")
}

/// Deserializes `T` from the entry, re-serializes it and asserts byte equality.
pub fn round_trip<T: Serialize + DeserializeOwned>(file: &str, key: &str) -> T {
    round_trip_value(&format!("{file}.{key}"), &entry(file, key))
}

/// [`round_trip`] for an already-loaded value.
pub fn round_trip_value<T: Serialize + DeserializeOwned>(label: &str, value: &Value) -> T {
    let json = compact(value);
    let typed: T = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("{label} does not deserialize: {e}\n{json}"));
    let back = serde_json::to_string(&typed).expect("serialize");
    assert_eq!(back, json, "{label} is not byte-equal after a round trip");
    typed
}

/// Asserts that `value` serializes to exactly the fixture entry's bytes.
pub fn assert_bytes<T: Serialize>(value: &T, file: &str, key: &str) {
    let want = compact(&entry(file, key));
    let got = serde_json::to_string(value).expect("serialize");
    assert_eq!(got, want, "{file}.{key} bytes differ");
}

/// Follows a `/`-separated path of object keys and array indices.
pub fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('/')
        .try_fold(v, |cur, seg| match seg.parse::<usize>() {
            Ok(i) => cur.get(i),
            Err(_) => cur.get(seg),
        })
}

/// Asserts the serialized form of `value` has no key at each path (optional fields).
pub fn assert_absent<T: Serialize>(value: &T, paths: &[&str]) {
    let v = serde_json::to_value(value).expect("to_value");
    for p in paths {
        let (parent, key) = p.rsplit_once('/').map_or(("", *p), |(a, b)| (a, b));
        let obj = if parent.is_empty() {
            Some(&v)
        } else {
            at(&v, parent)
        };
        let obj = obj.unwrap_or_else(|| panic!("no parent for {p}"));
        assert!(
            obj.as_object().is_some_and(|m| !m.contains_key(key)),
            "{p} should be omitted"
        );
    }
}

/// Asserts the serialized form of `value` has an explicit `null` at each path.
pub fn assert_null<T: Serialize>(value: &T, paths: &[&str]) {
    let v = serde_json::to_value(value).expect("to_value");
    for p in paths {
        assert_eq!(
            at(&v, p),
            Some(&Value::Null),
            "{p} should be present as null"
        );
    }
}
