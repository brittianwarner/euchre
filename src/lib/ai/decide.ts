/**
 * The decision ladder.
 *
 * ```text
 *  forced            0 API calls   exactly one legal move
 *  no provider       0 API calls   ANTHROPIC_API_KEY absent
 *  degraded          0 API calls   the seat's token circuit breaker has tripped
 *  constrained LLM   1 call        schema-locked to the candidate ids
 *  escalation        1 call        deliberate path only, clamped against deadlineAt
 *  heuristic top-1   0 API calls   always available, never fails
 * ```
 *
 * Every rung is terminal and the last requires no network, which is the property
 * that makes "the game never stalls" true rather than hoped for. Above this sits
 * the table's own watchdog, which auto-plays at the hard cap regardless of what
 * happens in here; this layer aborting on its own is belt to that brace, because a
 * watchdog cannot un-hang a request that is still holding a socket.
 *
 * A rejected model answer is **never retried as the same queue message**. That is
 * how a poison pill is built: a message that fails, redelivers, fails again, and
 * takes the seat down with it. Failure walks down the ladder, it does not loop.
 */

import { APICallError, NoObjectGeneratedError, generateObject } from 'ai';
import type { LegalMove, LegalMoveId, RankedMove } from '$lib/protocol';
import { allowsEscalation, isDeliberate, resolveBudget } from './config';
import { modelParams } from './model';
import { encodeForLlm } from './notation';
import { buildLayers, toInstructions } from './prompt';
import { SCHEMA_DESCRIPTION, decisionSchema, idsOf, schemaNameFor } from './schema';
import { cleanRationale } from './screen';
import {
	ZERO_USAGE,
	noopLog,
	type AnthropicModelId,
	type DecideDeps,
	type DecideOutcome,
	type DecideUsage,
	type DecisionRequest
} from './types';

/** The default when a model omits `confidence`. Advisory; never control flow. */
const DEFAULT_CONFIDENCE = 0.6;

/**
 * Thrown only when the caller hands in an empty legal set.
 *
 * Not a runtime condition: engine invariant V19 guarantees `legal` is non-empty
 * whenever it is your turn. Reaching here means the caller dispatched a decision
 * for a seat that has none, which is a bug in the dispatcher, and returning a
 * plausible-looking outcome would hide it.
 */
export class NoLegalMovesError extends Error {
	constructor() {
		super('decide() was called with an empty legal set');
		this.name = 'NoLegalMovesError';
	}
}

/* ========================================================================== */
/* One model call                                                             */
/* ========================================================================== */

export interface CallModelOptions {
	/** Use the deliberate tier even on a path that would normally use the fast one. */
	readonly escalate: boolean;
	/** Wall-clock budget for the whole call, retries included. */
	readonly budgetMs: number;
}

/**
 * Issue exactly one `generateObject` call.
 *
 * Three details are deliberate and each has bitten someone:
 *
 * - `maxRetries: 0`. `abortSignal` is a wall-clock budget spanning the entire
 *   call *including* the SDK's own retries, so stacking retries inside it is inert
 *   at best and silently eats the whole budget at worst. One retry budget, owned
 *   by the ladder.
 * - The abort signal is the caller's signal *combined* with our own timeout, so a
 *   cancelled turn tears the request down immediately rather than waiting out a
 *   budget that no longer matters.
 * - `experimental_repairText` pulls the first `{...}` out of the response before
 *   parsing. Models occasionally wrap JSON in a code fence or a sentence of
 *   preamble; that is a formatting slip, not a decision failure, and repairing it
 *   is far cheaper than a fallback. It cannot smuggle an illegal move in: the
 *   repaired text still has to satisfy the enum.
 */
export async function callModel(
	deps: DecideDeps,
	req: DecisionRequest,
	candidates: readonly LegalMove[],
	opts: CallModelOptions
): Promise<DecideOutcome> {
	const factory = deps.factory;
	if (factory === null) throw new Error('callModel requires a model factory');

	const budget = resolveBudget(deps.budget);
	const deliberate = isDeliberate(req.kind) || opts.escalate;
	const modelId: AnthropicModelId = deliberate ? deps.persona.bidModelId : deps.persona.modelId;
	const model = deliberate ? factory.bid(modelId) : factory.play(modelId);

	const ids = idsOf(candidates);
	const layers = buildLayers({
		kind: req.kind,
		persona: deps.persona,
		dossier: deps.dossier,
		nonce: deps.nonce,
		seat: req.seat,
		body: encodeForLlm(req.view, candidates, {
			...(req.ranking === undefined ? {} : { ranking: req.ranking }),
			...(deps.memoryLines === undefined ? {} : { notes: deps.memoryLines })
		}),
		...(deps.log === undefined ? {} : { log: deps.log })
	});

	const timeout = AbortSignal.timeout(opts.budgetMs);
	const signal = deps.signal === undefined ? timeout : AbortSignal.any([deps.signal, timeout]);

	const res = await generateObject({
		model,
		schema: decisionSchema(req.kind, ids),
		schemaName: schemaNameFor(req.kind),
		schemaDescription: SCHEMA_DESCRIPTION,
		instructions: toInstructions(layers),
		messages: [{ role: 'user', content: layers.user }],
		maxOutputTokens: isDeliberate(req.kind) ? budget.maxOutputTokensBid : budget.maxOutputTokensPlay,
		maxRetries: 0,
		abortSignal: signal,
		...modelParams(
			deps.factory?.slugFor(modelId) ?? modelId,
			deps.persona.temperature,
			deps.factory?.provider
		),
		experimental_repairText: async ({ text }) => {
			const m = /\{[\s\S]*\}/.exec(text);
			return m === null ? null : m[0];
		}
	});

	const obj = res.object;
	const engineWhy = topRanked(req.ranking, ids)?.why ?? '';

	return {
		moveId: obj.moveId,
		source: 'llm',
		rationale: cleanRationale(obj.why, engineWhy, req.view),
		confidence: typeof obj.confidence === 'number' ? obj.confidence : DEFAULT_CONFIDENCE,
		usage: flattenUsage(res.usage),
		modelId,
		attempts: 1
	};
}

interface SdkUsage {
	readonly inputTokens?: number | undefined;
	readonly outputTokens?: number | undefined;
	readonly inputTokenDetails?: {
		readonly cacheReadTokens?: number | undefined;
		readonly cacheWriteTokens?: number | undefined;
	};
}

function flattenUsage(u: SdkUsage | undefined): DecideUsage {
	return {
		inputTokens: u?.inputTokens ?? 0,
		outputTokens: u?.outputTokens ?? 0,
		cacheReadTokens: u?.inputTokenDetails?.cacheReadTokens ?? 0,
		cacheWriteTokens: u?.inputTokenDetails?.cacheWriteTokens ?? 0
	};
}

/* ========================================================================== */
/* The ladder                                                                 */
/* ========================================================================== */

export interface DecideOptions {
	/**
	 * The seat's token circuit breaker. Once tripped the seat plays on the
	 * heuristic for the rest of the match behind a quiet HUD chip — the game keeps
	 * its tempo and the bill stops growing.
	 */
	readonly degraded?: boolean;
}

/**
 * Choose one move.
 *
 * `candidates` is the set the model may pick from — after difficulty narrowing,
 * which is the AI seat's business, not this layer's. It must be a subset of
 * `req.legal`; anything outside is dropped, and if nothing survives the full legal
 * set is used, because a narrowing bug must not be able to produce an illegal
 * move or an empty enum.
 *
 * This function never throws for an operational reason — no key, no network, a
 * hung provider, a hostile persona and a malformed response all land on the
 * heuristic rung. It throws only for a caller bug (an empty legal set).
 */
export async function decide(
	deps: DecideDeps,
	req: DecisionRequest,
	candidates: readonly LegalMove[],
	opts: DecideOptions = {}
): Promise<DecideOutcome> {
	const log = deps.log ?? noopLog;
	const now = deps.now ?? Date.now;
	const budget = resolveBudget(deps.budget);

	if (req.legal.length === 0) throw new NoLegalMovesError();

	const legalIds = new Set<LegalMoveId>(idsOf(req.legal));
	const narrowed = candidates.filter((m) => legalIds.has(m.id));
	const set = narrowed.length > 0 ? narrowed : req.legal;
	const allowed = new Set<LegalMoveId>(idsOf(set));

	const heuristic = heuristicOutcome(req, set);

	// Rung 1: exactly one legal move. No prompt, no call, no cost, no risk.
	if (req.legal.length === 1) {
		log('ai_forced', { seat: req.seat, kind: req.kind });
		return { ...heuristic, moveId: req.legal[0].id, source: 'forced' };
	}

	// Rung 2: no provider configured. A missing API key degrades the opponent's
	// personality, never the game — this is a supported state, not an error.
	if (deps.factory === null) {
		log('ai_no_provider', { seat: req.seat, kind: req.kind });
		return heuristic;
	}

	// Rung 3: the circuit breaker has tripped for this seat.
	if (opts.degraded === true) {
		log('ai_degraded', { seat: req.seat, kind: req.kind });
		return heuristic;
	}

	let attempts = 0;
	for (const escalate of [false, true] as const) {
		if (escalate && !allowsEscalation(req.kind)) break;

		const budgetMs = escalate
			? Math.min(budget.escalationMs, req.deadlineAt - now() - budget.reserveMs)
			: Math.min(budget.firstAttemptMs, req.deadlineAt - now() - budget.reserveMs);
		if (budgetMs < budget.minAttemptMs) break;

		try {
			attempts += 1;
			const out = await callModel(deps, req, set, { escalate, budgetMs });

			// Defence in depth. The enum already makes this unreachable: it was built
			// from `set` in this same call. Keep the check anyway — it is free, and it
			// is the thing that catches a future refactor that widens the schema.
			if (allowed.has(out.moveId)) {
				log('ai_ok', { seat: req.seat, kind: req.kind, escalate, attempts });
				return { ...out, attempts };
			}
			log('ai_illegal_id', { seat: req.seat, got: out.moveId });
		} catch (e) {
			if (!shouldContinueAfter(e, log, { seat: req.seat, escalate })) break;
		}
	}

	return { ...heuristic, attempts };
}

/**
 * Classify a failure: `true` means "try the next rung", `false` means "stop now".
 *
 * A non-retryable API error (a 401, a 400 from a bad parameter, a 403) will fail
 * identically on the escalation, so spending another second on it just delays the
 * heuristic. A timeout or an unparseable response might not.
 */
function shouldContinueAfter(
	e: unknown,
	log: NonNullable<DecideDeps['log']>,
	ctx: Readonly<Record<string, unknown>>
): boolean {
	if (NoObjectGeneratedError.isInstance(e)) {
		log('ai_unparseable', { ...ctx, text: e.text?.slice(0, 200) });
		return true;
	}
	if (APICallError.isInstance(e)) {
		log('ai_fatal', { ...ctx, status: e.statusCode, retryable: e.isRetryable });
		return e.isRetryable === true;
	}
	if (e instanceof Error && (e.name === 'TimeoutError' || e.name === 'AbortError')) {
		log('ai_timeout', { ...ctx, name: e.name });
		return true;
	}
	log('ai_error', { ...ctx, name: e instanceof Error ? e.name : typeof e });
	return false;
}

/* ========================================================================== */
/* The heuristic rung                                                         */
/* ========================================================================== */

/**
 * The always-available fallback.
 *
 * The ranking is supplied by the caller (`AIDecideRequest.ranking`), because the
 * heuristic is euchre *judgement* and belongs with the AI seat, not in the LLM
 * adapter. When no ranking is supplied this degrades to the engine's own ordering
 * of the legal set — which is deterministic and legal, if less clever. Either way
 * there is always a move to return, which is the only property that matters here.
 */
export function heuristicOutcome(
	req: DecisionRequest,
	set: readonly LegalMove[]
): DecideOutcome {
	const ids = idsOf(set);
	const top = topRanked(req.ranking, ids);
	const moveId = top?.id ?? set[0]?.id ?? req.legal[0].id;
	return {
		moveId,
		source: 'fallback',
		rationale: (top?.why ?? '').slice(0, 120),
		confidence: 0.5,
		usage: ZERO_USAGE,
		modelId: null,
		attempts: 0
	};
}

/**
 * The best-ranked member of `ids`.
 *
 * Ties break on `id` so a replayed hand produces the same fallback, and entries
 * outside the candidate set are ignored rather than trusted — a ranking that
 * covers more than it should must not be able to widen the choice.
 */
function topRanked(
	ranking: readonly RankedMove[] | undefined,
	ids: readonly LegalMoveId[]
): RankedMove | undefined {
	if (ranking === undefined) return undefined;
	const allowed = new Set(ids);
	let best: RankedMove | undefined;
	for (const r of ranking) {
		if (!allowed.has(r.id)) continue;
		if (best === undefined || r.score > best.score || (r.score === best.score && r.id < best.id)) {
			best = r;
		}
	}
	return best;
}
