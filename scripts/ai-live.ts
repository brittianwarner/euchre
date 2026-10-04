/**
 * Live end-to-end AI verification.
 *
 * Drives a real game and routes every AI seat's turn through the same `decide()`
 * ladder the table actor uses, with the same per-seat personas. Asserts that each
 * returned move is legal, reports where it came from (`llm` vs `fallback`), and
 * proves the three opponents are genuinely distinct agents rather than one shared
 * brain.
 *
 *   bun run scripts/ai-live.ts        # one hand
 *   bun run scripts/ai-live.ts 2      # two hands
 */

import { createGame, legalMoves, apply, advance, project } from '../src/lib/euchre/index';
import type { GameState, Seat, LegalMove } from '../src/lib/euchre/index';
import { JEV_MODEL } from '../src/lib/ai/jev';
import { decide, modelFactoryFromEnv } from '../src/lib/ai';
import { defaultPersonas } from '../src/lib/server/personas';
import { topMove } from '../src/lib/ai/heuristic';

const SEATS: readonly Seat[] = [0, 1, 2, 3];
const NAME = ['South(you)', 'West', 'North', 'East'];
const GLYPH: Record<string, string> = { S: '♠', H: '♥', D: '♦', C: '♣' };
const card = (c: string) => `${c[0] === 'T' ? '10' : c[0]}${GLYPH[c[1]!]}`;

function kindFor(phase: string): 'cut' | 'bid1' | 'discard' | 'bid2' | 'play' | null {
	switch (phase) {
		case 'cutting':
			return 'cut';
		case 'bid_round_1':
			return 'bid1';
		case 'dealer_discard':
			return 'discard';
		case 'bid_round_2':
			return 'bid2';
		case 'trick_play':
			return 'play';
		default:
			return null;
	}
}

const factory = modelFactoryFromEnv(process.env);
if (!factory?.decision) {
	console.log('✗ no model factory — set OPENROUTER_API_KEY');
	process.exit(1);
}
console.log(`decisions: ${JEV_MODEL} via OpenRouter Decisions API\n`);

const personas = defaultPersonas();
for (const p of personas) {
	console.log(
		`  seat ${p.seat} ${NAME[p.seat]!.padEnd(11)} "${p.persona.name}" — ${p.persona.blurb}`
	);
}
console.log();

const HANDS = Number(process.argv[2] ?? 1);
let s: GameState = createGame({ gameId: 'ai-live', seed: 'ai-live-1' });

interface Stat {
	llm: number;
	fallback: number;
	forced: number;
	ms: number[];
	illegal: number;
}
const stats = new Map<Seat, Stat>();
for (const seat of SEATS) stats.set(seat, { llm: 0, fallback: 0, forced: 0, ms: [], illegal: 0 });

let steps = 0;
while (s.status === 'active' && s.hand.handNo < HANDS && steps < 400) {
	steps++;

	let seat: Seat | null = null;
	let legal: LegalMove[] = [];
	for (const x of SEATS) {
		const m = legalMoves(s, x);
		if (m.length > 0) {
			seat = x;
			legal = m;
			break;
		}
	}
	if (seat === null) {
		const r = advance(s);
		if (r.state === s) break;
		s = r.state;
		continue;
	}

	const kind = kindFor(s.hand.phase);
	const view = project(s, seat);

	// Seat 0 is the human — play the heuristic so the AI seats have an opponent.
	if (seat === 0 || kind === null) {
		const id = topMove(view, view.hand, legal) ?? legal[0]!.id;
		const m = legal.find((x) => x.id === id)!;
		if (m.move.t === 'play') console.log(`  ${NAME[seat]!.padEnd(11)} ${card(m.move.card)}`);
		s = apply(s, seat, m.move).state;
		continue;
	}

	const assignment = personas.find((p) => p.seat === seat)!;
	const t0 = Date.now();
	const outcome = await decide(
		{
			factory,
			persona: assignment.persona,
			dossier: assignment.dossier,
			nonce: 'abcdef0123456789'
		},
		{
			internalToken: s.internalToken,
			gameId: s.gameId,
			seat,
			turnId: s.turnId,
			kind,
			view,
			legal,
			deadlineAt: Date.now() + 20_000
		},
		legal
	);
	const ms = Date.now() - t0;

	const st = stats.get(seat)!;
	st.ms.push(ms);
	if (outcome.source === 'llm') st.llm++;
	else if (outcome.source === 'forced') st.forced++;
	else st.fallback++;

	const chosen = legal.find((m) => m.id === outcome.moveId);
	if (chosen === undefined) {
		st.illegal++;
		console.log(`  ✗ ${NAME[seat]} returned ILLEGAL ${outcome.moveId}`);
		s = apply(s, seat, legal[0]!.move).state;
		continue;
	}

	const label =
		chosen.move.t === 'play'
			? card(chosen.move.card)
			: chosen.move.t === 'discard'
				? 'discards'
				: chosen.label;
	const tag = outcome.source === 'llm' ? 'LLM' : outcome.source === 'forced' ? 'fcd' : 'heu';
	console.log(
		`  ${NAME[seat]!.padEnd(11)} ${label.padEnd(16)} [${tag} ${String(ms).padStart(5)}ms]` +
			(outcome.rationale ? `  ${outcome.rationale.slice(0, 62)}` : '')
	);

	s = apply(s, seat, chosen.move).state;
}

console.log('\n' + '─'.repeat(72));
let totalLlm = 0;
let totalIllegal = 0;
for (const seat of [1, 2, 3] as Seat[]) {
	const st = stats.get(seat)!;
	const n = st.llm + st.fallback + st.forced;
	const avg = st.ms.length ? Math.round(st.ms.reduce((a, b) => a + b, 0) / st.ms.length) : 0;
	totalLlm += st.llm;
	totalIllegal += st.illegal;
	console.log(
		`seat ${seat} ${NAME[seat]!.padEnd(11)} decisions=${String(n).padStart(2)}  ` +
			`llm=${st.llm} heuristic=${st.fallback} forced=${st.forced}  avg=${avg}ms  illegal=${st.illegal}`
	);
}
console.log('─'.repeat(72));
console.log(totalIllegal === 0 ? '✓ every AI move was legal' : `✗ ${totalIllegal} illegal moves`);
console.log(totalLlm > 0 ? `✓ ${totalLlm} moves came from the LLM` : '✗ no move used the LLM');
process.exit(totalIllegal === 0 && totalLlm > 0 ? 0 : 1);
