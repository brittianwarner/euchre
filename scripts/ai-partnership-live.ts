/** Opt-in, paid Jev regression probe through the production actor decision ladder.
 * Run: bun scripts/ai-partnership-live.ts [repetitions, default 1]
 * Uses OPENROUTER_API_KEY from the environment; logs no credentials or prompts.
 */
import { partnershipCases } from './fixtures/partnership';
import { runLadder } from '../src/lib/actors/ai-seat/decide';
import { defaultPersonas } from '../src/lib/server/personas';
import { JEV_MODEL } from '../src/lib/ai/jev';

if (!process.env.OPENROUTER_API_KEY) throw new Error('OPENROUTER_API_KEY is required');
const repetitions = Number(process.argv[2] ?? 1);
if (!Number.isInteger(repetitions) || repetitions < 1 || repetitions > 10) {
	throw new Error('Choose 1–10 repetitions');
}
const partner = defaultPersonas().find((assignment) => assignment.seat === 2)!;
let failures = 0;
let calls = 0;
const latencies: number[] = [];
for (let repeat = 0; repeat < repetitions; repeat++) {
	for (const [index, scenario] of partnershipCases.entries()) {
		const { view, expected, name } = scenario;
		// Reverse option ordering on alternate runs to catch accidental index bias.
		const legal = repeat % 2 ? [...view.legal].reverse() : view.legal;
		const started = Date.now();
		const outcome = await runLadder(
			{ ...partner, nonce: 'partnership-live', degraded: false, now: Date.now, log: () => {} },
			{
				internalToken: '',
				gameId: 'partnership-live',
				seat: view.you,
				turnId: `case-${index}-${repeat}`,
				kind: 'play',
				view: { ...view, legal },
				legal,
				deadlineAt: Date.now() + 20_000
			}
		);
		const ms = Date.now() - started;
		latencies.push(ms);
		calls += outcome?.llmCalls ?? 0;
		const passed = outcome?.source === 'llm' && outcome.moveId === expected;
		if (!passed) failures++;
		console.log(
			JSON.stringify({
				name,
				repeat,
				expected,
				actual: outcome?.moveId,
				source: outcome?.source,
				passed,
				ms
			})
		);
	}
}
console.log(
	JSON.stringify({
		model: JEV_MODEL,
		cases: latencies.length,
		calls,
		failures,
		maxMs: Math.max(...latencies),
		meanMs: Math.round(latencies.reduce((a, b) => a + b) / latencies.length)
	})
);
if (failures) process.exitCode = 1;
