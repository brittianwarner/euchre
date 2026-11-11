//! Server-to-client events (02 section 3) and their names.
//!
//! `sync` is per-connection (redacted for that connection's seat); the other
//! four are broadcast. A client correlates nothing by event order across
//! connections: `v` ordering plus `snapshot()` is the source of truth.

use euchre_engine::{PublicGameView, Seat, Step};
use serde::{Deserialize, Serialize};

use crate::serde_util::{True, string_enum};
use crate::wire;

/// The authoritative view for one seat, plus the steps that produced it (`SyncEvent`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    /// The match's monotonic state counter; equal to `view.v`.
    pub v: u64,
    /// The redacted view, written in TS key order.
    #[serde(serialize_with = "wire::serialize_view")]
    pub view: PublicGameView,
    /// Decoration for animation; `[]` on connect and on resync pushes.
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// An AI seat started or stopped deliberating (`ThinkingEvent`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingEvent {
    /// The deliberating seat.
    pub seat: Seat,
    /// `true` when thinking starts, `false` when it ends.
    pub on: bool,
    /// Declared for a long-think indicator; the TS table always sends `false`.
    pub extended: bool,
}

string_enum! {
    /// What a chat line is (`ChatKind`).
    ChatKind {
        /// A spoken bid or call ("Pick it up.").
        Call => "call",
        /// Persona table talk.
        Banter => "banter",
        /// A neutral table line (throw-in, misdeal).
        System => "system",
    }
}

/// One finished chat line (`ChatEvent`); `final` is always `true`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    /// Stable id; redelivery of the same id is a duplicate.
    pub msg_id: String,
    /// The speaking seat.
    pub seat: Seat,
    /// Call, banter or system.
    pub kind: ChatKind,
    /// Screened plain text; render as text, never markup.
    pub text: String,
    /// The literal `true`.
    #[serde(rename = "final")]
    pub is_final: True,
}

/// A streamed fragment of a chat line (`ChatDeltaEvent`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDeltaEvent {
    /// The id of the line this fragment belongs to.
    pub msg_id: String,
    /// The speaking seat.
    pub seat: Seat,
    /// The appended text.
    pub delta: String,
}

/// A seat's connection came or went (`PresenceEvent`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceEvent {
    /// The seat whose presence changed.
    pub seat: Seat,
    /// Whether it is now connected.
    pub online: bool,
}

string_enum! {
    /// The name of a table event (`TableEventName`).
    TableEventName {
        /// [`SyncEvent`], per-connection.
        Sync => "sync",
        /// [`ThinkingEvent`].
        Thinking => "thinking",
        /// [`ChatEvent`].
        Chat => "chat",
        /// [`ChatDeltaEvent`].
        ChatDelta => "chatDelta",
        /// [`PresenceEvent`].
        Presence => "presence",
    }
}

/// `TABLE_EVENT_NAMES`: every event name, in TS order.
pub const TABLE_EVENT_NAMES: [&str; 5] = ["sync", "thinking", "chat", "chatDelta", "presence"];

/// `PER_CONNECTION_EVENTS`: events sent per connection (redacted), never broadcast.
pub const PER_CONNECTION_EVENTS: [&str; 1] = ["sync"];

impl TableEventName {
    /// Whether this event is sent per connection rather than broadcast.
    pub fn is_per_connection(self) -> bool {
        PER_CONNECTION_EVENTS.contains(&self.as_str())
    }
}

/// Any table event, decoded by name (`TableEventMap`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TableEvent {
    /// `sync` (boxed: a view is far larger than the other payloads).
    Sync(Box<SyncEvent>),
    /// `thinking`.
    Thinking(ThinkingEvent),
    /// `chat`.
    Chat(ChatEvent),
    /// `chatDelta`.
    ChatDelta(ChatDeltaEvent),
    /// `presence`.
    Presence(PresenceEvent),
}

/// Why a named event payload could not be decoded.
#[derive(Debug)]
pub enum TableEventError {
    /// The event name is not one of [`TABLE_EVENT_NAMES`].
    UnknownName(String),
    /// The payload does not match the event's shape.
    Payload(serde_json::Error),
}

impl std::fmt::Display for TableEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableEventError::UnknownName(n) => write!(f, "unknown table event {n:?}"),
            TableEventError::Payload(e) => write!(f, "bad table event payload: {e}"),
        }
    }
}

impl std::error::Error for TableEventError {}

impl TableEvent {
    /// Decodes one event from its name and JSON payload.
    pub fn decode(name: &str, payload: serde_json::Value) -> Result<TableEvent, TableEventError> {
        let kind: TableEventName = name
            .parse()
            .map_err(|_| TableEventError::UnknownName(name.to_owned()))?;
        let p = TableEventError::Payload;
        Ok(match kind {
            TableEventName::Sync => {
                TableEvent::Sync(Box::new(serde_json::from_value(payload).map_err(p)?))
            }
            TableEventName::Thinking => {
                TableEvent::Thinking(serde_json::from_value(payload).map_err(p)?)
            }
            TableEventName::Chat => TableEvent::Chat(serde_json::from_value(payload).map_err(p)?),
            TableEventName::ChatDelta => {
                TableEvent::ChatDelta(serde_json::from_value(payload).map_err(p)?)
            }
            TableEventName::Presence => {
                TableEvent::Presence(serde_json::from_value(payload).map_err(p)?)
            }
        })
    }

    /// The event's wire name.
    pub fn name(&self) -> TableEventName {
        match self {
            TableEvent::Sync(_) => TableEventName::Sync,
            TableEvent::Thinking(_) => TableEventName::Thinking,
            TableEvent::Chat(_) => TableEventName::Chat,
            TableEvent::ChatDelta(_) => TableEventName::ChatDelta,
            TableEvent::Presence(_) => TableEventName::Presence,
        }
    }
}
