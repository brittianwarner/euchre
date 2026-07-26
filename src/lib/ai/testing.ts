/**
 * The deterministic test seam. **No production module may import this file.**
 *
 * It is the only place `ai/test` appears. Everything above it takes a
 * `ModelFactory` by injection, so a test swaps the whole provider wholesale and no
 * test ever reads an API key, opens a socket, or depends on a model's mood.
 *
 * Verified against the installed `ai@7.0.37`: the export is `MockLanguageModelV4`
 * from `ai/test` — not `MockLanguageModel`, and not `MockLanguageModelV3`, which
 * also exists and implements the previous specification version. `doGenerate`
 * accepts either a function or a **result, or an array of results**, one per
 * successive call, which is what scripts a whole deterministic hand.
 */

import { MockLanguageModelV4, simulateReadableStream } from 'ai/test';
import type { LanguageModel } from 'ai';
import { createStaticModelFactory } from './model';
import type { ModelFactory } from './types';

/** One scripted `generateObject` response. */
export interface ScriptedResult {
	/** Raw assistant text. Give it malformed JSON to exercise the repair path. */
	readonly text: string;
	readonly inputTokens?: number;
	readonly outputTokens?: number;
	readonly cacheReadTokens?: number;
	readonly cacheWriteTokens?: number;
	readonly finishReason?: 'stop' | 'length' | 'content-filter' | 'error' | 'tool-calls' | 'other';
}

type GenerateResult = Awaited<ReturnType<NonNullable<MockLanguageModelV4['doGenerate']>>>;
type StreamResult = Awaited<ReturnType<NonNullable<MockLanguageModelV4['doStream']>>>;

function toGenerateResult(r: ScriptedResult): GenerateResult {
	return {
		content: [{ type: 'text', text: r.text }],
		finishReason: { unified: r.finishReason ?? 'stop', raw: undefined },
		usage: {
			inputTokens: {
				total: r.inputTokens ?? 100,
				noCache: r.inputTokens ?? 100,
				cacheRead: r.cacheReadTokens ?? 0,
				cacheWrite: r.cacheWriteTokens ?? 0
			},
			outputTokens: { total: r.outputTokens ?? 20, text: r.outputTokens ?? 20, reasoning: 0 }
		},
		warnings: []
	};
}

function toStreamResult(r: ScriptedResult): StreamResult {
	return {
		stream: simulateReadableStream({
			chunks: [
				{ type: 'text-start', id: '0' },
				{ type: 'text-delta', id: '0', delta: r.text },
				{ type: 'text-end', id: '0' },
				{
					type: 'finish',
					finishReason: { unified: r.finishReason ?? 'stop', raw: undefined },
					usage: {
						inputTokens: {
							total: r.inputTokens ?? 100,
							noCache: r.inputTokens ?? 100,
							cacheRead: r.cacheReadTokens ?? 0,
							cacheWrite: r.cacheWriteTokens ?? 0
						},
						outputTokens: { total: r.outputTokens ?? 20, text: r.outputTokens ?? 20, reasoning: 0 }
					}
				}
			]
		})
	};
}

export interface MockModelHandle {
	readonly model: MockLanguageModelV4;
	readonly factory: ModelFactory;
	/** The recorded call options, for asserting what was actually sent. */
	readonly calls: () => readonly unknown[];
}

/**
 * A mock model that returns each scripted result in turn.
 *
 * Running past the end of the script repeats the last entry, so a test that only
 * cares about the first two calls does not have to pad.
 */
export function scriptedModel(results: readonly ScriptedResult[], modelId = 'mock'): MockModelHandle {
	if (results.length === 0) throw new Error('scriptedModel needs at least one result');
	let i = 0;
	const next = (): ScriptedResult => results[Math.min(i++, results.length - 1)];

	const model = new MockLanguageModelV4({
		modelId,
		provider: 'mock',
		doGenerate: async () => toGenerateResult(next()),
		doStream: async () => toStreamResult(next())
	});

	return {
		model,
		factory: createStaticModelFactory(model as unknown as LanguageModel),
		calls: () => model.doGenerateCalls
	};
}

/**
 * A model that never settles until aborted — the timeout rung of the ladder.
 *
 * `doGenerate` rejects with a `TimeoutError`-named error when the signal fires, so
 * the classifier in `decide.ts` sees exactly what a real abort produces.
 */
export function hangingModel(modelId = 'mock-hang'): MockModelHandle {
	const model = new MockLanguageModelV4({
		modelId,
		provider: 'mock',
		doGenerate: (options) =>
			new Promise((_resolve, reject) => {
				const signal = options.abortSignal;
				if (signal === undefined) return;
				const fail = () => {
					const err = new Error('aborted');
					err.name = 'TimeoutError';
					reject(err);
				};
				if (signal.aborted) fail();
				else signal.addEventListener('abort', fail, { once: true });
			})
	});

	return {
		model,
		factory: createStaticModelFactory(model as unknown as LanguageModel),
		calls: () => model.doGenerateCalls
	};
}

/**
 * A factory whose three tiers are backed by *different* mocks.
 *
 * Used to assert that the escalation rung really switched to the deliberate tier
 * rather than re-rolling the fast one.
 */
export function tieredFactory(fast: MockModelHandle, deliberate: MockModelHandle): ModelFactory {
	return {
		// Mocks never reach a real upstream, so the tag only has to be a valid
		// ProviderName; 'openrouter' keeps tests on the shipped code path.
		provider: 'openrouter',
		slugFor: (m) => m,
		play: () => fast.model as unknown as LanguageModel,
		talk: () => fast.model as unknown as LanguageModel,
		bid: () => deliberate.model as unknown as LanguageModel
	};
}

/** A factory-free deps fragment: forces every ladder call onto the heuristic rung. */
export const NO_PROVIDER: { readonly factory: null } = { factory: null };
