/**
 * `aiSeat/llm-bridge` — **the only file in `src/lib/actors/ai-seat/**` that knows
 * `$lib/ai` exists.**
 *
 * The AI seat owns the *ladder* — when to call a model, how long to wait, what to
 * do when the answer is wrong or late. It does not own the SDK. Prompt assembly,
 * `generateObject`, caching, model params and provider errors all live in
 * `$lib/ai`, written by another owner; this module is the seam between the two, and
 * it is deliberately one small file so that a rename on either side is a one-line
 * change rather than an archaeology exercise.
 *
 * ## The contract this seat needs
 *
 * `$lib/ai` must export a function that takes a single self-contained
 * {@link LlmDecideInput} and resolves to an {@link LlmDecideResult}, under any of
 * the names in {@link DECIDER_EXPORT_NAMES} (`decideMove` preferred). Optionally a
 * banter writer under {@link BANTER_EXPORT_NAMES}.
 *
 * ```ts
 * export async function decideMove(input: LlmDecideInput): Promise<LlmDecideResult>;
 * export async function generateBanter(input: LlmBanterInput): Promise<string | null>;
 * ```
 *
 * Three properties of the input are load-bearing and are the reason it is shaped
 * this way:
 *
 * 1. **`candidates` is the `z.enum` source.** `candidates.map(m => m.id)` is
 *    exactly the enum the output schema must be built from, rebuilt per decision.
 *    An illegal AI move is then unrepresentable rather than merely rejected.
 * 2. **There is no field capable of carrying `GameState`.** The seat is given
 *    `PublicGameView` and passes on the same object; a "stronger" opponent is not a
 *    reason to widen this, ever.
 * 3. **`budgetMs` is a wall-clock bound across the whole call including SDK
 *    retries.** The seat has already clamped it against the table's deadline, so
 *    the implementation must pass it to `abortSignal` and must set
 *    `maxRetries: 0` — stacking retries inside a wall-clock budget is inert.
 *
 * ## Failure is normal
 *
 * Every resolution and every call is defensive. A missing module export, a changed
 * signature, a rejected promise, a malformed result — all of them return `null` and
 * the ladder falls to the deterministic heuristic. The bridge never throws.
 */

import * as aiModule from '$lib/ai';
import type {
	AIDecisionKind,
	LegalMove,
	PersonaConfig,
	PublicGameView,
	RankedMove,
	Seat
} from '$lib/protocol';

/* ========================================================================== */
/* The contract                                                                */
/* ========================================================================== */

/** Model token accounting for one call, as reported by the SDK. */
export interface LlmUsage {
	readonly inputTokens: number;
	readonly outputTokens: number;
	readonly cacheReadTokens: number;
}

/** Everything `$lib/ai` needs to make one constrained decision. Self-contained. */
export interface LlmDecideInput {
	readonly kind: AIDecisionKind;
	readonly seat: Seat;
	/** `project(state, seat)`. The redaction boundary; never widened. */
	readonly view: PublicGameView;
	/** The narrowed legal set. `candidates.map(m => m.id)` is the `z.enum`. */
	readonly candidates: readonly LegalMove[];
	/** The heuristic's ordering over the same set, shown to the model as a prior. */
	readonly ranking: readonly RankedMove[];
	/** Snapshotted at match creation. `prompt` and `housePrompt` are UNTRUSTED. */
	readonly persona: PersonaConfig;
	/** Semi-trusted cross-game notes. Fenced exactly like the persona prompt. */
	readonly dossier: string;
	/** Per-game 12-hex fence nonce that delimits every untrusted block. */
	readonly nonce: string;
	/** Wall-clock budget for the entire call. Already clamped against the deadline. */
	readonly budgetMs: number;
	/** `true` on the single bid-path escalation: use the deliberate model tier. */
	readonly escalate: boolean;
	/** Server-clock deadline the table will not wait past. */
	readonly deadlineAt: number;
}

/** What the model chose. `moveId` is validated against the candidate set by the seat. */
export interface LlmDecideResult {
	readonly moveId: string;
	/** Free text; screened and clamped by the seat before it reaches a human. */
	readonly rationale?: string;
	readonly confidence?: number;
	readonly usage?: LlmUsage;
}

/** Everything `$lib/ai` needs to write one line of table talk. */
export interface LlmBanterInput {
	readonly seat: Seat;
	/**
	 * **The hand-stripped view.** The seat passes `publicOnlyView(view)`, so a
	 * persona demanding a card reveal is asking for data this prompt does not hold.
	 */
	readonly view: PublicGameView;
	/** A short, card-free description of the moment. */
	readonly situation: string;
	readonly persona: PersonaConfig;
	readonly dossier: string;
	readonly nonce: string;
	readonly budgetMs: number;
}

/** The decision function this seat calls. */
export type LlmDecider = (input: LlmDecideInput) => Promise<LlmDecideResult>;

/** The banter function this seat calls. Returning `null` means "stay quiet". */
export type LlmBanterWriter = (input: LlmBanterInput) => Promise<string | null>;

/* ========================================================================== */
/* Resolution                                                                  */
/* ========================================================================== */

/** Export names accepted for the decision function, most preferred first. */
export const DECIDER_EXPORT_NAMES = [
	'decideMove',
	'decideWithLlm',
	'decide',
	'callModel',
	'chooseMove'
] as const;

/** Export names accepted for the banter writer, most preferred first. */
export const BANTER_EXPORT_NAMES = [
	'generateBanter',
	'generateBanterLine',
	'writeBanter',
	'banter'
] as const;

let deciderOverride: LlmDecider | null = null;
let banterOverride: LlmBanterWriter | null = null;

/**
 * Inject a decider and/or a banter writer, replacing whatever `$lib/ai` exports.
 *
 * This is the test seam: `MockLanguageModelV4` swaps in wholesale here, so no test
 * in this package touches the network or reads an API key. It is also the escape
 * hatch if the module's export names drift.
 */
export function setLlmProvider(p: {
	readonly decide?: LlmDecider | null;
	readonly banter?: LlmBanterWriter | null;
}): void {
	if (p.decide !== undefined) deciderOverride = p.decide;
	if (p.banter !== undefined) banterOverride = p.banter;
}

function lookup(names: readonly string[]): unknown {
	let bag: Record<string, unknown>;
	try {
		bag = aiModule as unknown as Record<string, unknown>;
	} catch {
		return undefined;
	}
	for (const n of names) {
		const v = bag[n];
		if (typeof v === 'function') return v;
	}
	return undefined;
}

/** The decision function, or `null` when `$lib/ai` exposes none. Never throws. */
export function resolveDecider(): LlmDecider | null {
	if (deciderOverride !== null) return deciderOverride;
	const fn = lookup(DECIDER_EXPORT_NAMES);
	return typeof fn === 'function' ? (fn as LlmDecider) : null;
}

/** The banter writer, or `null` when `$lib/ai` exposes none. Never throws. */
export function resolveBanterWriter(): LlmBanterWriter | null {
	if (banterOverride !== null) return banterOverride;
	const fn = lookup(BANTER_EXPORT_NAMES);
	return typeof fn === 'function' ? (fn as LlmBanterWriter) : null;
}

/* ========================================================================== */
/* Guarded calls                                                               */
/* ========================================================================== */

function isRecord(v: unknown): v is Record<string, unknown> {
	return typeof v === 'object' && v !== null;
}

/**
 * Validate whatever the module handed back. A model that answered in prose, a
 * function with a different signature, or a result naming a move outside the
 * candidate set all reduce to `null` here — which is rung 2 of the ladder, not an
 * exception.
 */
export function parseDecideResult(
	value: unknown,
	allowedIds: ReadonlySet<string>
): LlmDecideResult | null {
	if (!isRecord(value)) return null;
	const moveId = value['moveId'];
	if (typeof moveId !== 'string' || !allowedIds.has(moveId)) return null;

	const rationale = value['rationale'] ?? value['why'];
	const confidence = value['confidence'];
	const usage = value['usage'];

	return {
		moveId,
		rationale: typeof rationale === 'string' ? rationale : undefined,
		confidence: typeof confidence === 'number' && Number.isFinite(confidence) ? confidence : undefined,
		usage: isRecord(usage)
			? {
					inputTokens: numberOr(usage['inputTokens'], 0),
					outputTokens: numberOr(usage['outputTokens'], 0),
					cacheReadTokens: numberOr(usage['cacheReadTokens'], 0)
				}
			: undefined
	};
}

function numberOr(v: unknown, fallback: number): number {
	return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
}

/**
 * One guarded model call, hard-bounded by `input.budgetMs` on this side as well as
 * inside the SDK — a provider that ignores its abort signal must not be able to
 * outrun the table's watchdog.
 *
 * Resolves `null` for every failure mode there is: no module, wrong signature,
 * rejection, timeout, unparseable result, or an id outside the candidate set.
 */
export async function callDecider(
	decider: LlmDecider,
	input: LlmDecideInput
): Promise<LlmDecideResult | null> {
	const allowed = new Set<string>(input.candidates.map((m) => m.id));
	try {
		const raw: unknown = await withDeadline(decider(input), input.budgetMs);
		return parseDecideResult(raw, allowed);
	} catch {
		return null;
	}
}

/** One guarded banter call. Resolves `null` on any failure; never throws. */
export async function callBanterWriter(
	writer: LlmBanterWriter,
	input: LlmBanterInput
): Promise<string | null> {
	try {
		const raw: unknown = await withDeadline(writer(input), input.budgetMs);
		return typeof raw === 'string' && raw.length > 0 ? raw : null;
	} catch {
		return null;
	}
}

/**
 * Reject after `ms` regardless of what the underlying promise does.
 *
 * The timer is cleared on settle so a resolved call cannot hold the actor awake,
 * and the loser of the race is abandoned rather than awaited — an abandoned model
 * call costs tokens, but a blocked run loop costs the game.
 */
function withDeadline<T>(p: Promise<T>, ms: number): Promise<T> {
	if (!Number.isFinite(ms) || ms <= 0) return Promise.reject(new Error('no budget'));
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_, reject) => {
		timer = setTimeout(() => reject(new Error('llm_timeout')), ms);
	});
	return Promise.race([p, timeout]).finally(() => {
		if (timer !== undefined) clearTimeout(timer);
	});
}
