/**
 * `score.test.ts` — the scoring table, the clamp, and game end.
 *
 * Covers `docs/02-GAME-RULES-ENGINE.md` § 5.1, § 5.2, § 9 cases 33–37 (the
 * scoring half of 37; the `handNo`/`dealerSeat`/`misdealStreak` half belongs to
 * `reduce.ts`), and invariant V11.
 *
 * Every one of the seven rows of the table is asserted with its exact
 * `HandResult` label **and** its exact point value, from every maker seat.
 */

import { describe, expect, it } from 'vitest';
import {
	applyDelta,
	isEuchre,
	isGameOver,
	isInTheBarn,
	rawPoints,
	scoreHand,
	throwInOutcome,
	winnerTeam
} from '../score';
import { partnerOf, teamOf } from '../seats';
import type { HandResult, Seat, Team } from '../types';

const ALL_SEATS: readonly Seat[] = [0, 1, 2, 3];

/** `tricksWon` with `made` going to `makers` and the rest to the defenders. */
const tricks = (makers: Team, made: number): [number, number] =>
	makers === 0 ? [made, 5 - made] : [5 - made, made];

/* -------------------------------------------------------------------------- */
/* The seven rows — § 9 case 33                                                */
/* -------------------------------------------------------------------------- */

interface Row {
	readonly made: number;
	readonly alone: boolean;
	readonly result: HandResult;
	readonly points: number;
	/** Who the points go to. */
	readonly to: 'makers' | 'defenders';
}

const TABLE: readonly Row[] = [
	{ made: 5, alone: false, result: 'march', points: 2, to: 'makers' },
	{ made: 4, alone: false, result: 'point', points: 1, to: 'makers' },
	{ made: 3, alone: false, result: 'point', points: 1, to: 'makers' },
	{ made: 2, alone: false, result: 'euchre', points: 2, to: 'defenders' },
	{ made: 1, alone: false, result: 'euchre', points: 2, to: 'defenders' },
	{ made: 0, alone: false, result: 'euchre', points: 2, to: 'defenders' },
	{ made: 5, alone: true, result: 'lone_march', points: 4, to: 'makers' },
	{ made: 4, alone: true, result: 'lone_point', points: 1, to: 'makers' },
	{ made: 3, alone: true, result: 'lone_point', points: 1, to: 'makers' },
	{ made: 2, alone: true, result: 'euchre', points: 2, to: 'defenders' },
	{ made: 1, alone: true, result: 'euchre', points: 2, to: 'defenders' },
	{ made: 0, alone: true, result: 'euchre', points: 2, to: 'defenders' }
];

describe('scoreHand — the full table (§ 5.1, § 9 case 33)', () => {
	for (const row of TABLE) {
		const who = row.alone ? 'lone maker' : 'makers';
		it(`${who} take ${row.made} → ${row.result}, ${row.points} to the ${row.to}`, () => {
			for (const makerSeat of ALL_SEATS) {
				const makers = teamOf(makerSeat);
				const defenders = (1 - makers) as Team;
				const aloneSeat = row.alone ? makerSeat : null;

				const out = scoreHand(makerSeat, aloneSeat, tricks(makers, row.made), [0, 0], 10);

				const want: [number, number] = [0, 0];
				want[row.to === 'makers' ? makers : defenders] = row.points;

				expect([makerSeat, out.result]).toEqual([makerSeat, row.result]);
				expect([makerSeat, out.delta]).toEqual([makerSeat, want]);
			}
		});
	}

	it('a lone maker who is euchred concedes exactly 2 — same as anyone else', () => {
		const lone = scoreHand(0, 0, [1, 4], [0, 0], 10);
		const pair = scoreHand(0, null, [1, 4], [0, 0], 10);
		expect(lone.result).toBe('euchre');
		expect(pair.result).toBe('euchre');
		expect(lone.delta).toEqual([0, 2]);
		expect(pair.delta).toEqual([0, 2]);
	});

	it('going alone changes the LABEL at 3–4 tricks but not the points', () => {
		expect(scoreHand(0, null, [3, 2], [0, 0], 10)).toEqual({ result: 'point', delta: [1, 0] });
		expect(scoreHand(0, 0, [3, 2], [0, 0], 10)).toEqual({ result: 'lone_point', delta: [1, 0] });
	});

	it('going alone doubles the march from 2 to 4', () => {
		expect(scoreHand(0, null, [5, 0], [0, 0], 10)).toEqual({ result: 'march', delta: [2, 0] });
		expect(scoreHand(0, 0, [5, 0], [0, 0], 10)).toEqual({ result: 'lone_march', delta: [4, 0] });
	});

	it('there is no super-euchre row: defenders taking all 5 is still worth 2', () => {
		expect(scoreHand(0, null, [0, 5], [0, 0], 10)).toEqual({ result: 'euchre', delta: [0, 2] });
		expect(scoreHand(0, 0, [0, 5], [0, 0], 10)).toEqual({ result: 'euchre', delta: [0, 2] });
	});

	it('reads `made` off the MAKERS’ index, not index 0', () => {
		// Team 1 makes 5. tricksWon = [0, 5]. Team 1 must get the march.
		expect(scoreHand(1, null, [0, 5], [0, 0], 10)).toEqual({ result: 'march', delta: [0, 2] });
		expect(scoreHand(3, null, [0, 5], [0, 0], 10)).toEqual({ result: 'march', delta: [0, 2] });
		// And the same tricksWon with a team-0 maker is a euchre against them.
		expect(scoreHand(0, null, [0, 5], [0, 0], 10)).toEqual({ result: 'euchre', delta: [0, 2] });
	});

	it('is identical for a seat and its partner — teams score, not seats', () => {
		for (const seat of ALL_SEATS) {
			for (const made of [0, 1, 2, 3, 4, 5]) {
				const makers = teamOf(seat);
				const t = tricks(makers, made);
				const a = scoreHand(seat, null, t, [2, 3], 10);
				const b = scoreHand(partnerOf(seat), null, t, [2, 3], 10);
				expect([seat, made, a]).toEqual([seat, made, b]);
			}
		}
	});

	it('the aloneSeat value itself is irrelevant — only null vs non-null matters', () => {
		// The engine guarantees aloneSeat === makerSeat (V4); scoreHand only reads
		// nullness, so this pins that it is not secretly consulted for a team.
		const a = scoreHand(0, 0, [5, 0], [0, 0], 10);
		const b = scoreHand(0, 2, [5, 0], [0, 0], 10);
		expect(a).toEqual(b);
		expect(a.delta).toEqual([4, 0]);
	});
});

/* -------------------------------------------------------------------------- */
/* The clamp — § 9 cases 34, 35, 36 / V11                                      */
/* -------------------------------------------------------------------------- */

describe('scoreHand — the clamp (V11)', () => {
	// § 9 case 34
	it('case 34: a march at 9 wins at 10, not 11', () => {
		const out = scoreHand(0, null, [5, 0], [9, 4], 10);
		expect(out.result).toBe('march');
		expect(out.delta).toEqual([1, 0]);
		expect(applyDelta([9, 4], out.delta)).toEqual([10, 4]);
	});

	// § 9 case 35
	it('case 35: a lone march from 7 gives 3, landing on 10 exactly', () => {
		const out = scoreHand(0, 0, [5, 0], [7, 2], 10);
		expect(out.result).toBe('lone_march');
		expect(out.delta).toEqual([3, 0]);
		expect(applyDelta([7, 2], out.delta)).toEqual([10, 2]);
	});

	// § 9 case 36
	it('case 36: a euchre credits the DEFENDERS even when the makers are the human’s team', () => {
		const out = scoreHand(0, null, [1, 4], [5, 5], 10);
		expect(out.result).toBe('euchre');
		expect(out.delta).toEqual([0, 2]);
		expect(applyDelta([5, 5], out.delta)).toEqual([5, 7]);
	});

	it('a euchre at 9 also clamps — the defenders win at 10, not 11', () => {
		const out = scoreHand(0, null, [1, 4], [3, 9], 10);
		expect(out.delta).toEqual([0, 1]);
		expect(applyDelta([3, 9], out.delta)).toEqual([3, 10]);
	});

	it('a team already at gameTo gains nothing', () => {
		expect(scoreHand(0, 0, [5, 0], [10, 3], 10).delta).toEqual([0, 0]);
	});

	it('clamps against non-default gameTo values', () => {
		expect(scoreHand(0, 0, [5, 0], [3, 0], 5).delta).toEqual([2, 0]);
		expect(scoreHand(0, null, [5, 0], [6, 0], 7).delta).toEqual([1, 0]);
		expect(scoreHand(0, 0, [5, 0], [8, 0], 11).delta).toEqual([3, 0]);
	});

	it('V11 exhaustively: delta is non-negative, single-component, and equals the clamped raw', () => {
		let checked = 0;
		for (const gameTo of [5, 7, 10, 11]) {
			for (const makerSeat of ALL_SEATS) {
				for (const alone of [false, true]) {
					for (let made = 0; made <= 5; made++) {
						for (let s0 = 0; s0 <= gameTo; s0++) {
							for (let s1 = 0; s1 <= gameTo; s1++) {
								const makers = teamOf(makerSeat);
								const score: [number, number] = [s0, s1];
								const out = scoreHand(
									makerSeat,
									alone ? makerSeat : null,
									tricks(makers, made),
									score,
									gameTo
								);
								checked++;

								const raw: [number, number] = [0, 0];
								const beneficiary = made <= 2 ? ((1 - makers) as Team) : makers;
								raw[beneficiary] = rawPoints(out.result);

								// Non-negative in both components.
								expect(out.delta[0]! >= 0 && out.delta[1]! >= 0).toBe(true);
								// At most one non-zero component.
								expect(out.delta[0]! === 0 || out.delta[1]! === 0).toBe(true);
								// score[i] + delta[i] === min(gameTo, score[i] + raw[i]).
								for (const i of [0, 1] as const) {
									expect([gameTo, makerSeat, alone, made, score, i, score[i] + out.delta[i]!]).toEqual(
										[
											gameTo,
											makerSeat,
											alone,
											made,
											score,
											i,
											Math.min(gameTo, score[i] + raw[i])
										]
									);
								}
								// Monotonic non-decreasing, and never past gameTo.
								const next = applyDelta(score, out.delta);
								expect(next[0] >= score[0] && next[1] >= score[1]).toBe(true);
								expect(next[0] <= gameTo && next[1] <= gameTo).toBe(true);
							}
						}
					}
				}
			}
		}
		expect(checked).toBe(
			[5, 7, 10, 11].reduce((n, g) => n + 4 * 2 * 6 * (g + 1) * (g + 1), 0)
		);
	});

	it('never mutates its score or tricksWon arguments', () => {
		const score: [number, number] = [9, 4];
		const won: [number, number] = [5, 0];
		Object.freeze(score);
		Object.freeze(won);
		expect(() => scoreHand(0, null, won, score, 10)).not.toThrow();
		expect(score).toEqual([9, 4]);
		expect(won).toEqual([5, 0]);
	});
});

/* -------------------------------------------------------------------------- */
/* throwInOutcome — § 9 case 37 (the scoring half)                             */
/* -------------------------------------------------------------------------- */

describe('throwInOutcome', () => {
	it('case 37: nobody scores', () => {
		expect(throwInOutcome()).toEqual({ result: 'throw_in', delta: [0, 0] });
	});

	it('leaves the score untouched when applied', () => {
		const out = throwInOutcome();
		expect(applyDelta([4, 7], out.delta)).toEqual([4, 7]);
	});

	it('returns a fresh outcome each call', () => {
		expect(throwInOutcome()).not.toBe(throwInOutcome());
	});
});

/* -------------------------------------------------------------------------- */
/* applyDelta                                                                  */
/* -------------------------------------------------------------------------- */

describe('applyDelta', () => {
	it('adds component-wise', () => {
		expect(applyDelta([3, 4], [2, 0])).toEqual([5, 4]);
		expect(applyDelta([0, 0], [0, 2])).toEqual([0, 2]);
	});

	it('returns a fresh tuple and never mutates the input', () => {
		const score: readonly [number, number] = Object.freeze([3, 4] as [number, number]);
		const next = applyDelta(score, [2, 0]);
		expect(next).not.toBe(score);
		expect(score).toEqual([3, 4]);
	});

	it('composes with scoreHand to reproduce the post-clamp score exactly', () => {
		let score: [number, number] = [0, 0];
		// Team 0 marches, marches, then euchres the opposition twice, then a point.
		score = applyDelta(score, scoreHand(0, null, [5, 0], score, 10).delta); // +2 → 2
		score = applyDelta(score, scoreHand(0, null, [5, 0], score, 10).delta); // +2 → 4
		score = applyDelta(score, scoreHand(1, null, [4, 1], score, 10).delta); // euchre → 6
		score = applyDelta(score, scoreHand(1, null, [4, 1], score, 10).delta); // euchre → 8
		score = applyDelta(score, scoreHand(0, null, [3, 2], score, 10).delta); // +1 → 9
		expect(score).toEqual([9, 0]);
		const last = scoreHand(0, 0, [5, 0], score, 10); // lone march, clamped
		expect(last.delta).toEqual([1, 0]);
		expect(applyDelta(score, last.delta)).toEqual([10, 0]);
	});
});

/* -------------------------------------------------------------------------- */
/* Game end                                                                    */
/* -------------------------------------------------------------------------- */

describe('winnerTeam / isGameOver', () => {
	it('is null while both teams are short of gameTo', () => {
		expect(winnerTeam([0, 0], 10)).toBeNull();
		expect(winnerTeam([9, 9], 10)).toBeNull();
		expect(isGameOver([9, 9], 10)).toBe(false);
	});

	it('names team 0 the instant it reaches gameTo', () => {
		expect(winnerTeam([10, 4], 10)).toBe(0);
		expect(isGameOver([10, 4], 10)).toBe(true);
	});

	it('names team 1 the instant it reaches gameTo', () => {
		expect(winnerTeam([4, 10], 10)).toBe(1);
		expect(isGameOver([4, 10], 10)).toBe(true);
	});

	it('respects a non-default gameTo', () => {
		expect(winnerTeam([5, 0], 10)).toBeNull();
		expect(winnerTeam([5, 0], 5)).toBe(0);
		expect(winnerTeam([6, 7], 7)).toBe(1);
	});

	it('agrees with isGameOver at every score up to gameTo', () => {
		for (let a = 0; a <= 10; a++) {
			for (let b = 0; b <= 10; b++) {
				const w = winnerTeam([a, b], 10);
				expect([a, b, isGameOver([a, b], 10)]).toEqual([a, b, w !== null]);
			}
		}
	});
});

describe('isInTheBarn', () => {
	it('is true at exactly gameTo - 1', () => {
		expect(isInTheBarn([9, 3], 0, 10)).toBe(true);
		expect(isInTheBarn([3, 9], 1, 10)).toBe(true);
	});

	it('is false at every other score', () => {
		for (let s = 0; s <= 10; s++) {
			expect([s, isInTheBarn([s, 0], 0, 10)]).toEqual([s, s === 9]);
		}
	});

	it('is per-team, not per-score-array', () => {
		expect(isInTheBarn([9, 3], 1, 10)).toBe(false);
		expect(isInTheBarn([3, 9], 0, 10)).toBe(false);
	});

	it('tracks a non-default gameTo', () => {
		expect(isInTheBarn([4, 0], 0, 5)).toBe(true);
		expect(isInTheBarn([9, 0], 0, 5)).toBe(false);
	});
});

/* -------------------------------------------------------------------------- */
/* isEuchre / rawPoints                                                        */
/* -------------------------------------------------------------------------- */

describe('isEuchre', () => {
	it('is true only for "euchre"', () => {
		const all: HandResult[] = ['point', 'march', 'lone_point', 'lone_march', 'euchre', 'throw_in'];
		for (const r of all) expect([r, isEuchre(r)]).toEqual([r, r === 'euchre']);
	});
});

describe('rawPoints', () => {
	it('matches the table for all six results', () => {
		expect(rawPoints('point')).toBe(1);
		expect(rawPoints('lone_point')).toBe(1);
		expect(rawPoints('march')).toBe(2);
		expect(rawPoints('euchre')).toBe(2);
		expect(rawPoints('lone_march')).toBe(4);
		expect(rawPoints('throw_in')).toBe(0);
	});

	it('equals the unclamped delta scoreHand produces from a fresh score', () => {
		for (const row of TABLE) {
			const makers = teamOf(0);
			const out = scoreHand(0, row.alone ? 0 : null, tricks(makers, row.made), [0, 0], 10);
			const total = out.delta[0]! + out.delta[1]!;
			expect([row.made, row.alone, total]).toEqual([row.made, row.alone, rawPoints(out.result)]);
		}
	});
});
