//! Budgets and the decision-kind tier map (`ai/config.ts`), plus the aiSeat
//! caps and ladder timings (`actors/ai-seat/types.ts`, ARCHITECTURE D24).

use euchre_protocol::AIDecisionKind;

use crate::types::{DecisionBudget, DecisionBudgetOverrides};

/// `DEFAULT_DECISION_BUDGET`: starting points from the spec's budget table.
pub const DEFAULT_DECISION_BUDGET: DecisionBudget = DecisionBudget {
    first_attempt_ms: 6500,
    escalation_ms: 3000,
    reserve_ms: 400,
    min_attempt_ms: 600,
    banter_ms: 4000,
    max_output_tokens_bid: 1600,
    max_output_tokens_play: 1200,
    max_output_tokens_banter: 800,
};

/// `resolveBudget(overrides)`: the defaults with any overrides applied.
pub fn resolve_budget(overrides: Option<&DecisionBudgetOverrides>) -> DecisionBudget {
    let d = DEFAULT_DECISION_BUDGET;
    let Some(o) = overrides else { return d };
    DecisionBudget {
        first_attempt_ms: o.first_attempt_ms.unwrap_or(d.first_attempt_ms),
        escalation_ms: o.escalation_ms.unwrap_or(d.escalation_ms),
        reserve_ms: o.reserve_ms.unwrap_or(d.reserve_ms),
        min_attempt_ms: o.min_attempt_ms.unwrap_or(d.min_attempt_ms),
        banter_ms: o.banter_ms.unwrap_or(d.banter_ms),
        max_output_tokens_bid: o.max_output_tokens_bid.unwrap_or(d.max_output_tokens_bid),
        max_output_tokens_play: o.max_output_tokens_play.unwrap_or(d.max_output_tokens_play),
        max_output_tokens_banter: o
            .max_output_tokens_banter
            .unwrap_or(d.max_output_tokens_banter),
    }
}

/// Which model tier a decision belongs to (`tierOf`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// `"fast"`: card play.
    Fast,
    /// `"deliberate"`: the cut, both bid rounds and the discard.
    Deliberate,
}

impl Tier {
    /// The TS string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::Fast => "fast",
            Tier::Deliberate => "deliberate",
        }
    }
}

/// `tierOf(kind)`.
pub fn tier_of(kind: AIDecisionKind) -> Tier {
    if kind == AIDecisionKind::Play {
        Tier::Fast
    } else {
        Tier::Deliberate
    }
}

/// `isDeliberate(kind)`.
pub fn is_deliberate(kind: AIDecisionKind) -> bool {
    tier_of(kind) == Tier::Deliberate
}

/// `allowsEscalation(kind)`: the one escalation rung is bid-path only.
pub fn allows_escalation(kind: AIDecisionKind) -> bool {
    is_deliberate(kind)
}

/* -------------------------------------------------------------------------- */
/* aiSeat caps (`ai-seat/types.ts`)                                            */
/* -------------------------------------------------------------------------- */

/// Distinct trump cards remembered per hand.
pub const MEM_TRUMP_SEEN_CAP: usize = 7;
/// Void suits remembered per seat.
pub const MEM_VOID_SUITS_CAP: usize = 3;
/// Leads remembered per hand.
pub const MEM_LEAD_CAP: usize = 5;
/// Bids remembered per hand.
pub const MEM_BID_CAP: usize = 8;
/// Cross-hand notes kept.
pub const MEM_NOTE_CAP: usize = 8;
/// Max note length (UTF-16 units).
pub const NOTE_MAX_CHARS: usize = 140;
/// Buffered episodes before the oldest is dropped.
pub const EPISODE_BUFFER_CAP: usize = 12;
/// Latency samples kept for the meter.
pub const LATENCY_SAMPLE_CAP: usize = 32;
/// Decision log entries kept.
pub const DECISION_LOG_CAP: usize = 24;
/// Match input-token budget before the circuit breaker trips.
pub const BUDGET_INPUT_TOKENS: u64 = 250_000;
/// Match output-token budget before the circuit breaker trips.
pub const BUDGET_OUTPUT_TOKENS: u64 = 20_000;
/// Match call budget before the circuit breaker trips.
pub const BUDGET_CALLS: u64 = 300;

/// First model attempt budget on the actor ladder (D24).
pub const LLM_FIRST_ATTEMPT_MS: u64 = 2200;
/// Escalation attempt budget (D24).
pub const LLM_ESCALATION_MS: u64 = 1400;
/// An attempt shorter than this is skipped (D24).
pub const LLM_MIN_ATTEMPT_MS: u64 = 400;
/// Time kept back from the table deadline (D24).
pub const DEADLINE_RESERVE_MS: u64 = 300;
/// Banter writer budget.
pub const BANTER_BUDGET_MS: u64 = 1800;
/// Table hard cap on an AI turn (D24).
pub const AI_HARD_CAP_MS: u64 = 4000;

/// Banter lines per trick.
pub const BANTER_MAX_PER_TRICK: u32 = 1;
/// Banter lines per hand.
pub const BANTER_MAX_PER_HAND: u32 = 4;
/// Minimum gap between banter lines.
pub const BANTER_COOLDOWN_MS: u64 = 2500;
