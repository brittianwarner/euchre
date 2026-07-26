/**
 * `project.test.ts` — the security boundary.
 *
 * `project()` and `projectSteps()` are the only two functions whose output ever
 * leaves the server. This file exists to prove that no `CardId` a seat is not
 * entitled to can appear in either of them, in any state reachable in a real
 * match — invariants **V8, V12, V13, V14, V19** of `docs/02-GAME-RULES-ENGINE.md`
 * § 8.
 *
 * The central test is a whole-match sweep: every reachable state, every seat,
 * serialise the view, and scan the bytes for the *actual* hidden `CardId`s. It is
 * deliberately written as a complement test — a card is legitimate for a seat iff
 * it is in that seat's own hand, is the up-card, or has been played face up —
 * because that phrasing cannot be weakened by adding a new field to the view.
 */

import { describe, expect, it } from 'vitest';
import {
	DECK,
	advance,
	apply,
	createGame,
	cutterOf,
	eldestOf,
	fnv1a,
	legalMoves,
	mulberry32,
	newGame,
	partnerOf,
	project,
	projectSteps,
	type ApplyResult,
	type CardId,
	type EngineConfig,
	type GameState,
	type LegalMove,
	type PublicGameView,
	type Seat,
	type Step
} from '../index';

/* -------------------------------------------------------------------------- */
/* Helpers                                                                     */
/* -------------------------------------------------------------------------- */

const SEATS_ALL: readonly Seat[] = [0, 1, 2, 3];

/**
 * Every `CardId`-shaped token in a serialised payload, wherever it sits — a bare
 * `"9C"`, a `play:JS` move id, a `discard:AH`.
 *
 * The engine's other strings cannot produce a false positive: card ids are
 * `[9TJQKA][SHDC]`, every generated label and spoken line is title case with a
 * lowercase remainder (`"Spades"`, `"Jack of diamonds"`, `"Turn it down."`), and
 * `turnId` is lowercase base-36. This is asserted below in its own test so the
 * scanner cannot silently rot.
 */
const CARD_TOKEN = /[9TJQKA][SHDC]/g;

function cardTokensIn(json: string): Set<CardId> {
	const out = new Set<CardId>();
	for (const m of json.matchAll(CARD_TOKEN)) out.add(m[0] as CardId);
	return out;
}

/** The cards `seat` is legitimately entitled to know at this instant. */
function visibleTo(state: GameState, seat: Seat): Set<CardId> {
	const h = state.hand;
	const vis = new Set<CardId>();
	// Own cards — but the loner's sitting partner is entitled to nothing (V8).
	if (seat !== h.sittingSeat) for (const c of h.hands[seat]) vis.add(c);
	// The up-card is public for the whole hand (V14).
	if (h.upCard !== null) vis.add(h.upCard);
	// Everything laid face up.
	for (const p of h.trick.plays) vis.add(p.card);
	for (const t of h.trickLog) for (const p of t.plays) vis.add(p.card);
	return vis;
}

const hiddenFrom = (state: GameState, seat: Seat): CardId[] => {
	const vis = visibleTo(state, seat);
	return DECK.filter((c) => !vis.has(c));
};

type Picker = (state: GameState, seat: Seat, legal: readonly LegalMove[]) => LegalMove;

function randomPicker(seed: string): Picker {
	const rnd = mulberry32(fnv1a(seed));
	return (_s, _seat, legal) => legal[Math.floor(rnd() * legal.length)] as LegalMove;
}

/** Prefers `+alone` bids, so every hand in the match is played as a loner. */
function alonePicker(seed: string): Picker {
	const fallback = randomPicker(seed);
	return (state, seat, legal) => {
		const alone = legal.find((m) => m.id.endsWith('+alone'));
		return alone ?? fallback(state, seat, legal);
	};
}

/**
 * Passes exactly `skip` times in round 1 before ordering up alone, which fixes
 * *which* seat goes alone and therefore which seat sits out.
 */
function aloneAtPicker(seed: string, skip: number): Picker {
	const fallback = randomPicker(`${seed}#${skip}`);
	return (state, seat, legal) => {
		if (state.hand.phase === 'bid_round_1') {
			if (state.hand.passes < skip) return legal.find((m) => m.id === 'pass') as LegalMove;
			return legal.find((m) => m.id === 'orderUp+alone') as LegalMove;
		}
		return fallback(state, seat, legal);
	};
}

interface RunHooks {
	readonly onState?: (s: GameState) => void;
	readonly onTransition?: (before: GameState, res: ApplyResult, seat: Seat | null) => void;
	readonly budget?: number;
}

function runGame(initial: GameState, pick: Picker, hooks: RunHooks = {}): GameState {
	const budget = hooks.budget ?? 20000;
	let state = initial;
	let n = 0;
	hooks.onState?.(state);
	while (state.status === 'active') {
		if (++n > budget) throw new Error(`runGame: budget exhausted in ${state.hand.phase}`);
		const seat = state.hand.turnSeat;
		const res =
			seat === null
				? advance(state)
				: apply(state, seat, pick(state, seat, legalMoves(state, seat)).move);
		if (res.state === state) throw new Error(`runGame: stalled in ${state.hand.phase}`);
		hooks.onTransition?.(state, res, seat);
		state = res.state;
		hooks.onState?.(state);
	}
	return state;
}

function deepFreeze<T>(o: T, seen: Set<unknown> = new Set()): T {
	if (o === null || typeof o !== 'object') return o;
	if (seen.has(o)) return o;
	seen.add(o);
	Object.freeze(o);
	for (const v of Object.values(o as Record<string, unknown>)) deepFreeze(v, seen);
	return o;
}

function opened(seed: string, firstDealer?: Seat, cfg?: Partial<EngineConfig>): GameState {
	return advance(createGame({ gameId: `g-${seed}`, seed, cfg, firstDealer })).state;
}

const turn = (s: GameState): Seat => s.hand.turnSeat as Seat;

/* -------------------------------------------------------------------------- */
/* The scanner itself must be trustworthy                                      */
/* -------------------------------------------------------------------------- */

describe('the card-id scanner', () => {
	it('finds every card id, wherever it is embedded', () => {
		expect([...cardTokensIn('{"hand":["9C","JD"],"legal":[{"id":"play:AS"}]}')].sort()).toEqual([
			'9C',
			'AS',
			'JD'
		]);
		expect([...cardTokensIn('"discard:TH"')]).toEqual(['TH']);
	});

	it('does not fire on any string the engine can generate', () => {
		const copy = [
			'Bump',
			"Run 'em",
			'Pass',
			'Pass.',
			'Turn it down.',
			'Order it up',
			'Order it up, alone',
			'Order it up. Alone.',
			'I assist',
			'I assist.',
			'I take it',
			'I take it.',
			'Next.',
			'Next. Alone.',
			'Crossing the creek.',
			'Spades',
			'Hearts',
			'Diamonds',
			'Clubs',
			'Spades, alone',
			'Spades, I guess.',
			'Nine of spades',
			'Ten of hearts',
			'Jack of diamonds',
			'Queen of clubs',
			'King of spades',
			'Ace of diamonds',
			'lobby',
			'cutting',
			'bid_round_1',
			'dealer_discard',
			'bid_round_2',
			'trick_play',
			'trick_resolve',
			'hand_score',
			'game_over',
			'lone_march',
			'throw_in',
			'active',
			'complete'
		];
		for (const s of copy) expect([...cardTokensIn(JSON.stringify(s))]).toEqual([]);
	});

	it('does not fire on a turn nonce, which is lowercase base-36', () => {
		let s = newGame('nonce-scan');
		const ids: string[] = [];
		for (let i = 0; i < 40 && s.status === 'active'; i++) {
			ids.push(s.turnId);
			const seat = s.hand.turnSeat;
			s =
				seat === null
					? advance(s).state
					: apply(s, seat, legalMoves(s, seat)[0]?.move as never).state;
		}
		expect(ids.length).toBeGreaterThan(20);
		for (const id of ids) {
			expect(id).toMatch(/^[0-9a-z]+$/);
			expect([...cardTokensIn(id)]).toEqual([]);
		}
	});
});

/* -------------------------------------------------------------------------- */
/* Shape and basic contract                                                    */
/* -------------------------------------------------------------------------- */

describe('project — shape', () => {
	it('reports the seat it was cut for and mirrors the public state', () => {
		const s = apply(opened('shape', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		for (const seat of SEATS_ALL) {
			const v = project(s, seat);
			expect(v.you).toBe(seat);
			expect(v.v).toBe(s.v);
			expect(v.turnId).toBe(s.turnId);
			expect(v.status).toBe('active');
			expect(v.phase).toBe('bid_round_1');
			expect(v.handNo).toBe(0);
			expect(v.dealerSeat).toBe(0);
			expect(v.turnSeat).toBe(eldestOf(0));
			expect(v.score).toEqual([0, 0]);
			expect(v.hand).toEqual(s.hand.hands[seat]);
			expect(v.handCounts).toEqual([5, 5, 5, 5]);
			expect(v.kittyCount).toBe(3);
			expect(v.upCard).toBe(s.hand.upCard);
			expect(v.trump).toBeNull();
			expect(v.result).toBeNull();
			expect(v.delta).toBeNull();
			expect(v.winnerTeam).toBeNull();
		}
	});

	it('offers a legal set only to the seat on turn, and never an empty one (V19)', () => {
		let checkedOnTurn = 0;
		let checkedOff = 0;
		runGame(newGame('legal-field', { gameTo: 10 }), randomPicker('legal-field'), {
			onState: (s) => {
				for (const seat of SEATS_ALL) {
					const v = project(s, seat);
					if (s.hand.turnSeat === seat && s.status === 'active') {
						expect(v.legal.length).toBeGreaterThan(0);
						expect(v.legal.map((m) => m.id)).toEqual(legalMoves(s, seat).map((m) => m.id));
						checkedOnTurn++;
					} else {
						expect(v.legal).toEqual([]);
						checkedOff++;
					}
				}
			}
		});
		expect(checkedOnTurn).toBeGreaterThan(200);
		expect(checkedOff).toBeGreaterThan(600);
	});

	it('never mutates a deep-frozen state', () => {
		const s = deepFreeze(
			apply(opened('frozen-proj', 0), cutterOf(0), { t: 'cut', cut: false }).state
		);
		for (const seat of SEATS_ALL) {
			const v = project(s, seat);
			expect(v.hand).toHaveLength(5);
			expect(Object.isFrozen(v.hand)).toBe(false);
		}
	});
});

/* -------------------------------------------------------------------------- */
/* V14 — the up-card is never nulled                                           */
/* -------------------------------------------------------------------------- */

describe('up-card persistence (V14)', () => {
	it('stays identified from the turn until the hand ends, in both branches', () => {
		const live = new Set([
			'bid_round_1',
			'dealer_discard',
			'bid_round_2',
			'trick_play',
			'trick_resolve',
			'hand_score'
		]);
		let checked = 0;
		for (const pick of [randomPicker('up-a'), alonePicker('up-b')]) {
			runGame(newGame('upcard', { gameTo: 10 }), pick, {
				onState: (s) => {
					if (!live.has(s.hand.phase)) return;
					for (const seat of SEATS_ALL) {
						const v = project(s, seat);
						expect(v.upCard).not.toBeNull();
						expect(v.upCard).toBe(s.hand.upCard);
						checked++;
					}
				}
			});
		}
		expect(checked).toBeGreaterThan(1000);
	});

	it('survives the dealer discarding the up-card itself', () => {
		const base = apply(opened('up-discard', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		const upCard = base.hand.upCard as CardId;
		const picked = apply(base, 1, { t: 'orderUp', alone: false }).state;
		const done = apply(picked, 0, { t: 'discard', card: upCard }).state;

		expect(done.hand.dealerDiscard).toBe(upCard);
		for (const seat of SEATS_ALL) {
			const v = project(done, seat);
			expect(v.upCard).toBe(upCard);
			expect(v.upCardTurnedDown).toBe(false);
			expect(v.kittyCount).toBe(4);
		}
		// And the discard is not inferable from the dealer's own hand either.
		expect(project(done, 0).hand).not.toContain(upCard);
	});
});

/* -------------------------------------------------------------------------- */
/* kittyCount and V13 — discard non-inference                                  */
/* -------------------------------------------------------------------------- */

describe('kittyCount (V13)', () => {
	it('is 3 before the up-card leaves the table and 4 after, in both branches', () => {
		const base = apply(opened('kitty', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		expect(project(base, 1).kittyCount).toBe(3);

		// Branch A: turned down. The up-card is buried.
		let turnedDown = base;
		for (let i = 0; i < 4; i++) {
			turnedDown = apply(turnedDown, turn(turnedDown), { t: 'pass' }).state;
		}
		expect(turnedDown.hand.upCardTurnedDown).toBe(true);
		expect(turnedDown.hand.dealerDiscard).toBeNull();
		expect(project(turnedDown, 1).kittyCount).toBe(4);

		// Branch B: picked up. Still 3 until the discard actually lands.
		const picked = apply(base, 1, { t: 'orderUp', alone: false }).state;
		expect(picked.hand.dealerDiscard).toBeNull();
		expect(project(picked, 1).kittyCount).toBe(3);
		const discarded = apply(picked, 0, {
			t: 'discard',
			card: picked.hand.hands[0][0] as CardId
		}).state;
		expect(project(discarded, 1).kittyCount).toBe(4);
	});

	it('two states differing only in the discard project identically (V13)', () => {
		const base = apply(opened('v13', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		const picked = apply(base, 1, { t: 'orderUp', alone: false }).state;
		const six = picked.hand.hands[0];
		expect(six).toHaveLength(6);

		const outcomes = six.map((card) => apply(picked, 0, { t: 'discard', card }).state);
		const reference = outcomes[0] as GameState;

		for (let i = 1; i < outcomes.length; i++) {
			const other = outcomes[i] as GameState;
			expect(other.hand.dealerDiscard).not.toBe(reference.hand.dealerDiscard);
			// Every seat but the dealer sees a byte-identical view.
			for (const seat of [1, 2, 3] as Seat[]) {
				expect(JSON.stringify(project(other, seat))).toBe(JSON.stringify(project(reference, seat)));
			}
			// The dealer's view differs only in their own five cards.
			const a: PublicGameView = project(reference, 0);
			const b: PublicGameView = project(other, 0);
			expect(JSON.stringify({ ...a, hand: null, legal: null })).toBe(
				JSON.stringify({ ...b, hand: null, legal: null })
			);
			expect(a.hand).not.toEqual(b.hand);
			expect(a.handCounts).toEqual(b.handCounts);
			expect(a.kittyCount).toBe(b.kittyCount);
		}
		expect(outcomes).toHaveLength(6);
	});
});

/* -------------------------------------------------------------------------- */
/* V8 — the loner's sitting partner                                            */
/* -------------------------------------------------------------------------- */

describe('the sitting partner (V8)', () => {
	it('is blanked from the moment the call lands, but kept in state for the journal', () => {
		// Eldest orders up alone: dealer 0, maker 1, sitting 3.
		const base = apply(opened('sit', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		const dead = base.hand.hands[3];
		const s = apply(base, 1, { t: 'orderUp', alone: true }).state;

		expect(s.hand.aloneSeat).toBe(1);
		expect(s.hand.sittingSeat).toBe(partnerOf(1));
		expect(s.hand.sittingSeat).toBe(3);
		expect(s.hand.hands[3]).toEqual(dead);
		expect(s.hand.hands[3]).toHaveLength(5);

		const own = project(s, 3);
		expect(own.hand).toEqual([]);
		expect(own.legal).toEqual([]);
		expect(own.sittingSeat).toBe(3);
		expect(own.aloneSeat).toBe(1);
		for (const seat of SEATS_ALL) {
			expect(project(s, seat).handCounts[3]).toBe(0);
		}
		expect(project(s, 0).handCounts).toEqual([6, 5, 5, 0]);
	});

	it('hides the dead hand from its own owner for the whole hand', () => {
		let checked = 0;
		const leaks: string[] = [];
		runGame(newGame('sit-sweep', { gameTo: 10 }), alonePicker('sit-sweep'), {
			onState: (s) => {
				const sitting = s.hand.sittingSeat;
				if (sitting === null) return;
				const view = project(s, sitting);
				expect(view.hand).toEqual([]);
				expect(view.handCounts[sitting]).toBe(0);
				const json = JSON.stringify(view);
				for (const c of s.hand.hands[sitting]) {
					if (json.includes(c) && c !== s.hand.upCard) {
						leaks.push(`${s.hand.phase}: seat ${sitting} saw its own dead card ${c}`);
					}
				}
				checked++;
			}
		});
		expect(leaks).toEqual([]);
		expect(checked).toBeGreaterThan(100);
	});
});

/* -------------------------------------------------------------------------- */
/* V12 — the redaction sweep                                                   */
/* -------------------------------------------------------------------------- */

interface SweepTally {
	states: number;
	views: number;
	foreignChecks: number;
	kittyChecks: number;
	discardChecks: number;
	stepScans: number;
	sittingStates: number;
	leaks: string[];
}

/**
 * Every way `json` — a payload addressed to `seat` — betrays a card that seat is
 * not entitled to, in the state `s`. Empty means clean.
 *
 * Shared by the sweep and by its negative control, so the control proves the
 * sweep itself is capable of failing.
 */
function payloadLeaks(s: GameState, seat: Seat, json: string, tally?: SweepTally): string[] {
	const h = s.hand;
	const found: string[] = [];
	const where = `h${s.handNo} ${h.phase} seat${seat}`;

	// 1. The literal requirement: not one hidden card id appears in the bytes.
	for (const c of hiddenFrom(s, seat)) {
		if (json.includes(c)) found.push(`${where}: hidden ${c} in payload`);
	}

	// 2. The same fact, stated by category, so a failure names the leak.
	for (const other of SEATS_ALL) {
		if (other === seat) continue;
		for (const c of h.hands[other]) {
			if (tally) tally.foreignChecks++;
			if (c === h.upCard) continue; // public: the dealer's picked-up up-card
			if (json.includes(c)) found.push(`${where}: hand of ${other} (${c})`);
		}
	}
	for (const c of h.kitty.slice(1)) {
		if (tally) tally.kittyChecks++;
		if (json.includes(c)) found.push(`${where}: buried kitty ${c}`);
	}
	if (h.dealerDiscard !== null && h.dealerDiscard !== h.upCard) {
		if (tally) tally.discardChecks++;
		if (json.includes(h.dealerDiscard)) found.push(`${where}: dealer discard ${h.dealerDiscard}`);
	}

	// 3. And the complement, which cannot be weakened by adding a field.
	const vis = visibleTo(s, seat);
	for (const c of cardTokensIn(json)) {
		if (!vis.has(c)) found.push(`${where}: unentitled token ${c}`);
	}
	return found;
}

function sweep(initial: GameState, pick: Picker, tally: SweepTally): GameState {
	const note = (m: string) => {
		if (tally.leaks.length < 20) tally.leaks.push(m);
	};

	const checkState = (s: GameState) => {
		tally.states++;
		if (s.hand.sittingSeat !== null) tally.sittingStates++;
		for (const seat of SEATS_ALL) {
			tally.views++;
			for (const m of payloadLeaks(s, seat, JSON.stringify(project(s, seat)), tally)) note(m);
		}
	};

	const checkSteps = (after: GameState, steps: readonly Step[]) => {
		if (steps.length === 0) return;
		for (const seat of SEATS_ALL) {
			const redacted = projectSteps(steps, seat);
			expect(redacted).toHaveLength(steps.length);
			const json = JSON.stringify(redacted);
			tally.stepScans++;
			const vis = visibleTo(after, seat);
			for (const c of cardTokensIn(json)) {
				if (!vis.has(c)) {
					note(`h${after.handNo} steps for seat${seat}: unentitled token ${c}`);
				}
			}
		}
	};

	return runGame(initial, pick, {
		onState: checkState,
		onTransition: (_before, res) => checkSteps(res.state, res.steps)
	});
}

describe('redaction (V12)', () => {
	const emptyTally = (): SweepTally => ({
		states: 0,
		views: 0,
		foreignChecks: 0,
		kittyChecks: 0,
		discardChecks: 0,
		stepScans: 0,
		sittingStates: 0,
		leaks: []
	});

	it('negative control: the detector fires on every category it claims to cover', () => {
		// A real mid-hand state with a loner, a discard, and a buried kitty.
		const base = apply(opened('control', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		const called = apply(base, 1, { t: 'orderUp', alone: true }).state;
		const drop = called.hand.hands[0].find((c) => c !== called.hand.upCard) as CardId;
		const s = apply(called, 0, { t: 'discard', card: drop }).state;

		expect(s.hand.sittingSeat).toBe(3);
		expect(s.hand.dealerDiscard).toBe(drop);
		expect(payloadLeaks(s, 0, JSON.stringify(project(s, 0)))).toEqual([]);

		const clean = JSON.stringify(project(s, 0));
		const widen = (extra: unknown) => JSON.stringify({ ...project(s, 0), extra });

		// A foreign hand.
		const stolen = s.hand.hands[2][0] as CardId;
		expect(clean).not.toContain(stolen);
		expect(widen({ opponent: stolen }).includes(stolen)).toBe(true);
		expect(payloadLeaks(s, 0, widen({ opponent: stolen }))).toEqual(
			expect.arrayContaining([expect.stringContaining(`hand of 2 (${stolen})`)])
		);

		// A buried kitty card.
		const buried = s.hand.kitty[2];
		expect(payloadLeaks(s, 0, widen({ kitty: buried }))).toEqual(
			expect.arrayContaining([expect.stringContaining(`buried kitty ${buried}`)])
		);

		// The dealer's own discard, projected back to the dealer.
		expect(payloadLeaks(s, 0, widen({ discarded: drop }))).toEqual(
			expect.arrayContaining([expect.stringContaining(`dealer discard ${drop}`)])
		);

		// The sitting partner's dead hand, projected back to its owner.
		const dead = s.hand.hands[3][0] as CardId;
		expect(payloadLeaks(s, 3, JSON.stringify(project(s, 3)))).toEqual([]);
		expect(
			payloadLeaks(s, 3, JSON.stringify({ ...project(s, 3), hand: [dead] })).length
		).toBeGreaterThan(0);

		// And an id-shaped leak, not just a bare card field.
		expect(payloadLeaks(s, 0, widen({ legal: [{ id: `play:${stolen}` }] })).length).toBeGreaterThan(
			0
		);
	});

	it('leaks nothing across a full match, every state, every seat', () => {
		const tally = emptyTally();
		const final = sweep(newGame('redact-1'), randomPicker('redact-1'), tally);

		expect(tally.leaks).toEqual([]);
		expect(final.status).toBe('complete');
		// The sweep must actually have exercised each hidden category.
		expect(tally.states).toBeGreaterThan(200);
		expect(tally.views).toBe(tally.states * 4);
		expect(tally.foreignChecks).toBeGreaterThan(1000);
		expect(tally.kittyChecks).toBeGreaterThan(1000);
		expect(tally.discardChecks).toBeGreaterThan(50);
		expect(tally.stepScans).toBeGreaterThan(200);
	});

	it('leaks nothing when every hand is played as a loner', () => {
		const tally = emptyTally();
		const final = sweep(newGame('redact-loner'), alonePicker('redact-loner'), tally);
		expect(tally.leaks).toEqual([]);
		expect(final.status).toBe('complete');
		expect(tally.sittingStates).toBeGreaterThan(100);
	});

	it('leaks nothing for any of the four sitting seats, or any first dealer', () => {
		const tally = emptyTally();
		for (let dealer = 0 as Seat; dealer < 4; dealer = (dealer + 1) as Seat) {
			for (let skip = 0; skip < 4; skip++) {
				const seed = `redact-d${dealer}-s${skip}`;
				sweep(
					createGame({ gameId: `g-${seed}`, seed, firstDealer: dealer, cfg: { gameTo: 5 } }),
					aloneAtPicker(seed, skip),
					tally
				);
			}
		}
		expect(tally.leaks).toEqual([]);
		expect(tally.states).toBeGreaterThan(1000);
		expect(tally.sittingStates).toBeGreaterThan(500);
	});

	it('leaks nothing with stick-the-dealer off, where hands can be thrown in', () => {
		const tally = emptyTally();
		const final = sweep(
			newGame('redact-throwin', { stickTheDealer: false, gameTo: 7 }),
			randomPicker('redact-throwin'),
			tally
		);
		expect(tally.leaks).toEqual([]);
		expect(final.status).toBe('complete');
		expect(tally.states).toBeGreaterThan(100);
	});
});

/* -------------------------------------------------------------------------- */
/* projectSteps                                                                */
/* -------------------------------------------------------------------------- */

describe('projectSteps', () => {
	it('blanks every deal packet not addressed to the receiving seat', () => {
		const s = opened('steps', 0);
		const res = apply(s, cutterOf(0), { t: 'cut', cut: false });
		const dealt = res.steps.find((x) => x.t === 'dealt') as Extract<Step, { t: 'dealt' }>;
		expect(dealt.packets.every((p) => p.cards !== null)).toBe(true);

		for (const seat of SEATS_ALL) {
			const redacted = projectSteps(res.steps, seat);
			const mine = redacted.find((x) => x.t === 'dealt') as Extract<Step, { t: 'dealt' }>;
			expect(mine.packets).toHaveLength(8);
			mine.packets.forEach((p, i) => {
				const original = dealt.packets[i];
				// Seat and count always survive, so the deal still animates faithfully.
				expect(p.seat).toBe(original?.seat);
				expect(p.count).toBe(original?.count);
				if (p.seat === seat) {
					expect(p.cards).toEqual(original?.cards);
					expect(p.cards).toHaveLength(p.count);
				} else {
					expect(p.cards).toBeNull();
				}
			});
			const own = mine.packets.filter((p) => p.seat === seat).flatMap((p) => p.cards ?? []);
			expect(own).toEqual(res.state.hand.hands[seat]);
			// Nothing but this seat's own five cards is anywhere in the payload.
			expect([...cardTokensIn(JSON.stringify(mine))].sort()).toEqual([...own].sort());
		}
	});

	it('passes every non-dealt step through by reference', () => {
		const s = apply(opened('steps-2', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		const bid = apply(s, 1, { t: 'orderUp', alone: true });
		expect(bid.steps.map((x) => x.t)).toEqual(['bid', 'trumpSet']);
		for (const seat of SEATS_ALL) {
			const redacted = projectSteps(bid.steps, seat);
			expect(redacted).toEqual(bid.steps);
			expect(redacted[0]).toBe(bid.steps[0]);
			expect(redacted[1]).toBe(bid.steps[1]);
		}
	});

	it('cannot name the dealer discard: the step has no card field', () => {
		const base = apply(opened('steps-3', 0), cutterOf(0), { t: 'cut', cut: false }).state;
		const picked = apply(base, 1, { t: 'orderUp', alone: false }).state;
		const drop = picked.hand.hands[0][2] as CardId;
		const res = apply(picked, 0, { t: 'discard', card: drop });

		expect(res.steps).toEqual([{ t: 'dealerDiscarded', seat: 0 }]);
		for (const seat of SEATS_ALL) {
			const json = JSON.stringify(projectSteps(res.steps, seat));
			expect(json).not.toContain(drop);
			expect([...cardTokensIn(json)]).toEqual([]);
		}
	});

	it('never mutates the steps it is given', () => {
		const s = opened('steps-4', 0);
		const res = apply(s, cutterOf(0), { t: 'cut', cut: false });
		const before = JSON.stringify(res.steps);
		deepFreeze(res.steps);
		projectSteps(res.steps, 0);
		projectSteps(res.steps, 2);
		expect(JSON.stringify(res.steps)).toBe(before);
	});
});
