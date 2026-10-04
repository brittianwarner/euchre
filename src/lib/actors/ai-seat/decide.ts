/**
 * `aiSeat/decide` — the decision ladder.
 *
 * Every rung is terminal, and the last one requires no network:
 *
 * ```
 * forced (0 API calls, one legal move)
 *   → constrained model call (wall-clock bounded)
 *     → one escalation, bid paths only
 *       → deterministic heuristic top-1
 *         → the first legal move
 * ```
 *
 * and above all of it, outside this module, the table's watchdog auto-plays at the
 * hard cap. **The game cannot stall on this function.**
 *
 * Three rules the implementation follows literally:
 *
 * - **A rejected model answer is never retried as the same message.** That is how
 *   a poison pill is built. Each attempt is a fresh, budgeted call, and after the
 *   attempts are spent the ladder descends rather than looping.
 * - **The model's answer is checked for membership of the candidate set** even
 *   though the output schema is a `z.enum` built from it. Defence 1 is structural;
 *   this is defence 2, and it is what survives a version skew between the rules
 *   package that built the enum and the one that will apply the move.
 * - **Every budget is clamped against `req.deadlineAt`**, so the table's cap holds
 *   by construction rather than by arithmetic luck. An attempt that cannot fit is
 *   skipped, not shortened to zero.
 *
 * The clock arrives as `deps.now`, never as a direct `Date.now()` call, so the
 * whole ladder is testable with fake timers and carries no ambient time.
 *
 * Normative source: `docs/03-AI-AGENTS.md` §7.
 */

import { rankMoves } from '#lib/ai/heuristic.ts';
import type {
	AIDecideRequest,
	AIDecisionSource,
	LegalMove,
	LegalMoveId,
	PersonaConfig,
	RankedMove
} from '#lib/protocol/index.ts';
import { narrowByDifficulty, turnRng } from './difficulty';
import { callDecider, resolveDecider, type LlmDecideInput, type LlmUsage } from './llm-bridge';
import { cleanRationale, publiclyNamedCards } from './screen';
import {
	DEADLINE_RESERVE_MS,
	LLM_ESCALATION_MS,
	LLM_FIRST_ATTEMPT_MS,
	LLM_MIN_ATTEMPT_MS
} from './types';

/** What the ladder needs that is not in the request. */
export interface LadderDeps {
	/** Snapshotted at match creation. `prompt` and `housePrompt` are UNTRUSTED. */
	readonly persona: PersonaConfig;
	readonly dossier: string;
	readonly nonce: string;
	/** `true` once the token circuit breaker has tripped: heuristic only. */
	readonly degraded: boolean;
	/** Injected clock. The only way time enters this module. */
	readonly now: () => number;
	readonly log: (event: string, fields: Readonly<Record<string, unknown>>) => void;
}

/** The result of one full descent of the ladder. */
export interface LadderOutcome {
	readonly moveId: LegalMoveId;
	readonly source: AIDecisionSource;
	/** Screened, clamped, safe to show a human. Falls back to the engine's own `why`. */
	readonly rationale: string;
	/** The ranking actually used, so the caller can journal or re-explain it. */
	readonly ranking: readonly RankedMove[];
	readonly candidates: number;
	readonly llmCalls: number;
	/** Times the model named an id outside the candidate set. */
	readonly illegalAttempts: number;
	/** `true` when even the heuristic produced nothing usable. */
	readonly firstLegalRescue: boolean;
	readonly usage: LlmUsage;
}

const ZERO_USAGE: LlmUsage = Object.freeze({
	inputTokens: 0,
	outputTokens: 0,
	cacheReadTokens: 0
});

/**
 * The heuristic ranking to use: the table's, when it supplied one that covers the
 * legal set exactly, otherwise one computed here.
 *
 * The table is not obliged to rank — `AIDecideRequest.ranking` is optional — and a
 * ranking that does not cover `legal` is worse than none, because it would silently
 * narrow a `casual` opponent's choices to a stale subset.
 */
export function rankingFor(req: AIDecideRequest, persona: PersonaConfig): readonly RankedMove[] {
	const supplied = req.ranking;
	if (supplied !== undefined && supplied.length === req.legal.length) {
		const legalIds = new Set<string>(req.legal.map((m) => m.id));
		if (supplied.every((r) => legalIds.has(r.id))) return supplied;
	}
	return rankMoves(req.view, req.view.hand, req.legal, {
		aggression: persona.aggression,
		risk: persona.risk
	});
}

/** The heuristic's choice, guaranteed to be a member of `legal`. */
function heuristicChoice(
	ranking: readonly RankedMove[],
	legal: readonly LegalMove[]
): { readonly id: LegalMoveId; readonly why: string; readonly rescued: boolean } {
	const legalIds = new Set<LegalMoveId>(legal.map((m) => m.id));
	for (const r of ranking) {
		if (legalIds.has(r.id)) return { id: r.id, why: r.why, rescued: false };
	}
	const first = legal[0];
	if (first === undefined) throw new Error('heuristicChoice: empty legal set');
	return { id: first.id, why: '', rescued: true };
}

/**
 * How long an attempt may run: its own nominal budget, clamped so the reply still
 * lands inside the table's deadline with time to spare.
 */
function budgetFor(nominal: number, deadlineAt: number, now: number): number {
	const remaining = deadlineAt - now - DEADLINE_RESERVE_MS;
	const budget = Math.min(nominal, remaining);
	return Number.isFinite(budget) ? budget : 0;
}

/**
 * Descend the ladder for one request.
 *
 * Returns `null` only when there is nothing to decide — an empty legal set, which
 * means the table asked the wrong seat. Otherwise it always returns a move that is
 * a member of `req.legal`, and it never throws.
 */
export async function runLadder(
	deps: LadderDeps,
	req: AIDecideRequest
): Promise<LadderOutcome | null> {
	const legal = req.legal;
	if (legal.length === 0) return null;

	const ranking = rankingFor(req, deps.persona);
	const fallbackPick = heuristicChoice(ranking, legal);
	const allowedCards = publiclyNamedCards(req.view);

	const base = {
		ranking,
		usage: ZERO_USAGE,
		illegalAttempts: 0,
		firstLegalRescue: fallbackPick.rescued
	};

	// Rung 0 — forced. One legal move is not a decision; spend nothing on it.
	const only = legal.length === 1 ? legal[0] : undefined;
	if (only !== undefined) {
		return {
			...base,
			moveId: only.id,
			source: 'forced',
			rationale: cleanRationale(undefined, fallbackPick.why, allowedCards),
			candidates: 1,
			llmCalls: 0,
			firstLegalRescue: false
		};
	}

	const heuristicOutcome = (
		llmCalls: number,
		illegalAttempts: number,
		usage: LlmUsage,
		candidates: number
	): LadderOutcome => ({
		...base,
		moveId: fallbackPick.id,
		source: 'fallback',
		rationale: cleanRationale(undefined, fallbackPick.why, allowedCards),
		candidates,
		llmCalls,
		illegalAttempts,
		usage
	});

	// Rung 1 short-circuit — the circuit breaker has tripped. No network at all.
	if (deps.degraded) return heuristicOutcome(0, 0, ZERO_USAGE, legal.length);

	// There are at most five card plays. Let Jev compare all of them: pruning by
	// the fallback's ranking can hide the only card that protects a partner.
	const candidates =
		req.kind === 'play'
			? legal
			: narrowByDifficulty(ranking, legal, deps.persona.difficulty, turnRng(req.turnId, req.seat));
	const candidateSet = new Map<string, LegalMove>(candidates.map((m) => [m.id, m]));

	const decider = resolveDecider();
	if (decider === null) {
		deps.log('ai_no_provider', { seat: req.seat, kind: req.kind });
		return heuristicOutcome(0, 0, ZERO_USAGE, candidates.length);
	}

	// Escalation is bid-path only: a second round trip does not fit inside the
	// play tempo budget, and a play is rarely worth a stronger model.
	const attempts: readonly boolean[] = req.kind === 'play' ? [false] : [false, true];

	let llmCalls = 0;
	let illegalAttempts = 0;
	let usage: LlmUsage = ZERO_USAGE;

	for (const escalate of attempts) {
		const nominal = escalate ? LLM_ESCALATION_MS : LLM_FIRST_ATTEMPT_MS;
		const budgetMs = budgetFor(nominal, req.deadlineAt, deps.now());
		if (budgetMs < LLM_MIN_ATTEMPT_MS) {
			deps.log('ai_no_budget', { seat: req.seat, escalate, budgetMs });
			break;
		}

		const input: LlmDecideInput = {
			kind: req.kind,
			seat: req.seat,
			view: req.view,
			candidates,
			ranking,
			persona: deps.persona,
			dossier: deps.dossier,
			nonce: deps.nonce,
			budgetMs,
			escalate,
			deadlineAt: req.deadlineAt
		};

		llmCalls += 1;
		const result = await callDecider(decider, input);
		if (result === null) {
			// Unparseable, late, refused, or an id outside the enum. Try the next
			// rung; never re-run the same rung.
			illegalAttempts += 1;
			deps.log('ai_attempt_failed', { seat: req.seat, kind: req.kind, escalate });
			continue;
		}

		usage = result.usage ?? ZERO_USAGE;

		// Independent membership check. The schema should have made this
		// impossible; a version skew is exactly when "should" stops holding.
		const chosen = candidateSet.get(result.moveId);
		if (chosen === undefined) {
			illegalAttempts += 1;
			deps.log('ai_illegal_id', { seat: req.seat, got: result.moveId });
			continue;
		}

		const why = ranking.find((r) => r.id === chosen.id)?.why ?? fallbackPick.why;
		return {
			...base,
			moveId: chosen.id,
			source: 'llm',
			rationale: cleanRationale(result.rationale, why, allowedCards),
			candidates: candidates.length,
			llmCalls,
			illegalAttempts,
			usage,
			firstLegalRescue: false
		};
	}

	return heuristicOutcome(llmCalls, illegalAttempts, usage, candidates.length);
}
