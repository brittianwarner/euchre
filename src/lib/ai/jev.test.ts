import { describe, expect, it, vi } from 'vitest';
import { createJevDecisionClient, JevDecisionError, JEV_MODEL } from './jev';
import { createAnthropicModelFactory, modelFactoryFromEnv } from './model';

const valid = {
	answers: {
		move: {
			type: 'choice',
			choice: 'move_1',
			confidence: 0.8,
			probabilities: { move_0: 0.1, move_1: 0.9 }
		}
	},
	usage: { input_tokens: 42, output_tokens: 0 }
};
const input = {
	state: 'seat-visible state',
	instructions: 'Choose the best partnership bid using the supplied hand and auction.',
	criteria: { move_0: 'Pass', move_1: 'Order up' },
	signal: new AbortController().signal
};

describe('Jev decision boundary', () => {
	it('uses the pinned Decisions model and propagates cancellation and usage', async () => {
		const request = vi.fn<typeof fetch>().mockResolvedValue(Response.json(valid));
		const decide = createJevDecisionClient({ apiKey: 'test', fetch: request });
		const result = await decide(input);
		expect(result).toMatchObject({ choice: 'move_1', usage: { inputTokens: 42, outputTokens: 0 } });
		expect(request.mock.calls[0][0]).toBe('https://openrouter.ai/api/alpha/decisions');
		const options = request.mock.calls[0][1]!;
		expect(options.signal).toBe(input.signal);
		expect(JSON.parse(options.body as string)).toEqual({
			model: JEV_MODEL,
			state: input.state,
			questions: {
				move: { type: 'choice', instructions: input.instructions, criteria: input.criteria }
			}
		});
	});
	it.each([
		{ ...valid, answers: { move: { ...valid.answers.move, choice: 'illegal' } } },
		{ ...valid, answers: { move: { ...valid.answers.move, confidence: 2 } } },
		{ ...valid, answers: { move: { ...valid.answers.move, probabilities: { move_0: 1 } } } },
		{
			...valid,
			answers: { move: { ...valid.answers.move, probabilities: { move_0: 0.8, move_1: 0.8 } } }
		},
		{ ...valid, answers: { move: { ...valid.answers.move, choice: 'move_0' } } },
		{ ...valid, answers: {} },
		{ answers: valid.answers }
	])('rejects malformed or out-of-set responses', async (body) => {
		const request = vi.fn<typeof fetch>().mockResolvedValue(Response.json(body));
		await expect(
			createJevDecisionClient({ apiKey: 'test', fetch: request })(input)
		).rejects.toThrow();
	});
	it('does not retry provider errors or send a cancelled request', async () => {
		const request = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 429 }));
		const decide = createJevDecisionClient({ apiKey: 'test', fetch: request });
		await expect(decide(input)).rejects.toThrow('429');
		expect(request).toHaveBeenCalledTimes(1);
		await expect(decide({ ...input, signal: AbortSignal.abort() })).rejects.toThrow();
		expect(request).toHaveBeenCalledTimes(1);
	});
	it('accepts native JSON descriptions and validates every batched judgment', async () => {
		const batched = {
			...input,
			state: { ownHand: ['Ace of hearts'] },
			additionalQuestions: {
				loner: {
					type: 'choice' as const,
					instructions: { question: 'Keep partner or go alone?' },
					criteria: { partner: { meaning: 'Keep partner' }, alone: { meaning: 'Play alone' } }
				}
			}
		};
		const body = {
			...valid,
			id: 'test-request',
			model: 'typesafe/jev-1.13-snapshot',
			answers: {
				...valid.answers,
				loner: {
					type: 'choice',
					choice: 'partner',
					confidence: 0.8,
					probabilities: { partner: 0.9, alone: 0.1 }
				}
			}
		};
		const request = vi.fn<typeof fetch>().mockResolvedValue(Response.json(body));
		const result = await createJevDecisionClient({ apiKey: 'test', fetch: request })(batched);
		expect(result.additionalAnswers?.loner.choice).toBe('partner');
		expect(result).toMatchObject({
			requestId: 'test-request',
			modelSnapshot: 'typesafe/jev-1.13-snapshot',
			probabilities: valid.answers.move.probabilities
		});
		expect(JSON.parse(request.mock.calls[0][1]!.body as string).state).toEqual(batched.state);
		request.mockResolvedValue(Response.json(valid));
		await expect(
			createJevDecisionClient({ apiKey: 'test', fetch: request })(batched)
		).rejects.toThrow();
	});
	it('never enables decisions through a text-only provider or legacy model overrides', () => {
		expect(createAnthropicModelFactory({ apiKey: 'test' })?.decision).toBeUndefined();
		const factory = modelFactoryFromEnv({
			OPENROUTER_API_KEY: 'test',
			OPENROUTER_MODEL_PLAY: 'other/model'
		});
		expect(factory?.decision).toBeTypeOf('function');
		expect(modelFactoryFromEnv({})).toBeNull();
	});
	it('classifies failures without retaining potentially sensitive response bodies', async () => {
		const request = vi
			.fn<typeof fetch>()
			.mockResolvedValue(new Response('private key and request data', { status: 503 }));
		const error = await createJevDecisionClient({ apiKey: 'test', fetch: request })(input).catch(
			(error) => error
		);
		expect(error).toBeInstanceOf(JevDecisionError);
		expect(error).toMatchObject({ reason: 'http_status', status: 503 });
		expect(String(error)).not.toContain('private key');
		request.mockResolvedValue(Response.json({ invalid: 'private hand' }));
		await expect(
			createJevDecisionClient({ apiKey: 'test', fetch: request })(input)
		).rejects.toMatchObject({ reason: 'response_schema' });
	});
});
