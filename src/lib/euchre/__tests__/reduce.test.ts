/**
 * `reduce.test.ts` — the phase state machine, the rejection codes, and the two
 * properties the whole system leans on: determinism (V20) and purity (V21).
 *
 * Normative source: `docs/02-GAME-RULES-ENGINE.md` §§ 3–8.
 *
 * Every assertion here is on an *expected value*, not on "did not throw". The
 * fixtures are built from real seeded deals wherever a real deal will do, and
 * hand-patched only where a specific card layout is the point of the test.
 */

import { describe, expect, it } from 'vitest';
import {
	DECK,
	RuleError,
	SUITS,
	advance,
	apply,
	createGame,
	cutForHand,
	cutterOf,
	eldestOf,
	effectiveSuit,
	fnv1a,
	isRuleError,
	isStuckDealer,
	legalMoveIds,
	legalMoves,
	moveId,
	mulberry32,
	newGame,
	nextSeat,
	partnerOf,
	project,
	sayForBid,
	shuffleForHand,
	startHand,
	suitOf,
	teamOf,
	type ApplyResult,
	type CardId,
	type EngineConfig,
	type GameState,
	type HandState,
	type LegalMove,
	type Move,
	type RuleCode,
	type Seat,
	type Step,
	type Suit
} from '../index';

/* -------------------------------------------------------------------------- */
/* Helpers                                                                     */
/* -------------------------------------------------------------------------- */

const SEATS_ALL: readonly Seat[] = [0, 1, 2, 3];

/** Assert `fn` rejects with exactly `code`, and hand the error back. */
function expectRule(code: RuleCode, fn: () => unknown): RuleError {
	let caught: unknown = undefined;
	let threw = false;
	try {
		fn();
	} catch (e) {
		threw = true;
		caught = e;
	}
	expect(threw, `expected RuleError(${code}) but nothing was thrown`).toBe(true);
	expect(isRuleError(caught), `expected RuleError(${code}), got ${String(caught)}`).toBe(true);
	const err = caught as RuleError;
	expect(err.code).toBe(code);
	return err;
}

/** A structural snapshot for "the state did not change" assertions. */
const snap = (s: GameState): GameState => structuredClone(s) as GameState;

function deepFreeze<T>(o: T, seen: Set<unknown> = new Set()): T {
	if (o === null || typeof o !== 'object') return o;
	if (seen.has(o)) return o;
	seen.add(o);
	Object.freeze(o);
	for (const v of Object.values(o as Record<string, unknown>)) deepFreeze(v, seen);
	return o;
}

/** A match in `cutting`, ready for the cut. */
function opened(seed: string, firstDealer?: Seat, cfg?: Partial<EngineConfig>): GameState {
	const g = createGame({ gameId: `g-${seed}`, seed, cfg, firstDealer });
	return advance(g).state;
}

const turn = (s: GameState): Seat => {
	const t = s.hand.turnSeat;
	expect(t, 'expected a seat to be on turn').not.toBeNull();
	return t as Seat;
};

/** Answer the cut and land in `bid_round_1`. */
function afterCut(state: GameState, cut = false): GameState {
	return apply(state, turn(state), { t: 'cut', cut }).state;
}

/** `n` passes by whoever is on turn. */
function passes(state: GameState, n: number): GameState {
	let s = state;
	for (let i = 0; i < n; i++) s = apply(s, turn(s), { t: 'pass' }).state;
	return s;
}

/** A hand-patched state. Only ever used where a specific card layout is the test. */
function patchHand(state: GameState, changes: Partial<HandState>): GameState {
	return { ...state, hand: { ...state.hand, ...changes } };
}

type Picker = (state: GameState, seat: Seat, legal: readonly LegalMove[]) => LegalMove;

function randomPicker(seed: string): Picker {
	const rnd = mulberry32(fnv1a(seed));
	return (_s, _seat, legal) => legal[Math.floor(rnd() * legal.length)] as LegalMove;
}

interface RunHooks {
	readonly onState?: (s: GameState) => void;
	readonly onTransition?: (before: GameState, res: ApplyResult, seat: Seat | null) => void;
	readonly budget?: number;
}

/** Drive a match to completion. Total: `advance` whenever no seat is on turn. */
function runGame(initial: GameState, pick: Picker, hooks: RunHooks = {}): GameState {
	const budget = hooks.budget ?? 20000;
	let state = initial;
	let n = 0;
	hooks.onState?.(state);
	while (state.status === 'active') {
		if (++n > budget) throw new Error(`runGame: budget exhausted in ${state.hand.phase}`);
		const seat = state.hand.turnSeat;
		let res: ApplyResult;
		if (seat === null) {
			res = advance(state);
		} else {
			const legal = legalMoves(state, seat);
			if (legal.length === 0) {
				throw new Error(`V19: empty legal set for seat ${seat} in ${state.hand.phase}`);
			}
			res = apply(state, seat, pick(state, seat, legal).move);
		}
		if (res.state === state) throw new Error(`runGame: stalled in ${state.hand.phase}`);
		hooks.onTransition?.(state, res, seat);
		state = res.state;
		hooks.onState?.(state);
	}
	return state;
}

/* -------------------------------------------------------------------------- */
/* createGame · startHand · lobby -> cutting                                   */
/* -------------------------------------------------------------------------- */

describe('createGame', () => {
	it('produces a lobby state with nothing dealt and no seat to act', () => {
		const g = newGame('lobby-seed');
		expect(g.schema).toBe(1);
		expect(g.status).toBe('active');
		expect(g.hand.phase).toBe('lobby');
		expect(g.hand.turnSeat).toBeNull();
		expect(g.score).toEqual([0, 0]);
		expect(g.handNo).toBe(0);
		expect(g.hand.handNo).toBe(0);
		expect(g.misdealStreak).toBe(0);
		expect(g.winnerTeam).toBeNull();
		expect(g.hand.upCard).toBeNull();
		expect(g.hand.trump).toBeNull();
		expect(g.hand.makerSeat).toBeNull();
		expect(g.hand.dealerDiscard).toBeNull();
		expect(g.hand.trick).toEqual({ index: 0, ledSuit: null, plays: [], winnerSeat: null });
		for (const s of SEATS_ALL) {
			expect(g.hand.hands[s]).toEqual([]);
			expect(legalMoves(g, s)).toEqual([]);
		}
	});

	it('persists a genuine 24-card permutation derived from the seed alone', () => {
		const g = newGame('perm-seed');
		expect(g.hand.deckOrder).toHaveLength(24);
		expect(new Set(g.hand.deckOrder).size).toBe(24);
		expect([...g.hand.deckOrder].sort()).toEqual([...DECK].sort());
		expect(g.hand.deckOrder).toEqual(shuffleForHand('perm-seed', 0));
		expect(g.hand.kitty).toEqual(g.hand.deckOrder.slice(20));
	});

	it('honours an explicit firstDealer and otherwise draws one from the seed', () => {
		for (const d of SEATS_ALL) {
			const g = createGame({ gameId: 'g', seed: 'fd', firstDealer: d });
			expect(g.firstDealer).toBe(d);
			expect(g.hand.dealerSeat).toBe(d);
		}
		const drawn = newGame('fd-drawn');
		expect(SEATS_ALL).toContain(drawn.firstDealer);
		expect(drawn.hand.dealerSeat).toBe(drawn.firstDealer);
	});

	it('applies cfg overrides on top of the v1 defaults', () => {
		const g = newGame('cfg', { gameTo: 5, stickTheDealer: false });
		expect(g.cfg.gameTo).toBe(5);
		expect(g.cfg.stickTheDealer).toBe(false);
		expect(g.cfg.deckVariant).toBe('24');
		expect(g.cfg.misdealLimit).toBe(3);
	});
});

describe('lobby -> cutting', () => {
	it('advance() opens the first hand and offers the cut to third seat', () => {
		const g = newGame('cut-open');
		const res = advance(g);
		expect(res.steps).toEqual([]);
		const s = res.state;
		expect(s.hand.phase).toBe('cutting');
		expect(s.hand.turnSeat).toBe(cutterOf(g.hand.dealerSeat));
		expect(s.hand.dealerSeat).toBe(g.hand.dealerSeat);
		expect(s.hand.deckOrder).toEqual(shuffleForHand('cut-open', 0));
		expect(s.turnId).not.toBe(g.turnId);
		expect(legalMoveIds(s, turn(s))).toEqual(['cut:yes', 'cut:no']);
	});

	it('offers nothing to the three seats that are not the cutter', () => {
		const s = opened('cut-open-2');
		const cutter = cutterOf(s.hand.dealerSeat);
		for (const seat of SEATS_ALL) {
			if (seat === cutter) continue;
			expect(legalMoves(s, seat)).toEqual([]);
		}
	});

	it('startHand() is a pure re-open of the same hand number and dealer', () => {
		const g = newGame('sh');
		const a = startHand(g).state;
		const b = startHand(g).state;
		expect(a.hand.phase).toBe('cutting');
		expect(a.hand.handNo).toBe(0);
		expect(a.hand.dealerSeat).toBe(g.hand.dealerSeat);
		expect(a.hand).toEqual(b.hand);
	});
});

/* -------------------------------------------------------------------------- */
/* cutting -> deal -> bid_round_1                                              */
/* -------------------------------------------------------------------------- */

describe('cutting -> deal -> bid_round_1', () => {
	it("run 'em deals from the untouched permutation and turns the up-card", () => {
		const s0 = opened('deal-a', 0);
		const before = s0.hand.deckOrder;
		const res = apply(s0, cutterOf(0), { t: 'cut', cut: false });
		const s = res.state;

		expect(s.hand.cut).toBe(false);
		expect(s.hand.deckOrder).toEqual(before);
		expect(s.hand.phase).toBe('bid_round_1');
		expect(s.hand.turnSeat).toBe(eldestOf(0));
		expect(s.hand.passes).toBe(0);
		expect(s.hand.bids).toEqual([]);
		expect(s.hand.upCard).toBe(before[20]);
		expect(s.hand.upCardTurnedDown).toBe(false);
		expect(s.hand.turnedDownSuit).toBeNull();
		expect(s.hand.kitty).toEqual(before.slice(20));
		for (const seat of SEATS_ALL) expect(s.hand.hands[seat]).toHaveLength(5);

		const dealt = [...s.hand.hands[0], ...s.hand.hands[1], ...s.hand.hands[2], ...s.hand.hands[3]];
		expect(dealt).toHaveLength(20);
		expect(new Set([...dealt, ...s.hand.kitty]).size).toBe(24);
	});

	it('bump rotates the permutation deterministically, never reshuffles', () => {
		const s0 = opened('deal-b', 2);
		const before = s0.hand.deckOrder;
		const s = apply(s0, cutterOf(2), { t: 'cut', cut: true }).state;
		expect(s.hand.cut).toBe(true);
		expect(s.hand.deckOrder).toEqual(cutForHand(before, 'deal-b', 0));
		expect(s.hand.deckOrder).not.toEqual(before);
		expect([...s.hand.deckOrder].sort()).toEqual([...before].sort());
		expect(s.hand.upCard).toBe(s.hand.deckOrder[20]);
	});

	it('emits cut, dealt and upCardTurned in that order, with eight faithful packets', () => {
		const s0 = opened('deal-c', 1);
		const res = apply(s0, cutterOf(1), { t: 'cut', cut: false });
		expect(res.steps.map((x) => x.t)).toEqual(['cut', 'dealt', 'upCardTurned']);

		const cut = res.steps[0] as Extract<Step, { t: 'cut' }>;
		expect(cut).toEqual({ t: 'cut', seat: cutterOf(1), cut: false });

		const dealt = res.steps[1] as Extract<Step, { t: 'dealt' }>;
		expect(dealt.packets).toHaveLength(8);
		expect(dealt.packets.reduce((n, p) => n + p.count, 0)).toBe(20);
		for (const seat of SEATS_ALL) {
			const mine = dealt.packets.filter((p) => p.seat === seat);
			expect(mine).toHaveLength(2);
			expect(mine.reduce((n, p) => n + p.count, 0)).toBe(5);
			expect(mine.flatMap((p) => p.cards ?? [])).toEqual(res.state.hand.hands[seat]);
		}
		// Packet order is dealing order: eldest first, two passes clockwise.
		expect(dealt.packets.map((p) => p.seat)).toEqual([2, 3, 0, 1, 2, 3, 0, 1]);

		const up = res.steps[2] as Extract<Step, { t: 'upCardTurned' }>;
		expect(up.card).toBe(res.state.hand.upCard);
	});

	it('rejects a cut from a seat that is not the cutter, and any non-cut move', () => {
		const s = opened('deal-d', 0);
		const cutter = cutterOf(0);
		for (const seat of SEATS_ALL) {
			if (seat === cutter) continue;
			expectRule('not_your_turn', () => apply(s, seat, { t: 'cut', cut: true }));
		}
		expectRule('wrong_phase', () => apply(s, cutter, { t: 'pass' }));
		expectRule('wrong_phase', () => apply(s, cutter, { t: 'orderUp', alone: false }));
		expectRule('wrong_phase', () => apply(s, cutter, { t: 'play', card: s.hand.deckOrder[0] }));
	});
});

/* -------------------------------------------------------------------------- */
/* bid_round_1                                                                 */
/* -------------------------------------------------------------------------- */

describe('bid_round_1', () => {
	it('offers exactly pass / orderUp / orderUp+alone with role-correct labels', () => {
		const s = afterCut(opened('b1-a', 0));
		expect(legalMoveIds(s, eldestOf(0))).toEqual(['pass', 'orderUp', 'orderUp+alone']);
		const labels = legalMoves(s, 1).map((m) => m.label);
		expect(labels).toEqual(['Pass', 'Order it up', 'Order it up, alone']);

		const atPartner = passes(s, 1);
		expect(legalMoves(atPartner, 2).map((m) => m.label)).toEqual([
			'Pass',
			'I assist',
			'I assist, alone'
		]);
		const atDealer = passes(s, 3);
		expect(legalMoves(atDealer, 0).map((m) => m.label)).toEqual([
			'Pass',
			'I take it',
			'I take it, alone'
		]);
	});

	it('rotates one seat left per pass and counts them', () => {
		let s = afterCut(opened('b1-b', 0));
		const order: Seat[] = [1, 2, 3, 0];
		for (let i = 0; i < 3; i++) {
			expect(s.hand.turnSeat).toBe(order[i]);
			expect(s.hand.passes).toBe(i);
			s = apply(s, order[i] as Seat, { t: 'pass' }).state;
			expect(s.hand.phase).toBe('bid_round_1');
			expect(s.hand.passes).toBe(i + 1);
			expect(s.hand.bids).toHaveLength(i + 1);
			expect(s.hand.bids[i]).toEqual({ seat: order[i], move: { t: 'pass' }, say: 'Pass.' });
		}
		expect(s.hand.turnSeat).toBe(0);
	});

	it('four passes turn the up-card down and open round 2 at eldest', () => {
		const s3 = passes(afterCut(opened('b1-c', 0)), 3);
		const upCard = s3.hand.upCard as CardId;
		const res = apply(s3, 0, { t: 'pass' });
		const s = res.state;

		expect(s.hand.phase).toBe('bid_round_2');
		expect(s.hand.turnSeat).toBe(eldestOf(0));
		expect(s.hand.upCardTurnedDown).toBe(true);
		expect(s.hand.turnedDownSuit).toBe(suitOf(upCard));
		expect(s.hand.upCard).toBe(upCard);
		expect(s.hand.passes).toBe(0);
		expect(s.hand.bids).toHaveLength(4);
		expect(s.hand.bids[3]).toEqual({ seat: 0, move: { t: 'pass' }, say: 'Turn it down.' });
		expect(s.hand.trump).toBeNull();
		expect(s.hand.makerSeat).toBeNull();
		expect(s.hand.dealerDiscard).toBeNull();

		expect(res.steps.map((x) => x.t)).toEqual(['bid', 'turnedDown']);
		expect(res.steps[1]).toEqual({ t: 'turnedDown', suit: suitOf(upCard) });
	});

	it('routes every successful orderUp to dealer_discard, whoever called', () => {
		const base = afterCut(opened('b1-d', 0));
		const upCard = base.hand.upCard as CardId;
		const order: Seat[] = [1, 2, 3, 0];

		for (let k = 0; k < 4; k++) {
			const caller = order[k] as Seat;
			const s0 = passes(base, k);
			const before = s0.hand.hands[0];
			const res = apply(s0, caller, { t: 'orderUp', alone: false });
			const s = res.state;

			expect(s.hand.phase).toBe('dealer_discard');
			expect(s.hand.turnSeat).toBe(0);
			expect(s.hand.trump).toBe(suitOf(upCard));
			expect(s.hand.makerSeat).toBe(caller);
			expect(s.hand.aloneSeat).toBeNull();
			expect(s.hand.sittingSeat).toBeNull();
			expect(s.hand.upCard).toBe(upCard);
			expect(s.hand.upCardTurnedDown).toBe(false);
			expect(s.hand.turnedDownSuit).toBeNull();
			expect(s.hand.dealerDiscard).toBeNull();
			expect(s.hand.hands[0]).toEqual([...before, upCard]);
			expect(s.hand.hands[0]).toHaveLength(6);
			for (const other of [1, 2, 3] as Seat[]) expect(s.hand.hands[other]).toHaveLength(5);

			expect(res.steps.map((x) => x.t)).toEqual(['bid', 'trumpSet']);
			expect(res.steps[1]).toEqual({
				t: 'trumpSet',
				suit: suitOf(upCard),
				makerSeat: caller,
				aloneSeat: null,
				sittingSeat: null,
				viaOrderUp: true
			});
		}
	});

	it('records the role-correct spoken line for each order-up', () => {
		const base = afterCut(opened('b1-e', 0));
		const said = (k: number, alone: boolean): string => {
			const s0 = passes(base, k);
			const seat = ([1, 2, 3, 0] as Seat[])[k] as Seat;
			return apply(s0, seat, { t: 'orderUp', alone }).state.hand.bids[k]?.say as string;
		};
		expect(said(0, false)).toBe('Order it up.');
		expect(said(1, false)).toBe('I assist.');
		expect(said(2, false)).toBe('Order it up.');
		expect(said(3, false)).toBe('I take it.');
		expect(said(0, true)).toBe('Order it up. Alone.');
		expect(said(3, true)).toBe('I take it. Alone.');
	});

	it('refuses round-2 vocabulary in round 1 (V2)', () => {
		const s = afterCut(opened('b1-f', 0));
		expectRule('wrong_phase', () => apply(s, 1, { t: 'call', suit: 'S', alone: false }));
		expectRule('wrong_phase', () => apply(s, 1, { t: 'cut', cut: true }));
		expectRule('wrong_phase', () =>
			apply(s, 1, { t: 'discard', card: s.hand.hands[1][0] as CardId })
		);
		expectRule('wrong_phase', () => apply(s, 1, { t: 'play', card: s.hand.hands[1][0] as CardId }));
	});
});

/* -------------------------------------------------------------------------- */
/* dealer_discard                                                              */
/* -------------------------------------------------------------------------- */

describe('dealer_discard', () => {
	const setup = (seed = 'dd-a') => {
		const base = afterCut(opened(seed, 0));
		const dealt = base.hand.hands[0];
		const upCard = base.hand.upCard as CardId;
		const s = apply(base, 1, { t: 'orderUp', alone: false }).state;
		return { base, s, dealt, upCard };
	};

	it('offers exactly the dealer six, including the up-card itself', () => {
		const { s, dealt, upCard } = setup();
		const ids = legalMoveIds(s, 0);
		expect(ids).toHaveLength(6);
		expect(ids).toEqual([...dealt, upCard].map((c) => `discard:${c}`));
		expect(ids).toContain(`discard:${upCard}`);
		for (const seat of [1, 2, 3] as Seat[]) expect(legalMoves(s, seat)).toEqual([]);
	});

	it('discarding the up-card leaves five cards and the original deal intact', () => {
		const { s, dealt, upCard } = setup();
		const res = apply(s, 0, { t: 'discard', card: upCard });
		const t = res.state;

		expect(t.hand.hands[0]).toEqual(dealt);
		expect(t.hand.hands[0]).toHaveLength(5);
		expect(t.hand.dealerDiscard).toBe(upCard);
		expect(t.hand.upCard).toBe(upCard);
		expect(t.hand.upCardTurnedDown).toBe(false);
		expect(t.hand.phase).toBe('trick_play');
		expect(t.hand.turnSeat).toBe(eldestOf(0));
		expect(t.hand.trick).toEqual({ index: 0, ledSuit: null, plays: [], winnerSeat: null });
		expect(res.steps).toEqual([{ t: 'dealerDiscarded', seat: 0 }]);
		// V12/V13: the step is structurally incapable of naming the card.
		expect(Object.keys(res.steps[0] as object).sort()).toEqual(['seat', 't']);
		expect(project(t, 0).upCard).toBe(upCard);
		expect(project(t, 1).kittyCount).toBe(4);
	});

	it('discarding a dealt card keeps the up-card in the dealer hand', () => {
		const { s, dealt, upCard } = setup('dd-b');
		const drop = dealt[2] as CardId;
		const t = apply(s, 0, { t: 'discard', card: drop }).state;
		expect(t.hand.hands[0]).toEqual([...dealt.filter((c) => c !== drop), upCard]);
		expect(t.hand.hands[0]).toHaveLength(5);
		expect(t.hand.hands[0]).toContain(upCard);
		expect(t.hand.dealerDiscard).toBe(drop);
	});

	it('rejects a discard of a card the dealer does not hold', () => {
		const { s } = setup('dd-c');
		const held = new Set(s.hand.hands[0]);
		const foreign = DECK.find((c) => !held.has(c)) as CardId;
		expectRule('card_not_in_hand', () => apply(s, 0, { t: 'discard', card: foreign }));
	});

	it('rejects bidding after trump is set with bidding_closed, not wrong_phase', () => {
		const { s } = setup('dd-d');
		expectRule('bidding_closed', () => apply(s, 0, { t: 'pass' }));
		expectRule('bidding_closed', () => apply(s, 0, { t: 'orderUp', alone: false }));
		expectRule('bidding_closed', () => apply(s, 0, { t: 'call', suit: 'S', alone: false }));
		expectRule('wrong_phase', () => apply(s, 0, { t: 'cut', cut: false }));
		expectRule('wrong_phase', () => apply(s, 0, { t: 'play', card: s.hand.hands[0][0] as CardId }));
	});

	it('rejects a discard from anyone but the dealer', () => {
		const { s } = setup('dd-e');
		for (const seat of [1, 2, 3] as Seat[]) {
			const err = expectRule('not_your_turn', () =>
				apply(s, seat, { t: 'discard', card: s.hand.hands[seat][0] as CardId })
			);
			expect(err.legal).toEqual([]);
		}
	});
});

/* -------------------------------------------------------------------------- */
/* bid_round_2 and stick the dealer                                            */
/* -------------------------------------------------------------------------- */

describe('bid_round_2', () => {
	/** Four round-1 passes, then `n` round-2 passes. */
	const round2 = (seed: string, n = 0, cfg?: Partial<EngineConfig>): GameState =>
		passes(afterCut(opened(seed, 0, cfg)), 4 + n);

	it('offers pass plus three suits x {plain, alone} and never the turned-down suit', () => {
		const s = round2('b2-a');
		const down = s.hand.turnedDownSuit as Suit;
		const ids = legalMoveIds(s, eldestOf(0));
		expect(ids).toHaveLength(7);
		expect(ids[0]).toBe('pass');
		expect(ids).not.toContain(`call:${down}`);
		expect(ids).not.toContain(`call:${down}+alone`);
		for (const suit of SUITS) {
			if (suit === down) continue;
			expect(ids).toContain(`call:${suit}`);
			expect(ids).toContain(`call:${suit}+alone`);
		}
	});

	it('rejects naming the turned-down suit as illegal_move', () => {
		const s = round2('b2-b');
		const down = s.hand.turnedDownSuit as Suit;
		const before = snap(s);
		const err = expectRule('illegal_move', () =>
			apply(s, 1, { t: 'call', suit: down, alone: false })
		);
		expectRule('illegal_move', () => apply(s, 1, { t: 'call', suit: down, alone: true }));
		expect(s).toEqual(before);
		expect(err.legal.map((m) => m.id)).toEqual(legalMoveIds(s, 1));
	});

	it('refuses round-1 vocabulary once the up-card is buried (V2)', () => {
		const s = round2('b2-c');
		expectRule('wrong_phase', () => apply(s, 1, { t: 'orderUp', alone: false }));
		expectRule('wrong_phase', () => apply(s, 1, { t: 'cut', cut: false }));
	});

	it('a successful call skips the discard and leads straight into trick_play', () => {
		const s = round2('b2-d');
		const down = s.hand.turnedDownSuit as Suit;
		const suit = SUITS.find((x) => x !== down) as Suit;
		const res = apply(s, 1, { t: 'call', suit, alone: false });
		const t = res.state;

		expect(t.hand.phase).toBe('trick_play');
		expect(t.hand.turnSeat).toBe(eldestOf(0));
		expect(t.hand.trump).toBe(suit);
		expect(t.hand.makerSeat).toBe(1);
		expect(t.hand.aloneSeat).toBeNull();
		expect(t.hand.dealerDiscard).toBeNull();
		expect(t.hand.upCard).toBe(s.hand.upCard);
		expect(t.hand.upCardTurnedDown).toBe(true);
		for (const seat of SEATS_ALL) expect(t.hand.hands[seat]).toHaveLength(5);

		expect(res.steps.map((x) => x.t)).toEqual(['bid', 'trumpSet']);
		expect(res.steps[1]).toEqual({
			t: 'trumpSet',
			suit,
			makerSeat: 1,
			aloneSeat: null,
			sittingSeat: null,
			viaOrderUp: false
		});
	});

	it('says "Next." for the colour mate and "Crossing the creek." otherwise', () => {
		const s = round2('b2-e');
		const down = s.hand.turnedDownSuit as Suit;
		const mate = ({ S: 'C', C: 'S', H: 'D', D: 'H' } as const)[down];
		const cross = SUITS.filter((x) => x !== down && x !== mate);

		expect(sayForBid(s, 1, { t: 'call', suit: mate, alone: false })).toBe('Next.');
		expect(sayForBid(s, 1, { t: 'call', suit: mate, alone: true })).toBe('Next. Alone.');
		for (const c of cross) {
			expect(sayForBid(s, 1, { t: 'call', suit: c, alone: false })).toBe('Crossing the creek.');
		}
		const t = apply(s, 1, { t: 'call', suit: mate, alone: false }).state;
		expect(t.hand.bids[4]).toEqual({
			seat: 1,
			move: { t: 'call', suit: mate, alone: false },
			say: 'Next.'
		});
		expect(t.hand.bids).toHaveLength(5);
	});

	it('bounds the auction at eight records over both rounds (V18)', () => {
		const s = round2('b2-f', 3);
		const down = s.hand.turnedDownSuit as Suit;
		const suit = SUITS.find((x) => x !== down) as Suit;
		const t = apply(s, 0, { t: 'call', suit, alone: false }).state;
		expect(t.hand.bids).toHaveLength(8);
		expect(t.hand.bids.filter((b) => b.move.t === 'pass')).toHaveLength(7);
	});
});

describe('stick the dealer', () => {
	const stuck = (seed: string): GameState => passes(afterCut(opened(seed, 0)), 7);

	it('removes pass from the dealer legal set at the third round-2 pass (V3)', () => {
		const s = stuck('std-a');
		expect(s.hand.phase).toBe('bid_round_2');
		expect(s.hand.turnSeat).toBe(0);
		expect(s.hand.passes).toBe(3);
		expect(isStuckDealer(s, 0)).toBe(true);
		expect(isStuckDealer(s, 1)).toBe(false);

		const ids = legalMoveIds(s, 0);
		expect(ids).toHaveLength(6);
		expect(ids).not.toContain('pass');
		expect(ids.every((id) => id.startsWith('call:'))).toBe(true);
	});

	it('rejects the dealer pass with dealer_must_call, never bidding_closed', () => {
		const s = stuck('std-b');
		const before = snap(s);
		const err = expectRule('dealer_must_call', () => apply(s, 0, { t: 'pass' }));
		expect(err.code).not.toBe('bidding_closed');
		expect(err.legal.map((m) => m.id)).toEqual(legalMoveIds(s, 0));
		expect(err.legal).toHaveLength(6);
		expect(s).toEqual(before);
	});

	it('accepts the forced call and speaks the stuck line', () => {
		const s = stuck('std-c');
		const down = s.hand.turnedDownSuit as Suit;
		const mate = ({ S: 'C', C: 'S', H: 'D', D: 'H' } as const)[down];
		// Even the colour mate is "I guess" from a stuck dealer, never "Next."
		expect(sayForBid(s, 0, { t: 'call', suit: mate, alone: false })).toMatch(/, I guess\.$/);

		const suit = SUITS.find((x) => x !== down) as Suit;
		const res = apply(s, 0, { t: 'call', suit, alone: false });
		expect(res.state.hand.phase).toBe('trick_play');
		expect(res.state.hand.trump).toBe(suit);
		expect(res.state.hand.makerSeat).toBe(0);
		expect(res.state.hand.turnSeat).toBe(eldestOf(0));
		expect(res.state.hand.bids[7]?.say).toBe(sayForBid(s, 0, { t: 'call', suit, alone: false }));
	});

	it('with stickTheDealer off the dealer may pass, and the hand is thrown in', () => {
		const s = passes(afterCut(opened('std-d', 0, { stickTheDealer: false })), 7);
		expect(isStuckDealer(s, 0)).toBe(false);
		expect(legalMoveIds(s, 0)).toHaveLength(7);
		expect(legalMoveIds(s, 0)).toContain('pass');

		const res = apply(s, 0, { t: 'pass' });
		const t = res.state;
		expect(res.steps.map((x) => x.t)).toEqual(['bid', 'throwIn']);
		expect(t.score).toEqual([0, 0]);
		expect(t.handNo).toBe(1);
		expect(t.hand.handNo).toBe(1);
		expect(t.hand.dealerSeat).toBe(nextSeat(0));
		expect(t.misdealStreak).toBe(1);
		expect(t.hand.phase).toBe('cutting');
		expect(t.hand.turnSeat).toBe(cutterOf(nextSeat(0)));
		expect(t.hand.deckOrder).toEqual(shuffleForHand('std-d', 1));
		expect(t.hand.bids).toEqual([]);
		expect(t.hand.trump).toBeNull();
		expect(t.hand.upCard).toBeNull();
		expect(t.winnerTeam).toBeNull();
		expect(t.status).toBe('active');
	});

	it('caps consecutive throw-ins at misdealLimit while still moving the deal left', () => {
		const cfg: Partial<EngineConfig> = { stickTheDealer: false, misdealLimit: 3 };
		let s = opened('std-e', 0, cfg);
		const streaks: number[] = [];
		const dealers: Seat[] = [];
		for (let i = 0; i < 5; i++) {
			s = passes(afterCut(s), 8);
			streaks.push(s.misdealStreak);
			dealers.push(s.hand.dealerSeat);
		}
		expect(streaks).toEqual([1, 2, 3, 3, 3]);
		expect(dealers).toEqual([1, 2, 3, 0, 1]);
		expect(s.handNo).toBe(5);
		expect(s.score).toEqual([0, 0]);
	});
});

/* -------------------------------------------------------------------------- */
/* trick_play — follow suit, resolution, and the trick_resolve hold            */
/* -------------------------------------------------------------------------- */

/**
 * A hand-patched `trick_play` state: hearts trump, seat 1 has led the ace of
 * hearts, seat 2 holds the left bower `JD` and four clubs.
 */
function bowerFixture(): GameState {
	const base = afterCut(opened('bower', 0));
	return patchHand(base, {
		phase: 'trick_play',
		turnSeat: 2,
		trump: 'H',
		makerSeat: 1,
		aloneSeat: null,
		sittingSeat: null,
		hands: {
			0: ['9S', 'TS', 'QS', 'KS', 'AS'],
			1: ['9H', 'TH', 'QH', 'KH'],
			2: ['JD', '9C', 'TC', 'KC', 'AC'],
			3: ['9D', 'TD', 'QD', 'KD', 'AD']
		},
		trick: { index: 0, ledSuit: 'H', plays: [{ seat: 1, card: 'AH' }], winnerSeat: null },
		trickLog: [],
		tricksWon: [0, 0]
	});
}

describe('trick_play', () => {
	it('makes the left bower the only legal follow to a trump lead', () => {
		const s = bowerFixture();
		expect(effectiveSuit('JD', 'H')).toBe('H');
		expect(legalMoveIds(s, 2)).toEqual(['play:JD']);
		expect(legalMoves(s, 2)[0]?.label).toBe('Jack of diamonds');
	});

	it('rejects a renege with must_follow_suit and returns the real legal set', () => {
		const s = bowerFixture();
		const before = snap(s);
		const err = expectRule('must_follow_suit', () => apply(s, 2, { t: 'play', card: '9C' }));
		expect(err.legal.map((m) => m.id)).toEqual(['play:JD']);
		expect(s).toEqual(before);
	});

	it('rejects a card the seat does not hold before it checks follow-suit', () => {
		const s = bowerFixture();
		expectRule('card_not_in_hand', () => apply(s, 2, { t: 'play', card: 'AS' }));
		expectRule('card_not_in_hand', () => apply(s, 2, { t: 'play', card: 'AH' }));
	});

	it('rejects bids as bidding_closed and other kinds as wrong_phase', () => {
		const s = bowerFixture();
		expectRule('bidding_closed', () => apply(s, 2, { t: 'pass' }));
		expectRule('bidding_closed', () => apply(s, 2, { t: 'call', suit: 'S', alone: false }));
		expectRule('wrong_phase', () => apply(s, 2, { t: 'discard', card: 'JD' }));
		expectRule('wrong_phase', () => apply(s, 2, { t: 'cut', cut: true }));
	});

	it('resolves the trick to the left bower and parks in trick_resolve', () => {
		let s = bowerFixture();
		s = apply(s, 2, { t: 'play', card: 'JD' }).state;
		expect(s.hand.turnSeat).toBe(3);
		expect(s.hand.trick.plays).toHaveLength(2);
		expect(s.hand.trick.ledSuit).toBe('H');
		expect(s.hand.hands[2]).toEqual(['9C', 'TC', 'KC', 'AC']);

		// Seat 3 is void in hearts (no jack of clubs), so anything goes.
		expect(legalMoveIds(s, 3)).toHaveLength(5);
		s = apply(s, 3, { t: 'play', card: 'AD' }).state;
		s = apply(s, 0, { t: 'play', card: 'AS' }).state;

		expect(s.hand.phase).toBe('trick_resolve');
		expect(s.hand.turnSeat).toBeNull();
		expect(s.hand.trick.winnerSeat).toBe(2);
		expect(s.hand.trick.plays.map((p) => p.card)).toEqual(['AH', 'JD', 'AD', 'AS']);
		expect(s.hand.trickLog).toHaveLength(1);
		expect(s.hand.tricksWon).toEqual([1, 0]);
		expect(legalMoves(s, 2)).toEqual([]);
	});

	it('emits trickWon with sealsEuchre only when the defenders reach three', () => {
		let s = bowerFixture();
		s = apply(s, 2, { t: 'play', card: 'JD' }).state;
		s = apply(s, 3, { t: 'play', card: 'AD' }).state;
		const res = apply(s, 0, { t: 'play', card: 'AS' });
		expect(res.steps.map((x) => x.t)).toEqual(['cardPlayed', 'trickWon']);
		expect(res.steps[1]).toEqual({
			t: 'trickWon',
			index: 0,
			winnerSeat: 2,
			plays: res.state.hand.trick.plays,
			sealsEuchre: false
		});

		// Same trick, but the defenders already hold two: this one seals the euchre.
		const primed = patchHand(s, { tricksWon: [2, 0] });
		const sealed = apply(primed, 0, { t: 'play', card: 'AS' });
		const won = sealed.steps[1] as Extract<Step, { t: 'trickWon' }>;
		expect(sealed.state.hand.tricksWon).toEqual([3, 0]);
		expect(teamOf(sealed.state.hand.makerSeat as Seat)).toBe(1);
		expect(won.sealsEuchre).toBe(true);
	});

	it('advance() releases the hold and gives the lead to the trick winner', () => {
		let s = bowerFixture();
		s = apply(s, 2, { t: 'play', card: 'JD' }).state;
		s = apply(s, 3, { t: 'play', card: 'AD' }).state;
		s = apply(s, 0, { t: 'play', card: 'AS' }).state;

		const res = advance(s);
		expect(res.steps).toEqual([]);
		expect(res.state.hand.phase).toBe('trick_play');
		expect(res.state.hand.turnSeat).toBe(2);
		expect(res.state.hand.trick).toEqual({
			index: 1,
			ledSuit: null,
			plays: [],
			winnerSeat: null
		});
		expect(res.state.hand.trickLog).toHaveLength(1);
		expect(legalMoveIds(res.state, 2)).toEqual(['play:9C', 'play:TC', 'play:KC', 'play:AC']);
	});
});

/* -------------------------------------------------------------------------- */
/* advance() totality                                                          */
/* -------------------------------------------------------------------------- */

describe('advance', () => {
	it('is the identity in every phase that has a seat to act', () => {
		const cutting = opened('adv-a', 0);
		const bidding = afterCut(cutting);
		const discard = apply(bidding, 1, { t: 'orderUp', alone: false }).state;
		const round2 = passes(bidding, 4);
		const play = apply(discard, 0, {
			t: 'discard',
			card: discard.hand.hands[0][0] as CardId
		}).state;

		for (const s of [cutting, bidding, discard, round2, play]) {
			const res = advance(s);
			expect(res.state).toBe(s);
			expect(res.steps).toEqual([]);
		}
	});

	it('is the identity once the match is complete', () => {
		const final = runGame(newGame('adv-b', { gameTo: 5 }), randomPicker('adv-b'));
		expect(final.status).toBe('complete');
		expect(final.hand.phase).toBe('game_over');
		const res = advance(final);
		expect(res.state).toBe(final);
		expect(res.steps).toEqual([]);
	});

	it('scores the hand after the fifth trick, then opens the next one', () => {
		// Play a whole hand, stopping at each engine-held phase to inspect it.
		let s = apply(afterCut(opened('adv-score', 0)), 1, { t: 'orderUp', alone: false }).state;
		s = apply(s, 0, { t: 'discard', card: s.hand.hands[0][0] as CardId }).state;
		const pick = randomPicker('adv-score');
		const releases: Array<{ from: number; to: string; steps: string[] }> = [];
		while (s.hand.phase !== 'hand_score') {
			if (s.hand.turnSeat === null) {
				expect(s.hand.phase).toBe('trick_resolve');
				expect(s.hand.trick.winnerSeat).not.toBeNull();
				const res = advance(s);
				releases.push({
					from: s.hand.trick.index,
					to: res.state.hand.phase,
					steps: res.steps.map((x) => x.t)
				});
				s = res.state;
			} else {
				s = apply(
					s,
					s.hand.turnSeat,
					pick(s, s.hand.turnSeat, legalMoves(s, s.hand.turnSeat)).move
				).state;
			}
		}
		// Four resolves hand the lead to the winner; the fifth scores the hand.
		expect(releases.map((r) => r.from)).toEqual([0, 1, 2, 3, 4]);
		expect(releases.map((r) => r.to)).toEqual([
			'trick_play',
			'trick_play',
			'trick_play',
			'trick_play',
			'hand_score'
		]);
		expect(releases.slice(0, 4).every((r) => r.steps.length === 0)).toBe(true);
		expect(releases[4]?.steps).toEqual(['handScored']);

		// hand_score: the recap is parked, the fifth trick is still in `trick`.
		expect(s.hand.turnSeat).toBeNull();
		expect(s.hand.trickLog).toHaveLength(5);
		expect(s.hand.trick).toEqual(s.hand.trickLog[4]);
		expect(s.hand.result).not.toBeNull();
		expect(s.hand.delta).not.toBeNull();
		expect(s.score).toEqual([
			(s.hand.delta as readonly [number, number])[0],
			(s.hand.delta as readonly [number, number])[1]
		]);
		expect(s.hand.tricksWon[0] + s.hand.tricksWon[1]).toBe(5);
		for (const seat of SEATS_ALL) expect(legalMoves(s, seat)).toEqual([]);

		// The recap releases into the next hand: deal left, streak cleared.
		const opened2 = advance(s);
		expect(opened2.steps).toEqual([]);
		const t = opened2.state;
		expect(t.hand.phase).toBe('cutting');
		expect(t.handNo).toBe(1);
		expect(t.hand.handNo).toBe(1);
		expect(t.hand.dealerSeat).toBe(nextSeat(0));
		expect(t.hand.turnSeat).toBe(cutterOf(nextSeat(0)));
		expect(t.misdealStreak).toBe(0);
		expect(t.score).toEqual(s.score);
		expect(t.hand.result).toBeNull();
		expect(t.hand.trickLog).toEqual([]);
		expect(t.hand.trump).toBeNull();
		expect(t.hand.deckOrder).toEqual(shuffleForHand('adv-score', 1));
	});

	it('emits handScored with the table row, and gameWon when the score is reached', () => {
		let scored: Extract<Step, { t: 'handScored' }> | null = null;
		let won: Extract<Step, { t: 'gameWon' }> | null = null;
		let handScoreSeen = 0;
		const final = runGame(newGame('adv-win', { gameTo: 5 }), randomPicker('adv-win'), {
			onTransition: (before, res) => {
				for (const st of res.steps) {
					if (st.t === 'handScored') {
						scored = st;
						expect(st.makerSeat).toBe(before.hand.makerSeat);
						expect(st.aloneSeat).toBe(before.hand.aloneSeat);
						expect(st.tricksWon).toEqual(before.hand.tricksWon);
						expect(st.score).toEqual(res.state.score);
						expect(st.delta).toEqual(res.state.hand.delta);
						handScoreSeen++;
					}
					if (st.t === 'gameWon') won = st;
				}
			}
		});
		expect(handScoreSeen).toBeGreaterThanOrEqual(2);
		expect(scored).not.toBeNull();
		expect(won).not.toBeNull();
		expect((won as unknown as Extract<Step, { t: 'gameWon' }>).team).toBe(final.winnerTeam);
		expect((won as unknown as Extract<Step, { t: 'gameWon' }>).score).toEqual(final.score);
		expect(final.status).toBe('complete');
		expect(final.hand.phase).toBe('game_over');
	});

	it('re-deals defensively from a bare deal phase', () => {
		const cut = opened('adv-c', 0);
		const forced = patchHand(cut, { phase: 'deal', turnSeat: null, cut: false });
		const res = advance(forced);
		expect(res.state.hand.phase).toBe('bid_round_1');
		expect(res.state.hand.turnSeat).toBe(eldestOf(0));
		expect(res.steps.map((x) => x.t)).toEqual(['dealt', 'upCardTurned']);
		expect(res.state.hand.upCard).toBe(cut.hand.deckOrder[20]);
	});
});

/* -------------------------------------------------------------------------- */
/* Seat mismatch and rejection atomicity                                       */
/* -------------------------------------------------------------------------- */

describe('the acting seat is validated, never inferred', () => {
	it('rejects every seat that is not turnSeat, in every phase with a turn', () => {
		const states: GameState[] = [];
		const cutting = opened('mismatch', 0);
		states.push(cutting);
		const bidding = afterCut(cutting);
		states.push(bidding);
		const discard = apply(bidding, 1, { t: 'orderUp', alone: false }).state;
		states.push(discard);
		states.push(passes(bidding, 4));
		states.push(
			apply(discard, 0, { t: 'discard', card: discard.hand.hands[0][0] as CardId }).state
		);

		for (const s of states) {
			const on = turn(s);
			const before = snap(s);
			for (const seat of SEATS_ALL) {
				if (seat === on) continue;
				// The legal move *for the seat on turn* — offered to the wrong seat.
				const move = legalMoves(s, on)[0]?.move as Move;
				const err = expectRule('not_your_turn', () => apply(s, seat, move));
				expect(err.legal).toEqual([]);
			}
			expect(s).toEqual(before);
			// And the correct seat still works: nothing was half-applied.
			expect(() => apply(s, on, legalMoves(s, on)[0]?.move as Move)).not.toThrow();
		}
	});

	it('rejects a stale turn nonce without touching the state', () => {
		const s = afterCut(opened('stale', 0));
		const before = snap(s);
		expectRule('stale_turn', () =>
			apply(s, 1, { t: 'pass' }, { turnId: `${s.turnId}-not-current` })
		);
		expect(s).toEqual(before);
		// The current nonce is accepted, and the next state carries a fresh one.
		const ok = apply(s, 1, { t: 'pass' }, { turnId: s.turnId }).state;
		expect(ok.hand.passes).toBe(1);
		expect(ok.turnId).not.toBe(s.turnId);
		expectRule('stale_turn', () => apply(ok, 2, { t: 'pass' }, { turnId: s.turnId }));
	});

	it('rejects everything once the match is complete', () => {
		const final = runGame(newGame('done', { gameTo: 5 }), randomPicker('done'));
		const before = snap(final);
		for (const seat of SEATS_ALL) {
			expectRule('wrong_phase', () => apply(final, seat, { t: 'pass' }));
			expect(legalMoves(final, seat)).toEqual([]);
		}
		expect(final).toEqual(before);
	});

	it('leaves the state byte-identical after a whole battery of rejections', () => {
		const s = passes(afterCut(opened('atomic', 0)), 7); // the stuck dealer
		const before = snap(s);
		const down = s.hand.turnedDownSuit as Suit;
		expectRule('dealer_must_call', () => apply(s, 0, { t: 'pass' }));
		expectRule('illegal_move', () => apply(s, 0, { t: 'call', suit: down, alone: false }));
		expectRule('wrong_phase', () => apply(s, 0, { t: 'orderUp', alone: false }));
		expectRule('not_your_turn', () => apply(s, 2, { t: 'call', suit: down, alone: false }));
		expectRule('stale_turn', () => apply(s, 0, { t: 'pass' }, { turnId: 'x' }));
		expect(s).toEqual(before);
	});
});

/* -------------------------------------------------------------------------- */
/* moveId is the exact inverse of the legal set                                */
/* -------------------------------------------------------------------------- */

describe('moveId', () => {
	it('round-trips every offered move over twenty whole matches', () => {
		let checked = 0;
		const kinds = new Set<string>();
		const mismatches: string[] = [];
		for (let i = 0; i < 20; i++) {
			const seed = `moveid-${i}`;
			runGame(newGame(seed), randomPicker(seed), {
				onState: (s) => {
					if (s.hand.turnSeat === null) return;
					for (const m of legalMoves(s, s.hand.turnSeat)) {
						if (moveId(m.move) !== m.id) mismatches.push(`${m.id} !== ${moveId(m.move)}`);
						checked++;
						kinds.add(m.move.t);
					}
				}
			});
		}
		expect(mismatches).toEqual([]);
		expect(checked).toBeGreaterThan(5000);
		expect([...kinds].sort()).toEqual(['call', 'cut', 'discard', 'orderUp', 'pass', 'play']);
	});

	it('is total over the move union', () => {
		expect(moveId({ t: 'pass' })).toBe('pass');
		expect(moveId({ t: 'orderUp', alone: false })).toBe('orderUp');
		expect(moveId({ t: 'orderUp', alone: true })).toBe('orderUp+alone');
		expect(moveId({ t: 'call', suit: 'H', alone: false })).toBe('call:H');
		expect(moveId({ t: 'call', suit: 'H', alone: true })).toBe('call:H+alone');
		expect(moveId({ t: 'cut', cut: true })).toBe('cut:yes');
		expect(moveId({ t: 'cut', cut: false })).toBe('cut:no');
		expect(moveId({ t: 'discard', card: '9C' })).toBe('discard:9C');
		expect(moveId({ t: 'play', card: 'JS' })).toBe('play:JS');
	});
});

/* -------------------------------------------------------------------------- */
/* V20 — determinism                                                           */
/* -------------------------------------------------------------------------- */

describe('determinism (V20)', () => {
	it('the same seed produces an identical deal', () => {
		const a = newGame('same');
		const b = newGame('same');
		expect(a).toEqual(b);
		expect(a.hand.deckOrder).toEqual(b.hand.deckOrder);
		expect(a.internalToken).toBe(b.internalToken);

		const dealtA = afterCut(advance(a).state, true);
		const dealtB = afterCut(advance(b).state, true);
		expect(dealtA.hand.hands).toEqual(dealtB.hand.hands);
		expect(dealtA.hand.kitty).toEqual(dealtB.hand.kitty);
		expect(dealtA.hand.upCard).toBe(dealtB.hand.upCard);

		const other = newGame('different');
		expect(other.hand.deckOrder).not.toEqual(a.hand.deckOrder);
	});

	it('replays an identical full game from the same seed and move sequence', () => {
		const seed = 'replay-seed';
		const recorded: Array<{ seat: Seat; move: Move }> = [];
		const traceA: string[] = [];

		const finalA = runGame(newGame(seed), randomPicker(seed), {
			onState: (s) => traceA.push(JSON.stringify(s))
		});

		// A second, independent run: same seed, a freshly rebuilt picker stream.
		// This is the journal a replay would be reconstructed from.
		let s = newGame(seed);
		const pickA = randomPicker(seed);
		while (s.status === 'active') {
			const seat = s.hand.turnSeat;
			if (seat === null) {
				s = advance(s).state;
				continue;
			}
			const move = pickA(s, seat, legalMoves(s, seat)).move;
			recorded.push({ seat, move });
			s = apply(s, seat, move).state;
		}
		expect(recorded.length).toBeGreaterThan(200);

		// Replay from the journal alone: { seed, moves[] }.
		const traceB: string[] = [];
		let t = newGame(seed);
		let i = 0;
		traceB.push(JSON.stringify(t));
		while (t.status === 'active') {
			const seat = t.hand.turnSeat;
			if (seat === null) {
				t = advance(t).state;
			} else {
				const rec = recorded[i++];
				expect(rec?.seat).toBe(seat);
				expect(legalMoveIds(t, seat)).toContain(moveId((rec as { move: Move }).move));
				t = apply(t, seat, (rec as { move: Move }).move).state;
			}
			traceB.push(JSON.stringify(t));
		}

		expect(i).toBe(recorded.length);
		expect(traceB).toEqual(traceA);
		expect(t).toEqual(finalA);
		expect(t.status).toBe('complete');
	});

	it('mints turn nonces from a hash chain, so every state is reproducible', () => {
		const a = runGame(newGame('nonce', { gameTo: 5 }), randomPicker('nonce'));
		const b = runGame(newGame('nonce', { gameTo: 5 }), randomPicker('nonce'));
		expect(a.turnId).toBe(b.turnId);
		expect(a.turnId).toMatch(/^[0-9a-z]+$/);
		expect(a).toEqual(b);
	});
});

/* -------------------------------------------------------------------------- */
/* V21 — purity                                                                */
/* -------------------------------------------------------------------------- */

describe('purity (V21)', () => {
	it('never mutates a deep-frozen state, across a whole match', () => {
		let states = 0;
		const final = runGame(deepFreeze(newGame('frozen')), randomPicker('frozen'), {
			onState: (s) => {
				deepFreeze(s);
				states++;
			}
		});
		expect(states).toBeGreaterThan(200);
		expect(final.status).toBe('complete');
		expect(Object.isFrozen(final)).toBe(true);
	});

	it('legalMoves and project hand back fresh arrays every call', () => {
		const s = afterCut(opened('fresh', 0));
		const a = legalMoves(s, 1);
		const b = legalMoves(s, 1);
		expect(a).toEqual(b);
		expect(a).not.toBe(b);

		const va = project(s, 1);
		const vb = project(s, 1);
		expect(va).toEqual(vb);
		expect(va.hand).not.toBe(vb.hand);
		expect(va.hand).not.toBe(s.hand.hands[1]);
		expect(va.trickLog).not.toBe(s.hand.trickLog);
	});
});
