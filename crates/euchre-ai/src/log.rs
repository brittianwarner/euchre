//! The production log sink: every AI and aiSeat event through `tracing`.
//!
//! The event name (byte-exact, from [`crate::types::AiLogEvent`] or
//! [`crate::types::seat_events`]) is the `event` field; the structured fields
//! are one JSON object (`fields`), serialized with JS number formatting.

use serde_json::{Map, Value};

use crate::js::stringify;
use crate::types::{AiLog, AiLogEvent, SeatLog};

/// Emits through `tracing` at target `euchre_ai`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TracingLog;

fn render(fields: &Map<String, Value>) -> String {
    stringify(&Value::Object(fields.clone()))
}

impl AiLog for TracingLog {
    fn log(&self, event: AiLogEvent, fields: &Map<String, Value>) {
        tracing::info!(target: "euchre_ai", event = event.as_str(), fields = %render(fields));
    }
}

impl SeatLog for TracingLog {
    fn info(&self, event: &str, fields: &Map<String, Value>) {
        tracing::info!(target: "euchre_ai", event = event, fields = %render(fields));
    }

    fn warn(&self, message: &str, fields: &Map<String, Value>) {
        tracing::warn!(target: "euchre_ai", event = message, fields = %render(fields));
    }

    fn error(&self, message: &str, fields: &Map<String, Value>) {
        tracing::error!(target: "euchre_ai", event = message, fields = %render(fields));
    }
}
