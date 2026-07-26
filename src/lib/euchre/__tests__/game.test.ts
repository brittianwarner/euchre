/**
 * `game.test.ts` — whole-hand and whole-match behaviour.
 *
 * Three things live here that cannot be tested one transition at a time:
 *
 * 1. **Going alone, end to end.** All four loner configurations of § 3.5, each
 *    played out to the score: the partner sits out, the legal set for that seat is
 *    empty in every phase, every trick has exactly three plays, fifteen cards are
 *    played, and the trick-1 lead passes clockwise when the sitting partner would
 *    have led.
 * 2. **A seeded match to the target score**, with the § 5.1 table checked against
 *    every hand it scores.
 * 3. **A headless fuzz harness**: 300+ complete random-but-legal matches, with the
 *    § 8 invariants asserted on every reachable state.
 */

import { describe, expect, it } from 'vitest';
import {
	DECK,
	advance,
	apply,
	createGame,
	cutterOf,
	eldestOf,
	firstActiveFrom,
	fnv1a,
	isRuleError,
	legalMoves,
	mulberry32,
	newGame,
	nextSeat,
	partnerOf,
	project,
	rawPoints,
	scoreHand,
	teamOf,
	type ApplyResult,
	type CardId,
	type EngineConfig,
	type GameState,
	type HandResult,
	type LegalMove,
	type Seat,
	type Step,
	type Team,
	type Trick
} from '../index';

/* -------------------------------------------------------------------------- */
/* Harness                                                                     */
/* -------------------------------------------------------------------------- */

const SEATS_ALL: readonly Seat[] = [0, 1, 2, 3];

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

/** Drive a match to completion; `advance` whenever no seat is on turn. */
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

/** Drive until `pred` holds, then stop. Used to play out exactly one hand. */
function runUntil(
	initial: GameState,
	pick: Picker,
	pred: (s: GameState) => boolean,
	hooks: RunHooks = {}
): GameState {
	const budget = hooks.budget ?? 2000;
	let state = initial;
	let n = 0;
	hooks.onState?.(state);
	while (!pred(state)) {
		if (++n > budget) throw new Error(`runUntil: budget exhausted in ${state.hand.phase}`);
		if (state.status !== 'active') throw new Error('runUntil: match ended before the predicate');
		const seat = state.hand.turnSeat;
		const res =
			seat === null
				? advance(state)
				: apply(state, seat, pick(state, seat, legalMoves(state, seat)).move);
		if (res.state === state) throw new Error(`runUntil: stalled in ${state.hand.phase}`);
		hooks.onTransition?.(state, res, seat);
		state = res.state;
		hooks.onState?.(state);
	}
	return state;
}

function opened(seed: string, firstDealer?: Seat, cfg?: Partial<EngineConfig>): GameState {
	return advance(createGame({ gameId: `g-${seed}`, seed, cfg, firstDealer })).state;
}

/** Cut, then bid: `skip` passes and then an order-up, optionally alone. */
function orderUpAfter(seed: string, dealer: Seat, skip: number, alone: boolean): GameState {
	let s = opened(seed, dealer);
	s = apply(s, cutterOf(dealer), { t: 'cut', cut: false }).state;
	for (let i = 0; i < skip; i++) s = apply(s, s.hand.turnSeat as Seat, { t: 'pass' }).state;
	return apply(s, s.hand.turnSeat as Seat, { t: 'orderUp', alone }).state;
}

/* -------------------------------------------------------------------------- */
/* Invariants (docs/02-GAME-RULES-ENGINE.md § 8)                               */
/* -------------------------------------------------------------------------- */

const ENGINE_ONLY = new Set(['lobby', 'deal', 'trick_resolve', 'hand_score', 'game_over']);
const DEALT = new Set([
	'bid_round_1',
	'dealer_discard',
	'bid_round_2',
	'trick_play',
	'trick_resolve',
	'hand_score'
]);

/** Every structural invariant that must hold of a single reachable state. */
function stateViolations(s: GameState): string[] {
	const bad: string[] = [];
	const h = s.hand;
	const at = `h${s.handNo}/${h.phase}`;

	// V1 — turn/phase agreement.
	if (ENGINE_ONLY.has(h.phase) && h.turnSeat !== null)
		bad.push(`${at}: engine phase has a turnSeat`);
	if (!ENGINE_ONLY.has(h.phase) && h.turnSeat === null)
		bad.push(`${at}: seat phase has no turnSeat`);

	// V4 — going alone is coherent and simultaneous with trump.
	if (h.aloneSeat !== null) {
		if (h.aloneSeat !== h.makerSeat) bad.push(`${at}: aloneSeat !== makerSeat`);
		if (h.sittingSeat !== partnerOf(h.aloneSeat)) bad.push(`${at}: sittingSeat is not the partner`);
		if (h.trump === null) bad.push(`${at}: alone without trump`);
	}
	if (h.sittingSeat !== null && h.aloneSeat === null) bad.push(`${at}: sitting without a loner`);

	// V8 — the sitting partner never acts and never appears in a trick.
	if (h.sittingSeat !== null) {
		const sit = h.sittingSeat;
		if (h.turnSeat === sit) bad.push(`${at}: the sitting seat is on turn`);
		if (legalMoves(s, sit).length !== 0) bad.push(`${at}: the sitting seat has legal moves`);
		if (h.trick.plays.some((p) => p.seat === sit)) bad.push(`${at}: sitting seat played`);
		for (const t of h.trickLog) {
			if (t.plays.some((p) => p.seat === sit)) bad.push(`${at}: sitting seat played (log)`);
			if (t.winnerSeat === sit) bad.push(`${at}: sitting seat took a trick (V9)`);
		}
	}

	// V19 — the acting seat always has something to do.
	if (h.turnSeat !== null && s.status === 'active' && legalMoves(s, h.turnSeat).length === 0) {
		bad.push(`${at}: empty legal set for seat ${h.turnSeat}`);
	}

	// V18 — bounded collections.
	if (h.trickLog.length > 5) bad.push(`${at}: trickLog ${h.trickLog.length}`);
	if (h.bids.length > 8) bad.push(`${at}: bids ${h.bids.length}`);
	if (h.passes > 4) bad.push(`${at}: passes ${h.passes}`);
	if (h.trick.plays.length > 4) bad.push(`${at}: trick.plays ${h.trick.plays.length}`);
	if (s.misdealStreak > s.cfg.misdealLimit) bad.push(`${at}: misdealStreak ${s.misdealStreak}`);

	// V5 — a discard exists exactly when a round-1 call succeeded.
	const roundOneCall = h.trump !== null && !h.upCardTurnedDown;
	if (h.dealerDiscard !== null && !roundOneCall) bad.push(`${at}: discard without a round-1 call`);
	if (h.upCardTurnedDown && h.dealerDiscard !== null) bad.push(`${at}: discard after a turn-down`);

	// V11 — the score is bounded and the winner flag agrees with it.
	for (const i of [0, 1] as const) {
		if (s.score[i] < 0 || s.score[i] > s.cfg.gameTo) bad.push(`${at}: score ${s.score.join('-')}`);
	}
	const leader = s.score[0] >= s.cfg.gameTo ? 0 : s.score[1] >= s.cfg.gameTo ? 1 : null;
	if (s.winnerTeam !== leader) bad.push(`${at}: winnerTeam ${s.winnerTeam} vs score`);

	// V10 — card accounting. Twenty cards are in play, twenty-one once the
	// up-card has been taken into the dealer's hand by a round-1 call.
	if (DEALT.has(h.phase)) {
		// A resolved `hand.trick` is the same object as the last `trickLog` entry —
		// it is left in place for the recap — so only an in-progress trick is extra.
		const inFlight = h.trick.winnerSeat === null ? h.trick.plays.map((p) => p.card) : [];
		const played: CardId[] = [
			...h.trickLog.flatMap((t) => t.plays.map((p) => p.card)),
			...inFlight
		];
		const owned: CardId[] = [
			...h.hands[0],
			...h.hands[1],
			...h.hands[2],
			...h.hands[3],
			...played,
			...(h.dealerDiscard === null ? [] : [h.dealerDiscard])
		];
		const expected = 20 + (roundOneCall ? 1 : 0);
		if (owned.length !== expected)
			bad.push(`${at}: ${owned.length} cards in play, want ${expected}`);
		const ownedSet = new Set(owned);
		if (ownedSet.size !== owned.length) bad.push(`${at}: a card is in two places`);
		for (const c of h.kitty.slice(1)) {
			if (ownedSet.has(c)) bad.push(`${at}: buried kitty card ${c} is in play`);
		}
		if (new Set([...owned, ...h.kitty]).size !== 24) bad.push(`${at}: the deck does not close`);
		if (h.upCard !== h.kitty[0]) bad.push(`${at}: upCard is not kitty[0]`);
	}

	// V10 — five tricks, and every trick has the right width.
	const width = h.sittingSeat === null ? 4 : 3;
	for (const t of h.trickLog) {
		if (t.plays.length !== width) bad.push(`${at}: trick ${t.index} has ${t.plays.length} plays`);
		if (t.winnerSeat === null) bad.push(`${at}: logged trick ${t.index} is unresolved`);
	}
	if (h.phase === 'hand_score' && h.trickLog.length !== 5) {
		bad.push(`${at}: scored with ${h.trickLog.length} tricks`);
	}
	if (h.tricksWon[0] + h.tricksWon[1] !== h.trickLog.length) bad.push(`${at}: tricksWon mismatch`);

	return bad;
}

/** The § 5.1 row a finished hand must have scored. */
function expectedRow(
	alone: boolean,
	made: number
): { result: HandResult; raw: number; toMakers: boolean } {
	if (made <= 2) return { result: 'euchre', raw: 2, toMakers: false };
	if (made <= 4) return { result: alone ? 'lone_point' : 'point', raw: 1, toMakers: true };
	return { result: alone ? 'lone_march' : 'march', raw: alone ? 4 : 2, toMakers: true };
}

/** What one played-out match produced, for the fuzz assertions. */
interface MatchReport {
	final: GameState;
	hands: number;
	throwIns: number;
	loners: number;
	tricks: number;
	/** Hands whose raw table points were cut short by the `gameTo` clamp (V11). */
	clamps: number;
	cardsPlayed: number[];
	results: HandResult[];
	violations: string[];
	scoreDeltas: Array<readonly [number, number]>;
}

function playMatch(initial: GameState, pick: Picker, deep: boolean): MatchReport {
	const report: MatchReport = {
		final: initial,
		hands: 0,
		throwIns: 0,
		loners: 0,
		tricks: 0,
		clamps: 0,
		cardsPlayed: [],
		results: [],
		violations: [],
		scoreDeltas: []
	};
	const note = (m: string) => {
		if (report.violations.length < 25) report.violations.push(m);
	};

	report.final = runGame(initial, pick, {
		onState: (s) => {
			if (deep) for (const v of stateViolations(s)) note(v);
		},
		onTransition: (before, res) => {
			// Scores move only when a hand scores, and only by a table amount.
			const d: readonly [number, number] = [
				res.state.score[0] - before.score[0],
				res.state.score[1] - before.score[1]
			];
			if (d[0] !== 0 || d[1] !== 0) report.scoreDeltas.push(d);
			if (d[0] < 0 || d[1] < 0) note(`score decreased by ${d.join(',')}`);

			for (const step of res.steps) {
				if (step.t === 'trickWon') report.tricks++;
				if (step.t === 'throwIn') {
					report.throwIns++;
					if (d[0] !== 0 || d[1] !== 0) note('a throw-in moved the score');
				}
				if (step.t !== 'handScored') continue;

				report.hands++;
				report.results.push(step.result);
				const h = before.hand;
				const maker = h.makerSeat as Seat;
				const alone = h.aloneSeat !== null;
				if (alone) report.loners++;

				const cards = h.trickLog.reduce((n: number, t: Trick) => n + t.plays.length, 0);
				report.cardsPlayed.push(cards);
				if (cards !== (alone ? 15 : 20)) note(`hand played ${cards} cards, alone=${alone}`);

				const makers = teamOf(maker);
				const row = expectedRow(alone, h.tricksWon[makers]);
				if (step.result !== row.result) note(`result ${step.result}, want ${row.result}`);

				const credited: Team = row.toMakers ? makers : ((1 - makers) as Team);
				const want: [number, number] = [0, 0];
				want[credited] =
					Math.min(before.cfg.gameTo, before.score[credited] + row.raw) - before.score[credited];
				if (want[credited] < row.raw) report.clamps++;
				if (step.delta[0] !== want[0] || step.delta[1] !== want[1]) {
					note(`delta ${step.delta.join(',')}, want ${want.join(',')}`);
				}
				if (d[0] !== want[0] || d[1] !== want[1])
					note(`applied ${d.join(',')}, want ${want.join(',')}`);
				if (step.delta[0] !== 0 && step.delta[1] !== 0) note('delta has two non-zero components');

				// Cross-check against the standalone table implementation.
				const direct = scoreHand(maker, h.aloneSeat, h.tricksWon, before.score, before.cfg.gameTo);
				if (direct.result !== step.result) note('scoreHand disagrees on the result');
				if (direct.delta[0] !== step.delta[0] || direct.delta[1] !== step.delta[1]) {
					note('scoreHand disagrees on the delta');
				}
			}
		}
	});

	return report;
}

/* -------------------------------------------------------------------------- */
/* Going alone, end to end                                                     */
/* -------------------------------------------------------------------------- */

interface LonerCase {
	readonly name: string;
	/** Round-1 passes before the alone bid, which fixes the maker. */
	readonly skip: number;
	readonly maker: Seat;
	readonly sitting: Seat;
	readonly lead: Seat;
	/** `true` when the dealer is the one sitting out, collapsing the discard. */
	readonly dealerSitsOut: boolean;
}

// Dealer is seat 0 throughout, so eldest is 1 and the cutter is 3.
const LONER_CASES: readonly LonerCase[] = [
	{ name: 'loner is eldest', skip: 0, maker: 1, sitting: 3, lead: 1, dealerSitsOut: false },
	{
		name: "loner is dealer's partner",
		skip: 1,
		maker: 2,
		sitting: 0,
		lead: 1,
		dealerSitsOut: true
	},
	{ name: 'loner is third seat', skip: 2, maker: 3, sitting: 1, lead: 2, dealerSitsOut: false },
	{ name: 'loner is the dealer', skip: 3, maker: 0, sitting: 2, lead: 1, dealerSitsOut: false }
];

describe('going alone', () => {
	for (const c of LONER_CASES) {
		it(`${c.name}: seats out ${c.sitting} and leads trick 1 from ${c.lead}`, () => {
			const seed = `alone-${c.maker}`;
			const called = orderUpAfter(seed, 0, c.skip, true);

			expect(called.hand.makerSeat).toBe(c.maker);
			expect(called.hand.aloneSeat).toBe(c.maker);
			expect(called.hand.sittingSeat).toBe(c.sitting);
			expect(called.hand.sittingSeat).toBe(partnerOf(c.maker));
			expect(c.lead).toBe(firstActiveFrom(eldestOf(0), c.sitting));

			// The dealer's discard is only skipped when the dealer is the one sitting.
			let atLead = called;
			if (c.dealerSitsOut) {
				expect(called.hand.phase).toBe('trick_play');
				expect(called.hand.dealerDiscard).toBe(called.hand.upCard);
				expect(called.hand.hands[0]).toHaveLength(5);
			} else {
				expect(called.hand.phase).toBe('dealer_discard');
				expect(called.hand.turnSeat).toBe(0);
				expect(called.hand.hands[0]).toHaveLength(6);
				atLead = apply(called, 0, {
					t: 'discard',
					card: called.hand.hands[0][0] as CardId
				}).state;
				expect(atLead.hand.dealerDiscard).toBe(called.hand.hands[0][0]);
				expect(atLead.hand.hands[0]).toHaveLength(5);
			}

			expect(atLead.hand.phase).toBe('trick_play');
			expect(atLead.hand.turnSeat).toBe(c.lead);
			expect(atLead.hand.turnSeat).not.toBe(c.sitting);
			expect(atLead.hand.trick.index).toBe(0);
			expect(atLead.hand.hands[c.sitting]).toHaveLength(5);
		});

		it(`${c.name}: three plays a trick, fifteen cards, and a dead partner`, () => {
			const seed = `alone-play-${c.maker}`;
			const called = orderUpAfter(seed, 0, c.skip, true);
			const start = c.dealerSitsOut
				? called
				: apply(called, 0, { t: 'discard', card: called.hand.hands[0][0] as CardId }).state;

			const seenLeaders: Seat[] = [];
			const violations: string[] = [];
			const scored = runUntil(start, randomPicker(seed), (s) => s.hand.phase === 'hand_score', {
				onState: (s) => {
					// The sitting partner is offered nothing, in every phase.
					if (legalMoves(s, c.sitting).length !== 0) violations.push(`${s.hand.phase}: legal set`);
					if (s.hand.turnSeat === c.sitting) violations.push(`${s.hand.phase}: on turn`);
					const view = project(s, c.sitting);
					if (view.hand.length !== 0) violations.push(`${s.hand.phase}: own hand shown`);
					if (view.handCounts[c.sitting] !== 0) violations.push(`${s.hand.phase}: count shown`);
					if (s.hand.phase === 'trick_play' && s.hand.trick.plays.length === 0) {
						seenLeaders.push(s.hand.turnSeat as Seat);
					}
				}
			});

			expect(violations).toEqual([]);
			expect(scored.hand.trickLog).toHaveLength(5);
			for (const t of scored.hand.trickLog) {
				expect(t.plays).toHaveLength(3);
				expect(t.plays.map((p) => p.seat)).not.toContain(c.sitting);
				expect(t.winnerSeat).not.toBe(c.sitting);
				expect(new Set(t.plays.map((p) => p.seat)).size).toBe(3);
			}
			const cards = scored.hand.trickLog.flatMap((t) => t.plays.map((p) => p.card));
			expect(cards).toHaveLength(15);
			expect(new Set(cards).size).toBe(15);

			// Every active hand empties simultaneously; the dead hand is untouched.
			for (const seat of SEATS_ALL) {
				expect(scored.hand.hands[seat]).toHaveLength(seat === c.sitting ? 5 : 0);
			}
			expect(scored.hand.tricksWon[0] + scored.hand.tricksWon[1]).toBe(5);

			// The lead: trick 1 by rule, thereafter always the previous winner (V9).
			expect(seenLeaders[0]).toBe(c.lead);
			expect(seenLeaders).toHaveLength(5);
			for (let i = 1; i < 5; i++) {
				expect(seenLeaders[i]).toBe(scored.hand.trickLog[i - 1]?.winnerSeat);
				expect(seenLeaders[i]).not.toBe(c.sitting);
			}

			// The § 5.1 loner rows.
			const made = scored.hand.tricksWon[teamOf(c.maker)];
			const row = expectedRow(true, made);
			expect(scored.hand.result).toBe(row.result);
			expect(['lone_point', 'lone_march', 'euchre']).toContain(scored.hand.result);
			expect(scored.hand.delta?.[row.toMakers ? teamOf(c.maker) : 1 - teamOf(c.maker)]).toBe(
				row.raw
			);
		});

		it(`${c.name}: rejects every action from the sitting seat with sitting_out`, () => {
			const seed = `alone-reject-${c.maker}`;
			const called = orderUpAfter(seed, 0, c.skip, true);
			const sit = c.sitting;
			const anyCard = called.hand.hands[sit][0] as CardId;
			const before = structuredClone(called) as GameState;

			for (const move of [
				{ t: 'pass' } as const,
				{ t: 'orderUp', alone: false } as const,
				{ t: 'call', suit: 'S', alone: false } as const,
				{ t: 'cut', cut: true } as const,
				{ t: 'discard', card: anyCard } as const,
				{ t: 'play', card: anyCard } as const
			]) {
				let caught: unknown;
				try {
					apply(called, sit, move);
				} catch (e) {
					caught = e;
				}
				expect(isRuleError(caught)).toBe(true);
				expect((caught as { code: string }).code).toBe('sitting_out');
			}
			expect(called).toEqual(before);
		});
	}

	it('never lets the loner take the lead from their own sitting partner', () => {
		// The one case the folk rule gets wrong: the sitting partner is eldest, so
		// the opening lead passes clockwise to the dealer's partner — a defender.
		const s = orderUpAfter('folk-rule', 0, 2, true);
		expect(s.hand.aloneSeat).toBe(3);
		expect(s.hand.sittingSeat).toBe(1);
		const lead = apply(s, 0, { t: 'discard', card: s.hand.hands[0][0] as CardId }).state;
		expect(lead.hand.turnSeat).toBe(2);
		expect(lead.hand.turnSeat).not.toBe(s.hand.aloneSeat);
		expect(teamOf(lead.hand.turnSeat as Seat)).not.toBe(teamOf(3));
	});

	it('an ordinary call keeps all four seats in, with four plays a trick', () => {
		const called = orderUpAfter('not-alone', 0, 0, false);
		expect(called.hand.aloneSeat).toBeNull();
		expect(called.hand.sittingSeat).toBeNull();
		const start = apply(called, 0, { t: 'discard', card: called.hand.hands[0][0] as CardId }).state;
		const scored = runUntil(start, randomPicker('not-alone'), (s) => s.hand.phase === 'hand_score');
		expect(scored.hand.trickLog).toHaveLength(5);
		for (const t of scored.hand.trickLog) expect(t.plays).toHaveLength(4);
		const cards = scored.hand.trickLog.flatMap((t) => t.plays.map((p) => p.card));
		expect(cards).toHaveLength(20);
		expect(new Set(cards).size).toBe(20);
		for (const seat of SEATS_ALL) expect(scored.hand.hands[seat]).toEqual([]);
		expect(['point', 'march', 'euchre']).toContain(scored.hand.result);
	});
});

/* -------------------------------------------------------------------------- */
/* A seeded match, and the score table                                         */
/* -------------------------------------------------------------------------- */

describe('a full match', () => {
	it('ends with exactly one team on the target score', () => {
		const report = playMatch(newGame('full-10'), randomPicker('full-10'), true);
		const s = report.final;

		expect(report.violations).toEqual([]);
		expect(s.status).toBe('complete');
		expect(s.hand.phase).toBe('game_over');
		expect(s.hand.turnSeat).toBeNull();
		expect(s.winnerTeam).not.toBeNull();
		expect(s.score[s.winnerTeam as Team]).toBe(10);
		expect(s.score[1 - (s.winnerTeam as Team)]).toBeLessThan(10);
		expect(report.hands).toBeGreaterThanOrEqual(3);
		expect(report.tricks).toBe(report.hands * 5);
		expect(report.cardsPlayed.every((n) => n === 20 || n === 15)).toBe(true);
		for (const seat of SEATS_ALL) expect(legalMoves(s, seat)).toEqual([]);
	});

	it('emits gameWon exactly once, as the last step of the match', () => {
		const wins: Array<Extract<Step, { t: 'gameWon' }>> = [];
		let lastSteps: readonly Step[] = [];
		const final = runGame(newGame('gamewon'), randomPicker('gamewon'), {
			onTransition: (_b, res) => {
				for (const st of res.steps) if (st.t === 'gameWon') wins.push(st);
				if (res.steps.length > 0) lastSteps = res.steps;
			}
		});
		expect(wins).toHaveLength(1);
		expect(wins[0]?.team).toBe(final.winnerTeam);
		expect(wins[0]?.score).toEqual(final.score);
		expect(lastSteps[lastSteps.length - 1]).toBe(wins[0]);
	});

	it('rotates the deal one seat left after every scored hand', () => {
		const dealers: Seat[] = [];
		runGame(createGame({ gameId: 'g', seed: 'rotate', firstDealer: 0 }), randomPicker('rotate'), {
			onTransition: (before, res) => {
				if (res.steps.some((st) => st.t === 'handScored' || st.t === 'throwIn')) {
					dealers.push(before.hand.dealerSeat);
				}
			}
		});
		expect(dealers.length).toBeGreaterThanOrEqual(3);
		expect(dealers[0]).toBe(0);
		for (let i = 1; i < dealers.length; i++) {
			expect(dealers[i]).toBe(nextSeat(dealers[i - 1] as Seat));
		}
	});

	it('clamps the winning score, never overshooting the target (V11)', () => {
		// gameTo 5 makes the 4-point lone march overshoot often, so the clamp is
		// genuinely exercised rather than merely asserted.
		let clamps = 0;
		for (let i = 0; i < 60; i++) {
			const seed = `clamp-${i}`;
			const report = playMatch(newGame(seed, { gameTo: 5 }), randomPicker(seed), false);
			expect(report.violations).toEqual([]);
			expect(report.final.score[report.final.winnerTeam as Team]).toBe(5);
			expect(report.final.score[1 - (report.final.winnerTeam as Team)]).toBeLessThan(5);
			for (const d of report.scoreDeltas) {
				expect([1, 2, 3, 4]).toContain(d[0] + d[1]);
				expect(d[0] === 0 || d[1] === 0).toBe(true);
			}
			clamps += report.clamps;
		}
		// A clamp is `delta < rawPoints(result)`: the recorded row was worth more
		// than the score was allowed to move.
		expect(clamps).toBeGreaterThan(0);
	});

	it('never awards more than the table row, clamped, on any hand', () => {
		const report = playMatch(
			newGame('raw-vs-delta', { gameTo: 7 }),
			randomPicker('raw-vs-delta'),
			true
		);
		expect(report.violations).toEqual([]);
		expect(report.results).toHaveLength(report.scoreDeltas.length);
		report.results.forEach((result, i) => {
			const d = report.scoreDeltas[i] as readonly [number, number];
			expect(d[0] + d[1]).toBeLessThanOrEqual(rawPoints(result));
			expect(d[0] + d[1]).toBeGreaterThan(0);
		});
	});

	it.each([5, 7, 10, 11])('reaches a target score of %i exactly', (gameTo) => {
		const seed = `to-${gameTo}`;
		const report = playMatch(newGame(seed, { gameTo }), randomPicker(seed), true);
		expect(report.violations).toEqual([]);
		expect(report.final.cfg.gameTo).toBe(gameTo);
		expect(report.final.score[report.final.winnerTeam as Team]).toBe(gameTo);
		expect(report.final.score[1 - (report.final.winnerTeam as Team)]).toBeLessThan(gameTo);
		expect(report.hands).toBeLessThanOrEqual(2 * gameTo);
	});

	it('every hand scores under stick-the-dealer, so no hand is ever thrown in', () => {
		const report = playMatch(newGame('no-throwin'), randomPicker('no-throwin'), false);
		expect(report.throwIns).toBe(0);
		expect(report.results).not.toContain('throw_in');
		expect(report.results.length).toBe(report.hands);
	});
});

/* -------------------------------------------------------------------------- */
/* The fuzz harness                                                            */
/* -------------------------------------------------------------------------- */

describe('fuzz: complete random-but-legal matches', () => {
	/**
	 * A picker that passes four times out of five while bidding, so the rare
	 * corners — the turn-down, the stuck dealer, and (without stick-the-dealer)
	 * the four-pass throw-in — are actually reached instead of merely permitted.
	 */
	function passHeavyPicker(seed: string): Picker {
		const rnd = mulberry32(fnv1a(`${seed}#pass-heavy`));
		return (state, _seat, legal) => {
			const bidding = state.hand.phase === 'bid_round_1' || state.hand.phase === 'bid_round_2';
			if (bidding && rnd() < 0.8) {
				const pass = legal.find((m) => m.id === 'pass');
				if (pass !== undefined) return pass;
			}
			return legal[Math.floor(rnd() * legal.length)] as LegalMove;
		};
	}

	interface Batch {
		readonly label: string;
		readonly games: number;
		readonly cfg?: Partial<EngineConfig>;
		readonly maxHands: number;
		readonly picker?: (seed: string) => Picker;
		/** `'none'` = every hand must score; `'some'` = the throw-in path must fire. */
		readonly throwIns: 'none' | 'some';
	}

	const batches: readonly Batch[] = [
		{ label: 'default', games: 200, maxHands: 25, throwIns: 'none' },
		{ label: 'to-5', games: 40, cfg: { gameTo: 5 }, maxHands: 15, throwIns: 'none' },
		{ label: 'to-11', games: 30, cfg: { gameTo: 11 }, maxHands: 30, throwIns: 'none' },
		{
			label: 'no-stick',
			games: 40,
			cfg: { stickTheDealer: false },
			maxHands: 60,
			throwIns: 'none'
		},
		{
			// Bids that mostly pass: the stuck dealer is forced to call on most hands.
			label: 'pass-heavy-stick',
			games: 40,
			maxHands: 30,
			picker: passHeavyPicker,
			throwIns: 'none'
		},
		{
			// The same, with stick-the-dealer off, so hands are genuinely thrown in.
			label: 'pass-heavy-throwin',
			games: 40,
			cfg: { stickTheDealer: false },
			maxHands: 40,
			picker: passHeavyPicker,
			throwIns: 'some'
		}
	];

	for (const b of batches) {
		it(`plays ${b.games} matches to completion (${b.label}) with no invariant broken`, () => {
			const failures: string[] = [];
			let totalHands = 0;
			let totalTricks = 0;
			let totalLoners = 0;
			let totalThrowIns = 0;
			const seenResults = new Set<HandResult>();
			const seenDeltas = new Set<string>();

			for (let i = 0; i < b.games; i++) {
				const seed = `fuzz-${b.label}-${i}`;
				const start = createGame({
					gameId: `g-${seed}`,
					seed,
					cfg: b.cfg,
					firstDealer: (i % 4) as Seat
				});
				let report: MatchReport;
				try {
					report = playMatch(start, (b.picker ?? randomPicker)(seed), true);
				} catch (e) {
					failures.push(`${seed}: threw ${String(e)}`);
					continue;
				}

				for (const v of report.violations) failures.push(`${seed}: ${v}`);

				const s = report.final;
				const gameTo = s.cfg.gameTo;
				if (s.status !== 'complete') failures.push(`${seed}: status ${s.status}`);
				if (s.hand.phase !== 'game_over') failures.push(`${seed}: phase ${s.hand.phase}`);
				if (s.winnerTeam === null) failures.push(`${seed}: no winner`);
				else {
					if (s.score[s.winnerTeam] !== gameTo)
						failures.push(`${seed}: winner at ${s.score[s.winnerTeam]}`);
					if (s.score[1 - s.winnerTeam] >= gameTo)
						failures.push(`${seed}: both teams at ${gameTo}`);
				}
				if (report.hands < 1) failures.push(`${seed}: no hand scored`);
				if (report.hands > b.maxHands) failures.push(`${seed}: ${report.hands} scored hands`);
				if (report.tricks !== report.hands * 5) {
					failures.push(`${seed}: ${report.tricks} tricks over ${report.hands} hands`);
				}
				const bound = (b.maxHands + report.throwIns) * 5;
				if (report.tricks > bound)
					failures.push(`${seed}: ${report.tricks} tricks exceeds ${bound}`);
				for (const n of report.cardsPlayed) {
					if (n !== 20 && n !== 15) failures.push(`${seed}: ${n} cards in a hand`);
				}
				for (const d of report.scoreDeltas) {
					const total = d[0] + d[1];
					if (!(total === 1 || total === 2 || total === 3 || total === 4)) {
						failures.push(`${seed}: score moved by ${d.join(',')}`);
					}
					if (d[0] !== 0 && d[1] !== 0) failures.push(`${seed}: both scores moved`);
					seenDeltas.add(d.join(','));
				}
				if (report.scoreDeltas.length !== report.hands) {
					failures.push(`${seed}: ${report.scoreDeltas.length} score moves, ${report.hands} hands`);
				}

				totalHands += report.hands;
				totalTricks += report.tricks;
				totalLoners += report.loners;
				totalThrowIns += report.throwIns;
				for (const r of report.results) seenResults.add(r);
			}

			const ROWS: readonly HandResult[] = ['euchre', 'lone_march', 'lone_point', 'march', 'point'];

			expect(failures).toEqual([]);
			expect(totalHands).toBeGreaterThan(b.games);
			expect(totalTricks).toBe(totalHands * 5);
			expect(totalLoners).toBeGreaterThan(0);
			expect(seenDeltas.size).toBeGreaterThan(1);
			// No row outside the § 5.1 table is ever produced...
			for (const r of seenResults) expect(ROWS).toContain(r);
			// ...and the big batch must actually reach all five of them, or it
			// proves nothing about the ones it never generated.
			if (b.games >= 200) expect([...seenResults].sort()).toEqual([...ROWS]);
			// Stick-the-dealer guarantees every hand scores; without it, this batch
			// is tuned so the four-pass throw-in is genuinely reached.
			if (b.throwIns === 'none') expect(totalThrowIns).toBe(0);
			else expect(totalThrowIns).toBeGreaterThan(0);
		});
	}

	it('never deals a card twice, in any of 200 opening deals', () => {
		const sorted = [...DECK].sort();
		for (let i = 0; i < 200; i++) {
			const seed = `deal-fuzz-${i}`;
			const cut = advance(createGame({ gameId: 'g', seed, firstDealer: (i % 4) as Seat })).state;
			const dealt = apply(cut, cut.hand.turnSeat as Seat, { t: 'cut', cut: i % 2 === 0 }).state;
			const h = dealt.hand;
			const all = [...h.hands[0], ...h.hands[1], ...h.hands[2], ...h.hands[3], ...h.kitty];
			expect(all).toHaveLength(24);
			expect([...all].sort()).toEqual(sorted);
			expect(h.upCard).toBe(h.kitty[0]);
			for (const seat of SEATS_ALL) expect(h.hands[seat]).toHaveLength(5);
		}
	});
});
