/**
 * `cards.test.ts` — the bower-aware ranking primitives.
 *
 * Covers `docs/02-GAME-RULES-ENGINE.md` § 9 cases 1–7 and invariants V15/V16.
 *
 * The ground truth in this file is written out **by hand** — the left bower of
 * each trump, the seven trump in order, the six plain ranks — rather than
 * re-derived from `COLOR_MATE`. A reference implementation that mirrors the
 * implementation cannot fail when the implementation is wrong.
 */

import { describe, expect, it } from 'vitest';
import {
	CARD_IDS,
	DECK,
	RANKS,
	SUITS,
	bowersOf,
	effectiveSuit,
	isLeftBower,
	plainRank,
	rankOf,
	sameColor,
	suitOf,
	trumpRank
} from '../cards';
import type { CardId, Rank, Suit } from '../types';

/* -------------------------------------------------------------------------- */
/* Hand-written ground truth                                                   */
/* -------------------------------------------------------------------------- */

/** The one card per trump whose printed suit is a lie. Written out, not derived. */
const LEFT_BOWER: Readonly<Record<Suit, CardId>> = {
	S: 'JC',
	C: 'JS',
	H: 'JD',
	D: 'JH'
};

/** The seven trump for each suit, strictly high → low. Right · Left · A K Q T 9. */
const TRUMP_ORDER: Readonly<Record<Suit, readonly CardId[]>> = {
	S: ['JS', 'JC', 'AS', 'KS', 'QS', 'TS', '9S'],
	H: ['JH', 'JD', 'AH', 'KH', 'QH', 'TH', '9H'],
	D: ['JD', 'JH', 'AD', 'KD', 'QD', 'TD', '9D'],
	C: ['JC', 'JS', 'AC', 'KC', 'QC', 'TC', '9C']
};

/** The rank each position of {@link TRUMP_ORDER} must report. */
const TRUMP_RANKS = [8, 7, 6, 5, 4, 3, 2] as const;

/** The colour-mate suit — the one that loses its jack. Written out, not derived. */
const MATE: Readonly<Record<Suit, Suit>> = { S: 'C', C: 'S', H: 'D', D: 'H' };

const ALL_SUITS: readonly Suit[] = ['S', 'H', 'D', 'C'];

/* -------------------------------------------------------------------------- */
/* Deck shape                                                                  */
/* -------------------------------------------------------------------------- */

describe('deck constants', () => {
	it('SUITS is exactly S H D C, in that order', () => {
		expect(SUITS).toEqual(['S', 'H', 'D', 'C']);
	});

	it('RANKS is exactly 9 T J Q K A, low to high', () => {
		expect(RANKS).toEqual(['9', 'T', 'J', 'Q', 'K', 'A']);
	});

	it('DECK is the 24 cards in canonical suit-major, rank-ascending order', () => {
		expect(DECK).toEqual([
			'9S',
			'TS',
			'JS',
			'QS',
			'KS',
			'AS',
			'9H',
			'TH',
			'JH',
			'QH',
			'KH',
			'AH',
			'9D',
			'TD',
			'JD',
			'QD',
			'KD',
			'AD',
			'9C',
			'TC',
			'JC',
			'QC',
			'KC',
			'AC'
		]);
	});

	it('DECK has 24 distinct cards — no 2–8, no joker', () => {
		expect(DECK).toHaveLength(24);
		expect(new Set(DECK).size).toBe(24);
	});

	it('CARD_IDS is the same frozen array as DECK, not a copy', () => {
		expect(CARD_IDS).toBe(DECK);
	});

	it('DECK and SUITS and RANKS are frozen', () => {
		expect(Object.isFrozen(DECK)).toBe(true);
		expect(Object.isFrozen(SUITS)).toBe(true);
		expect(Object.isFrozen(RANKS)).toBe(true);
	});
});

/* -------------------------------------------------------------------------- */
/* rankOf / suitOf                                                             */
/* -------------------------------------------------------------------------- */

describe('rankOf / suitOf', () => {
	it('splits every one of the 24 CardIds into its printed rank and suit', () => {
		for (const suit of ALL_SUITS) {
			for (const rank of RANKS) {
				const c = `${rank}${suit}` as CardId;
				expect(rankOf(c)).toBe(rank);
				expect(suitOf(c)).toBe(suit);
			}
		}
	});

	it('suitOf reports the PRINTED suit, even for the left bower', () => {
		// The whole reason suitOf must not be compared outside cards.ts.
		expect(suitOf('JD')).toBe('D');
		expect(rankOf('JD')).toBe('J');
	});

	it('round-trips: rankOf(c) + suitOf(c) === c for all 24', () => {
		for (const c of DECK) expect(`${rankOf(c)}${suitOf(c)}`).toBe(c);
	});
});

/* -------------------------------------------------------------------------- */
/* sameColor                                                                   */
/* -------------------------------------------------------------------------- */

describe('sameColor', () => {
	it('matches the full 4×4 truth table, and is reflexive', () => {
		const expected: Record<Suit, Record<Suit, boolean>> = {
			S: { S: true, H: false, D: false, C: true },
			H: { S: false, H: true, D: true, C: false },
			D: { S: false, H: true, D: true, C: false },
			C: { S: true, H: false, D: false, C: true }
		};
		for (const a of ALL_SUITS) {
			for (const b of ALL_SUITS) {
				expect([a, b, sameColor(a, b)]).toEqual([a, b, expected[a][b]]);
			}
		}
	});

	it('is symmetric', () => {
		for (const a of ALL_SUITS) for (const b of ALL_SUITS) expect(sameColor(a, b)).toBe(sameColor(b, a));
	});
});

/* -------------------------------------------------------------------------- */
/* effectiveSuit — § 9 cases 1, 2, 3 and V15                                   */
/* -------------------------------------------------------------------------- */

describe('effectiveSuit', () => {
	// § 9 case 1
	it('reports the left bower as trump', () => {
		expect(effectiveSuit('JD', 'H')).toBe('H');
		expect(effectiveSuit('JH', 'D')).toBe('D');
		expect(effectiveSuit('JC', 'S')).toBe('S');
		expect(effectiveSuit('JS', 'C')).toBe('C');
	});

	// § 9 case 1, second half
	it('reports the right bower as trump (identity, but load-bearing)', () => {
		expect(effectiveSuit('JH', 'H')).toBe('H');
		expect(effectiveSuit('JS', 'S')).toBe('S');
		expect(effectiveSuit('JD', 'D')).toBe('D');
		expect(effectiveSuit('JC', 'C')).toBe('C');
	});

	// § 9 case 2
	it('leaves a jack of the OTHER colour alone', () => {
		expect(effectiveSuit('JD', 'S')).toBe('D');
		expect(effectiveSuit('JD', 'C')).toBe('D');
		expect(effectiveSuit('JS', 'H')).toBe('S');
		expect(effectiveSuit('JS', 'D')).toBe('S');
	});

	it('never promotes a non-jack, however suggestive its suit', () => {
		expect(effectiveSuit('AD', 'H')).toBe('D');
		expect(effectiveSuit('9D', 'H')).toBe('D');
		expect(effectiveSuit('QC', 'S')).toBe('C');
	});

	// § 9 case 3 / V15
	it('is the identity when trump is null, for all 24 cards', () => {
		for (const c of DECK) expect(effectiveSuit(c, null)).toBe(suitOf(c));
	});

	// V15: identity except for exactly one card per trump suit
	it('differs from suitOf for exactly one card per trump — the left bower', () => {
		for (const trump of ALL_SUITS) {
			const moved = DECK.filter((c) => effectiveSuit(c, trump) !== suitOf(c));
			expect([trump, moved]).toEqual([trump, [LEFT_BOWER[trump]]]);
		}
	});

	// § 9 case 7, made exhaustive rather than random: 24 cards × 4 trumps = 96 pairs.
	it('matches the hand-written bower table over all 96 (card, trump) pairs', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				const want = c === LEFT_BOWER[trump] ? trump : suitOf(c);
				expect([c, trump, effectiveSuit(c, trump)]).toEqual([c, trump, want]);
			}
		}
	});

	it('only ever returns a suit the card could plausibly be: printed suit or trump', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				const e = effectiveSuit(c, trump);
				expect([c, trump, e === suitOf(c) || e === trump]).toEqual([c, trump, true]);
			}
		}
	});

	// The spec's headline example: a hand that is VOID in its printed suit.
	it('makes JD 9C TC KC AC void in diamonds when hearts are trump', () => {
		const hand: CardId[] = ['JD', '9C', 'TC', 'KC', 'AC'];
		const diamonds = hand.filter((c) => effectiveSuit(c, 'H') === 'D');
		expect(diamonds).toEqual([]);
		// …and holding exactly one heart, which is the diamond jack.
		const hearts = hand.filter((c) => effectiveSuit(c, 'H') === 'H');
		expect(hearts).toEqual(['JD']);
	});
});

/* -------------------------------------------------------------------------- */
/* Trump census — § 9 case 5 / V16                                             */
/* -------------------------------------------------------------------------- */

describe('trump census (V16)', () => {
	it('gives trump 7 cards, the same-colour suit 5, and each off-colour suit 6', () => {
		for (const trump of ALL_SUITS) {
			const counts: Record<Suit, number> = { S: 0, H: 0, D: 0, C: 0 };
			for (const c of DECK) counts[effectiveSuit(c, trump)]++;

			const mate = MATE[trump];
			expect([trump, 'trump', counts[trump]]).toEqual([trump, 'trump', 7]);
			expect([trump, 'mate', counts[mate]]).toEqual([trump, 'mate', 5]);
			for (const other of ALL_SUITS) {
				if (other === trump || other === mate) continue;
				expect([trump, other, counts[other]]).toEqual([trump, other, 6]);
			}
			expect(counts.S + counts.H + counts.D + counts.C).toBe(24);
		}
	});

	it('names the seven trump exactly, for every trump suit', () => {
		for (const trump of ALL_SUITS) {
			const trumps = DECK.filter((c) => effectiveSuit(c, trump) === trump);
			expect([trump, [...trumps].sort()]).toEqual([trump, [...TRUMP_ORDER[trump]].sort()]);
		}
	});
});

/* -------------------------------------------------------------------------- */
/* trumpRank — § 9 case 4                                                      */
/* -------------------------------------------------------------------------- */

describe('trumpRank', () => {
	// § 9 case 4, verbatim
	it('matches the spec sample values', () => {
		expect(trumpRank('JH', 'H')).toBe(8);
		expect(trumpRank('JD', 'H')).toBe(7);
		expect(trumpRank('AH', 'H')).toBe(6);
		expect(trumpRank('9H', 'H')).toBe(2);
		expect(trumpRank('AS', 'H')).toBe(0);
	});

	it('orders trump right > left > A > K > Q > T > 9, for every trump suit', () => {
		for (const trump of ALL_SUITS) {
			const got = TRUMP_ORDER[trump].map((c) => trumpRank(c, trump));
			expect([trump, got]).toEqual([trump, [...TRUMP_RANKS]]);
			// Strictly descending — no ties anywhere in the trump suit.
			for (let i = 1; i < got.length; i++) expect(got[i - 1]! > got[i]!).toBe(true);
		}
	});

	it('scores exactly 7 of the 24 cards non-zero, for every trump (V16)', () => {
		for (const trump of ALL_SUITS) {
			const nonZero = DECK.filter((c) => trumpRank(c, trump) > 0);
			expect([trump, nonZero.length]).toEqual([trump, 7]);
		}
	});

	it('is > 0 exactly when effectiveSuit === trump, over all 96 pairs', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				const isTrump = effectiveSuit(c, trump) === trump;
				expect([c, trump, trumpRank(c, trump) > 0]).toEqual([c, trump, isTrump]);
			}
		}
	});

	it('returns 0 for every off-suit card, including the jack of the wrong colour', () => {
		expect(trumpRank('JD', 'S')).toBe(0);
		expect(trumpRank('JS', 'H')).toBe(0);
		expect(trumpRank('AD', 'S')).toBe(0);
		expect(trumpRank('9C', 'D')).toBe(0);
	});

	it('the right bower is the single highest card at 8, the left the single second at 7', () => {
		for (const trump of ALL_SUITS) {
			const eights = DECK.filter((c) => trumpRank(c, trump) === 8);
			const sevens = DECK.filter((c) => trumpRank(c, trump) === 7);
			expect([trump, eights]).toEqual([trump, [`J${trump}` as CardId]]);
			expect([trump, sevens]).toEqual([trump, [LEFT_BOWER[trump]]]);
		}
	});
});

/* -------------------------------------------------------------------------- */
/* plainRank — § 9 case 6                                                      */
/* -------------------------------------------------------------------------- */

describe('plainRank', () => {
	// § 9 case 6, verbatim: in a plain suit the jack sits BELOW the queen.
	it('puts the jack below the queen', () => {
		expect(plainRank('JS')).toBe(3);
		expect(plainRank('QS')).toBe(4);
		expect(plainRank('JS') < plainRank('QS')).toBe(true);
	});

	it('matches the full A K Q J T 9 table', () => {
		const want: Record<Rank, number> = { A: 6, K: 5, Q: 4, J: 3, T: 2, '9': 1 };
		for (const suit of ALL_SUITS) {
			for (const rank of RANKS) {
				const c = `${rank}${suit}` as CardId;
				expect([c, plainRank(c)]).toEqual([c, want[rank]]);
			}
		}
	});

	it('depends on rank alone — the same rank scores the same in all four suits', () => {
		for (const rank of RANKS) {
			const vals = ALL_SUITS.map((s) => plainRank(`${rank}${s}` as CardId));
			expect(new Set(vals).size).toBe(1);
		}
	});

	it('is strictly ascending over RANKS (which is ordered low to high)', () => {
		const vals = RANKS.map((r) => plainRank(`${r}S` as CardId));
		expect(vals).toEqual([1, 2, 3, 4, 5, 6]);
	});
});

/* -------------------------------------------------------------------------- */
/* bowersOf / isLeftBower                                                      */
/* -------------------------------------------------------------------------- */

describe('bowersOf', () => {
	it('returns [right, left] for every trump', () => {
		expect(bowersOf('H')).toEqual(['JH', 'JD']);
		expect(bowersOf('D')).toEqual(['JD', 'JH']);
		expect(bowersOf('S')).toEqual(['JS', 'JC']);
		expect(bowersOf('C')).toEqual(['JC', 'JS']);
	});

	it('agrees with trumpRank: [8, 7]', () => {
		for (const trump of ALL_SUITS) {
			const [right, left] = bowersOf(trump);
			expect([trump, trumpRank(right, trump), trumpRank(left, trump)]).toEqual([trump, 8, 7]);
		}
	});
});

describe('isLeftBower', () => {
	it('is true for exactly one card per trump, over all 96 pairs', () => {
		for (const trump of ALL_SUITS) {
			for (const c of DECK) {
				expect([c, trump, isLeftBower(c, trump)]).toEqual([c, trump, c === LEFT_BOWER[trump]]);
			}
		}
	});

	it('is false for the right bower — the right bower is not the left one', () => {
		for (const trump of ALL_SUITS) expect(isLeftBower(`J${trump}` as CardId, trump)).toBe(false);
	});

	it('is false for every card when trump is null', () => {
		for (const c of DECK) expect(isLeftBower(c, null)).toBe(false);
	});
});
