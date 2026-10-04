import { describe, expect, it, vi } from 'vitest';
import { createJevDecisionClient, JEV_MODEL } from './jev';
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
				move: { type: 'choice', instructions: expect.any(String), criteria: input.criteria }
			}
		});
	});
	it.each([
		{ ...valid, answers: { move: { ...valid.answers.move, choice: 'illegal' } } },
		{ ...valid, answers: { move: { ...valid.answers.move, confidence: 2 } } },
		{ ...valid, answers: { move: { ...valid.answers.move, probabilities: { move_0: 1 } } } },
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
	it('never enables decisions through a text-only provider or legacy model overrides', () => {
		expect(createAnthropicModelFactory({ apiKey: 'test' })?.decision).toBeUndefined();
		const factory = modelFactoryFromEnv({
			OPENROUTER_API_KEY: 'test',
			OPENROUTER_MODEL_PLAY: 'other/model'
		});
		expect(factory?.decision).toBeTypeOf('function');
		expect(modelFactoryFromEnv({})).toBeNull();
	});
});
