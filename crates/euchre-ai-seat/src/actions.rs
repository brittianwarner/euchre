//! Wire types of every `aiSeat` action.
//!
//! The four TS queues (`decide`, `resetHand`, `handEnd`, `gameEnd`) are actions
//! of the same names that take the queue body as their one argument and return
//! `null` as soon as the message is accepted (03 §16.7: the seat acks
//! immediately and answers later through the table's `aiDecision` action).
//!
//! Each action is a newtype over the **whole positional argument array**
//! (`Vec<Value>`): `decode_positional` binds the array to the `Vec` on its first
//! try (G32), so the handler sees the raw body and a malformed one is logged
//! like the TS run loop did (`aiSeat queue handler failed`) instead of failing
//! as a masked decode error (G14). Build calls with the `new` constructors,
//! which serialize to the same positional array a TS client sends.

use euchre_ai::ai_seat::state::DecisionRecord;
use euchre_ai::ai_seat::status::AiStatus;
use euchre_protocol::PersonaView;
use rivetkit::Action;
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! seat_action {
    ($(#[$doc:meta])* $ty:ident, $name:literal, $out:ty) => {
        $(#[$doc])*
        #[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
        pub struct $ty(pub Vec<Value>);

        impl Action for $ty {
            type Output = $out;
            const NAME: &'static str = $name;
        }

        impl $ty {
            /// The positional argument at `index` (`undefined` and `null` alike
            /// read as absent).
            pub fn arg(&self, index: usize) -> Option<&Value> {
                self.0.get(index).filter(|v| !v.is_null())
            }
        }
    };
}

macro_rules! message_action {
    ($(#[$doc:meta])* $ty:ident, $name:literal) => {
        seat_action!($(#[$doc])* $ty, $name, ());

        impl $ty {
            /// The call `[body]`.
            pub fn new<T: Serialize>(body: &T) -> serde_json::Result<Self> {
                Ok(Self(vec![serde_json::to_value(body)?]))
            }

            /// The message body (argument 0), `null` when absent.
            pub fn body(&self) -> Value {
                self.0.first().cloned().unwrap_or(Value::Null)
            }
        }
    };
}

macro_rules! read_action {
    ($(#[$doc:meta])* $ty:ident, $name:literal, $out:ty) => {
        seat_action!($(#[$doc])* $ty, $name, $out);

        impl $ty {
            /// The call `[]`.
            pub fn new() -> Self {
                Self(Vec::new())
            }
        }
    };
}

message_action!(
    /// TS queue `decide`: `AIDecideRequest` → `null`. The `AIDecision` goes
    /// back to `euchreTable.aiDecision`.
    Decide,
    "decide"
);
message_action!(
    /// TS queue `resetHand`: `AiSeatLifecycleMessage` → `null`. Clears the
    /// per-hand memory.
    ResetHand,
    "resetHand"
);
message_action!(
    /// TS queue `handEnd`: `AiSeatLifecycleMessage` → `null`. Notes the hand
    /// and flushes buffered episodes to `playerProfile.recordEpisodes`.
    HandEnd,
    "handEnd"
);
message_action!(
    /// TS queue `gameEnd`: `AiSeatLifecycleMessage` → `null`. Flushes episodes,
    /// then the match's token spend to `playerProfile.recordTokens`.
    GameEnd,
    "gameEnd"
);
read_action!(
    /// `getStatus()` → `AiStatus` (no card identity, no prompt text).
    GetStatus,
    "getStatus",
    AiStatus
);
read_action!(
    /// `getPersona()` → `PersonaView` (no prompt, no model ids).
    GetPersona,
    "getPersona",
    PersonaView
);
read_action!(
    /// `getDecisionLog()` → `DecisionRecord[]` (debug surface).
    GetDecisionLog,
    "getDecisionLog",
    Vec<DecisionRecord>
);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calls_serialize_to_the_ts_positional_array() {
        let call = Decide::new(&json!({ "turnId": "t1" })).map_err(|e| e.to_string());
        assert_eq!(
            call.as_ref().map(|c| serde_json::to_value(c).ok()),
            Ok(Some(json!([{ "turnId": "t1" }])))
        );
        assert_eq!(serde_json::to_value(GetStatus::new()).ok(), Some(json!([])));
        assert_eq!(Decide::NAME, "decide");
        assert_eq!(ResetHand::NAME, "resetHand");
        assert_eq!(HandEnd::NAME, "handEnd");
        assert_eq!(GameEnd::NAME, "gameEnd");
        assert_eq!(GetStatus::NAME, "getStatus");
        assert_eq!(GetPersona::NAME, "getPersona");
        assert_eq!(GetDecisionLog::NAME, "getDecisionLog");
    }

    #[test]
    fn a_missing_or_null_body_reads_as_null() {
        assert_eq!(HandEnd(Vec::new()).body(), Value::Null);
        assert_eq!(HandEnd(vec![Value::Null]).arg(0), None);
        assert_eq!(GameEnd(vec![json!({"a": 1})]).body(), json!({"a": 1}));
    }
}
