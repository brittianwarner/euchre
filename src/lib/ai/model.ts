/**
 * Model construction and the sampling-parameter allowlist.
 *
 * Two responsibilities, both of which exist because getting them wrong is a
 * production incident rather than a test failure:
 *
 * 1. **`modelParams()` is the only place sampling parameters are decided.** The
 *    Opus-5 family rejects `temperature`, `top_p` and `top_k` with HTTP 400 and
 *    the AI SDK provider does not strip them, so a stray `temperature: 0.7` on a
 *    bid call is a hard 400 on every bid of every game. The allowlist below is
 *    keyed per model family and **fails closed**: an id nobody has classified gets
 *    no sampling parameters at all, which is degraded output quality rather than a
 *    dead bid path.
 * 2. **The provider is built at a composition root and injected.** Nothing in this
 *    directory reads an API key at decision time. `modelFactoryFromEnv()` returns
 *    `null` when `ANTHROPIC_API_KEY` is absent, and `null` is a supported,
 *    non-throwing state that the ladder degrades through to the heuristic.
 */

import { createAnthropic } from '@ai-sdk/anthropic';
import { createOpenRouter } from '@openrouter/ai-sdk-provider';
import type { LanguageModel } from 'ai';
import {
	LLM_TEMPERATURE_MAX,
	LLM_TEMPERATURE_MIN,
	MODEL_BID,
	MODEL_PLAY,
	NO_SAMPLING_MODELS,
	type AnthropicModelId
} from '$lib/protocol';
import type {
	ModelFactory,
	ModelParams,
	ProviderName,
	ProviderOptionsShape,
	SamplingParam
} from './types';

/* ========================================================================== */
/* The sampling allowlist                                                     */
/* ========================================================================== */

/**
 * Exact model ids we have classified.
 *
 * `NO_SAMPLING_MODELS` in `$lib/protocol` is the *shipped* subset (today just
 * `claude-opus-5`); this table is wider because the same 400 applies across the
 * whole no-sampling family and re-pointing a persona at a sibling model must not
 * silently start returning HTTP 400.
 */
const SAMPLING_ALLOWLIST: Readonly<Record<string, readonly SamplingParam[]>> = {
	'claude-haiku-4-5': ['temperature'],
	'claude-opus-5': [],
	'claude-opus-4-8': [],
	'claude-opus-4-7': [],
	'claude-fable-5': [],
	'claude-sonnet-5': [],
	// Gemini accepts sampling parameters; the no-sampling rule is Anthropic-specific.
	'google/gemini-3.6-flash': ['temperature']
};

/**
 * Prefix rules for ids not listed exactly. Ordered; first match wins.
 *
 * Only the Haiku line is granted sampling by prefix. Everything else — including
 * anything new — falls through to the closed default.
 */
const FAMILY_RULES: readonly (readonly [string, readonly SamplingParam[]])[] = [
	['claude-haiku-', ['temperature']],
	['google/', ['temperature']],
	['openai/', ['temperature']],
	['meta-llama/', ['temperature']]
];

/** Fail-closed default: send no sampling parameters at all. */
const NO_SAMPLING: readonly SamplingParam[] = [];

/**
 * Which sampling parameters may be sent to this model id.
 *
 * Exported so `modelParams.spec.ts` can assert, for every id in the no-sampling
 * family, that the result is empty — the CI gate described in
 * `docs/03-AI-AGENTS.md` §13.
 */
export function allowedSamplingParams(modelId: string): readonly SamplingParam[] {
	const exact = SAMPLING_ALLOWLIST[modelId];
	if (exact !== undefined) return exact;
	for (const [prefix, params] of FAMILY_RULES) {
		if (modelId.startsWith(prefix)) return params;
	}
	return NO_SAMPLING;
}

/** `true` when this id must receive no `temperature` / `top_p` / `top_k`. */
export function isNoSamplingModel(modelId: string): boolean {
	return allowedSamplingParams(modelId).length === 0;
}

/**
 * Sanity check between the protocol's shipped list and this file's family table.
 * Exported for a test rather than asserted at import time — a module that throws
 * on load takes the whole actor down for a typo.
 */
export function noSamplingListIsConsistent(): boolean {
	return NO_SAMPLING_MODELS.every((id) => isNoSamplingModel(id));
}

/* ========================================================================== */
/* modelParams                                                                */
/* ========================================================================== */

/**
 * The single helper every call site uses to build sampling and provider options.
 *
 * Two invariants it enforces:
 *
 * - For a no-sampling model the returned object has **no `temperature` key at
 *   all** — not `temperature: undefined`. That makes the CI assertion a plain
 *   `Object.keys` check.
 * - `thinking: { type: 'disabled' }` is paired with `effort: 'low'` on the
 *   no-sampling family, because disabling thinking is only legal there at
 *   `effort <= 'high'`. Disabling it is deliberate: these are sub-second card
 *   decisions with a pre-computed legal set, and adaptive thinking would blow the
 *   tempo budget for no gain.
 *
 * Disabled thinking can let internal `<thinking>` markup leak into free-text
 * fields, which is why `cleanRationale()` strips markup from every `why` and why
 * `why` never drives control flow.
 */
export function modelParams(
	modelId: string,
	personaTemperature: number,
	provider: ProviderName = 'openrouter'
): ModelParams {
	const allowed = allowedSamplingParams(modelId);
	const noSampling = !allowed.includes('temperature');

	// `providerOptions` is keyed by provider name. Anthropic-shaped keys sent
	// through OpenRouter are not understood by the upstream, so each provider gets
	// only its own namespace and nothing else.
	if (provider !== 'anthropic') {
		// gemini-3.6-flash cannot have reasoning disabled ("Reasoning is mandatory
		// for this endpoint"), but it CAN be turned down. Measured on the live
		// endpoint: effort 'low' halves per-decision latency (3093 ms -> 1584 ms)
		// with the same chosen move. That matters because tempo is the difference
		// between opponents that feel alive and a game that stalls every turn.
		//
		// Note an empty `{ openrouter: {} }` breaks structured-output parsing
		// outright, so this object must never be allowed to become empty.
		const providerOptions: ProviderOptionsShape = { openrouter: { reasoning: { effort: 'low' } } };
		return noSampling
			? { providerOptions }
			: { temperature: clampTemperature(personaTemperature), providerOptions };
	}

	const providerOptions: ProviderOptionsShape = {
		anthropic: noSampling
			? { thinking: { type: 'disabled' }, effort: 'low' }
			: { thinking: { type: 'disabled' } }
	};

	if (noSampling) return { providerOptions };
	return { temperature: clampTemperature(personaTemperature), providerOptions };
}

/**
 * A persona dial is authored on `0..1`; the usable band for card play is narrower.
 * Below `0.2` every seat plays the same move; above `0.8` the JSON itself starts
 * degrading and the schema repair path gets exercised for no character gain.
 */
export function clampTemperature(t: number): number {
	if (!Number.isFinite(t)) return LLM_TEMPERATURE_MIN;
	return Math.min(LLM_TEMPERATURE_MAX, Math.max(LLM_TEMPERATURE_MIN, t));
}

/* ========================================================================== */
/* Factories                                                                  */
/* ========================================================================== */

export interface AnthropicFactoryOptions {
	/** Absent or empty means "no provider": the caller degrades to the heuristic. */
	readonly apiKey?: string | undefined;
	readonly baseURL?: string | undefined;
	readonly headers?: Readonly<Record<string, string>> | undefined;
}

/**
 * Build a real Anthropic-backed factory, or `null` when there is no key.
 *
 * Returning `null` rather than throwing is the whole point: a developer without
 * `ANTHROPIC_API_KEY` gets three heuristic opponents and a playable game, not a
 * crash loop in an actor.
 */
export function createAnthropicModelFactory(opts: AnthropicFactoryOptions): ModelFactory | null {
	const apiKey = opts.apiKey?.trim();
	if (apiKey === undefined || apiKey.length === 0) return null;

	const provider = createAnthropic({
		apiKey,
		...(opts.baseURL === undefined ? {} : { baseURL: opts.baseURL }),
		...(opts.headers === undefined ? {} : { headers: { ...opts.headers } })
	});

	const make = (modelId: AnthropicModelId): LanguageModel => provider(modelId);
	return { provider: 'anthropic', slugFor: (m) => m, play: make, bid: make, talk: make };
}

/* -------------------------------------------------------------------------- */
/* OpenRouter — the shipped default                                            */
/* -------------------------------------------------------------------------- */

/**
 * Canonical tier id → OpenRouter model slug.
 *
 * OpenRouter namespaces by vendor and writes minor versions with a dot
 * (`anthropic/claude-haiku-4.5`), while our canonical ids use a dash
 * (`claude-haiku-4-5`). Mapping here keeps every other module — including the
 * sampling allowlist, which keys on the canonical id — completely unaware of
 * which upstream is in play. Slugs verified against the live
 * `https://openrouter.ai/api/v1/models` catalogue.
 */
/**
 * The single model behind every AI decision.
 *
 * One model for both tiers is deliberate: a euchre decision is small and highly
 * constrained (the legal set is pre-computed and enforced by a `z.enum`), so the
 * win is tempo and cost, not raw reasoning depth. Verified on the live OpenRouter
 * catalogue: 1M context, $1.50/M in, $7.50/M out.
 *
 * Overridable per tier via `OPENROUTER_MODEL_PLAY` / `OPENROUTER_MODEL_BID`.
 */
export const DEFAULT_OPENROUTER_MODEL = 'google/gemini-3.6-flash';

const OPENROUTER_SLUGS: Readonly<Record<AnthropicModelId, string>> = {
	'claude-haiku-4-5': DEFAULT_OPENROUTER_MODEL,
	'claude-opus-5': DEFAULT_OPENROUTER_MODEL
};

export interface OpenRouterFactoryOptions {
	/** Absent or empty means no provider: the caller degrades to the heuristic. */
	readonly apiKey?: string | undefined;
	readonly baseURL?: string | undefined;
	/** Optional per-slug overrides, e.g. to pin a cheaper play model. */
	readonly slugs?: Partial<Record<AnthropicModelId, string>> | undefined;
}

/** Build an OpenRouter-backed factory, or `null` when there is no key. */
export function createOpenRouterModelFactory(opts: OpenRouterFactoryOptions): ModelFactory | null {
	const apiKey = opts.apiKey?.trim();
	if (apiKey === undefined || apiKey.length === 0) return null;

	const openrouter = createOpenRouter({
		apiKey,
		...(opts.baseURL === undefined ? {} : { baseURL: opts.baseURL })
	});

	const slugs = { ...OPENROUTER_SLUGS, ...(opts.slugs ?? {}) };
	const slugFor = (modelId: AnthropicModelId): string =>
		slugs[modelId] ?? OPENROUTER_SLUGS[modelId];
	const make = (modelId: AnthropicModelId): LanguageModel => openrouter.chat(slugFor(modelId));

	return { provider: 'openrouter', slugFor, play: make, bid: make, talk: make };
}

/**
 * Read the key from an environment record.
 *
 * Called **once**, at a composition root, and the result injected. The `env`
 * parameter is explicit so a test never has to mutate `process.env`, and the
 * `process` access is guarded so importing this module from a bundle that also
 * ships to the browser is inert rather than fatal. (It should never reach the
 * browser — but "should never" is not a defence.)
 */
export function modelFactoryFromEnv(
	env: Readonly<Record<string, string | undefined>> = readProcessEnv()
): ModelFactory | null {
	// OpenRouter first: it is the shipped path, so a machine holding both keys
	// behaves the same as production. Anthropic direct stays as a fallback so an
	// existing ANTHROPIC_API_KEY keeps working with no config change.
	const viaOpenRouter = createOpenRouterModelFactory({
		apiKey: env.OPENROUTER_API_KEY,
		baseURL: env.OPENROUTER_BASE_URL,
		slugs: {
			...(env.OPENROUTER_MODEL_PLAY === undefined
				? {}
				: { 'claude-haiku-4-5': env.OPENROUTER_MODEL_PLAY }),
			...(env.OPENROUTER_MODEL_BID === undefined
				? {}
				: { 'claude-opus-5': env.OPENROUTER_MODEL_BID })
		}
	});
	if (viaOpenRouter !== null) return viaOpenRouter;

	return createAnthropicModelFactory({
		apiKey: env.ANTHROPIC_API_KEY,
		baseURL: env.ANTHROPIC_BASE_URL
	});
}

function readProcessEnv(): Readonly<Record<string, string | undefined>> {
	const g = globalThis as { process?: { env?: Record<string, string | undefined> } };
	return g.process?.env ?? {};
}

/**
 * Wrap a single already-built `LanguageModel` as a factory for all three tiers.
 *
 * Lives here rather than in `testing.ts` so production code can use it too (a
 * middleware-wrapped model, a gateway model), and so `testing.ts` — which imports
 * `ai/test` — stays out of any production import graph.
 */
export function createStaticModelFactory(
	model: LanguageModel,
	provider: ProviderName = 'openrouter'
): ModelFactory {
	const make = (): LanguageModel => model;
	return { provider, slugFor: (m) => m, play: make, bid: make, talk: make };
}

/** Convenience: the shipped tier defaults, for a persona that does not override. */
export const DEFAULT_MODELS = { play: MODEL_PLAY, bid: MODEL_BID } as const satisfies Readonly<
	Record<'play' | 'bid', AnthropicModelId>
>;
