import { page } from 'vitest/browser';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'vitest-browser-svelte';
import { rankMoves } from '#lib/ai/heuristic.ts';
import {
	advance,
	apply,
	legalMoves,
	newGame,
	project,
	type GameState,
	type PublicGameView
} from '#lib/euchre/index.ts';
import Hint from './Hint.svelte';

const ENABLED_KEY = 'euchre:hints:v1:enabled';
const STREAK_KEY = 'euchre:hints:v1:streaks';
const JARGON = [
	/\bruff\b/i,
	/\blay off\b/i,
	/\bthird hand high\b/i,
	/\bsecond hand\b/i,
	/\bbower\b/i
];

interface HintCase {
	readonly view: PublicGameView;
	readonly term: RegExp;
}

/** Seat 0's decisions whose top hint uses a glossed term, from heuristic self-play. */
function jargonDecisions(seed: string): HintCase[] {
	let state: GameState = advance(newGame(seed)).state;
	const out: HintCase[] = [];
	for (let step = 0; step < 4000 && state.status === 'active'; step++) {
		const seat = state.hand.turnSeat;
		if (seat === null) {
			state = advance(state).state;
			continue;
		}
		const view = project(state, seat);
		const top = rankMoves(view)[0];
		const legal = legalMoves(state, seat);
		const move = legal.find((m) => m.id === top?.id) ?? legal[0];
		if (move === undefined) throw new Error('empty legal set');
		if (seat === 0 && legal.length > 1 && top !== undefined) {
			const term = JARGON.find((p) => p.test(top.why));
			if (term !== undefined) out.push({ view, term });
		}
		state = apply(state, seat, move.move).state;
	}
	return out;
}

/** Two seat-0 decisions on different turns whose hints use the same term. */
function repeatedTerm(): readonly [HintCase, HintCase] {
	for (let n = 0; n < 200; n++) {
		const cases = jargonDecisions(`hint-${n}`);
		for (let i = 0; i < cases.length; i++) {
			const first = cases[i] as HintCase;
			const later = cases
				.slice(i + 1)
				.find((c) => c.term === first.term && c.view.turnId !== first.view.turnId);
			if (later !== undefined) return [first, later];
		}
	}
	throw new Error('no seed gives two hints with the same term');
}

describe('Hint.svelte', () => {
	beforeEach(() => {
		localStorage.setItem(ENABLED_KEY, '1');
		localStorage.removeItem(STREAK_KEY);
	});
	afterEach(() => {
		localStorage.removeItem(ENABLED_KEY);
		localStorage.removeItem(STREAK_KEY);
	});

	it('renders a jargon hint without writing state during render, and explains the term once', async () => {
		const [first, later] = repeatedTerm();
		const status = page.getByRole('status');

		// Before the fix, this threw `state_unsafe_mutation` and crashed the table.
		const screen = await render(Hint, { view: first.view });
		await expect.element(status).toMatchTextContent(/— meaning /);

		// A fresh view object for the same turn keeps the explanation on screen.
		await screen.rerender({ view: { ...first.view } });
		await expect.element(status).toMatchTextContent(/— meaning /);

		// A later turn with the same term does not explain it again.
		await screen.rerender({ view: later.view });
		await expect.element(status).toBeInTheDocument();
		await expect.element(status).not.toMatchTextContent(/— meaning /);
	});
});
