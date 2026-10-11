/** Paid regression probe through the same decision ladder as production.
 * bun scripts/ai-bidding-live.ts [repetitions 1–10]
 * All three personas, rotated/reversed option order. No secrets or private state logged.
 */
import { biddingCases } from './fixtures/bidding';
import { acceptanceBiddingCases } from './fixtures/bidding-acceptance';
import { runLadder } from '../src/lib/actors/ai-seat/decide';
import { defaultPersonas } from '../src/lib/server/personas';
import { JEV_MODEL } from '../src/lib/ai/jev';
import { callModel } from '../src/lib/ai/decide';
import { modelFactoryFromEnv } from '../src/lib/ai/model';
import { setLlmProvider } from '../src/lib/actors/ai-seat/llm-bridge';

if (!process.env.OPENROUTER_API_KEY) throw new Error('OPENROUTER_API_KEY is required');
const repetitions = Number(process.argv[2] ?? 1);
if (!Number.isInteger(repetitions) || repetitions < 1 || repetitions > 10) {
	throw new Error('Choose 1–10 repetitions');
}
const results = [];
const scenarios = process.argv.includes('--acceptance') ? acceptanceBiddingCases : biddingCases;
const factory = modelFactoryFromEnv(process.env)!;
for (let repeat = 0; repeat < repetitions; repeat++) {
	// Permute every question's answer descriptions too, including support and loner.
	// This adapter changes only transport order; it calls the real Jev client and composer.
	const reorder = <T>(record: Readonly<Record<string, T>>) =>
		Object.fromEntries(repeat % 2 ? Object.entries(record).reverse() : Object.entries(record));
	setLlmProvider({
		decide: async (input) =>
			callModel(
				{
					factory: {
						...factory,
						decision: (question) =>
							factory.decision!({
								...question,
								criteria: reorder(question.criteria),
								additionalQuestions: Object.fromEntries(
									Object.entries(question.additionalQuestions ?? {}).map(([id, q]) => [
										id,
										{ ...q, criteria: reorder(q.criteria) }
									])
								)
							})
					},
					persona: input.persona,
					dossier: input.dossier,
					nonce: input.nonce
				},
				{
					internalToken: '',
					gameId: 'bidding-live',
					seat: input.seat,
					turnId: '',
					kind: input.kind,
					view: input.view,
					legal: input.candidates,
					ranking: input.ranking,
					deadlineAt: input.deadlineAt
				},
				input.candidates,
				{ budgetMs: input.budgetMs, escalate: input.escalate }
			)
	});
	for (const assignment of defaultPersonas()) {
		for (const [index, scenario] of scenarios.entries()) {
			const { view, expected, name } = scenario;
			const offset = (repeat + assignment.seat - 1) % view.legal.length;
			const rotated = [...view.legal.slice(offset), ...view.legal.slice(0, offset)];
			const legal = repeat % 2 ? rotated.reverse() : rotated;
			const started = Date.now();
			const outcome = await runLadder(
				{ ...assignment, nonce: 'bidding-live', degraded: false, now: Date.now, log: () => {} },
				{
					internalToken: '',
					gameId: 'bidding-live',
					seat: view.you,
					turnId: `case-${index}-${repeat}`,
					kind: view.phase === 'bid_round_1' ? 'bid1' : 'bid2',
					view: { ...view, legal },
					legal,
					deadlineAt: Date.now() + 20_000
				}
			);
			const result = {
				name,
				persona: assignment.persona.name,
				repeat,
				expected,
				actual: outcome?.moveId,
				source: outcome?.source,
				passed: outcome?.source === 'llm' && expected.includes(outcome.moveId),
				ms: Date.now() - started,
				inputTokens: outcome?.usage.inputTokens ?? 0,
				calls: outcome?.llmCalls ?? 0
			};
			results.push(result);
			console.log(JSON.stringify(result));
		}
	}
}
const failures = results.filter((result) => !result.passed).length;
console.log(
	JSON.stringify({
		model: JEV_MODEL,
		cases: results.length,
		failures,
		calls: results.reduce((sum, result) => sum + result.calls, 0),
		meanMs: Math.round(results.reduce((sum, result) => sum + result.ms, 0) / results.length),
		meanInputTokens: Math.round(
			results.reduce((sum, result) => sum + result.inputTokens, 0) / results.length
		)
	})
);
if (failures) process.exitCode = 1;
