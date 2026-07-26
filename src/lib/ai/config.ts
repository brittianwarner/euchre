/**
 * Provisional budgets and the decision-kind → model-tier map.
 *
 * Every number here is a **default**, not a constant of nature. `docs/03-AI-AGENTS.md`
 * §12 is explicit that no number in it is frozen before it is measured, and
 * `$lib/protocol` deliberately declares no timing constants for exactly that reason
 * — timing is table policy, expressed on the wire only as
 * `AIDecideRequest.deadlineAt`. Everything below is overridable per call through
 * `DecideDeps.budget`, and every attempt is additionally clamped against
 * `deadlineAt`, so lowering the table's cap cannot be defeated by a stale default.
 */

import type { AIDecisionKind } from '$lib/protocol';
import type { DecisionBudget } from './types';

/**
 * Starting points from the spec's budget table. Sources:
 * `docs/03-AI-AGENTS.md` §4 (per-decision budgets) and §7 (the ladder).
 */
export const DEFAULT_DECISION_BUDGET: DecisionBudget = {
	// Measured against gemini-3.6-flash on OpenRouter: 2.9-3.2 s per decision at default reasoning, ~1.6 s at effort 'low',
	// because mandatory reasoning runs before the object is emitted. The previous
	// 2200 ms cap aborted every real call, so the ladder fell through to the
	// heuristic 100% of the time while looking healthy. Headroom over the measured
	// p50 is deliberate: an abort costs the whole LLM turn, whereas waiting costs
	// tempo the think-floor is already absorbing.
	firstAttemptMs: 4500,
	escalationMs: 3000,
	reserveMs: 400,
	minAttemptMs: 600,
	banterMs: 4000,
	// Reasoning models spend output tokens BEFORE emitting the JSON, and
	// gemini-3.6-flash cannot have reasoning disabled ("Reasoning is mandatory for
	// this endpoint"). Measured against the live endpoint: ~196 reasoning tokens on
	// a trivial three-move prompt, more on a real one. At the previous 220/300 caps
	// the response was truncated mid-object and every call failed with
	// "No object generated: could not parse the response". These budgets carry the
	// reasoning prefix plus the small object; the object itself is ~40 tokens.
	maxOutputTokensBid: 1600,
	maxOutputTokensPlay: 1200,
	maxOutputTokensBanter: 800
};

export function resolveBudget(overrides?: Partial<DecisionBudget>): DecisionBudget {
	return overrides === undefined ? DEFAULT_DECISION_BUDGET : { ...DEFAULT_DECISION_BUDGET, ...overrides };
}

/**
 * Which tier a decision belongs to.
 *
 * `play` is the fast tier: the legal set is already computed, the rulebook is not
 * needed, and tempo matters more than depth. Everything else — the cut, both bid
 * rounds, the dealer's discard, and therefore also going alone, which is a member
 * of the bid enum rather than a separate call — takes the deliberate tier.
 */
export function tierOf(kind: AIDecisionKind): 'fast' | 'deliberate' {
	return kind === 'play' ? 'fast' : 'deliberate';
}

/** `true` for every kind that gets the full rules digest and the cached prefix. */
export function isDeliberate(kind: AIDecisionKind): boolean {
	return tierOf(kind) === 'deliberate';
}

/**
 * The one escalation rung is bid-path only.
 *
 * On the play path a second call would spend the tempo budget to re-roll a
 * decision the heuristic can make for free and instantly; on the bid path a bad
 * call costs two to four points, so one retry is worth 1.4 seconds.
 */
export function allowsEscalation(kind: AIDecisionKind): boolean {
	return isDeliberate(kind);
}
