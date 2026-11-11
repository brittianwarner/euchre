//! The output schema (`ai/schema.ts`): the mechanism that makes an illegal AI
//! move unrepresentable.
//!
//! `moveId` is an enum built fresh for every decision from the candidate set the
//! table computed with `legal_moves()`. The TS file builds a zod object; the
//! Rust port keeps the same three rules as data ([`MoveSchema`]) plus a
//! validator, and leaves rendering it to a provider's JSON schema to the LLM
//! stage.

use euchre_engine::{LegalMove, LegalMoveId};
use euchre_protocol::AIDecisionKind;

use crate::js::utf16_len;

/// `why` cap for bids, cuts and discards (they are journalled).
pub const WHY_MAX_BID: usize = 110;
/// `why` cap for card play.
pub const WHY_MAX_PLAY: usize = 90;

/// `SCHEMA_DESCRIPTION`.
pub const SCHEMA_DESCRIPTION: &str =
    "The single legal move id this player chooses, copied verbatim from LEGAL:.";

/// Asked for a schema over an empty candidate set (engine invariant V19 broken).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("cannot build a move schema over an empty candidate set")]
pub struct EmptyCandidateSetError;

impl EmptyCandidateSetError {
    /// The TS error `name`.
    pub const NAME: &'static str = "EmptyCandidateSetError";
}

/// `idsOf(candidates)`: the only supported way to feed [`move_id_enum`].
pub fn ids_of(candidates: &[LegalMove]) -> Vec<LegalMoveId> {
    candidates.iter().map(|m| m.id.clone()).collect()
}

/// `moveIdEnum(ids)`: the permitted ids for exactly one decision.
pub fn move_id_enum(ids: &[LegalMoveId]) -> Result<Vec<LegalMoveId>, EmptyCandidateSetError> {
    if ids.is_empty() {
        return Err(EmptyCandidateSetError);
    }
    Ok(ids.to_vec())
}

/// The shape every decision returns (`DecisionObject`).
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionObject {
    /// One of the enum's ids.
    pub move_id: LegalMoveId,
    /// A short reason, capped per path.
    pub why: String,
    /// Advisory confidence in `[0, 1]`.
    pub confidence: Option<f64>,
}

/// Why a [`DecisionObject`] fails its schema.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SchemaViolation {
    /// `moveId` is not in the enum.
    #[error("moveId is not one of the permitted ids")]
    UnknownMoveId,
    /// `why` exceeds its cap (UTF-16 units, as zod counts).
    #[error("why exceeds {0} characters")]
    WhyTooLong(usize),
    /// `confidence` is outside `[0, 1]` or not finite.
    #[error("confidence must be between 0 and 1")]
    ConfidenceOutOfRange,
}

/// `z.object({ moveId: z.enum(ids), why: z.string().max(whyMax), confidence })`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveSchema {
    /// The enum members, in candidate order.
    pub ids: Vec<LegalMoveId>,
    /// The `why` cap.
    pub why_max: usize,
}

impl MoveSchema {
    /// Validates a decoded object against the schema.
    pub fn validate(&self, object: &DecisionObject) -> Result<(), SchemaViolation> {
        if !self.ids.contains(&object.move_id) {
            return Err(SchemaViolation::UnknownMoveId);
        }
        if utf16_len(&object.why) > self.why_max {
            return Err(SchemaViolation::WhyTooLong(self.why_max));
        }
        if let Some(c) = object.confidence
            && !(0.0..=1.0).contains(&c)
        {
            return Err(SchemaViolation::ConfidenceOutOfRange);
        }
        Ok(())
    }
}

fn schema_for(ids: &[LegalMoveId], why_max: usize) -> Result<MoveSchema, EmptyCandidateSetError> {
    Ok(MoveSchema {
        ids: move_id_enum(ids)?,
        why_max,
    })
}

/// `bidSchema(ids)`: both bidding rounds, loner ids included.
pub fn bid_schema(ids: &[LegalMoveId]) -> Result<MoveSchema, EmptyCandidateSetError> {
    schema_for(ids, WHY_MAX_BID)
}

/// `discardSchema(ids)`.
pub fn discard_schema(ids: &[LegalMoveId]) -> Result<MoveSchema, EmptyCandidateSetError> {
    schema_for(ids, WHY_MAX_BID)
}

/// `playSchema(ids)`.
pub fn play_schema(ids: &[LegalMoveId]) -> Result<MoveSchema, EmptyCandidateSetError> {
    schema_for(ids, WHY_MAX_PLAY)
}

/// Parameters of [`decision_schema`].
#[derive(Clone, Copy, Debug)]
pub struct DecisionSchemaParams<'a> {
    /// The decision kind.
    pub kind: AIDecisionKind,
    /// The permitted ids.
    pub ids: &'a [LegalMoveId],
}

/// `decisionSchema(kind, ids)`: dispatch by decision kind.
pub fn decision_schema(
    params: DecisionSchemaParams<'_>,
) -> Result<MoveSchema, EmptyCandidateSetError> {
    match params.kind {
        AIDecisionKind::Play => play_schema(params.ids),
        AIDecisionKind::Discard => discard_schema(params.ids),
        AIDecisionKind::Cut | AIDecisionKind::Bid1 | AIDecisionKind::Bid2 => bid_schema(params.ids),
    }
}

/// `schemaNameFor(kind)`.
pub fn schema_name_for(kind: AIDecisionKind) -> &'static str {
    if kind == AIDecisionKind::Play {
        "EuchrePlay"
    } else {
        "EuchreBid"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_fails_loudly() {
        assert_eq!(bid_schema(&[]), Err(EmptyCandidateSetError));
        assert_eq!(
            EmptyCandidateSetError.to_string(),
            "cannot build a move schema over an empty candidate set"
        );
    }

    #[test]
    fn enum_rejects_unknown_ids_and_long_why() {
        let ids = vec!["pass".to_owned(), "orderUp".to_owned()];
        let schema = play_schema(&ids).unwrap_or_else(|_| unreachable!());
        let ok = DecisionObject {
            move_id: "pass".to_owned(),
            why: "x".repeat(90),
            confidence: Some(1.0),
        };
        assert_eq!(schema.validate(&ok), Ok(()));
        let long = DecisionObject {
            why: "x".repeat(91),
            ..ok.clone()
        };
        assert_eq!(schema.validate(&long), Err(SchemaViolation::WhyTooLong(90)));
        let bad = DecisionObject {
            move_id: "play:JS".to_owned(),
            ..ok
        };
        assert_eq!(schema.validate(&bad), Err(SchemaViolation::UnknownMoveId));
        assert_eq!(schema_name_for(AIDecisionKind::Bid1), "EuchreBid");
    }
}
