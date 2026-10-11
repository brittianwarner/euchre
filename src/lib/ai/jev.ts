/** Jev's Decisions API is intentionally separate from every text-generation SDK. */
import { z } from 'zod';
import type { ModelFactory } from './types';

export const JEV_MODEL = 'typesafe/jev-1.13';
/** Safe diagnostics: never retain a response body or private request state. */
export class JevDecisionError extends Error {
	constructor(
		readonly reason: 'http_status' | 'response_schema' | 'choice_distribution',
		readonly status?: number,
		readonly diagnostic?: { requestId?: string; questionId: string; violation: string }
	) {
		super(`Jev ${reason}${status ? ` (${status})` : ''}`);
		this.name = 'JevDecisionError';
	}
}
const probability = z.number().finite().min(0).max(1);
const answerSchema = z.object({
	type: z.literal('choice'),
	choice: z.string(),
	confidence: probability,
	probabilities: z.record(z.string(), probability)
});
const responseSchema = z.object({
	id: z.string().optional(),
	model: z.string().optional(),
	answers: z.record(z.string(), answerSchema),
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
	return async ({ state, instructions, criteria, additionalQuestions = {}, signal }) => {
		signal.throwIfAborted();
		const questions = {
			...additionalQuestions,
			move: { type: 'choice' as const, instructions, criteria }
		};
		const response = await request(`${base}/alpha/decisions`, {
			method: 'POST',
			signal,
			headers: { Authorization: `Bearer ${options.apiKey}`, 'Content-Type': 'application/json' },
			body: JSON.stringify({
				model: JEV_MODEL,
				state,
				questions
			})
		});
		if (!response.ok) {
			await response.body?.cancel();
			throw new JevDecisionError('http_status', response.status);
		}
		let parsed: z.infer<typeof responseSchema>;
		try {
			parsed = responseSchema.parse(await response.json());
		} catch (error) {
			if (signal.aborted) throw error;
			throw new JevDecisionError('response_schema');
		}
		for (const [id, question] of Object.entries(questions)) {
			const answer = parsed.answers[id];
			const keys = Object.keys(question.criteria);
			const violation = !answer
				? 'missing_answer'
				: !Object.hasOwn(question.criteria, answer.choice)
					? 'unknown_choice'
					: keys.some((key) => !Object.hasOwn(answer.probabilities, key)) ||
						  Object.keys(answer.probabilities).some(
								(key) => !Object.hasOwn(question.criteria, key)
						  )
						? 'option_keys'
						: Math.abs(
									Object.values(answer.probabilities).reduce((sum, value) => sum + value, 0) - 1
							  ) > 0.02
							? 'probability_total'
							: keys.some(
										(key) =>
											answer.probabilities[key]! > answer.probabilities[answer.choice]! + 0.000001
								  )
								? 'choice_not_maximum'
								: null;
			if (violation)
				throw new JevDecisionError('choice_distribution', undefined, {
					requestId: parsed.id,
					questionId: id,
					violation
				});
		}
		const answer = parsed.answers.move!;
		return {
			choice: answer.choice,
			confidence: answer.confidence,
			probabilities: answer.probabilities,
			requestId: parsed.id,
			modelSnapshot: parsed.model,
			additionalAnswers: Object.fromEntries(
				Object.entries(parsed.answers).filter(([id]) => id !== 'move')
			),
			usage: {
				inputTokens: parsed.usage.input_tokens,
				outputTokens: parsed.usage.output_tokens,
				cacheReadTokens: 0,
				cacheWriteTokens: 0
			}
		};
	};
}
