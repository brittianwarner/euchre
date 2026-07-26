/**
 * `score.ts` — the euchre scoring table, and game end.
 *
 * | Situation                          | `HandResult` | Points | To        |
 * |------------------------------------|--------------|--------|-----------|
 * | Makers take 3 or 4 tricks          | `point`      | 1      | Makers    |
 * | Makers take all 5 (march)          | `march`      | 2      | Makers    |
 * | Lone maker takes 3 or 4            | `lone_point` | 1      | Makers    |
 * | Lone maker takes all 5             | `lone_march` | 4      | Makers    |
 * | Makers take 0–2 (euchred)          | `euchre`     | 2      | Defenders |
 * | Lone maker takes 0–2               | `euchre`     | 2      | Defenders |
 * | Four passes, `stickTheDealer:false`| `throw_in`   | 0      | —         |
 *
 * There is no super-euchre row: `cfg.superEuchre` is the literal `false`, and the
 * 4-point all-tricks-to-defenders variant has no code path in v1.
 */

import { teamOf } from './seats';
import type { HandResult, Seat, Team } from './types';

/** The result of applying the table to one finished hand. */
export interface ScoreOutcome {
	readonly result: HandResult;
	/** **Post-clamp**, so `score[i] + delta[i]` is always the new score. */
	readonly delta: readonly [number, number];
}

/**
 * Apply the scoring table to a completed hand.
 *
 * `makers = teamOf(makerSeat)`, `made = tricksWon[makers]`. Going alone changes
 * the *label* at 3–4 tricks (`lone_point`) and the *points* at 5 (4 instead of
 * 2); a lone maker who is euchred still concedes exactly 2, same as anyone else.
 *
 * `delta` is reported **after** clamping at `gameTo`: a march at 9 wins at 10,
 * not 11. `delta` is therefore non-negative in both components and has at most
 * one non-zero component (V11).
 *
 * Pure: reads `tricksWon` and `score`, mutates neither.
 */
export function scoreHand(
	makerSeat: Seat,
	aloneSeat: Seat | null,
	tricksWon: readonly [number, number],
	score: readonly [number, number],
	gameTo: number
): ScoreOutcome {
	const makers = teamOf(makerSeat);
	const defenders = (1 - makers) as Team;
	const made = tricksWon[makers]!;
	const alone = aloneSeat !== null;

	const raw: [number, number] = [0, 0];
	let result: HandResult;
	if (made <= 2) {
		result = 'euchre';
		raw[defenders] = 2;
	} else if (made <= 4) {
		result = alone ? 'lone_point' : 'point';
		raw[makers] = 1;
	} else {
		result = alone ? 'lone_march' : 'march';
		raw[makers] = alone ? 4 : 2;
	}

	const next: [number, number] = [
		Math.min(gameTo, score[0]! + raw[0]),
		Math.min(gameTo, score[1]! + raw[1])
	];
	return { result, delta: [next[0] - score[0]!, next[1] - score[1]!] };
}

/**
 * The outcome of a thrown-in hand: nobody scores. Reachable only with
 * `cfg.stickTheDealer === false`, when all four seats pass in round 2. The deal
 * still moves left and `misdealStreak` still increments — that is the caller's
 * job, not this function's.
 */
export function throwInOutcome(): ScoreOutcome {
	return { result: 'throw_in', delta: [0, 0] };
}

/**
 * The new score after applying an outcome. Returns a fresh tuple; never mutates
 * `score`. Because `delta` is already clamped, this is a plain addition.
 */
export function applyDelta(
	score: readonly [number, number],
	delta: readonly [number, number]
): [number, number] {
	return [score[0]! + delta[0]!, score[1]! + delta[1]!];
}

/**
 * The team that has won, or `null` if the match is still live. A team wins the
 * instant its score reaches `gameTo`; scores clamp there, so this is `>=` for
 * safety rather than necessity. Team `0` is checked first — both reaching
 * `gameTo` simultaneously is impossible, since only one team scores per hand.
 */
export function winnerTeam(score: readonly [number, number], gameTo: number): Team | null {
	if (score[0]! >= gameTo) return 0;
	if (score[1]! >= gameTo) return 1;
	return null;
}

/** Whether the match has been won. Equivalent to `winnerTeam(...) !== null`. */
export function isGameOver(score: readonly [number, number], gameTo: number): boolean {
	return winnerTeam(score, gameTo) !== null;
}

/**
 * Whether a team is one point from winning — "in the barn", the table line spoken
 * at 9. Drives both the announcement and the AI's donate-a-point heuristic.
 */
export function isInTheBarn(score: readonly [number, number], team: Team, gameTo: number): boolean {
	return score[team]! === gameTo - 1;
}

/**
 * `true` when a hand's outcome credits the defenders rather than the makers —
 * i.e. the makers were euchred. Kept here so the celebration copy, the tempo
 * gate and the score animation all read the same predicate.
 */
export function isEuchre(result: HandResult): boolean {
	return result === 'euchre';
}

/**
 * The points this result is worth, before clamping. Exposed for the invariant
 * check `score[i] + delta[i] === min(gameTo, score[i] + raw)` (V11) and for the
 * recap copy; the reducer should use {@link scoreHand}'s clamped `delta`.
 */
export function rawPoints(result: HandResult): number {
	switch (result) {
		case 'point':
		case 'lone_point':
			return 1;
		case 'march':
		case 'euchre':
			return 2;
		case 'lone_march':
			return 4;
		case 'throw_in':
			return 0;
	}
}
