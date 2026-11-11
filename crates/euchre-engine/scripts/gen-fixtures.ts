/**
 * Differential fixture generator for the `euchre-engine` Rust port.
 *
 * Runs the real TS engine (read-only, imported by absolute path) and writes the
 * JSON fixtures the Rust tests compare against:
 *
 *   tests/fixtures/deal.json   RNG, hashing, shuffle, cut, deal plan and deal vectors
 *   tests/fixtures/trace.json  sampled GameStates from real matches, with the TS
 *                              `project` views, `projectSteps` output and
 *                              `applyOptimistic` results for every legal move
 *
 * Usage (from the repo root):  bun run crates/euchre-engine/scripts/gen-fixtures.ts
 *
 * Override the TS source location with EUCHRE_TS_SRC (default ~/goods/euchre).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

const SRC = process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods/euchre');
const E = await import(join(SRC, 'src/lib/euchre/index.ts'));

const OUT = join(dirname(new URL(import.meta.url).pathname), '..', 'tests', 'fixtures');
mkdirSync(OUT, { recursive: true });

/* ------------------------------------------------------------------------ */
/* deal.json                                                                 */
/* ------------------------------------------------------------------------ */

const SEEDS = ['abc', 'seed-1', 'full-10', 'x1', 'nat', 'héllo', '😀', 'g-7f3a9c', '', 'house-3'];
const HASH_STRINGS = [
	'',
	'a',
	'foobar',
	'seed#0',
	'héllo',
	'😀',
	'Ω≈ç√',
	'tab\tnew\nline',
	'abc#0#cut',
	'abc#firstDealer'
];

const deal = {
	fnv1a: HASH_STRINGS.map((s) => ({ s, h: E.fnv1a(s) })),
	base36: [0, 1, 35, 36, 1295, 1296, 2166136261, 4294967295].map((n) => ({
		n,
		s: n.toString(36)
	})),
	mulberry32: [0, 1, 2, 42, 0x6d2b79f5, 0xffffffff, 2166136261].map((seed) => {
		const r = E.mulberry32(seed);
		return { seed, draws: Array.from({ length: 8 }, () => r()) };
	}),
	seeds: SEEDS.map((seed) => {
		const firstDealer = E.pickFirstDealer(E.mulberry32(E.fnv1a(`${seed}#firstDealer`)));
		const hands = [0, 1, 2, 3].map((handNo) => {
			const shuffled = E.shuffleForHand(seed, handNo);
			const cut = E.cutForHand(shuffled, seed, handNo);
			const dealer = ((firstDealer + handNo) & 3) as 0 | 1 | 2 | 3;
			const run = E.dealHand(shuffled, dealer, seed, handNo);
			const bump = E.dealHand(cut, dealer, seed, handNo);
			return {
				handNo,
				hashSeed: E.hashSeed(seed, handNo),
				startsWithThree: E.startsWithThree(seed, handNo),
				shuffled,
				cut,
				dealer,
				run: {
					plan: run.plan,
					hands: run.hands,
					kitty: run.kitty,
					upCard: run.upCard,
					packets: E.dealtPackets(shuffled, run.plan)
				},
				bump: {
					hands: bump.hands,
					kitty: bump.kitty,
					upCard: bump.upCard,
					packets: E.dealtPackets(cut, bump.plan)
				}
			};
		});
		return { seed, firstDealer, hands };
	}),
	dealPlans: [0, 1, 2, 3].flatMap((dealer) =>
		[true, false].map((threeFirst) => ({ dealer, threeFirst, plan: E.dealPlan(dealer, threeFirst) }))
	),
	cutAt: [0, 1, 7].map((seed) => {
		const r = E.mulberry32(seed);
		return { seed, deck: E.cutAt(E.DECK, r) };
	})
};

/* ------------------------------------------------------------------------ */
/* trace.json                                                                */
/* ------------------------------------------------------------------------ */

type Picker = (state: any, seat: number, legal: readonly any[]) => any;

function randomPicker(seed: string): Picker {
	const rnd = E.mulberry32(E.fnv1a(seed));
	return (_s, _seat, legal) => legal[Math.floor(rnd() * legal.length)];
}

function alonePicker(seed: string): Picker {
	const fallback = randomPicker(seed);
	return (state, seat, legal) => legal.find((m: any) => m.id.endsWith('+alone')) ?? fallback(state, seat, legal);
}

function passHeavyPicker(seed: string): Picker {
	const rnd = E.mulberry32(E.fnv1a(`${seed}#pass-heavy`));
	const fallback = randomPicker(seed);
	return (state, seat, legal) => {
		const bidding = state.hand.phase === 'bid_round_1' || state.hand.phase === 'bid_round_2';
		const pass = legal.find((m: any) => m.id === 'pass');
		if (bidding && pass && rnd() < 0.8) return pass;
		return fallback(state, seat, legal);
	};
}

interface Sample {
	label: string;
	state: unknown;
	views: unknown[];
	optimistic: { seat: number; moveId: string; view: unknown }[];
	steps: unknown[] | null;
	projectedSteps: unknown[][] | null;
}

function trace(
	label: string,
	init: { gameId: string; seed: string; cfg?: object; firstDealer?: number },
	pick: Picker,
	keep: (i: number, phaseChanged: boolean) => boolean,
	cap: number
): Sample[] {
	const out: Sample[] = [];
	let state = E.createGame(init);
	let steps: unknown[] | null = null;
	let prevPhase = '';
	let i = 0;
	const record = () => {
		const phaseChanged = state.hand.phase !== prevPhase;
		prevPhase = state.hand.phase;
		if (out.length >= cap || !keep(i, phaseChanged)) return;
		const seat = state.hand.turnSeat;
		const optimistic: Sample['optimistic'] = [];
		if (seat !== null) {
			const view = E.project(state, seat);
			for (const lm of E.legalMoves(state, seat)) {
				optimistic.push({ seat, moveId: lm.id, view: E.applyOptimistic(view, lm.move) });
			}
		}
		out.push({
			label: `${label}#${i}`,
			state: JSON.parse(JSON.stringify(state)),
			views: [0, 1, 2, 3].map((s) => E.project(state, s)),
			optimistic,
			steps,
			projectedSteps: steps === null ? null : [0, 1, 2, 3].map((s) => E.projectSteps(steps!, s))
		});
	};
	record();
	while (state.status === 'active' && i < 5000) {
		i++;
		const seat = state.hand.turnSeat;
		const res =
			seat === null ? E.advance(state) : E.apply(state, seat, pick(state, seat, E.legalMoves(state, seat)).move);
		state = res.state;
		steps = res.steps as unknown[];
		record();
	}
	return out;
}

const samples: Sample[] = [
	// The whole first hand of the `abc` replay, plus the closing transitions.
	...trace('abc', { gameId: 'g-abc', seed: 'abc' }, randomPicker('abc'), (i) => i < 28 || i >= 277, 40),
	// Every hand alone: sitting seats, the dealer-sits-out edge, three-play tricks.
	...trace('alone', { gameId: 'g-alone', seed: 'alone', firstDealer: 0 }, alonePicker('alone'), (i) => i < 20, 20),
	// Stick off, natural trump on: throw-ins, the natural-trump filter, scoring and game end.
	...(() => {
		const all = trace(
			'nat',
			{ gameId: 'g-nat-off', seed: 'nat-off', cfg: { requireNaturalTrump: true, stickTheDealer: false, gameTo: 5 }, firstDealer: 2 },
			passHeavyPicker('nat-off'),
			() => true,
			100000
		);
		const has = (s: Sample, t: string) => s.steps !== null && s.steps.some((x: any) => x.t === t);
		return [
			...all.slice(0, 6),
			...all.filter((s) => (s.state as any).hand.phase === 'bid_round_2').slice(0, 8),
			...all.filter((s) => has(s, 'throwIn')).slice(0, 3),
			...all.filter((s) => has(s, 'handScored')).slice(0, 2),
			...all.filter((s) => has(s, 'gameWon'))
		];
	})()
];

writeFileSync(join(OUT, 'deal.json'), JSON.stringify(deal));
writeFileSync(join(OUT, 'trace.json'), JSON.stringify({ samples }));
console.log(`deal.json + trace.json written to ${OUT} (${samples.length} samples)`);
