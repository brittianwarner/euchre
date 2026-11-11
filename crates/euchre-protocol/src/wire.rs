//! Byte-exact serialization of [`PublicGameView`] as the table sends it.
//!
//! The TS table builds its view as `{ ...project(game, seat), players,
//! observations, reviewTricks, rules, awaitingTrickReview }`, so on the wire the
//! five actor metadata keys come **after** `winnerTeam`. The engine struct
//! declares them first. [`WireView`] reorders them so every payload that embeds
//! a view (`SyncEvent`, `AIDecideRequest`, `snapshot()`) matches the TS bytes.
//! Deserialization is order-independent and needs no help.

use euchre_engine::PublicGameView;
use serde::{Serialize, Serializer};
use serde_json::Value;

/// The actor-added view keys, in the order the TS table appends them.
pub const VIEW_METADATA_KEYS: [&str; 5] = [
    "players",
    "observations",
    "reviewTricks",
    "rules",
    "awaitingTrickReview",
];

/// The view as a JSON value with the actor metadata keys moved to the end.
pub fn view_to_value(view: &PublicGameView) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(view)?;
    if let Value::Object(map) = &mut value {
        for key in VIEW_METADATA_KEYS {
            if let Some(v) = map.shift_remove(key) {
                map.insert(key.to_owned(), v);
            }
        }
    }
    Ok(value)
}

/// `#[serde(serialize_with = "serialize_view")]` for a `PublicGameView` field.
pub fn serialize_view<S: Serializer>(
    view: &PublicGameView,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    view_to_value(view)
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

/// A borrowed view that serializes in TS key order (the `snapshot()` return value).
#[derive(Clone, Copy, Debug)]
pub struct WireView<'a>(pub &'a PublicGameView);

impl Serialize for WireView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_view(self.0, serializer)
    }
}
