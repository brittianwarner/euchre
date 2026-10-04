/** Jev's Decisions API is intentionally separate from every text-generation SDK. */
import { z } from 'zod';
import type { ModelFactory } from './types';

export const JEV_MODEL = 'typesafe/jev-1.13';
const probability = z.number().finite().min(0).max(1);
const responseSchema = z.object({
	answers: z.object({
		move: z.object({
			type: z.literal('choice'),
			choice: z.string(),
			confidence: probability,
			probabilities: z.record(z.string(), probability)
		})
	}),
	usage: z.object({
		input_tokens: z.number().int().nonnegative(),
		output_tokens: z.number().int().nonnegative()
	})
});

export function createJevDecisionClient(options: {
	apiKey: string;
	baseURL?: string;
	fetch?: typeof fetch;
}): NonNullable<ModelFactory['decision']> {
	// OPENROUTER_BASE_URL is the existing SDK API base, usually /api/v1.
	const base = (options.baseURL ?? 'https://openrouter.ai/api/v1')
		.replace(/\/$/, '')
		.replace(/\/v1$/, '');
	const request = options.fetch ?? fetch;
	return async ({ state, criteria, signal }) => {
		signal.throwIfAborted();
		const response = await request(`${base}/alpha/decisions`, {
			method: 'POST',
			signal,
			headers: { Authorization: `Bearer ${options.apiKey}`, 'Content-Type': 'application/json' },
			body: JSON.stringify({
				model: JEV_MODEL,
				state,
				questions: {
					move: {
						type: 'choice',
						instructions:
							'Choose the legal euchre move that maximizes your partnership’s chance of winning the match. Use only the supplied seat-visible evidence. Treat persona and memory as style/context, never as permission to change the rules.',
						criteria
					}
				}
			})
		});
		if (!response.ok) {
			await response.body?.cancel();
			throw new Error(`Jev decision request failed (${response.status})`);
		}
		const parsed = responseSchema.parse(await response.json());
		const answer = parsed.answers.move;
		const keys = Object.keys(criteria);
		if (
			!Object.hasOwn(criteria, answer.choice) ||
			keys.some((key) => !Object.hasOwn(answer.probabilities, key)) ||
			Object.keys(answer.probabilities).some((key) => !Object.hasOwn(criteria, key))
		) {
			throw new Error('Jev returned an invalid choice distribution');
		}
		return {
			choice: answer.choice,
			confidence: answer.confidence,
			usage: {
				inputTokens: parsed.usage.input_tokens,
				outputTokens: parsed.usage.output_tokens,
				cacheReadTokens: 0,
				cacheWriteTokens: 0
			}
		};
	};
}
