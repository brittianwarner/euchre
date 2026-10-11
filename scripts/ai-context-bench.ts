/** Offline prompt preparation benchmark over actual public game projections. */
import { advance, apply, createGame, project, type Seat } from '../src/lib/euchre';
import { topMove } from '../src/lib/ai/heuristic';
import { buildDecisionQuestion } from '../src/lib/ai/decision-context';
import { defaultPersonas } from '../src/lib/server/personas';
const elapsed: number[] = [];
let bidQuestions = 0;
let playQuestions = 0;
for (let seed = 0; seed < 10; seed++) {
	let state = createGame({
		gameId: 'context-bench',
		seed: `context-bench-${seed}`,
		firstDealer: (seed % 4) as Seat
	});
	for (let step = 0; state.hand.handNo < 3 && step < 250; step++) {
		const seat = state.hand.turnSeat;
		if (seat === null) {
			state = advance(state).state;
			continue;
		}
		const view = project(state, seat);
		if (view.legal.length > 1 && view.phase !== 'cutting') {
			const start = performance.now();
			buildDecisionQuestion(view, view.legal, defaultPersonas()[seed % 3]!.persona);
			elapsed.push(performance.now() - start);
			if (view.phase === 'trick_play') playQuestions++;
			else if (view.phase.startsWith('bid_')) bidQuestions++;
		}
		const id = view.phase === 'cutting' ? 'cut:no' : topMove(view, view.hand, view.legal);
		state = apply(state, seat, view.legal.find((move) => move.id === id)!.move).state;
	}
}
elapsed.sort((a, b) => a - b);
console.log(
	JSON.stringify({
		decisions: elapsed.length,
		bidQuestions,
		playQuestions,
		meanMs: Math.round((elapsed.reduce((a, b) => a + b, 0) / elapsed.length) * 100) / 100,
		p95Ms: Math.round(elapsed[Math.floor(elapsed.length * 0.95)]! * 100) / 100,
		maxMs: Math.round(elapsed.at(-1)! * 100) / 100
	})
);
