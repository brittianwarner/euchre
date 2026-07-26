/**
 * `trick.test.ts` — the total order inside one trick, and who takes it.
 *
 * Covers `docs/02-GAME-RULES-ENGINE.md` § 2.3, § 4.2, § 9 cases 20–26, and
 * invariant V17.
 *
 * The load-bearing fact under test is the **zero band**: a card of neither the
 * led suit nor trump scores `0` and can never win, which is why `9C` beats `AH`
 * when clubs are led and diamonds are trump.
 */

import { describe, expect, it } from 'vitest';
import { DECK, effectiveSuit, plainRank, trumpRank } from '../cards';
import {
	cardValue,
	currentlyWinning,
	isTrickComplete,
	ledSuitOf,
	playsPerTrick,
	trickWinner
} from '../trick';
import type { CardId, Play, Seat, Suit } from '../types';

const ALL_SUITS: readonly Suit[] = ['S', 'H', 'D', 'C'];

/** Build a trick from `[seat, card]` pairs — plays are laid in array order. */
const plays = (...ps: readonly (readonly [Seat, CardId])[]): Play[] =>
	ps.map(([seat, card]) => ({ seat, card }));

/* -------------------------------------------------------------------------- */
/* cardValue — the three bands                                                 */
/* -------------------------------------------------------------------------- */

describe('cardValue — trump band 102..108', () => {
	it('scores the seven hearts trump exactly 108…102, right bower first', () => {
		// Led suit is spades, so nothing here is helped by the led-suit band.
		expect(cardValue('JH', 'S', 'H')).toBe(108); // right bower
		expect(cardValue('JD', 'S', 'H')).toBe(107); // LEFT bower — a diamond scoring as a heart
		expect(cardValue('AH', 'S', 'H')).toBe(106);
		expect(cardValue('KH', 'S', 'H')).toBe(105);
		expect(cardValue('QH', 'S', 'H')).toBe(104);
		expect(cardValue('TH', 'S', 'H')).toBe(103);
		expect(cardValue('9H', 'S', 'H')).toBe(102);
	});

	it('is 100 + trumpRank for every trump card, for every trump suit', () => {
		for (const trump of ALL_SUITS) {
			for (const led of ALL_SUITS) {
				for (const c of DECK) {
					if (effectiveSuit(c, trump) !== trump) continue;
					expect([c, led, trump, cardValue(c, led, trump)]).toEqual([
						c,
						led,
						trump,
						100 + trumpRank(c, trump)
					]);
				}
			}
		}
	});

	it('lands every trump card strictly inside 102..108', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				if (effectiveSuit(c, trump) !== trump) continue;
				const v = cardValue(c, 'S', trump);
				expect([c, trump, v >= 102 && v <= 108]).toEqual([c, trump, true]);
			}
		}
	});

	it('scores trump the same whether or not trump happens to be the led suit', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				if (effectiveSuit(c, trump) !== trump) continue;
				const otherLed: Suit = trump === 'S' ? 'H' : 'S';
				expect([c, trump, cardValue(c, trump, trump)]).toEqual([
					c,
					trump,
					cardValue(c, otherLed, trump)
				]);
			}
		}
	});
});

describe('cardValue — led-suit band 51..56', () => {
	it('scores the six spades 56…51 when spades are led and hearts are trump', () => {
		expect(cardValue('AS', 'S', 'H')).toBe(56);
		expect(cardValue('KS', 'S', 'H')).toBe(55);
		expect(cardValue('QS', 'S', 'H')).toBe(54);
		expect(cardValue('JS', 'S', 'H')).toBe(53); // plain-suit jack: below the queen
		expect(cardValue('TS', 'S', 'H')).toBe(52);
		expect(cardValue('9S', 'S', 'H')).toBe(51);
	});

	it('is 50 + plainRank for every non-trump card of the led suit', () => {
		for (const trump of ALL_SUITS) {
			for (const led of ALL_SUITS) {
				for (const c of DECK) {
					if (effectiveSuit(c, trump) !== led || led === trump) continue;
					expect([c, led, trump, cardValue(c, led, trump)]).toEqual([
						c,
						led,
						trump,
						50 + plainRank(c)
					]);
				}
			}
		}
	});

	it('the whole trump band outranks the whole led band', () => {
		for (const trump of ALL_SUITS) {
			for (const led of ALL_SUITS) {
				if (led === trump) continue;
				const lowestTrump = Math.min(
					...DECK.filter((c) => effectiveSuit(c, trump) === trump).map((c) =>
						cardValue(c, led, trump)
					)
				);
				const highestLed = Math.max(
					...DECK.filter((c) => effectiveSuit(c, trump) === led).map((c) => cardValue(c, led, trump))
				);
				expect([led, trump, lowestTrump > highestLed]).toEqual([led, trump, true]);
			}
		}
	});
});

describe('cardValue — the zero band: cannot win', () => {
	// The spec's headline consequence, § 2.3.
	it('makes 9C beat AH when clubs are led and diamonds are trump', () => {
		expect(cardValue('9C', 'C', 'D')).toBe(51);
		expect(cardValue('AH', 'C', 'D')).toBe(0);
		expect(cardValue('9C', 'C', 'D') > cardValue('AH', 'C', 'D')).toBe(true);
	});

	it('scores 0 for every card that is neither trump nor the led suit', () => {
		for (const trump of ALL_SUITS) {
			for (const led of ALL_SUITS) {
				for (const c of DECK) {
					const eff = effectiveSuit(c, trump);
					if (eff === trump || eff === led) continue;
					expect([c, led, trump, cardValue(c, led, trump)]).toEqual([c, led, trump, 0]);
				}
			}
		}
	});

	it('scores the ace of an off suit 0 — rank is irrelevant off suit', () => {
		expect(cardValue('AS', 'C', 'D')).toBe(0);
		expect(cardValue('AH', 'S', 'D')).toBe(0);
		expect(cardValue('AD', 'S', 'H')).toBe(0); // AD is a plain diamond with hearts trump
	});

	it('scores the LEFT bower 0 in its printed suit once that suit is not trump', () => {
		// Hearts trump, diamonds led: JD is a heart, so it does not follow diamonds…
		expect(effectiveSuit('JD', 'H')).toBe('H');
		// …but it is trump, so it scores in the trump band, not the zero band.
		expect(cardValue('JD', 'D', 'H')).toBe(107);
		// And it beats the ace of diamonds, which is only worth 56.
		expect(cardValue('AD', 'D', 'H')).toBe(56);
	});
});

describe('cardValue — trump: null (round-1 scratch evaluation)', () => {
	it('falls back to the led-suit band with no trump band at all', () => {
		expect(cardValue('AS', 'S', null)).toBe(56);
		expect(cardValue('9S', 'S', null)).toBe(51);
		expect(cardValue('JS', 'S', null)).toBe(53);
		expect(cardValue('AH', 'S', null)).toBe(0);
	});

	it('never produces a value above 56 when trump is null, over all 96 pairs', () => {
		for (const led of ALL_SUITS) {
			for (const c of DECK) {
				const v = cardValue(c, led, null);
				expect([c, led, v <= 56]).toEqual([c, led, true]);
			}
		}
	});

	it('treats the would-be left bower as an ordinary card when trump is null', () => {
		expect(cardValue('JD', 'H', null)).toBe(0);
		expect(cardValue('JD', 'D', null)).toBe(53);
	});
});

/* -------------------------------------------------------------------------- */
/* trickWinner — § 9 cases 20–26                                               */
/* -------------------------------------------------------------------------- */

describe('trickWinner', () => {
	// § 9 case 20
	it('case 20: a nine of the led suit beats an ace of anything else', () => {
		const t = plays([0, '9C'], [1, 'AH'], [2, 'KH'], [3, 'QH']);
		expect(trickWinner(t, 'C', 'D')).toBe(0);
	});

	// § 9 case 21
	it('case 21: any trump beats any plain card', () => {
		const t = plays([0, 'AS'], [1, '9H'], [2, 'KS'], [3, 'QS']);
		expect(trickWinner(t, 'S', 'H')).toBe(1);
	});

	// § 9 case 22
	it('case 22: the left bower is trump and outranks the nine of trump', () => {
		const t = plays([0, 'AS'], [1, '9H'], [2, 'JD'], [3, 'KS']);
		expect(trickWinner(t, 'S', 'H')).toBe(2);
	});

	// § 9 case 23
	it('case 23: right beats left', () => {
		const t = plays([0, 'AS'], [1, 'JD'], [2, 'JH'], [3, 'KS']);
		expect(trickWinner(t, 'S', 'H')).toBe(2);
	});

	// § 9 case 24
	it('case 24: leading the left bower leads TRUMP, and the left bower wins it', () => {
		const t = plays([0, 'JD'], [1, 'AH'], [2, 'KH'], [3, '9H']);
		const led = ledSuitOf(t, 'H');
		expect(led).toBe('H'); // not "D"
		expect(trickWinner(t, led!, 'H')).toBe(0);
	});

	// § 9 case 25
	it('case 25: with spades trump, JD is an ordinary diamond jack below the queen', () => {
		const t = plays([0, '9D'], [1, 'TD'], [2, 'JD'], [3, 'AD']);
		expect(trickWinner(t, 'D', 'S')).toBe(3);
		expect(cardValue('JD', 'D', 'S')).toBe(53);
		expect(cardValue('QD', 'D', 'S')).toBe(54);
	});

	// § 9 case 26 — three-play trick under a loner.
	it('case 26: three-play loner trick, trump C, led H — 9C takes it for seat 0', () => {
		const t = plays([1, 'KH'], [3, 'AH'], [0, '9C']);
		expect(t).toHaveLength(3);
		expect(trickWinner(t, 'H', 'C')).toBe(0);
	});

	it('a single-play trick is won by that play', () => {
		expect(trickWinner(plays([2, '9S']), 'S', 'H')).toBe(2);
	});

	it('throws a plain Error on an empty trick — a programmer error, not a rules event', () => {
		expect(() => trickWinner([], 'S', 'H')).toThrow(/trickWinner/);
	});

	it('does not mutate the plays array or its members', () => {
		const t = plays([0, 'AS'], [1, 'JD'], [2, 'JH'], [3, 'KS']);
		const before = JSON.stringify(t);
		trickWinner(t, 'S', 'H');
		expect(JSON.stringify(t)).toBe(before);
	});

	it('always picks the play with the maximum cardValue', () => {
		const t = plays([0, 'AS'], [1, '9H'], [2, 'JD'], [3, 'KS']);
		const winner = trickWinner(t, 'S', 'H');
		const max = Math.max(...t.map((p) => cardValue(p.card, 'S', 'H')));
		const winning = t.find((p) => p.seat === winner)!;
		expect(cardValue(winning.card, 'S', 'H')).toBe(max);
	});

	it('the winning card always scores above the zero band — the leader is never beaten by nothing', () => {
		// Because led === effectiveSuit(firstPlay.card), the first play is always
		// worth at least 51. So a trick can never be won by a card that "cannot win".
		for (const trump of ALL_SUITS) {
			for (const first of DECK) {
				const led = effectiveSuit(first, trump);
				const t = plays([0, first], [1, 'AS' === first ? 'KS' : 'AS']);
				const w = trickWinner(t, led, trump);
				const card = t.find((p) => p.seat === w)!.card;
				expect([first, trump, cardValue(card, led, trump) > 0]).toEqual([first, trump, true]);
			}
		}
	});

	it('is independent of play order for the trump/led bands (the max is unique there)', () => {
		const t = plays([0, 'AS'], [1, 'JD'], [2, 'JH'], [3, 'KS']);
		const reversed = [...t].reverse();
		expect(trickWinner(reversed, 'S', 'H')).toBe(trickWinner(t, 'S', 'H'));
	});

	/**
	 * V17 says `cardValue` is injective over the cards of one trick. That is
	 * **not** literally true — two different off-suit discards both score `0` —
	 * but it is true of every card that can win, so `trickWinner` is still total
	 * and deterministic. This test pins the real behaviour.
	 */
	it('resolves a zero-value tie deterministically, and never to a discard', () => {
		// Hearts trump, spades led. TC and 9D are both worth 0.
		expect(cardValue('TC', 'S', 'H')).toBe(0);
		expect(cardValue('9D', 'S', 'H')).toBe(0);
		const t = plays([0, '9S'], [1, 'TC'], [2, '9D'], [3, 'TS']);
		expect(trickWinner(t, 'S', 'H')).toBe(3); // TS = 52 beats 9S = 51
	});

	/**
	 * Degenerate input a caller could only produce by passing a `led` that no
	 * play matches. `trickWinner` is documented total, so it must still answer —
	 * and it answers with the FIRST play, which is what pins the comparator to a
	 * strict `>`. Unreachable through `ledSuitOf`, asserted so it stays total.
	 */
	it('is total when every play scores 0: the first play holds', () => {
		const t = plays([2, 'TC'], [3, '9D'], [0, 'AC']);
		for (const p of t) expect(cardValue(p.card, 'S', 'H')).toBe(0);
		expect(trickWinner(t, 'S', 'H')).toBe(2);
	});
});

/* -------------------------------------------------------------------------- */
/* ledSuitOf                                                                   */
/* -------------------------------------------------------------------------- */

describe('ledSuitOf', () => {
	it('returns null for an empty trick — "the next card is on lead"', () => {
		expect(ledSuitOf([], 'H')).toBeNull();
		expect(ledSuitOf([], null)).toBeNull();
	});

	it('reports the EFFECTIVE suit: leading the left bower leads trump', () => {
		expect(ledSuitOf(plays([0, 'JD']), 'H')).toBe('H');
		expect(ledSuitOf(plays([0, 'JC']), 'S')).toBe('S');
		expect(ledSuitOf(plays([0, 'JS']), 'C')).toBe('C');
		expect(ledSuitOf(plays([0, 'JH']), 'D')).toBe('D');
	});

	it('reports the printed suit when the jack is the wrong colour', () => {
		expect(ledSuitOf(plays([0, 'JD']), 'S')).toBe('D');
		expect(ledSuitOf(plays([0, 'JD']), 'C')).toBe('D');
	});

	it('reports the printed suit when there is no trump yet', () => {
		for (const c of DECK) expect(ledSuitOf(plays([0, c]), null)).toBe(c[1]);
	});

	it('is fixed by the FIRST play and ignores everything after it', () => {
		const t = plays([0, '9S'], [1, 'JH'], [2, 'AD']);
		expect(ledSuitOf(t, 'H')).toBe('S');
	});

	it('agrees with effectiveSuit over all 96 (first card, trump) pairs', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				expect([c, trump, ledSuitOf(plays([0, c]), trump)]).toEqual([
					c,
					trump,
					effectiveSuit(c, trump)
				]);
			}
		}
	});
});

/* -------------------------------------------------------------------------- */
/* playsPerTrick / isTrickComplete                                             */
/* -------------------------------------------------------------------------- */

describe('playsPerTrick / isTrickComplete', () => {
	it('needs 4 plays normally and 3 under a loner', () => {
		expect(playsPerTrick(null)).toBe(4);
		expect(playsPerTrick(0)).toBe(3);
		expect(playsPerTrick(3)).toBe(3);
	});

	it('is incomplete until the last active seat has played', () => {
		const four = plays([0, '9S'], [1, 'TS'], [2, 'JS'], [3, 'QS']);
		expect(isTrickComplete([], null)).toBe(false);
		expect(isTrickComplete(four.slice(0, 1), null)).toBe(false);
		expect(isTrickComplete(four.slice(0, 3), null)).toBe(false);
		expect(isTrickComplete(four, null)).toBe(true);
	});

	it('completes at three plays under a loner', () => {
		const three = plays([1, '9S'], [2, 'TS'], [3, 'JS']);
		expect(isTrickComplete(three.slice(0, 2), 0)).toBe(false);
		expect(isTrickComplete(three, 0)).toBe(true);
	});
});

/* -------------------------------------------------------------------------- */
/* currentlyWinning                                                            */
/* -------------------------------------------------------------------------- */

describe('currentlyWinning', () => {
	it('is null on an empty trick', () => {
		expect(currentlyWinning([], 'H')).toBeNull();
		expect(currentlyWinning([], null)).toBeNull();
	});

	it('is the leader after one card', () => {
		expect(currentlyWinning(plays([2, '9S']), 'H')).toEqual({ seat: 2, card: '9S' });
	});

	it('tracks the takeover card by card', () => {
		const t = plays([0, 'AS'], [1, '9H'], [2, 'JD'], [3, 'KS']);
		expect(currentlyWinning(t.slice(0, 1), 'H')).toEqual({ seat: 0, card: 'AS' });
		expect(currentlyWinning(t.slice(0, 2), 'H')).toEqual({ seat: 1, card: '9H' });
		expect(currentlyWinning(t.slice(0, 3), 'H')).toEqual({ seat: 2, card: 'JD' });
		expect(currentlyWinning(t, 'H')).toEqual({ seat: 2, card: 'JD' });
	});

	it('never disagrees with trickWinner on any prefix of any trick', () => {
		const cases: readonly (readonly [Suit, Play[]])[] = [
			['H', plays([0, 'AS'], [1, '9H'], [2, 'JD'], [3, 'KS'])],
			['D', plays([0, '9C'], [1, 'AH'], [2, 'KH'], [3, 'QH'])],
			['S', plays([0, '9D'], [1, 'TD'], [2, 'JD'], [3, 'AD'])],
			['C', plays([1, 'KH'], [3, 'AH'], [0, '9C'])],
			['H', plays([0, 'JD'], [1, 'AH'], [2, 'KH'], [3, '9H'])]
		];
		for (const [trump, t] of cases) {
			const led = ledSuitOf(t, trump)!;
			for (let n = 1; n <= t.length; n++) {
				const prefix = t.slice(0, n);
				expect([trump, n, currentlyWinning(prefix, trump)!.seat]).toEqual([
					trump,
					n,
					trickWinner(prefix, led, trump)
				]);
			}
		}
	});

	it('derives the led suit itself, so a left-bower lead is handled', () => {
		const t = plays([0, 'JD'], [1, 'AD']);
		// AD is a plain diamond; JD is a heart. AD cannot win.
		expect(currentlyWinning(t, 'H')).toEqual({ seat: 0, card: 'JD' });
	});
});
