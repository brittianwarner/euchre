/**
 * `cards.ts` — card identity, colour, and the bower-aware ranking primitives.
 *
 * This is the **only** file permitted to compare the result of `suitOf()`.
 * Voidness, follow-suit legality, led-suit determination and the trick comparator
 * all route through `effectiveSuit()` instead; a `suitOf()` comparison anywhere
 * else is a bug, because it silently gets the left bower wrong.
 */

import type { CardId, Rank, Suit } from './types';

export type { CardId, Rank, Suit } from './types';

/** Canonical suit order. Iteration order is load-bearing: it fixes `DECK`. */
export const SUITS: readonly Suit[] = Object.freeze(['S', 'H', 'D', 'C'] as const);

/** Canonical rank order, low to high in a plain suit. */
export const RANKS: readonly Rank[] = Object.freeze(['9', 'T', 'J', 'Q', 'K', 'A'] as const);

/**
 * The 24-card euchre deck in canonical, unshuffled order (suit-major, rank
 * ascending). Deterministic shuffling starts from exactly this permutation, so
 * changing the order changes every seeded game — treat it as frozen.
 */
export const DECK: readonly CardId[] = Object.freeze([
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
] as const);

/**
 * Alias of {@link DECK}, under the name used by the implementation plan's file
 * manifest. Same frozen array, not a copy.
 */
export const CARD_IDS: readonly CardId[] = DECK;

/**
 * The rank half of a `CardId`. Total: every `CardId` starts with a `Rank`.
 * @example rankOf("JD") === "J"
 */
export const rankOf = (c: CardId): Rank => c[0] as Rank;

/**
 * The **printed** suit of a card. Do not compare this outside `cards.ts` — with
 * trump set, the left bower's printed suit is a lie. Use {@link effectiveSuit}.
 * @example suitOf("JD") === "D", even when hearts are trump
 */
export const suitOf = (c: CardId): Suit => c[1] as Suit;

/** The other suit of the same colour. `♠↔♣` black, `♥↔♦` red. */
const COLOR_MATE: Readonly<Record<Suit, Suit>> = Object.freeze({
	S: 'C',
	C: 'S',
	H: 'D',
	D: 'H'
});

/**
 * Whether two suits share a colour. **Reflexive**: `sameColor(s, s)` is `true`.
 * This relationship is what defines the left bower, so it is encoded exactly
 * once — here.
 * @example sameColor("H", "D") === true; sameColor("H", "S") === false
 */
export const sameColor = (a: Suit, b: Suit): boolean => a === b || COLOR_MATE[a] === b;

/**
 * The suit a card **behaves as**, which is the only suit that matters for
 * voidness, following suit, and led-suit determination.
 *
 * Identity for every card except one per trump suit: the jack of trump's colour
 * mate — the *left bower* — reports as trump. `effectiveSuit(c, null)` is always
 * `suitOf(c)` (V15), so this is safe to call before trump exists.
 *
 * @example effectiveSuit("JD", "H") === "H"  // left bower: a diamond that is a heart
 * @example effectiveSuit("JD", "S") === "D"  // wrong colour: an ordinary diamond
 */
export function effectiveSuit(card: CardId, trump: Suit | null): Suit {
	const s = suitOf(card);
	if (trump !== null && rankOf(card) === 'J' && s === COLOR_MATE[trump]) return trump;
	return s;
}

/**
 * Rank within the trump suit, or `0` for any card that is not trump.
 * Right bower `8` · left bower `7` · A `6` · K `5` · Q `4` · T `3` · 9 `2`.
 *
 * Exactly seven of the 24 cards score non-zero for any given trump (V16), and
 * `trumpRank(c, t) > 0` iff `effectiveSuit(c, t) === t`.
 *
 * @example trumpRank("JH", "H") === 8; trumpRank("JD", "H") === 7; trumpRank("AS", "H") === 0
 */
export function trumpRank(card: CardId, trump: Suit): number {
	if (effectiveSuit(card, trump) !== trump) return 0;
	const r = rankOf(card);
	if (r === 'J') return suitOf(card) === trump ? 8 : 7;
	return { A: 6, K: 5, Q: 4, T: 3, '9': 2, J: 0 }[r];
}

/**
 * Rank within a plain suit: A `6` · K `5` · Q `4` · J `3` · T `2` · 9 `1`.
 *
 * Meaningful **only** for a card that is not trump — in a plain suit the jack
 * sits below the queen, which is why this is a different table from
 * {@link trumpRank}. Depends on rank alone, so it needs no trump argument.
 *
 * @example plainRank("JS") === 3; plainRank("QS") === 4
 */
export function plainRank(card: CardId): number {
	return { A: 6, K: 5, Q: 4, J: 3, T: 2, '9': 1 }[rankOf(card)];
}

/**
 * The two jacks that are trump under `trump`, as `[right, left]`. Convenience for
 * UI copy ("`♥` TRUMP · `J♦` is the left bower") and for the AI's trump census;
 * never used for legality, which goes through {@link effectiveSuit}.
 */
export function bowersOf(trump: Suit): readonly [CardId, CardId] {
	return [`J${trump}`, `J${COLOR_MATE[trump]}`];
}

/**
 * Whether `card` is the left bower under `trump` — a jack whose printed suit is
 * trump's colour mate. `false` when `trump` is `null`.
 */
export function isLeftBower(card: CardId, trump: Suit | null): boolean {
	if (trump === null) return false;
	return rankOf(card) === 'J' && suitOf(card) === COLOR_MATE[trump];
}
