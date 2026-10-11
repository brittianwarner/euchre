/** Paid full-match acceptance/competition probe. Two paired games per seed,
 * Jev and the existing expert heuristic swap teams on identical seeded deals.
 * This is a benchmark against that heuristic, not proof of optimal euchre play.
 * bun scripts/ai-match-eval.ts [seed pairs 1–20, default 6]
 */
import {
	advance,
	apply,
	assertInvariants,
	createGame,
	project,
	type Seat,
	type Team
} from '../src/lib/euchre/index';
import { topMove } from '../src/lib/ai/heuristic';
import { runLadder } from '../src/lib/actors/ai-seat/decide';
import { defaultPersonas } from '../src/lib/server/personas';
import { JEV_MODEL } from '../src/lib/ai/jev';
import { bidRollouts } from '../src/lib/ai/bid-rollouts';

if (!process.env.OPENROUTER_API_KEY) throw new Error('OPENROUTER_API_KEY is required');
const pairs = Number(process.argv[2] ?? 6);
if (!Number.isInteger(pairs) || pairs < 1 || pairs > 20) throw new Error('Choose 1–20 seed pairs');
const seedPrefix = process.argv.includes('--edge')
	? 'jev-edge-validation-oct11'
	: process.argv.includes('--fresh')
		? 'jev-frozen-validation-oct11'
		: 'jev-acceptance-oct11';
const personas = defaultPersonas();
const latencies: number[] = [];
const errors: Record<string, number> = {};
const counters = {
	llm: 0,
	forced: 0,
	fallback: 0,
	illegal: 0,
	failedMatches: 0,
	inputTokens: 0,
	calls: 0
};

async function match(pair: number, jevTeam: Team) {
	let state = createGame({
		gameId: `eval-${pair}-${jevTeam}`,
		seed: `${seedPrefix}-${pair}`,
		firstDealer: (pair % 4) as Seat,
		cfg: { stickTheDealer: pair % 2 === 0, requireNaturalTrump: pair % 3 === 0 }
	});
	const hands = [];
	let steps = 0;
	while (state.status === 'active' && steps++ < 2500) {
		assertInvariants(state);
		const seat = state.hand.turnSeat;
		if (seat === null) {
			const next = advance(state).state;
			if (next === state) throw new Error('Engine stopped advancing');
			if (next.hand.phase === 'hand_score') {
				const makerTeam = next.hand.makerSeat === null ? null : next.hand.makerSeat % 2;
				hands.push({
					maker: makerTeam === null ? null : makerTeam === jevTeam ? 'jev' : 'heuristic',
					result: next.hand.result,
					alone: next.hand.aloneSeat !== null
				});
			}
			state = next;
			continue;
		}
		const view = project(state, seat);
		const legal = view.legal;
		if (!legal.length) throw new Error('Acting seat has no legal move');
		let moveId;
		if (seat % 2 === jevTeam && state.hand.phase !== 'cutting') {
			const assignment = personas[(seat + pair) % personas.length]!;
			const phase = state.hand.phase;
			const kind =
				phase === 'bid_round_1'
					? 'bid1'
					: phase === 'bid_round_2'
						? 'bid2'
						: phase === 'dealer_discard'
							? 'discard'
							: 'play';
			const started = Date.now();
			const result = await runLadder(
				{
					...assignment,
					nonce: 'match-eval',
					degraded: false,
					now: Date.now,
					log: (event, fields) => {
						if (event === 'jev_failure') {
							const reason = [fields.reason, fields.violation, fields.status]
								.filter((value) => value !== undefined)
								.join(':');
							errors[reason] = (errors[reason] ?? 0) + 1;
						}
					}
				},
				{
					internalToken: '',
					gameId: state.gameId,
					seat,
					turnId: state.turnId,
					kind,
					view,
					legal,
					deadlineAt: Date.now() + 20_000
				}
			);
			if (result?.llmCalls) latencies.push(Date.now() - started);
			if (!result) throw new Error('Jev returned no decision');
			if (result.source === 'llm') counters.llm++;
			else if (result.source === 'forced') counters.forced++;
			else counters.fallback++;
			counters.calls += result.llmCalls;
			counters.inputTokens += result.usage.inputTokens;
			moveId = result.moveId;
			if (process.argv.includes('--trace-bids') && (kind === 'bid1' || kind === 'bid2'))
				console.log(
					JSON.stringify({
						bidTrace: true,
						pair,
						jevTeam,
						handNo: view.handNo,
						seat,
						dealerSeat: view.dealerSeat,
						phase: view.phase,
						hand: view.hand,
						upCard: view.upCard,
						selected: moveId,
						source: result.source,
						estimates: bidRollouts(view, legal)
					})
				);
			if (process.argv.includes('--trace') && pair === 0 && jevTeam === 0)
				console.log(
					JSON.stringify({
						trace: true,
						handNo: view.handNo,
						seat,
						phase: view.phase,
						hand: view.hand,
						trump: view.trump,
						upCard: view.upCard,
						plays: view.trick.plays,
						selected: moveId,
						reference: topMove(view, view.hand, legal)
					})
				);
		} else {
			moveId = state.hand.phase === 'cutting' ? 'cut:no' : topMove(view, view.hand, legal);
		}
		const move = legal.find((candidate) => candidate.id === moveId);
		if (!move) {
			counters.illegal++;
			throw new Error('Decision outside legal set');
		}
		state = apply(state, seat, move.move).state;
	}
	if (state.status !== 'complete') throw new Error('Match did not finish');
	const jevBids = hands.filter((hand) => hand.maker === 'jev');
	const heuristicBids = hands.filter((hand) => hand.maker === 'heuristic');
	const result = {
		pair,
		jevTeam,
		won: state.winnerTeam === jevTeam,
		score: state.score,
		hands: hands.length,
		jevCalls: jevBids.length,
		jevEuchred: jevBids.filter((hand) => hand.result === 'euchre').length,
		heuristicCalls: heuristicBids.length,
		heuristicEuchred: heuristicBids.filter((hand) => hand.result === 'euchre').length,
		jevLoners: jevBids.filter((hand) => hand.alone).length,
		jevLonerSweeps: jevBids.filter((hand) => hand.result === 'lone_march').length
	};
	console.log(JSON.stringify(result));
	return result;
}

const results = [];
// At most two independent matches in flight; never parallelize actions within a table.
for (let pair = 0; pair < pairs; pair++) {
	const games = await Promise.allSettled([match(pair, 0), match(pair, 1)]);
	for (const game of games) {
		if (game.status === 'fulfilled') results.push(game.value);
		else {
			counters.failedMatches++;
			console.error(String(game.reason));
		}
	}
	console.log(JSON.stringify({ progress: pair + 1, pairs, ...counters }));
}
latencies.sort((a, b) => a - b);
console.log(
	JSON.stringify({
		model: JEV_MODEL,
		seedPrefix,
		games: results.length,
		wins: results.filter((result) => result.won).length,
		hands: results.reduce((sum, result) => sum + result.hands, 0),
		jevCalls: results.reduce((sum, result) => sum + result.jevCalls, 0),
		jevEuchred: results.reduce((sum, result) => sum + result.jevEuchred, 0),
		heuristicCalls: results.reduce((sum, result) => sum + result.heuristicCalls, 0),
		heuristicEuchred: results.reduce((sum, result) => sum + result.heuristicEuchred, 0),
		...counters,
		errors,
		p95ModelDecisionMs: latencies[Math.floor(latencies.length * 0.95)] ?? 0,
		meanModelDecisionMs: Math.round(
			latencies.reduce((sum, value) => sum + value, 0) / latencies.length
		)
	})
);
if (counters.failedMatches || counters.illegal || counters.fallback) process.exitCode = 1;
