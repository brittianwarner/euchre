/**
 * `deal.ts` — seeded determinism and the physical deal.
 *
 * Every hand is a pure function of `(seed, handNo)`. `Math.random`, `Date.now`
 * and every other ambient input are **forbidden** in this module and in every
 * module that feeds it (V20): a replay is `{ seed, moves[] }` and nothing else,
 * so a shuffle must be reproducible from the seed alone, on any machine, after
 * any crash.
 *
 * The PRNG closure is deliberately *not* part of `GameState`. The seed is
 * persisted and the closure is rebuilt from it on every wake — that split is the
 * reason a restart reproduces the same shuffle.
 */

import { DECK } from './cards';
import { nextSeat } from './seats';
import type { CardId, DealPacket, DealtPacket, Seat } from './types';

/* -------------------------------------------------------------------------- */
/* PRNG                                                                        */
/* -------------------------------------------------------------------------- */

/**
 * FNV-1a over a string, as an unsigned 32-bit integer. Deterministic, fast, and
 * stable across engines — the only string hash used anywhere in the rules engine.
 */
export function fnv1a(s: string): number {
	let h = 0x811c9dc5;
	for (let i = 0; i < s.length; i++) {
		h ^= s.charCodeAt(i);
		h = Math.imul(h, 0x01000193) >>> 0;
	}
	return h >>> 0;
}

/**
 * The 32-bit seed for one hand: FNV-1a over `` `${seed}#${handNo}` ``. Two hands
 * of the same match get unrelated shuffles; two matches with the same seed get
 * identical ones.
 */
export function hashSeed(seed: string, handNo: number): number {
	return fnv1a(`${seed}#${handNo}`);
}

/**
 * mulberry32 — a small, fast, well-distributed 32-bit PRNG. Returns a stateful
 * closure yielding numbers in `[0, 1)`. The closure is the *only* mutable thing
 * in this module and is never persisted.
 */
export function mulberry32(a: number): () => number {
	let t = a >>> 0;
	return () => {
		t = (t + 0x6d2b79f5) >>> 0;
		let x = t;
		x = Math.imul(x ^ (x >>> 15), x | 1);
		x ^= x + Math.imul(x ^ (x >>> 7), x | 61);
		return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
	};
}

/**
 * The RNG stream for one hand, positioned at zero. Draws are consumed in a fixed
 * order (shuffle first, then anything else), so callers must not interleave.
 */
export function handRng(seed: string, handNo: number): () => number {
	return mulberry32(hashSeed(seed, handNo));
}

/**
 * Fisher–Yates, driven by `rnd`. Returns a new array; never mutates `items`.
 * Consumes exactly `items.length - 1` draws.
 */
export function shuffle<T>(items: readonly T[], rnd: () => number): T[] {
	const a = items.slice();
	for (let i = a.length - 1; i > 0; i--) {
		const j = Math.floor(rnd() * (i + 1));
		const t = a[i]!;
		a[i] = a[j]!;
		a[j] = t;
	}
	return a;
}

/**
 * A single deterministic cut: rotate the deck at a seeded index that is never
 * within 4 of either end, so it is always a real cut. Returns a new array.
 * Consumes exactly one draw from `rnd`.
 */
export function cutAt(deck: readonly CardId[], rnd: () => number): CardId[] {
	const i = 4 + Math.floor(rnd() * (deck.length - 8));
	return [...deck.slice(i), ...deck.slice(0, i)];
}

/**
 * The restart-safe form of {@link cutAt}: the cut index is derived from
 * `(seed, handNo)` alone rather than from the position of a live RNG stream.
 *
 * Prefer this in the reducer. The cut is answered in a *separate transition* from
 * the shuffle, and a crash in between rebuilds the RNG at position zero — so a
 * stream-positional cut would not reproduce. Returns a new array.
 */
export function cutForHand(deck: readonly CardId[], seed: string, handNo: number): CardId[] {
	const span = deck.length - 8;
	const i = 4 + (fnv1a(`${seed}#${handNo}#cut`) % span);
	return [...deck.slice(i), ...deck.slice(0, i)];
}

/**
 * The dealer for hand 0, drawn once at game creation. Consumes one draw.
 * There is no draw-for-deal ceremony; after this the deal moves one seat left
 * after every scored hand and every throw-in.
 */
export function pickFirstDealer(rnd: () => number): Seat {
	return Math.floor(rnd() * 4) as Seat;
}

/* -------------------------------------------------------------------------- */
/* The deck                                                                    */
/* -------------------------------------------------------------------------- */

/** A fresh, mutable copy of the canonical 24-card deck. `DECK` itself is frozen. */
export function newDeck(): CardId[] {
	return DECK.slice();
}

/**
 * The persisted 24-card permutation for one hand, before the cut. Computed and
 * stored **before** the `cutting` phase opens, so a crash cannot reshuffle a hand
 * a player has already partly seen.
 */
export function shuffleForHand(seed: string, handNo: number): CardId[] {
	return shuffle(DECK, handRng(seed, handNo));
}

/**
 * Asserts that `deckOrder` is a genuine permutation of the 24-card deck — right
 * length, no duplicates, nothing foreign. Throws a plain `Error` on violation;
 * this is a programmer error (V10), never a rules event.
 */
export function assertDeckOrder(deckOrder: readonly CardId[]): void {
	if (deckOrder.length !== DECK.length) {
		throw new Error(`deckOrder must hold ${DECK.length} cards, got ${deckOrder.length}`);
	}
	const seen = new Set<CardId>(deckOrder);
	if (seen.size !== DECK.length) throw new Error('deckOrder contains duplicate cards');
	for (const c of DECK) {
		if (!seen.has(c)) throw new Error(`deckOrder is missing ${c}`);
	}
}

/* -------------------------------------------------------------------------- */
/* The deal                                                                    */
/* -------------------------------------------------------------------------- */

/**
 * Which alternation this hand uses, derived deterministically from
 * `hashSeed(seed, handNo) & 1` — no extra state, fully replayable.
 * `true` means the first packet is three cards (3-2-3-2, then 2-3-2-3).
 */
export function startsWithThree(seed: string, handNo: number): boolean {
	return (hashSeed(seed, handNo) & 1) === 1;
}

/**
 * The canonical 3-2 / 2-3 alternation, in dealing order: two passes clockwise
 * from eldest, each seat receiving the complement of its first packet on the
 * second pass. Always eight packets summing to 5 per seat and 20 total.
 *
 * Returns a fresh array; the client animates the deal from exactly this order.
 */
export function dealPlan(dealerSeat: Seat, threeFirst: boolean): DealPacket[] {
	const order: Seat[] = [];
	let s = nextSeat(dealerSeat);
	for (let i = 0; i < 4; i++) {
		order.push(s);
		s = nextSeat(s);
	}
	const first = order.map((seat, i) => ({ seat, count: (i % 2 === 0) === threeFirst ? 3 : 2 }));
	const second = first.map((p) => ({ seat: p.seat, count: 5 - p.count }));
	return [...first, ...second];
}

/**
 * Deal `deckOrder` out according to `plan`.
 *
 * Cards are taken off the top in packet order, so `hands[seat]` is in deal order,
 * not sorted. The four undealt cards are the kitty: `deckOrder[20..23]`.
 * `kitty[0]` is turned face up as the up-card; `kitty[1..3]` are buried and are
 * never revealed to any client, ever, including in post-game replay (V12).
 *
 * Pure: `deckOrder` and `plan` are not mutated, and every returned array is new.
 * Throws if the plan does not consume exactly 20 cards from a 24-card deck.
 */
export function dealFrom(
	deckOrder: readonly CardId[],
	plan: readonly DealPacket[]
): {
	hands: Record<Seat, CardId[]>;
	kitty: [CardId, CardId, CardId, CardId];
} {
	assertDeckOrder(deckOrder);
	const dealt = plan.reduce((n, p) => n + p.count, 0);
	if (dealt !== 20) throw new Error(`deal plan must account for 20 cards, got ${dealt}`);

	const hands: Record<Seat, CardId[]> = { 0: [], 1: [], 2: [], 3: [] };
	let i = 0;
	for (const p of plan) for (let k = 0; k < p.count; k++) hands[p.seat].push(deckOrder[i++]!);
	const kitty: [CardId, CardId, CardId, CardId] = [
		deckOrder[20]!,
		deckOrder[21]!,
		deckOrder[22]!,
		deckOrder[23]!
	];
	return { hands, kitty };
}

/**
 * The unredacted `dealt` step payload: the same packets as `plan`, each carrying
 * the cards it delivered.
 *
 * This is the **server-side** form. `projectSteps()` must blank `cards` to `null`
 * on every packet not addressed to the receiving seat before it leaves the
 * server, or the choreography channel leaks the deal (V12).
 */
export function dealtPackets(
	deckOrder: readonly CardId[],
	plan: readonly DealPacket[]
): DealtPacket[] {
	let i = 0;
	return plan.map((p) => {
		const cards = deckOrder.slice(i, i + p.count);
		i += p.count;
		return { seat: p.seat, count: p.count, cards };
	});
}

/** Everything one deal produces, ready to be written into a fresh `HandState`. */
export interface DealResult {
	/** The 24-card permutation actually dealt from (post-cut, if a cut was taken). */
	readonly deckOrder: readonly CardId[];
	/** The eight packets, in dealing order — replayed verbatim by the animation. */
	readonly plan: readonly DealPacket[];
	readonly hands: Record<Seat, CardId[]>;
	readonly kitty: [CardId, CardId, CardId, CardId];
	/** `kitty[0]`, turned face up. Stays publicly identified for the whole hand. */
	readonly upCard: CardId;
}

/**
 * Deal one hand from an already-persisted, already-cut `deckOrder`.
 *
 * The alternation is derived from `(seed, handNo)`, so this is a pure function of
 * its arguments and reproduces bit-for-bit on replay. Does not shuffle and does
 * not cut — both happen earlier and are persisted first, by design.
 */
export function dealHand(
	deckOrder: readonly CardId[],
	dealerSeat: Seat,
	seed: string,
	handNo: number
): DealResult {
	const plan = dealPlan(dealerSeat, startsWithThree(seed, handNo));
	const { hands, kitty } = dealFrom(deckOrder, plan);
	return { deckOrder, plan, hands, kitty, upCard: kitty[0] };
}
