/**
 * V22 — optimistic agreement (spec 02-GAME-RULES-ENGINE.md §6.5, test case 38).
 *
 * `applyOptimistic(project(s, seat), move)` must equal
 * `project(apply(s, seat, move).state, seat)` on every field in
 * `OPTIMISTIC_FIELDS`, for every reachable state and every legal move.
 *
 * This is what lets the client render your move instantly instead of waiting for
 * a round trip to Rivet Cloud. If it drifts, the UI renders a lie for one frame.
 */

import { describe, expect, it } from 'vitest';
import { createGame, legalMoves, apply, advance, project } from '../index';
import { applyOptimistic, OPTIMISTIC_FIELDS } from '../optimistic';
import type { GameState, PublicGameView, Seat } from '../types';

const SEATS: readonly Seat[] = [0, 1, 2, 3];

function mulberry32(a: number) {
	return () => {
		a |= 0;
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

function pick<T>(xs: readonly T[], rng: () => number): T {
	return xs[Math.floor(rng() * xs.length)]!;
}

/** Which seat, if any, must act now. */
function actingSeat(s: GameState): Seat | null {
	for (const seat of SEATS) if (legalMoves(s, seat).length > 0) return seat;
	return null;
}

interface Mismatch {
	field: string;
	moveId: string;
	phase: string;
	seat: Seat;
	expected: string;
	actual: string;
}

/**
 * Drive random legal games, and at every decision point check EVERY legal move
 * (not just the one taken) against the authoritative reducer.
 */
function sweep(games: number, seedBase: number): { checks: number; mismatches: Mismatch[] } {
	const mismatches: Mismatch[] = [];
	let checks = 0;

	for (let g = 0; g < games; g++) {
		const rng = mulberry32(seedBase + g * 7919);
		let s = createGame({ gameId: `v22-${g}`, seed: `v22-${g}` });

		for (let step = 0; step < 4000 && s.status === 'active'; step++) {
			const seat = actingSeat(s);
			if (seat === null) {
				const r = advance(s);
				if (r.state === s) break;
				s = r.state;
				continue;
			}

			const moves = legalMoves(s, seat);
			const view = project(s, seat);

			// Check every legal move from this state, not merely the one we take.
			for (const lm of moves) {
				// `cut` triggers the deal from the server-held seeded shuffle, so it is
				// explicitly outside the optimistic contract (see optimistic.ts).
				if (lm.move.t === 'cut') continue;
				const expected: PublicGameView = project(apply(s, seat, lm.move).state, seat);
				const actual: PublicGameView = applyOptimistic(view, lm.move);
				checks++;

				for (const field of OPTIMISTIC_FIELDS) {
					const e = JSON.stringify(expected[field]);
					const a = JSON.stringify(actual[field]);
					if (e !== a && mismatches.length < 25) {
						mismatches.push({
							field,
							moveId: lm.id,
							phase: s.hand.phase,
							seat,
							expected: e ?? 'undefined',
							actual: a ?? 'undefined'
						});
					}
				}
			}

			s = apply(s, seat, pick(moves, rng).move).state;
		}
	}

	return { checks, mismatches };
}

describe('V22 — optimistic agreement', () => {
	it('agrees with the authoritative reducer on every legal move across many games', () => {
		const { checks, mismatches } = sweep(60, 1);

		if (mismatches.length > 0) {
			const detail = mismatches
				.map(
					(m) =>
						`  ${m.field} after ${m.moveId} (phase ${m.phase}, seat ${m.seat})\n` +
						`    expected ${m.expected}\n    actual   ${m.actual}`
				)
				.join('\n');
			throw new Error(`${mismatches.length} field mismatches over ${checks} checks:\n${detail}`);
		}

		expect(mismatches).toEqual([]);
		// Guard against the sweep silently doing nothing.
		expect(checks).toBeGreaterThan(5000);
	});

	it('is identity for a card the seat does not hold', () => {
		let s = createGame({ gameId: 'v22-id', seed: 'v22-id' });
		for (let i = 0; i < 50 && s.hand.phase !== 'trick_play'; i++) {
			const seat = actingSeat(s);
			s = seat === null ? advance(s).state : apply(s, seat, legalMoves(s, seat)[0]!.move).state;
		}
		const seat = actingSeat(s);
		if (seat === null) return;
		const view = project(s, seat);
		const notHeld = (['9S', 'TS', 'JS', 'QS'] as const).find((c) => !view.hand.includes(c));
		if (!notHeld) return;
		expect(applyOptimistic(view, { t: 'play', card: notHeld })).toBe(view);
	});

	it('never mutates the view it was given', () => {
		let s = createGame({ gameId: 'v22-imm', seed: 'v22-imm' });
		for (let i = 0; i < 60 && s.hand.phase !== 'trick_play'; i++) {
			const seat = actingSeat(s);
			s = seat === null ? advance(s).state : apply(s, seat, legalMoves(s, seat)[0]!.move).state;
		}
		const seat = actingSeat(s);
		if (seat === null) return;
		const view = project(s, seat);
		const snapshot = JSON.stringify(view);
		for (const lm of legalMoves(s, seat)) applyOptimistic(view, lm.move);
		expect(JSON.stringify(view)).toBe(snapshot);
	});
});
