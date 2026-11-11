//! Wire types of every `euchreTable` action (03 §6, §16.3).
//!
//! Each action is a newtype over the **whole positional argument array**
//! (`Vec<Value>`), so any arity decodes (G32) and the handler validates each
//! argument as TS did. A shape error is therefore the TS `user/<code>` error,
//! never a masked decode failure (G14).
//!
//! | Action | Arguments | Result | Who |
//! | --- | --- | --- | --- |
//! | `snapshot` | none | `PublicGameView` (wire JSON) | player, internal |
//! | `submitMove` | `SubmitMoveRequest` | `MoveAck` | player |
//! | `reviewTrick` | `ReviewTrickRequest` | `{ queued: true }` | player |
//! | `getHandReview` | `handNo` | `CompletedHandReview \| null` | player |
//! | `lastTrick` | none | `Trick \| null` | player, internal |
//! | `move` | `MoveMsg` | `MoveAck` | player (former queue) |
//! | `review` | `ReviewMsg` | `null` | internal (former queue) |
//! | `aiDecision` | `AIDecision` | `null` | internal (former queue) |
//! | `aiSay` | `AiSayMsg` | `null` | internal (former queue) |
//! | `tick` | `TickMsg` | `null` | internal (former queue) |
//! | `onTempoGate` / `releaseAiMove` / `onNudge` / `onAbandon` | `turnId` | `null` | schedule |
//! | `onAiTimeout` | `turnId, seat` | `null` | schedule |
//! | `flushProfile` / `onReap` | none | `null` | schedule |

use euchre_engine::Trick;
use euchre_protocol::{CompletedHandReview, MoveAck, ReviewTrickAck};
use rivetkit::Action;
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! table_action {
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

            /// The first argument, or JSON `null`.
            pub fn first(&self) -> Value {
                self.0.first().cloned().unwrap_or(Value::Null)
            }
        }
    };
}

table_action!(
    /// `snapshot()` → the caller's `PublicGameView`.
    Snapshot,
    "snapshot",
    Value
);
table_action!(
    /// `submitMove(request)` → `MoveAck`.
    SubmitMove,
    "submitMove",
    MoveAck
);
table_action!(
    /// `reviewTrick(request)` → `{ queued: true }`.
    ReviewTrick,
    "reviewTrick",
    ReviewTrickAck
);
table_action!(
    /// `getHandReview(handNo)` → `CompletedHandReview | null`.
    GetHandReview,
    "getHandReview",
    Option<CompletedHandReview>
);
table_action!(
    /// `lastTrick()` → the last completed trick, or `null`.
    LastTrick,
    "lastTrick",
    Option<Trick>
);
table_action!(
    /// Former queue `move`: `MoveMsg` → `MoveAck`.
    MoveQueue,
    "move",
    MoveAck
);
table_action!(
    /// Former queue `review`: `ReviewMsg` → `null`.
    ReviewQueue,
    "review",
    ()
);
table_action!(
    /// Former queue `aiDecision`: `AIDecision` → `null`.
    AiDecisionQueue,
    "aiDecision",
    ()
);
table_action!(
    /// Former queue `aiSay`: `AiSayMsg` → `null`.
    AiSayQueue,
    "aiSay",
    ()
);
table_action!(
    /// Former queue `tick`: `TickMsg` → `null`.
    TickQueue,
    "tick",
    ()
);
table_action!(
    /// Schedule target `onTempoGate(turnId)`.
    OnTempoGate,
    "onTempoGate",
    ()
);
table_action!(
    /// Schedule target `releaseAiMove(turnId)`.
    ReleaseAiMove,
    "releaseAiMove",
    ()
);
table_action!(
    /// Schedule target `onAiTimeout(turnId, seat)`.
    OnAiTimeout,
    "onAiTimeout",
    ()
);
table_action!(
    /// Legacy schedule target `onNudge(turnId)`: no game effect (D7).
    OnNudge,
    "onNudge",
    ()
);
table_action!(
    /// Legacy schedule target `onAbandon(turnId)`: no game effect (D7).
    OnAbandon,
    "onAbandon",
    ()
);
table_action!(
    /// Schedule target `flushProfile()`: retry recording the match.
    FlushProfile,
    "flushProfile",
    ()
);
table_action!(
    /// Schedule target `onReap()`: destroy a recorded table.
    OnReap,
    "onReap",
    ()
);

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_protocol::{table_actions as a, table_queues as q};

    #[test]
    fn names_match_the_protocol() {
        assert_eq!(
            [
                Snapshot::NAME,
                SubmitMove::NAME,
                ReviewTrick::NAME,
                GetHandReview::NAME,
                LastTrick::NAME,
            ],
            [
                a::SNAPSHOT,
                a::SUBMIT_MOVE,
                a::REVIEW_TRICK,
                a::GET_HAND_REVIEW,
                a::LAST_TRICK
            ]
        );
        assert_eq!(
            [
                OnTempoGate::NAME,
                ReleaseAiMove::NAME,
                OnAiTimeout::NAME,
                OnNudge::NAME,
                OnAbandon::NAME,
                FlushProfile::NAME,
                OnReap::NAME,
            ],
            [
                a::ON_TEMPO_GATE,
                a::RELEASE_AI_MOVE,
                a::ON_AI_TIMEOUT,
                a::ON_NUDGE,
                a::ON_ABANDON,
                a::FLUSH_PROFILE,
                a::ON_REAP
            ]
        );
        assert_eq!(
            [
                MoveQueue::NAME,
                ReviewQueue::NAME,
                AiDecisionQueue::NAME,
                AiSayQueue::NAME,
                TickQueue::NAME,
            ],
            [q::MOVE, q::REVIEW, q::AI_DECISION, q::AI_SAY, q::TICK]
        );
    }

    #[test]
    fn args_read_null_as_absent() {
        let a = GetHandReview(vec![Value::Null, Value::from(2)]);
        assert_eq!(a.arg(0), None);
        assert_eq!(a.arg(1), Some(&Value::from(2)));
        assert_eq!(Snapshot::default().first(), Value::Null);
    }
}
