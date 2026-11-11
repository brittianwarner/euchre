/**
 * Differential parity generator for the `euchre-engine` Rust port.
 *
 * Plays full matches with the real TS engine (read-only, imported by absolute
 * path). At the initial state and after every transition it builds one point
 * string:
 *
 *   JSON.stringify({ views: [project(s, 0..3)], steps: [projectSteps(steps, 0..3)],
 *                    legal: [legalMoveIds(s, 0..3)] })
 *   + "\n" + JSON.stringify(state)
 *   + "\n" + the rejection battery (see `battery`)
 *
 * and records two things:
 *
 *   - per point, the engine `fnv1a` (32-bit, UTF-16) of the point string, so a
 *     mismatch is located to one transition;
 *   - per game, the SHA-256 (hex) of every point string in order, each followed
 *     by "\n". This is the byte-identity guarantee: any differing byte in any
 *     view, step list, legal set, state or rejection changes the digest.
 *
 * plus the final `turnId`. The Rust test `tests/differential.rs` replays the
 * same matches and must reproduce every hash.
 *
 * The rejection battery tries a fixed list of candidate moves for every seat
 * (every move kind, cards held and not held, plus one stale `turnId`) and
 * records each outcome as `ok:<new turnId>;` or `<RuleCode>:<legal ids>;`, so
 * every rejection code and the legal set the `RuleError` carries are compared
 * too, including at the final (game over) state.
 *
 * Games: the full cross product of gameTo {5,10} x stickTheDealer x
 * requireNaturalTrump x first dealer {drawn, 0, 1, 2, 3} x picker {random,
 * passHeavy, alone, randomAlt} (160 games), plus rows for gameTo 7 and 11,
 * misdealLimit 1 and 5, an explicit internalToken, and the four spec 01
 * section 15 replays.
 *
 * Usage (from the repo root):
 *   bun run crates/euchre-engine/scripts/differential.ts            write the fixture
 *   bun run crates/euchre-engine/scripts/differential.ts --dump <seed> <index>
 *                                                     print the TS point string
 *
 * Override the TS source location with EUCHRE_TS_SRC (default ~/goods/euchre).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

const SRC = process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods/euchre');
const E = await import(join(SRC, 'src/lib/euchre/index.ts'));

const OUT = join(dirname(new URL(import.meta.url).pathname), '..', 'tests', 'fixtures');

type Seat = 0 | 1 | 2 | 3;
type Picker = (state: any, seat: Seat, legal: readonly any[]) => any;
type PickerKind = 'random' | 'passHeavy' | 'passMost' | 'alone' | 'randomAlt';

/** `randomPicker` of reduce/game/project.test.ts. */
function randomPicker(seed: string): Picker {
	const rnd = E.mulberry32(E.fnv1a(seed));
	return (_s, _seat, legal) => legal[Math.floor(rnd() * legal.length)];
}

/** `passHeavyPicker` of game.test.ts (one stream), with a configurable pass rate. */
function passPicker(stream: string, rate: number): Picker {
	const rnd = E.mulberry32(E.fnv1a(stream));
	return (state, _seat, legal) => {
		const bidding = state.hand.phase === 'bid_round_1' || state.hand.phase === 'bid_round_2';
		if (bidding && rnd() < rate) {
			const pass = legal.find((m: any) => m.id === 'pass');
			if (pass !== undefined) return pass;
		}
		return legal[Math.floor(rnd() * legal.length)];
	};
}

/** `alonePicker` of project.test.ts. */
function alonePicker(seed: string): Picker {
	const fallback = randomPicker(seed);
	return (state, seat, legal) => {
		const alone = legal.find((m: any) => m.id.endsWith('+alone'));
		return alone ?? fallback(state, seat, legal);
	};
}

function makePicker(kind: PickerKind, seed: string): Picker {
	switch (kind) {
		case 'random':
			return randomPicker(seed);
		case 'passHeavy':
			return passPicker(`${seed}#pass-heavy`, 0.8);
		case 'passMost':
			// Throw-in streaks long enough to hit a misdealLimit cap.
			return passPicker(`${seed}#pass-most`, 0.98);
		case 'alone':
			return alonePicker(seed);
		case 'randomAlt':
			return randomPicker(`${seed}#alt`);
	}
}

interface GameSpec {
	seed: string;
	gameId: string;
	cfg: Record<string, unknown> | null;
	firstDealer: Seat | null;
	internalToken: string | null;
	picker: PickerKind;
}

const PICKERS: readonly PickerKind[] = ['random', 'passHeavy', 'alone', 'randomAlt'];
const DEALERS: readonly (Seat | null)[] = [null, 0, 1, 2, 3];

function specs(): GameSpec[] {
	const out: GameSpec[] = [];
	const row = (
		seed: string,
		cfg: Record<string, unknown> | null,
		firstDealer: Seat | null,
		picker: PickerKind,
		internalToken: string | null = null
	) => out.push({ seed, gameId: `g-${seed}`, cfg, firstDealer, internalToken, picker });

	let i = 0;
	for (const gameTo of [5, 10]) {
		for (const stickTheDealer of [true, false]) {
			for (const requireNaturalTrump of [false, true]) {
				for (const dealer of DEALERS) {
					for (const picker of PICKERS) {
						row(`diff-${i}`, { gameTo, stickTheDealer, requireNaturalTrump }, dealer, picker);
						i++;
					}
				}
			}
		}
	}
	// Config values outside the cross product.
	row('diff-to-7', { gameTo: 7 }, 1, 'random');
	row('diff-to-11', { gameTo: 11, stickTheDealer: false }, null, 'passHeavy');
	row('diff-misdeal-1', { gameTo: 5, stickTheDealer: false, misdealLimit: 1 }, 2, 'passMost');
	row('diff-misdeal-5', { gameTo: 5, stickTheDealer: false, misdealLimit: 5 }, 3, 'passMost');
	row('diff-token', { gameTo: 5 }, 0, 'random', 'tok-explicit-0123456789');
	// The spec 01 section 15 replays.
	row('abc', null, null, 'random');
	row('full-10', null, null, 'random');
	row('x1', { gameTo: 5, stickTheDealer: false }, 2, 'random');
	row('nat', { requireNaturalTrump: true }, 0, 'random');
	return out;
}

const SEATS: readonly Seat[] = [0, 1, 2, 3];
const SUITS = ['S', 'H', 'D', 'C'] as const;

/** The fixed candidate moves tried for `seat` (mirrored in tests/differential.rs). */
function candidates(state: any, seat: Seat): any[] {
	const held: string[] = state.hand.hands[seat];
	const out: any[] = [
		{ t: 'pass' },
		{ t: 'orderUp', alone: false },
		{ t: 'orderUp', alone: true }
	];
	for (const suit of SUITS) out.push({ t: 'call', suit, alone: false });
	out.push({ t: 'call', suit: 'H', alone: true });
	out.push({ t: 'cut', cut: true });
	out.push({ t: 'cut', cut: false });
	out.push({ t: 'discard', card: held[0] ?? '9S' });
	out.push({ t: 'discard', card: 'AS' });
	for (const card of held) out.push({ t: 'play', card });
	out.push({ t: 'play', card: 'AS' });
	out.push({ t: 'play', card: '9H' });
	return out;
}

function outcome(state: any, seat: Seat, move: any, opts?: { turnId: string }): string {
	try {
		return `ok:${E.apply(state, seat, move, opts).state.turnId};`;
	} catch (e: any) {
		if (E.isRuleError(e)) return `${e.code}:${e.legal.map((m: any) => m.id).join(',')};`;
		return `fault:${e.message};`;
	}
}

/** Every seat's outcome for every candidate, plus a stale-turnId pass. */
function battery(state: any): string {
	let buf = '';
	for (const seat of SEATS) {
		buf += `${seat}|stale:${outcome(state, seat, { t: 'pass' }, { turnId: 'stale' })}`;
		for (const move of candidates(state, seat)) buf += outcome(state, seat, move);
		buf += '\n';
	}
	return buf;
}

/** The compared point string after one transition. */
function point(state: any, steps: readonly unknown[]): string {
	const view = JSON.stringify({
		views: SEATS.map((s) => E.project(state, s)),
		steps: SEATS.map((s) => E.projectSteps(steps, s)),
		legal: SEATS.map((s) => E.legalMoveIds(state, s))
	});
	return `${view}\n${JSON.stringify(state)}\n${battery(state)}`;
}

/** Plays one match; calls `visit(index, point)` for the initial state and each transition. */
function play(spec: GameSpec, visit: (i: number, p: () => string) => void): string {
	let state = E.createGame({
		gameId: spec.gameId,
		seed: spec.seed,
		cfg: spec.cfg ?? undefined,
		internalToken: spec.internalToken ?? undefined,
		firstDealer: spec.firstDealer ?? undefined
	});
	const pick = makePicker(spec.picker, spec.seed);
	let i = 0;
	visit(i, () => point(state, []));
	while (state.status === 'active') {
		if (i > 20000) throw new Error(`${spec.seed}: budget exhausted`);
		const seat = state.hand.turnSeat;
		const res =
			seat === null
				? E.advance(state)
				: E.apply(state, seat, pick(state, seat, E.legalMoves(state, seat)).move);
		state = res.state;
		i++;
		const s = state;
		visit(i, () => point(s, res.steps));
	}
	return state.turnId;
}

const args = process.argv.slice(2);
if (args[0] === '--dump') {
	const seed = args[1];
	const index = Number(args[2]);
	const spec = specs().find((s) => s.seed === seed);
	if (!spec || !Number.isInteger(index)) {
		console.error('usage: differential.ts --dump <seed> <index>');
		process.exit(2);
	}
	let found = false;
	play(spec, (i, p) => {
		if (i !== index) return;
		console.log(p());
		found = true;
	});
	if (!found) {
		console.error(`${seed}: no transition ${index}`);
		process.exit(1);
	}
} else {
	let bytes = 0;
	const games = specs().map((spec) => {
		const points: number[] = [];
		const sha = new Bun.CryptoHasher('sha256');
		const finalTurnId = play(spec, (_i, p) => {
			const pt = p();
			bytes += pt.length;
			points.push(E.fnv1a(pt));
			sha.update(pt + '\n');
		});
		return {
			...spec,
			transitions: points.length - 1,
			finalTurnId,
			sha256: sha.digest('hex'),
			points
		};
	});
	mkdirSync(OUT, { recursive: true });
	const body = JSON.stringify({ generatedBy: 'scripts/differential.ts', games });
	writeFileSync(join(OUT, 'differential.json'), body + '\n');
	const total = games.reduce((n, g) => n + g.points.length, 0);
	console.log(
		`differential.json: ${games.length} games, ${total} points, ${bytes} compared bytes, ${body.length} fixture bytes`
	);
}
