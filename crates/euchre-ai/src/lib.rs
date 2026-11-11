//! euchre-ai: the euchre AI (port of `src/lib/ai` and `src/lib/actors/ai-seat`).
//!
//! The deterministic core never touches the network or the clock: the
//! heuristic ([`rank_moves`], [`top_move`]) is shared with the table actor (D8);
//! the rollouts are CPU-bound and run under `spawn_blocking`; the Jev request
//! body ([`decision_context::jev_request_body`]) is byte-identical to the TS
//! `JSON.stringify` output.
//!
//! The model layer sits behind injected seams: [`jev::JevClient`] (the only
//! decision path), [`talk::TalkClient`] (streamed table talk),
//! [`model::ModelFactory`] (built once from the environment), and
//! [`types::Clock`] / [`types::AiLog`] / [`types::SeatLog`]. The aiSeat ladder
//! and queue handlers ([`ai_seat::ladder`], [`ai_seat::handlers`]) take an
//! [`ai_seat::bridge::LlmProvider`] instead of module globals. [`testing`] holds
//! scripted stand-ins; nothing in the tests reaches a real network.

// The model, clock and seat layers need tokio and reqwest, which are native-only; the browser
// build of the desktop app uses only the deterministic heuristic.
#[cfg(not(target_arch = "wasm32"))]
pub mod ai_seat;
#[cfg(not(target_arch = "wasm32"))]
pub mod banter;
pub mod bid_rollouts;
#[cfg(not(target_arch = "wasm32"))]
pub mod clock;
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod decide;
pub mod decision_context;
pub mod heuristic;
#[cfg(not(target_arch = "wasm32"))]
pub mod jev;
pub mod js;
pub mod log;
#[cfg(not(target_arch = "wasm32"))]
pub mod model;
pub mod notation;
pub mod partnership;
pub mod play_rollouts;
pub mod prompt;
pub mod rng;
pub mod schema;
pub mod screen;
pub mod table_context;
#[cfg(not(target_arch = "wasm32"))]
pub mod talk;
#[cfg(not(target_arch = "wasm32"))]
pub mod testing;
pub mod types;

pub use bid_rollouts::{BidRollout, BidRolloutParams, RolloutError, bid_rollouts};
pub use decision_context::{
    DecisionQuestion, DecisionQuestionParams, build_decision_question, jev_request_body,
};
pub use heuristic::{HeuristicOptions, RankParams, rank_moves, top_move};
pub use play_rollouts::{PlayRollout, PlayRolloutParams, play_rollouts};

use euchre_engine::{AiDecisionSource, LegalMove};
use euchre_protocol::RankedMove;

use crate::js::utf16_prefix;
use crate::types::{DecideOutcome, ZERO_USAGE};

/// The `rationale` cap of [`heuristic_outcome`].
pub const HEURISTIC_RATIONALE_MAX: usize = 120;

/// Parameters of [`heuristic_outcome`] (the parts of `DecisionRequest` it reads).
#[derive(Clone, Copy, Debug)]
pub struct HeuristicOutcomeParams<'a> {
    /// The request's full legal set.
    pub legal: &'a [LegalMove],
    /// The request's ranking, when supplied.
    pub ranking: Option<&'a [RankedMove]>,
    /// The candidate set the fallback must stay inside.
    pub set: &'a [LegalMove],
}

/// `topRanked(ranking, ids)`: best score inside `set`, ties to the smaller id.
fn top_ranked<'a>(ranking: Option<&'a [RankedMove]>, set: &[LegalMove]) -> Option<&'a RankedMove> {
    let mut best: Option<&RankedMove> = None;
    for r in ranking? {
        if !set.iter().any(|m| m.id == r.id) {
            continue;
        }
        best = match best {
            Some(b) if !(r.score > b.score || (r.score == b.score && r.id < b.id)) => Some(b),
            _ => Some(r),
        };
    }
    best
}

/// `heuristicOutcome(req, set)` (`ai/decide.ts`): the fallback decision —
/// source `fallback`, confidence `0.5`, zero usage, no model, zero attempts, and
/// the rationale cut to 120 UTF-16 units. `None` only when both `set` and
/// `legal` are empty (where TS throws).
pub fn heuristic_outcome(params: HeuristicOutcomeParams<'_>) -> Option<DecideOutcome> {
    let top = top_ranked(params.ranking, params.set);
    let move_id = top
        .map(|t| t.id.clone())
        .or_else(|| params.set.first().map(|m| m.id.clone()))
        .or_else(|| params.legal.first().map(|m| m.id.clone()))?;
    let why = top.map_or("", |t| t.why.as_str());
    Some(DecideOutcome {
        move_id,
        source: AiDecisionSource::Fallback,
        rationale: utf16_prefix(why, HEURISTIC_RATIONALE_MAX).to_owned(),
        confidence: 0.5,
        usage: ZERO_USAGE,
        model_id: None,
        attempts: 0,
    })
}
