/**
 * Differential parity generator for the `euchre-ai` Rust port.
 *
 * Plays full matches with the real TS engine and the real TS AI (read-only,
 * imported by absolute path). At every decision point (a seat whose turn it is)
 * it takes that seat's view, augmented the way the table actor augments it on
 * some games (`rules` plus a public `observations` log), and builds six parts:
 *
 *   0 rank      rankMoves for the neutral options and the three default
 *               personas (aggression, risk, gameTo): `id|String(score)|why;`
 *               per move, then `top=<topMove>`, one line per preset
 *   1 notation  encodeForLlm(view, legal, { ranking: neutral, notes })
 *               where notes is the seat memory's auction lines
 *   2 bid       JSON.stringify(bidRollouts(view, legal)) while bidding, else ''
 *   3 play      JSON.stringify(playRollouts(view, legal))
 *   4 jev       the Jev `POST /alpha/decisions` body for buildDecisionQuestion
 *               (persona rotates Rita, Ned, Grandpa by decision index)
 *   5 memory    JSON.stringify(seat memory after absorbPublicState) \n
 *               situationLine \n String(momentSalience)
 *
 * The point string is the parts joined by "\n§\n". The fixture keeps, per
 * point, the engine fnv1a of each part (to locate a mismatch to one part) and,
 * per game, the SHA-256 of every point string in order, each followed by "\n"
 * (the byte-identity guarantee), plus the final turnId.
 *
 * Usage (from the repo root):
 *   bun run crates/euchre-ai/scripts/differential.ts               write the fixture
 *   bun run crates/euchre-ai/scripts/differential.ts --dump <seed> <index> [part]
 *                                                    print the TS point (or one part)
 *
 * Override the TS source location with EUCHRE_TS_SRC (default ~/goods/euchre).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

const SRC = process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods/euchre');
const E = await import(join(SRC, 'src/lib/euchre/index.ts'));
const H = await import(join(SRC, 'src/lib/ai/heuristic.ts'));
const N = await import(join(SRC, 'src/lib/ai/notation.ts'));
const BR = await import(join(SRC, 'src/lib/ai/bid-rollouts.ts'));
const PR = await import(join(SRC, 'src/lib/ai/play-rollouts.ts'));
const DC = await import(join(SRC, 'src/lib/ai/decision-context.ts'));
const J = await import(join(SRC, 'src/lib/ai/jev.ts'));
const M = await import(join(SRC, 'src/lib/actors/ai-seat/memory.ts'));
const P = await import(join(SRC, 'src/lib/server/personas.ts'));

const OUT = join(dirname(new URL(import.meta.url).pathname), '..', 'tests', 'fixtures');

type Seat = 0 | 1 | 2 | 3;
type PickerKind = 'random' | 'heuristic' | 'mixed' | 'passHeavy' | 'alone';

interface GameSpec {
	seed: string;
	gameId: string;
	cfg: Record<string, unknown> | null;
	firstDealer: Seat | null;
	picker: PickerKind;
	augment: boolean;
}

function specs(): GameSpec[] {
	const out: GameSpec[] = [];
	const row = (
		seed: string,
		cfg: Record<string, unknown> | null,
		firstDealer: Seat | null,
		picker: PickerKind,
		augment: boolean
	) => out.push({ seed, gameId: `g-${seed}`, cfg, firstDealer, picker, augment });
	row('ai-0', null, null, 'random', true);
	row('ai-1', null, 1, 'heuristic', true);
	row('ai-2', { gameTo: 5, requireNaturalTrump: true }, null, 'heuristic', true);
	row('ai-3', { gameTo: 5, stickTheDealer: false }, 3, 'passHeavy', false);
	row('ai-4', { gameTo: 5 }, 0, 'alone', true);
	row('ai-5', { gameTo: 5, requireNaturalTrump: true }, 2, 'passHeavy', true);
	row('ai-6', { gameTo: 7 }, 2, 'mixed', false);
	row('ai-7', { gameTo: 5, stickTheDealer: false, requireNaturalTrump: true }, null, 'alone', true);
	row('ai-8', null, 0, 'mixed', true);
	row('ai-9', { gameTo: 5 }, 1, 'random', false);
	row('ai-10', { gameTo: 5, stickTheDealer: false }, null, 'heuristic', true);
	row('ai-11', { gameTo: 5 }, 3, 'alone', false);
	// A rotating cross section: every picker against every rule pair.
	const cfgs: Record<string, unknown>[] = [
		{ gameTo: 10 },
		{ gameTo: 10, stickTheDealer: false },
		{ gameTo: 10, requireNaturalTrump: true },
		{ gameTo: 10, stickTheDealer: false, requireNaturalTrump: true }
	];
	const pickers: PickerKind[] = ['passHeavy', 'alone', 'heuristic', 'mixed', 'random'];
	const dealers: (Seat | null)[] = [null, 0, 1, 2, 3];
	for (let i = 0; i < 16; i++) {
		row(`ai-x${i}`, cfgs[i % 4], dealers[i % 5], pickers[i % 5], i % 3 !== 2);
	}
	return out;
}

const PERSONAS = P.defaultPersonas().map((a: any) => a.persona);

/** The legal move a picker takes. `rnd` is the game's one stream. */
function pick(kind: PickerKind, rnd: () => number, view: any, seat: Seat, legal: any[]): any {
	const random = () => legal[Math.floor(rnd() * legal.length)];
	const heuristic = () => {
		const id = H.topMove(view, view.hand, legal, {});
		return legal.find((m) => m.id === id) ?? legal[0];
	};
	switch (kind) {
		case 'random':
			return random();
		case 'heuristic':
			return heuristic();
		case 'mixed':
			return seat === 0 ? random() : heuristic();
		case 'passHeavy': {
			const bidding = view.phase === 'bid_round_1' || view.phase === 'bid_round_2';
			if (bidding && rnd() < 0.8) {
				const pass = legal.find((m) => m.id === 'pass');
				if (pass !== undefined) return pass;
			}
			return heuristic();
		}
		case 'alone': {
			const alone = legal.find((m) => m.id.endsWith('+alone'));
			return alone ?? random();
		}
	}
}

const KIND: Record<string, string> = {
	cutting: 'cut',
	bid_round_1: 'bid1',
	bid_round_2: 'bid2',
	dealer_discard: 'discard',
	trick_play: 'play'
};

function rankPart(view: any, legal: any[], gameTo: number): string {
	const presets: any[] = [
		{},
		...PERSONAS.map((p: any) => ({ aggression: p.aggression, risk: p.risk, gameTo }))
	];
	return presets
		.map((opts) => {
			const ranked = H.rankMoves(view, view.hand, legal, opts);
			const top = H.topMove(view, view.hand, legal, opts);
			return `${ranked.map((r: any) => `${r.id}|${String(r.score)}|${r.why};`).join('')}top=${top}`;
		})
		.join('\n');
}

function jevBody(view: any, legal: any[], persona: any): string {
	const q = DC.buildDecisionQuestion(view, legal, persona);
	return JSON.stringify({
		model: J.JEV_MODEL,
		state: q.state,
		questions: {
			...q.additionalQuestions,
			move: { type: 'choice', instructions: q.instructions, criteria: q.criteria }
		}
	});
}

function parts(view: any, legal: any[], memory: any, seat: Seat, gameTo: number, n: number): string[] {
	const neutral = H.rankMoves(view, view.hand, legal, {});
	const bidding = view.phase === 'bid_round_1' || view.phase === 'bid_round_2';
	return [
		rankPart(view, legal, gameTo),
		N.encodeForLlm(view, legal, { ranking: neutral, notes: memory.bidHistory }),
		bidding ? JSON.stringify(BR.bidRollouts(view, legal)) : '',
		JSON.stringify(PR.playRollouts(view, legal)),
		jevBody(view, legal, PERSONAS[n % 3]),
		`${JSON.stringify(memory)}\n${M.situationLine(view, seat)}\n${String(M.momentSalience(view, gameTo))}`
	];
}

/** Plays one match; calls `visit(index, parts)` at every decision point. */
function play(spec: GameSpec, visit: (i: number, p: () => string[]) => void): string {
	let state = E.createGame({
		gameId: spec.gameId,
		seed: spec.seed,
		cfg: spec.cfg ?? undefined,
		firstDealer: spec.firstDealer ?? undefined
	});
	const rnd = E.mulberry32(E.fnv1a(spec.seed));
	const memories = [0, 1, 2, 3].map(() => M.emptyMemory(0));
	const observations: any[] = [];
	let steps = 0;
	let n = 0;
	while (state.status === 'active') {
		if (steps++ > 20000) throw new Error(`${spec.seed}: budget exhausted`);
		const seat: Seat | null = state.hand.turnSeat;
		if (seat === null) {
			state = E.advance(state).state;
			continue;
		}
		const base = E.project(state, seat);
		const view = spec.augment
			? {
					...base,
					rules: {
						stickTheDealer: state.cfg.stickTheDealer,
						requireNaturalTrump: state.cfg.requireNaturalTrump
					},
					observations: observations.map((o) => ({ ...o }))
				}
			: base;
		const legal = view.legal;
		const memory = memories[seat];
		M.absorbPublicState(memory, view);
		const i = n;
		const snapshot = JSON.parse(JSON.stringify(memory));
		visit(i, () => parts(view, legal, snapshot, seat, state.cfg.gameTo, i));
		n++;
		const chosen = pick(spec.picker, rnd, view, seat, legal);
		const kind = KIND[state.hand.phase];
		observations.push({
			handNo: state.hand.handNo,
			seat,
			kind,
			action: chosen.label,
			elapsedMs: seat === 0 ? 300 + ((i * 7919) % 5000) : null,
			timing: seat === 0 ? 'human-observed' : 'computer-paced'
		});
		while (observations.length > 40) observations.shift();
		state = E.apply(state, seat, chosen.move).state;
	}
	return state.turnId;
}

const SEP = '\n§\n';
const args = process.argv.slice(2);
if (args[0] === '--dump') {
	const seed = args[1];
	const index = Number(args[2]);
	const part = args[3] === undefined ? null : Number(args[3]);
	const spec = specs().find((s) => s.seed === seed);
	if (!spec || !Number.isInteger(index)) {
		console.error('usage: differential.ts --dump <seed> <index> [part]');
		process.exit(2);
	}
	let found = false;
	play(spec, (i, p) => {
		if (i !== index) return;
		const ps = p();
		console.log(part === null ? ps.join(SEP) : ps[part]);
		found = true;
	});
	if (!found) {
		console.error(`${seed}: no decision ${index}`);
		process.exit(1);
	}
} else {
	let bytes = 0;
	const coverage: Record<string, number> = {};
	const games = specs().map((spec) => {
		const points: number[][] = [];
		const sha = new Bun.CryptoHasher('sha256');
		const finalTurnId = play(spec, (_i, p) => {
			const ps = p();
			const pt = ps.join(SEP);
			bytes += pt.length;
			points.push(ps.map((x) => E.fnv1a(x)));
			sha.update(pt + '\n');
			const jev = JSON.parse(ps[4]);
			const phase = jev.state.phase as string;
			coverage[phase] = (coverage[phase] ?? 0) + 1;
			if (ps[1].includes('SITTING=')) coverage.sitting = (coverage.sitting ?? 0) + 1;
			if (ps[0].includes('+alone')) coverage.alone_option = (coverage.alone_option ?? 0) + 1;
			if (jev.questions.loner_0 || Object.keys(jev.questions).some((k) => k.startsWith('loner_')))
				coverage.loner_question = (coverage.loner_question ?? 0) + 1;
			if (Object.keys(jev.questions).some((k) => k.startsWith('support_')))
				coverage.support_question = (coverage.support_question ?? 0) + 1;
			if (phase === 'bid_round_2' && !jev.state.auction.passAvailable)
				coverage.stick_the_dealer = (coverage.stick_the_dealer ?? 0) + 1;
			if (jev.state.auction?.rules?.requireNaturalTrump)
				coverage.natural_trump = (coverage.natural_trump ?? 0) + 1;
			if ((jev.state.observedActions ?? jev.state.table?.observedActions ?? []).length > 0)
				coverage.observations = (coverage.observations ?? 0) + 1;
		});
		return { ...spec, decisions: points.length, finalTurnId, sha256: sha.digest('hex'), points };
	});
	mkdirSync(OUT, { recursive: true });
	const body = JSON.stringify({ generatedBy: 'crates/euchre-ai/scripts/differential.ts', coverage, games });
	writeFileSync(join(OUT, 'differential.json'), body + '\n');
	const total = games.reduce((k, g) => k + g.points.length, 0);
	console.log(
		`differential.json: ${games.length} games, ${total} points, ${bytes} compared bytes, ${body.length} fixture bytes`
	);
	console.log(JSON.stringify(coverage));
}
