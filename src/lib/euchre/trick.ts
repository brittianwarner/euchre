/**
 * `trick.ts` — the total order inside one trick, and who takes it.
 *
 * `cardValue` collapses trump rank, led-suit rank and "cannot win" into a single
 * comparable integer. It is the *only* comparator used by trick resolution and by
 * the heuristic ranker; nothing else is permitted to reason about relative card
 * strength.
 */

import { effectiveSuit, plainRank, trumpRank, type CardId, type Suit } from './cards';
import { activeSeatCount } from './seats';
import type { Play, Seat } from './types';

/**
 * Strength of `card` in a trick whose led suit is `led`, under `trump`.
 *
 * Three bands, and the gaps between them are the whole rule:
 * - `102..108` — trump, left bower included, ordered by {@link trumpRank};
 * - `51..56` — the led suit, ordered by {@link plainRank};
 * - `0` — neither: **cannot win**, so `9C` beats `AH` when clubs are led and
 *   diamonds are trump.
 *
 * Comparable only within one trick. Ties are impossible over the 24 distinct
 * cards (V17), so `trickWinner` is total and tie-free.
 *
 * `trump` may be `null` — safe to call during round-1 scratch evaluation, before
 * any trump exists.
 */
export function cardValue(card: CardId, led: Suit, trump: Suit | null): number {
	const eff = effectiveSuit(card, trump);
	if (trump !== null && eff === trump) return 100 + trumpRank(card, trump);
	if (eff === led) return 50 + plainRank(card);
	return 0;
}

/**
 * The seat that takes the trick: the highest trump if any trump was played,
 * otherwise the highest card of the led suit.
 *
 * `plays` must be non-empty and must all belong to the same trick. Does not
 * mutate `plays`. Throws a plain `Error` on an empty trick — a programmer error,
 * never a rules event.
 */
export function trickWinner(plays: readonly Play[], led: Suit, trump: Suit): Seat {
	if (plays.length === 0) throw new Error('trickWinner: a trick has no plays');
	let best = plays[0]!;
	let bestValue = cardValue(best.card, led, trump);
	for (let i = 1; i < plays.length; i++) {
		const p = plays[i]!;
		const v = cardValue(p.card, led, trump);
		if (v > bestValue) {
			bestValue = v;
			best = p;
		}
	}
	return best.seat;
}

/**
 * The **effective** suit led by the first play, which is what every seat must
 * follow. Leading the left bower leads trump, not its printed suit. Returns
 * `null` for an empty trick, meaning "the next card is on lead".
 */
export function ledSuitOf(plays: readonly Play[], trump: Suit | null): Suit | null {
	const first = plays[0];
	if (first === undefined) return null;
	return effectiveSuit(first.card, trump);
}

/**
 * How many plays complete a trick: `4` normally, `3` under a loner, because the
 * sitting partner is skipped by `nextActiveSeat`.
 */
export function playsPerTrick(sittingSeat: Seat | null): number {
	return activeSeatCount(sittingSeat);
}

/** Whether every active seat has played to this trick. */
export function isTrickComplete(plays: readonly Play[], sittingSeat: Seat | null): boolean {
	return plays.length >= playsPerTrick(sittingSeat);
}

/**
 * The play currently winning an in-progress trick, or `null` if nothing has been
 * laid. Used by the coach and by the "your partner is good" table line; identical
 * comparator, so it can never disagree with {@link trickWinner}.
 */
export function currentlyWinning(plays: readonly Play[], trump: Suit | null): Play | null {
	const led = ledSuitOf(plays, trump);
	if (led === null) return null;
	let best = plays[0]!;
	let bestValue = cardValue(best.card, led, trump);
	for (let i = 1; i < plays.length; i++) {
		const p = plays[i]!;
		const v = cardValue(p.card, led, trump);
		if (v > bestValue) {
			bestValue = v;
			best = p;
		}
	}
	return best;
}
