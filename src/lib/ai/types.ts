/**
 * `$lib/ai` — internal types for the LLM decision layer.
 *
 * This layer turns an {@link AIDecideRequest} into one legal move id. It is a
 * **pure library**: no `rivetkit`, no actor context, no `c.state`, no schedule, no
 * global clock and no global randomness. The `aiSeat` actor owns all of that and
 * calls in here.
 *
 * The security posture this file exists to support:
 *
 * - The only game state that reaches a model is `PublicGameView` — the same
 *   redaction a human in that chair receives. There is no field on any type here
 *   capable of carrying `GameState`, a foreign hand, the buried kitty or the
 *   dealer's discard.
 * - The output is schema-constrained to a `z.enum` rebuilt per decision from the
 *   table-supplied legal set, so an illegal move is unrepresentable rather than
 *   rejected after the fact.
 * - The persona text is untrusted. It is fenced, sanitised and length-capped, and
 *   it can influence style only — never legality, never what is visible.
 */

import type { LanguageModel } from 'ai';
import type {
	AIDecideRequest,
	AIDecisionKind,
	AIDecisionSource,
	AnthropicModelId,
	LegalMoveId,
	PersonaConfig
} from '$lib/protocol';

/* ========================================================================== */
/* Model plumbing                                                             */
/* ========================================================================== */

/**
 * The seam that keeps tests off the network.
 *
 * A factory is built **once, at a composition root** (the registry / actor create
 * input) and injected. Constructing a provider from `process.env` deep inside a
 * decision is a defect: that seam is precisely what lets `MockLanguageModelV4`
 * from `ai/test` swap in wholesale without touching an environment variable.
 *
 * Three named tiers rather than one `model(id)` so the call sites read as intent
 * (`factory.bid(...)`) and so a future per-tier middleware (telemetry, rate limit)
 * has somewhere to live.
 */
export interface ModelFactory {
	/**
	 * Which upstream this factory talks to. Carried on the factory rather than
	 * inferred at the call site, because `providerOptions` is keyed by provider
	 * name and sending Anthropic-shaped options through OpenRouter silently drops
	 * them (or 400s, depending on the upstream).
	 */
	readonly provider: ProviderName;
	/**
	 * The real upstream model id a canonical tier id resolves to.
	 *
	 * Sampling rules belong to the model that actually runs, not to the canonical
	 * tier name. Gemini accepts `temperature`; the Opus-5 family 400s on it. With
	 * OpenRouter in front, only the factory knows which one is really being called.
	 */
	slugFor(modelId: AnthropicModelId): string;
	/** Fast tier: card plays. */
	play(modelId: AnthropicModelId): LanguageModel;
	/** Deliberate tier: the cut, bidding, the dealer discard, going alone. */
	bid(modelId: AnthropicModelId): LanguageModel;
	/** Fast tier: table talk. Separate from `play` so it can be throttled alone. */
	talk(modelId: AnthropicModelId): LanguageModel;
}

/**
 * Sampling parameters the provider is allowed to send for a given model family.
 *
 * The Opus-5 family rejects `temperature`, `top_p` and `top_k` with **HTTP 400**,
 * and the AI SDK provider does not strip them. The allowlist is the single place
 * that fact is encoded; see `modelParams()`.
 */
export type SamplingParam = 'temperature' | 'topP' | 'topK';

/**
 * Provider-agnostic call settings produced by `modelParams()` and spread into a
 * `generateObject` / `streamText` call.
 *
 * `temperature` is **absent**, not `undefined`, for no-sampling models: the SDK
 * forwards an explicit `undefined` as a missing key, but keeping the property off
 * the object entirely makes the CI assertion ("no sampling key was ever sent")
 * trivially true by inspection of `Object.keys`.
 */
export interface ModelParams {
	readonly temperature?: number;
	/**
	 * Absent for OpenRouter, and absent means absent — not `{}`.
	 *
	 * Verified against the live endpoint: sending `providerOptions: { openrouter: {} }`
	 * makes `generateObject` fail with "could not parse the response", while omitting
	 * the key entirely succeeds. Provider knobs through OpenRouter are model-specific
	 * (gemini-3.6-flash additionally rejects `reasoning: { enabled: false }` outright),
	 * so the portable choice is to send none and let each upstream use its defaults.
	 */
	readonly providerOptions?: ProviderOptionsShape;
}

/**
 * Structural stand-in for the SDK's `ProviderOptions`
 * (`Record<string, Record<string, JSONValue>>`).
 *
 * Declared as a `type` alias rather than an `interface` on purpose: only aliases
 * get an implicit index signature, which is what makes this assignable to the
 * SDK's parameter type without a cast.
 */
export type ProviderOptionsShape = {
	readonly anthropic?: {
		readonly thinking?: { readonly type: 'disabled' };
		readonly effort?: 'low' | 'medium' | 'high';
	};
	readonly openrouter?: {
		readonly reasoning?: { readonly enabled: boolean; readonly effort?: 'low' | 'medium' | 'high' };
	};
};

/** The upstreams this layer can talk to. OpenRouter is the shipped default. */
export type ProviderName = 'openrouter' | 'anthropic';

/* ========================================================================== */
/* Logging                                                                    */
/* ========================================================================== */

/**
 * Structured events this layer emits. A closed union so the actor's log sink can
 * switch exhaustively and so a grep for a symptom finds the emitter.
 */
export type AiLogEvent =
	| 'ai_no_provider'
	| 'ai_degraded'
	| 'ai_forced'
	| 'ai_illegal_id'
	| 'ai_unparseable'
	| 'ai_timeout'
	| 'ai_fatal'
	| 'ai_error'
	| 'ai_ok'
	| 'banter_skipped'
	| 'banter_screened'
	| 'banter_error'
	| 'rationale_screened'
	| 'persona_truncated';

/** Never receives a card the seat should not know, and never the internal token. */
export type AiLogFn = (event: AiLogEvent, data: Readonly<Record<string, unknown>>) => void;

/** A sink that discards everything. The default, so callers may omit `log`. */
export const noopLog: AiLogFn = () => {};

/* ========================================================================== */
/* Decision inputs and outputs                                                */
/* ========================================================================== */

/** Token accounting for one call, flattened from the SDK's nested `usage`. */
export interface DecideUsage {
	readonly inputTokens: number;
	readonly outputTokens: number;
	readonly cacheReadTokens: number;
	readonly cacheWriteTokens: number;
}

/** The zero value, used by every rung of the ladder that makes no API call. */
export const ZERO_USAGE: DecideUsage = {
	inputTokens: 0,
	outputTokens: 0,
	cacheReadTokens: 0,
	cacheWriteTokens: 0
};

/**
 * What one decision produced. The `aiSeat` actor maps this onto the protocol's
 * `AIDecision` (adding `internalToken`, `gameId`, `turnId` and `latencyMs`) — this
 * layer deliberately does not know those fields exist.
 */
export interface DecideOutcome {
	/** Always a member of the candidate set handed in. */
	readonly moveId: LegalMoveId;
	readonly source: AIDecisionSource;
	/** Screened, markup-stripped and clamped. Safe to show a human. Never control flow. */
	readonly rationale: string;
	/** Advisory only. Defaults to `0.6` when the model omits it. */
	readonly confidence: number;
	readonly usage: DecideUsage;
	/** `null` when no API call was made (`forced`, degraded, or no provider). */
	readonly modelId: AnthropicModelId | null;
	/** How many model calls were actually issued. `0` on the no-network rungs. */
	readonly attempts: number;
}

/**
 * Everything the decision layer needs that is not part of the request.
 *
 * `factory: null` is the **no-API-key** state and is a first-class, non-throwing
 * path: the ladder returns the heuristic's top-ranked move with
 * `source: "fallback"`. An absent key degrades the opponent's personality, never
 * the game.
 */
export interface DecideDeps {
	/** `null` when no provider is configured. See `modelFactoryFromEnv()`. */
	readonly factory: ModelFactory | null;
	/** Snapshotted at match create and never re-read mid-game. */
	readonly persona: PersonaConfig;
	/** Rendered cross-game episodes. Semi-trusted; fenced exactly like the prompt. */
	readonly dossier: string;
	/** Per-game 12-hex fence nonce. Unguessable, so the fence cannot be escaped. */
	readonly nonce: string;
	readonly log?: AiLogFn;
	/** Injectable clock. Tests pass a fake; production passes nothing. */
	readonly now?: () => number;
	/** Overrides for the provisional timing/token budget. */
	readonly budget?: Partial<DecisionBudget>;
	/** Caller's cancellation (the actor's step timeout). Combined with our own. */
	readonly signal?: AbortSignal;
	/**
	 * Optional per-hand notes the seat has accumulated **from the public view only**
	 * (voids, trump seen, reads). Rendered as `NOTES=` lines. Capped hard by
	 * `encodeForLlm`, because it is model-authored text re-entering a prompt.
	 */
	readonly memoryLines?: readonly string[];
}

/**
 * The provisional latency and token budget.
 *
 * **None of these numbers is frozen.** They are defaults, overridable per call via
 * `DecideDeps.budget`, and they are clamped against `AIDecideRequest.deadlineAt`
 * so the table's hard cap holds by construction rather than by arithmetic luck.
 * The measured values land in `docs/LEDGER.md` after the metering harness runs.
 */
export interface DecisionBudget {
	/** Wall-clock abort for the first model call. */
	readonly firstAttemptMs: number;
	/** Ceiling for the single escalation attempt (bid path only). */
	readonly escalationMs: number;
	/** Head-room left before `deadlineAt` so the reply still lands in time. */
	readonly reserveMs: number;
	/** Below this, an attempt is not worth starting. */
	readonly minAttemptMs: number;
	/** Wall-clock abort for a banter stream. */
	readonly banterMs: number;
	readonly maxOutputTokensBid: number;
	readonly maxOutputTokensPlay: number;
	readonly maxOutputTokensBanter: number;
}

/** The narrowed request shape this layer actually reads. */
export type DecisionRequest = AIDecideRequest;

/** Re-exported for call sites that only import from `$lib/ai`. */
export type { AIDecisionKind, AIDecisionSource, AnthropicModelId, LegalMoveId, PersonaConfig };
