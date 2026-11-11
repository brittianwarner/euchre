/**
 * `legal.test.ts` — follow-suit legality and the move-level API.
 *
 * Covers `docs/02-GAME-RULES-ENGINE.md` § 4.1, § 6.3, § 9 cases 8–19, and
 * invariants V3, V7, V8, V19, V21.
 *
 * The `GameState` fixtures here are built by hand rather than driven through
 * `reduce.ts`. That is deliberate: these are unit tests of `legal.ts`, and a
 * fixture built by the reducer would make a `legal.ts` regression show up as a
 * `reduce.ts` failure.
 *
 * The `apply()`-side halves of cases 16 and 18 (`RuleError("dealer_must_call")`,
 * `RuleError("sitting_out")`) belong to `reduce.ts` and are out of scope for
 * this file; the legal-set halves are asserted here.
 */

import { describe, expect, it } from 'vitest';
import { DECK, effectiveSuit } from '../cards';
import {
	RANK_NAME,
	SUIT_NAME,
	SUIT_NOUN,
	cardName,
	cardNameLower,
	findLegalMove,
	getLegalPlays,
	isStuckDealer,
	legalMoveIds,
	legalMoves,
	orderLabel,
	whyIllegal
} from '../legal';
import { partnerOf } from '../seats';
import {
	DEFAULT_ENGINE_CONFIG,
	type CardId,
	type EngineConfig,
	type GamePhase,
	type GameState,
	type HandState,
	type Seat,
	type Suit,
	type Trick
} from '../types';

const ALL_SUITS: readonly Suit[] = ['S', 'H', 'D', 'C'];
const ALL_SEATS: readonly Seat[] = [0, 1, 2, 3];
/** `null` means "on lead". */
const LEDS: readonly (Suit | null)[] = [null, 'S', 'H', 'D', 'C'];

/* -------------------------------------------------------------------------- */
/* Fixtures                                                                    */
/* -------------------------------------------------------------------------- */

const emptyTrick = (index = 0): Trick => ({ index, ledSuit: null, plays: [], winnerSeat: null });

function makeState(
	hand: Partial<HandState> = {},
	cfg: Partial<EngineConfig> = {},
	top: Partial<Pick<GameState, 'status' | 'score' | 'winnerTeam'>> = {}
): GameState {
	return {
		schema: 1,
		v: 1,
		gameId: 'g-test',
		internalToken: 'tok',
		seed: 'seed',
		cfg: { ...DEFAULT_ENGINE_CONFIG, ...cfg },
		firstDealer: 0,
		status: 'active',
		score: [0, 0],
		handNo: 0,
		misdealStreak: 0,
		winnerTeam: null,
		hand: {
			handNo: 0,
			phase: 'trick_play',
			dealerSeat: 0,
			turnSeat: 0,
			deckOrder: DECK,
			cut: false,
			hands: { 0: [], 1: [], 2: [], 3: [] },
			kitty: ['9S', 'TS', 'JS', 'QS'],
			upCard: '9S',
			upCardTurnedDown: false,
			turnedDownSuit: null,
			trump: null,
			makerSeat: null,
			aloneSeat: null,
			sittingSeat: null,
			dealerDiscard: null,
			passes: 0,
			bids: [],
			trick: emptyTrick(),
			trickLog: [],
			tricksWon: [0, 0],
			result: null,
			delta: null,
			...hand
		},
		turnId: 't0',
		turnDeadlineAt: null,
		timerId: null,
		watchdogId: null,
		revealId: null,
		pending: null,
		recentMoveIds: [],
		appliedSeq: 0,
		...top
	};
}

/** Deep-freeze, so a mutation inside `legalMoves` throws in strict mode (V21). */
function deepFreeze<T>(o: T): T {
	if (o === null || typeof o !== 'object') return o;
	for (const v of Object.values(o as Record<string, unknown>)) deepFreeze(v);
	return Object.freeze(o);
}

/** A small deterministic PRNG — no `Math.random()` anywhere in these tests. */
function lcg(seed: number): () => number {
	let s = seed >>> 0;
	return () => {
		s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
		return s / 0x100000000;
	};
}

function randomHand(rnd: () => number, size: number): CardId[] {
	const pool = [...DECK];
	for (let i = pool.length - 1; i > 0; i--) {
		const j = Math.floor(rnd() * (i + 1));
		[pool[i], pool[j]] = [pool[j]!, pool[i]!];
	}
	return pool.slice(0, size);
}

/* -------------------------------------------------------------------------- */
/* getLegalPlays — § 9 cases 8–14                                              */
/* -------------------------------------------------------------------------- */

describe('getLegalPlays — the follow-suit primitive', () => {
	const VOID_HAND: CardId[] = ['JD', '9C', 'TC', 'KC', 'AC'];

	// § 9 case 8 — the hand is VOID in diamonds because JD is a heart.
	it('case 8: JD 9C TC KC AC with hearts trump is void in DIAMONDS — all five legal', () => {
		expect(getLegalPlays(VOID_HAND, 'D', 'H')).toEqual(['JD', '9C', 'TC', 'KC', 'AC']);
	});

	// § 9 case 9
	it('case 9: to a HEART lead that same hand has exactly one legal play — JD', () => {
		expect(getLegalPlays(VOID_HAND, 'H', 'H')).toEqual(['JD']);
	});

	// § 9 case 10
	it('case 10: on lead (ledSuit null) everything is legal', () => {
		expect(getLegalPlays(VOID_HAND, null, 'H')).toEqual(['JD', '9C', 'TC', 'KC', 'AC']);
	});

	it('to a CLUB lead that hand must follow with its four clubs', () => {
		expect(getLegalPlays(VOID_HAND, 'C', 'H')).toEqual(['9C', 'TC', 'KC', 'AC']);
	});

	// § 9 case 11
	it('case 11: AS KS 9H TD QC with spades trump, spades led → AS KS', () => {
		const hand: CardId[] = ['AS', 'KS', '9H', 'TD', 'QC'];
		expect(getLegalPlays(hand, 'S', 'S')).toEqual(['AS', 'KS']);
	});

	// § 9 case 12
	it('case 12: same hand with clubs trump — one legal card per lead', () => {
		const hand: CardId[] = ['AS', 'KS', '9H', 'TD', 'QC'];
		expect(getLegalPlays(hand, 'C', 'C')).toEqual(['QC']);
		expect(getLegalPlays(hand, 'H', 'C')).toEqual(['9H']);
		expect(getLegalPlays(hand, 'D', 'C')).toEqual(['TD']);
		// Spades are led and the hand holds two — but with clubs trump, JS would be
		// the left bower; there is no JS here, so both spades simply follow.
		expect(getLegalPlays(hand, 'S', 'C')).toEqual(['AS', 'KS']);
	});

	// § 9 case 13 — the left bower MUST be played to follow a trump lead.
	it('case 13: spades trump, spades led, hand JC 9D → JC is the only legal play', () => {
		expect(getLegalPlays(['JC', '9D'], 'S', 'S')).toEqual(['JC']);
	});

	// § 9 case 14 — and the mirror image: JC does not satisfy a CLUB lead.
	it('case 14: spades trump, clubs led, hand JC AH → void in clubs, both legal', () => {
		expect(getLegalPlays(['JC', 'AH'], 'C', 'S')).toEqual(['JC', 'AH']);
	});

	it('is measured by effectiveSuit, never suitOf: JD follows hearts, not diamonds', () => {
		expect(getLegalPlays(['JD', 'AD'], 'H', 'H')).toEqual(['JD']);
		expect(getLegalPlays(['JD', 'AD'], 'D', 'H')).toEqual(['AD']);
	});

	it('there is no obligation to trump and none to overtrump', () => {
		// Spades led, hearts trump, void in spades: trumping is allowed, not forced.
		const hand: CardId[] = ['AH', '9C', 'AD'];
		expect(getLegalPlays(hand, 'S', 'H')).toEqual(['AH', '9C', 'AD']);
	});

	it('preserves hand order in the returned subset', () => {
		expect(getLegalPlays(['9S', 'AS', 'KS', 'TH'], 'S', 'H')).toEqual(['9S', 'AS', 'KS']);
	});

	it('returns a fresh array and never mutates the hand', () => {
		const hand: CardId[] = ['JD', '9C', 'TC', 'KC', 'AC'];
		const snapshot = [...hand];

		const onLead = getLegalPlays(hand, null, 'H');
		expect(onLead).not.toBe(hand);
		onLead.push('AS');

		const voidCase = getLegalPlays(hand, 'D', 'H');
		expect(voidCase).not.toBe(hand);
		voidCase.length = 0;

		expect(hand).toEqual(snapshot);
	});

	it('works before trump exists, on printed suits', () => {
		expect(getLegalPlays(['JD', 'AD', 'AH'], 'D', null)).toEqual(['JD', 'AD']);
		expect(getLegalPlays(['JD', 'AD', 'AH'], 'H', null)).toEqual(['AH']);
	});

	it('returns [] only for an empty hand', () => {
		expect(getLegalPlays([], 'S', 'H')).toEqual([]);
		expect(getLegalPlays([], null, 'H')).toEqual([]);
	});

	/**
	 * § 9 case 19, made deterministic: 2 000 pseudo-random hands × 4 trumps ×
	 * 5 leads = 40 000 checks, seeded, no `fast-check` dependency.
	 */
	it('case 19: never empty, always a subset, and never admits a renege (V7/V19)', () => {
		const rnd = lcg(0xc0ffee);
		let checks = 0;
		for (let i = 0; i < 2000; i++) {
			const size = 1 + Math.floor(rnd() * 5);
			const hand = randomHand(rnd, size);
			for (const trump of ALL_SUITS) {
				for (const led of LEDS) {
					const legal = getLegalPlays(hand, led, trump);
					checks++;

					// Non-empty for a non-empty hand.
					expect([hand, led, trump, legal.length > 0]).toEqual([hand, led, trump, true]);
					// A subset of the hand, with no duplicates.
					expect([hand, led, trump, legal.every((c) => hand.includes(c))]).toEqual([
						hand,
						led,
						trump,
						true
					]);
					expect(new Set(legal).size).toBe(legal.length);

					if (led === null) {
						expect([hand, trump, legal]).toEqual([hand, trump, hand]);
						continue;
					}
					const following = hand.filter((c) => effectiveSuit(c, trump) === led);
					if (following.length > 0) {
						// Renege-free: nothing outside the led suit is offered.
						expect([hand, led, trump, legal]).toEqual([hand, led, trump, following]);
					} else {
						expect([hand, led, trump, legal]).toEqual([hand, led, trump, hand]);
					}
				}
			}
		}
		expect(checks).toBe(2000 * 4 * 5);
	});
});

/* -------------------------------------------------------------------------- */
/* whyIllegal                                                                  */
/* -------------------------------------------------------------------------- */

describe('whyIllegal', () => {
	it('reproduces the spec example verbatim', () => {
		expect(whyIllegal(['JD', '9C'], '9C', 'H', 'H')).toBe(
			'Hearts were led. Your Jack of diamonds is a heart right now — you have to follow.'
		);
	});

	it('is null for a legal play', () => {
		expect(whyIllegal(['JD', '9C'], 'JD', 'H', 'H')).toBeNull();
		expect(whyIllegal(['JD', '9C'], '9C', null, 'H')).toBeNull();
		// Void in the led suit: anything goes.
		expect(whyIllegal(['9C', 'TC'], '9C', 'H', 'H')).toBeNull();
	});

	it('rejects a card that is not in the hand', () => {
		expect(whyIllegal(['JD', '9C'], 'AS', 'H', 'H')).toBe("That card isn't in your hand.");
		expect(whyIllegal(['JD', '9C'], 'AS', null, 'H')).toBe("That card isn't in your hand.");
	});

	it('lists the cards the seat still holds, joined with a comma and "and"', () => {
		expect(whyIllegal(['AS', 'KS', 'TH'], 'TH', 'S', 'H')).toBe(
			'Spades were led. You still hold the ace of spades and the king of spades — you have to follow suit.'
		);
		expect(whyIllegal(['AS', 'KS', 'QS', 'TH'], 'TH', 'S', 'H')).toBe(
			'Spades were led. You still hold the ace of spades, the king of spades and the queen of spades — you have to follow suit.'
		);
	});

	it('uses the singular form for a single held card', () => {
		expect(whyIllegal(['AS', 'TH'], 'TH', 'S', 'H')).toBe(
			'Spades were led. You still hold the ace of spades — you have to follow suit.'
		);
	});

	it('prefers the bower explanation over the list when the left bower is held', () => {
		// Spades trump, spades led, holding JC and AS. The message names the bower.
		expect(whyIllegal(['JC', 'AS', '9H'], '9H', 'S', 'S')).toBe(
			'Spades were led. Your Jack of clubs is a spade right now — you have to follow.'
		);
	});

	it('agrees with getLegalPlays on every card of a hand, over every lead and trump', () => {
		const hand: CardId[] = ['JD', 'AD', 'AS', '9C', 'TH'];
		for (const trump of ALL_SUITS) {
			for (const led of LEDS) {
				const legal = getLegalPlays(hand, led, trump);
				for (const c of hand) {
					const why = whyIllegal(hand, c, led, trump);
					expect([c, led, trump, why === null]).toEqual([c, led, trump, legal.includes(c)]);
				}
			}
		}
	});
});

/* -------------------------------------------------------------------------- */
/* Naming tables                                                               */
/* -------------------------------------------------------------------------- */

describe('naming', () => {
	it('SUIT_NAME / SUIT_NOUN / RANK_NAME are exact and frozen', () => {
		expect(SUIT_NAME).toEqual({ S: 'Spades', H: 'Hearts', D: 'Diamonds', C: 'Clubs' });
		expect(SUIT_NOUN).toEqual({ S: 'spade', H: 'heart', D: 'diamond', C: 'club' });
		expect(RANK_NAME).toEqual({
			'9': 'Nine',
			T: 'Ten',
			J: 'Jack',
			Q: 'Queen',
			K: 'King',
			A: 'Ace'
		});
		expect(Object.isFrozen(SUIT_NAME)).toBe(true);
	});

	it('cardName names the PRINTED card, never the effective one', () => {
		expect(cardName('JD')).toBe('Jack of diamonds');
		expect(cardName('9C')).toBe('Nine of clubs');
		expect(cardName('AS')).toBe('Ace of spades');
		expect(cardNameLower('JD')).toBe('jack of diamonds');
	});

	it('names all 24 cards distinctly', () => {
		const names = DECK.map(cardName);
		expect(new Set(names).size).toBe(24);
	});

	it('orderLabel is "I take it" for the dealer, "I assist" for their partner, else "Order it up"', () => {
		for (const dealer of ALL_SEATS) {
			expect([dealer, orderLabel(dealer, dealer)]).toEqual([dealer, 'I take it']);
			expect([dealer, orderLabel(partnerOf(dealer), dealer)]).toEqual([dealer, 'I assist']);
			for (const s of ALL_SEATS) {
				if (s === dealer || s === partnerOf(dealer)) continue;
				expect([dealer, s, orderLabel(s, dealer)]).toEqual([dealer, s, 'Order it up']);
			}
		}
	});
});

/* -------------------------------------------------------------------------- */
/* isStuckDealer — V3                                                          */
/* -------------------------------------------------------------------------- */

describe('isStuckDealer (V3)', () => {
	const stuckState = (over: Partial<HandState> = {}, cfg: Partial<EngineConfig> = {}) =>
		makeState({ phase: 'bid_round_2', dealerSeat: 0, turnSeat: 0, passes: 3, ...over }, cfg);

	it('holds only for the dealer, in bid_round_2, at passes === 3, with the flag on', () => {
		expect(isStuckDealer(stuckState(), 0)).toBe(true);
	});

	it('is false for any seat that is not the dealer', () => {
		for (const s of [1, 2, 3] as Seat[]) expect(isStuckDealer(stuckState(), s)).toBe(false);
	});

	it('is false at fewer than three passes', () => {
		for (const passes of [0, 1, 2]) {
			expect([passes, isStuckDealer(stuckState({ passes }), 0)]).toEqual([passes, false]);
		}
	});

	it('is false in round 1 — the dealer may always pass the up-card', () => {
		expect(isStuckDealer(stuckState({ phase: 'bid_round_1' }), 0)).toBe(false);
	});

	it('is false with stickTheDealer off', () => {
		expect(isStuckDealer(stuckState({}, { stickTheDealer: false }), 0)).toBe(false);
	});
});

/* -------------------------------------------------------------------------- */
/* legalMoves — § 6.3, § 9 cases 15–18                                         */
/* -------------------------------------------------------------------------- */

describe('legalMoves — guards', () => {
	it('is empty for any seat that is not to act', () => {
		const s = makeState({ phase: 'bid_round_1', turnSeat: 1, dealerSeat: 0 });
		expect(legalMoves(s, 0)).toEqual([]);
		expect(legalMoves(s, 2)).toEqual([]);
		expect(legalMoves(s, 3)).toEqual([]);
		expect(legalMoves(s, 1).length).toBeGreaterThan(0);
	});

	it('is empty unless status is "active"', () => {
		for (const status of ['complete', 'abandoned'] as const) {
			const s = makeState({ phase: 'bid_round_1', turnSeat: 1 }, {}, { status });
			expect([status, legalMoves(s, 1)]).toEqual([status, []]);
		}
	});

	// § 9 case 18 — the legal-set half. (`apply` → "sitting_out" lives in reduce.ts.)
	it('case 18: is empty for the sitting seat in EVERY phase, even on its turn', () => {
		const phases: GamePhase[] = [
			'lobby',
			'cutting',
			'deal',
			'bid_round_1',
			'dealer_discard',
			'bid_round_2',
			'trick_play',
			'trick_resolve',
			'hand_score',
			'game_over'
		];
		for (const phase of phases) {
			const s = makeState({
				phase,
				turnSeat: 3,
				sittingSeat: 3,
				aloneSeat: 1,
				makerSeat: 1,
				trump: 'H',
				hands: { 0: ['AS'], 1: ['KS'], 2: ['QS'], 3: ['9S', 'TS', 'JS', 'QH', 'KH'] }
			});
			expect([phase, legalMoves(s, 3)]).toEqual([phase, []]);
		}
	});

	it('returns [] in every engine-only phase', () => {
		for (const phase of ['lobby', 'deal', 'trick_resolve', 'hand_score', 'game_over'] as const) {
			const s = makeState({ phase, turnSeat: 0 });
			expect([phase, legalMoves(s, 0)]).toEqual([phase, []]);
		}
	});
});

describe('legalMoves — cutting', () => {
	it('offers exactly Bump and Run ’em', () => {
		const s = makeState({ phase: 'cutting', turnSeat: 3, dealerSeat: 0 });
		expect(legalMoves(s, 3)).toEqual([
			{ id: 'cut:yes', move: { t: 'cut', cut: true }, label: 'Bump' },
			{ id: 'cut:no', move: { t: 'cut', cut: false }, label: "Run 'em" }
		]);
	});
});

describe('legalMoves — bid_round_1', () => {
	it('offers pass, orderUp, orderUp+alone with the role-correct label', () => {
		const s = makeState({ phase: 'bid_round_1', turnSeat: 2, dealerSeat: 0 });
		expect(legalMoves(s, 2)).toEqual([
			{ id: 'pass', move: { t: 'pass' }, label: 'Pass' },
			{ id: 'orderUp', move: { t: 'orderUp', alone: false }, label: 'I assist' },
			{ id: 'orderUp+alone', move: { t: 'orderUp', alone: true }, label: 'I assist, alone' }
		]);
	});

	it('labels the dealer’s own acceptance "I take it"', () => {
		const s = makeState({ phase: 'bid_round_1', turnSeat: 0, dealerSeat: 0 });
		expect(legalMoves(s, 0).map((m) => m.label)).toEqual(['Pass', 'I take it', 'I take it, alone']);
	});

	it('gives all four seats the same three move ids', () => {
		for (const seat of ALL_SEATS) {
			const s = makeState({ phase: 'bid_round_1', turnSeat: seat, dealerSeat: 0 });
			expect([seat, legalMoveIds(s, seat)]).toEqual([seat, ['pass', 'orderUp', 'orderUp+alone']]);
		}
	});
});

describe('legalMoves — dealer_discard', () => {
	it('offers all six cards, including the up-card itself', () => {
		const six: CardId[] = ['9S', 'TS', 'JS', 'QS', 'KS', 'AS'];
		const s = makeState({
			phase: 'dealer_discard',
			turnSeat: 0,
			dealerSeat: 0,
			upCard: 'AS',
			trump: 'S',
			hands: { 0: six, 1: [], 2: [], 3: [] }
		});
		const moves = legalMoves(s, 0);
		expect(moves).toHaveLength(6);
		expect(moves.map((m) => m.id)).toEqual([
			'discard:9S',
			'discard:TS',
			'discard:JS',
			'discard:QS',
			'discard:KS',
			'discard:AS'
		]);
		expect(moves.map((m) => m.id)).toContain('discard:AS'); // the up-card
		expect(moves[0]).toEqual({
			id: 'discard:9S',
			move: { t: 'discard', card: '9S' },
			label: 'Nine of spades'
		});
	});
});

describe('legalMoves — bid_round_2', () => {
	const round2 = (over: Partial<HandState> = {}, cfg: Partial<EngineConfig> = {}) =>
		makeState(
			{
				phase: 'bid_round_2',
				turnSeat: 0,
				dealerSeat: 0,
				upCard: '9H',
				upCardTurnedDown: true,
				turnedDownSuit: 'H',
				passes: 0,
				...over
			},
			cfg
		);

	// § 9 case 15
	it('case 15: 7 entries with turnedDownSuit "H", and no call:H anywhere', () => {
		const moves = legalMoves(round2({ turnSeat: 1 }), 1);
		expect(moves).toHaveLength(7);
		expect(moves.map((m) => m.id)).toEqual([
			'pass',
			'call:S',
			'call:S+alone',
			'call:D',
			'call:D+alone',
			'call:C',
			'call:C+alone'
		]);
		expect(moves.some((m) => m.id.startsWith('call:H'))).toBe(false);
	});

	it('labels the calls by suit name, with ", alone" on the loner variant', () => {
		const moves = legalMoves(round2({ turnSeat: 1 }), 1);
		expect(moves.map((m) => m.label)).toEqual([
			'Pass',
			'Spades',
			'Spades, alone',
			'Diamonds',
			'Diamonds, alone',
			'Clubs',
			'Clubs, alone'
		]);
		expect(moves[1]!.move).toEqual({ t: 'call', suit: 'S', alone: false });
		expect(moves[2]!.move).toEqual({ t: 'call', suit: 'S', alone: true });
	});

	it('excludes the turned-down suit whichever suit it was', () => {
		for (const down of ALL_SUITS) {
			const moves = legalMoves(round2({ turnSeat: 1, turnedDownSuit: down }), 1);
			expect([down, moves.length]).toEqual([down, 7]);
			expect([down, moves.some((m) => m.id.startsWith(`call:${down}`))]).toEqual([down, false]);
		}
	});

	// § 9 case 16 — the legal-set half.
	it('case 16: the stuck dealer gets 6 entries and NO pass', () => {
		const s = round2({ turnSeat: 0, dealerSeat: 0, passes: 3 }, { stickTheDealer: true });
		const moves = legalMoves(s, 0);
		expect(moves).toHaveLength(6);
		expect(moves.map((m) => m.id)).toEqual([
			'call:S',
			'call:S+alone',
			'call:D',
			'call:D+alone',
			'call:C',
			'call:C+alone'
		]);
		expect(moves.some((m) => m.id === 'pass')).toBe(false);
		// V19 still holds: the set is not empty, so the dealer can always move.
		expect(moves.length).toBeGreaterThan(0);
	});

	// § 9 case 17
	it('case 17: with stickTheDealer off the same state keeps pass, at 7 entries', () => {
		const s = round2({ turnSeat: 0, dealerSeat: 0, passes: 3 }, { stickTheDealer: false });
		const moves = legalMoves(s, 0);
		expect(moves).toHaveLength(7);
		expect(moves[0]).toEqual({ id: 'pass', move: { t: 'pass' }, label: 'Pass' });
	});

	it('a non-dealer at passes === 3 still gets pass (only the dealer is stuck)', () => {
		const s = round2({ turnSeat: 2, dealerSeat: 0, passes: 3 }, { stickTheDealer: true });
		expect(legalMoveIds(s, 2)).toContain('pass');
		expect(legalMoves(s, 2)).toHaveLength(7);
	});

	it('offers all four suits when nothing has been turned down', () => {
		const s = round2({ turnSeat: 1, turnedDownSuit: null });
		expect(legalMoves(s, 1)).toHaveLength(9);
	});
});

describe('legalMoves — trick_play', () => {
	it('wraps getLegalPlays and prefixes ids with "play:"', () => {
		const s = makeState({
			phase: 'trick_play',
			turnSeat: 1,
			trump: 'H',
			trick: { index: 0, ledSuit: 'H', plays: [{ seat: 0, card: 'AH' }], winnerSeat: null },
			hands: { 0: [], 1: ['JD', '9C', 'TC', 'KC', 'AC'], 2: [], 3: [] }
		});
		expect(legalMoves(s, 1)).toEqual([
			{ id: 'play:JD', move: { t: 'play', card: 'JD' }, label: 'Jack of diamonds' }
		]);
	});

	it('offers the whole hand on lead', () => {
		const s = makeState({
			phase: 'trick_play',
			turnSeat: 1,
			trump: 'H',
			trick: emptyTrick(),
			hands: { 0: [], 1: ['JD', '9C', 'TC'], 2: [], 3: [] }
		});
		expect(legalMoveIds(s, 1)).toEqual(['play:JD', 'play:9C', 'play:TC']);
	});

	it('offers the whole hand when void — the left bower does not follow its printed suit', () => {
		const s = makeState({
			phase: 'trick_play',
			turnSeat: 1,
			trump: 'H',
			trick: { index: 0, ledSuit: 'D', plays: [{ seat: 0, card: 'AD' }], winnerSeat: null },
			hands: { 0: [], 1: ['JD', '9C', 'TC', 'KC', 'AC'], 2: [], 3: [] }
		});
		expect(legalMoveIds(s, 1)).toEqual(['play:JD', 'play:9C', 'play:TC', 'play:KC', 'play:AC']);
	});
});

/* -------------------------------------------------------------------------- */
/* V19, V21 and the lookup helpers                                             */
/* -------------------------------------------------------------------------- */

describe('legalMoves — V19: never empty for the acting seat', () => {
	it('holds in every phase that has an acting seat, across all four seats', () => {
		const actionable: readonly GamePhase[] = [
			'cutting',
			'bid_round_1',
			'dealer_discard',
			'bid_round_2',
			'trick_play'
		];
		const rnd = lcg(0x5eed);
		for (const phase of actionable) {
			for (const seat of ALL_SEATS) {
				for (const dealer of ALL_SEATS) {
					for (const passes of [0, 1, 2, 3]) {
						for (const stick of [true, false]) {
							const hand = randomHand(rnd, phase === 'dealer_discard' ? 6 : 5);
							const s = makeState(
								{
									phase,
									turnSeat: seat,
									dealerSeat: dealer,
									passes,
									trump: 'H',
									turnedDownSuit: phase === 'bid_round_2' ? 'H' : null,
									trick: {
										index: 0,
										ledSuit: 'S',
										plays: [{ seat: 3, card: 'AS' }],
										winnerSeat: null
									},
									hands: { 0: hand, 1: hand, 2: hand, 3: hand }
								},
								{ stickTheDealer: stick }
							);
							const n = legalMoves(s, seat).length;
							expect([phase, seat, dealer, passes, stick, n > 0]).toEqual([
								phase,
								seat,
								dealer,
								passes,
								stick,
								true
							]);
						}
					}
				}
			}
		}
	});
});

describe('legalMoves — V21: purity', () => {
	it('does not mutate a deep-frozen state, in any actionable phase', () => {
		const phases: readonly GamePhase[] = [
			'cutting',
			'bid_round_1',
			'dealer_discard',
			'bid_round_2',
			'trick_play'
		];
		for (const phase of phases) {
			const s = deepFreeze(
				makeState({
					phase,
					turnSeat: 0,
					dealerSeat: 0,
					trump: 'H',
					turnedDownSuit: 'H',
					trick: { index: 0, ledSuit: 'S', plays: [{ seat: 3, card: 'AS' }], winnerSeat: null },
					hands: { 0: ['AS', 'KS', '9H', 'TD', 'QC'], 1: [], 2: [], 3: [] }
				})
			);
			expect(() => legalMoves(s, 0)).not.toThrow();
			expect([phase, legalMoves(s, 0).length > 0]).toEqual([phase, true]);
			// Same state, same answer: nothing was consumed or reordered in place.
			expect([phase, legalMoves(s, 0)]).toEqual([phase, legalMoves(s, 0)]);
		}
	});

	it('does not mutate a deep-frozen hand array in getLegalPlays', () => {
		const hand = deepFreeze<CardId[]>(['JD', '9C', 'TC', 'KC', 'AC']);
		expect(() => getLegalPlays(hand, 'H', 'H')).not.toThrow();
		expect(getLegalPlays(hand, 'H', 'H')).toEqual(['JD']);
		expect(hand).toEqual(['JD', '9C', 'TC', 'KC', 'AC']);
	});

	it('returns fresh arrays and objects on every call', () => {
		const s = makeState({ phase: 'cutting', turnSeat: 3 });
		const a = legalMoves(s, 3);
		const b = legalMoves(s, 3);
		expect(a).not.toBe(b);
		expect(a[0]).not.toBe(b[0]);
		expect(a).toEqual(b);
	});
});

describe('legalMoveIds / findLegalMove', () => {
	it('legalMoveIds is legalMoves.map(id), in the same order', () => {
		const s = makeState({
			phase: 'bid_round_2',
			turnSeat: 1,
			dealerSeat: 0,
			turnedDownSuit: 'H'
		});
		expect(legalMoveIds(s, 1)).toEqual(legalMoves(s, 1).map((m) => m.id));
	});

	it('findLegalMove returns the offered move, or null when it is not on offer', () => {
		const s = makeState({
			phase: 'bid_round_2',
			turnSeat: 1,
			dealerSeat: 0,
			turnedDownSuit: 'H'
		});
		expect(findLegalMove(s, 1, 'call:S+alone')).toEqual({
			id: 'call:S+alone',
			move: { t: 'call', suit: 'S', alone: true },
			label: 'Spades, alone'
		});
		expect(findLegalMove(s, 1, 'call:H')).toBeNull(); // turned down
		expect(findLegalMove(s, 1, 'play:AS')).toBeNull(); // wrong phase
		expect(findLegalMove(s, 0, 'pass')).toBeNull(); // not seat 1's turn
	});

	it('finds every id legalMoveIds reports, and nothing else', () => {
		const s = makeState({
			phase: 'trick_play',
			turnSeat: 2,
			trump: 'S',
			trick: emptyTrick(),
			hands: { 0: [], 1: [], 2: ['AS', 'KH', '9D'], 3: [] }
		});
		for (const id of legalMoveIds(s, 2)) expect(findLegalMove(s, 2, id)).not.toBeNull();
		expect(findLegalMove(s, 2, 'play:QC')).toBeNull();
	});
});
