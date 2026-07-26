/**
 * Independent end-to-end harness for the euchre engine.
 *
 * Deliberately NOT written against the unit tests' assumptions: it drives the
 * public API the way a Rivet actor will, and re-derives every invariant from the
 * rules of euchre rather than from the implementation.
 *
 * What counts as SECRET here is precise, and follows the spec:
 *   - another seat's hand                                  (never projected)
 *   - kitty[1..3], the three buried cards                   (never projected)
 *   - the dealer's discard                                  (never projected, V12)
 * The up-card (kitty[0]) is PUBLIC for the whole hand (V14) and stays public even
 * after the dealer takes it into hand, so it is excluded from the secret set.
 * `kitty` is the immutable dealt packet, so the up-card legitimately appears both
 * there and in the dealer's hand — that is representation, not duplication.
 *
 *   bun run scripts/e2e-play.ts             # narrate one game, then fuzz 300
 *   bun run scripts/e2e-play.ts 1000 quiet  # fuzz only
 */

import {
	createGame,
	legalMoves,
	apply,
	advance,
	project,
	effectiveSuit,
	trumpRank,
	plainRank,
	DECK,
	type GameState,
	type Seat,
	type CardId,
	assertInvariants,
	assertLegalNonEmpty,
	InvariantError,
	type LegalMove,
	type Trick
} from '../src/lib/euchre/index';

function mulberry32(a: number) {
	return () => {
		a |= 0;
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

const SEATS: readonly Seat[] = [0, 1, 2, 3];

/**
 * Every trick, each exactly once.
 *
 * During `trick_resolve` the engine deliberately leaves `hand.trick` pointing at
 * the trick it has *already* pushed to `trickLog`, so the UI can render the
 * completed trick before the collection animation. Naively concatenating the two
 * double-counts four cards and inflates every seat's played count — so dedupe on
 * the trick index.
 */
function allTricks(h: GameState['hand']): readonly Trick[] {
	const log = h.trickLog ?? [];
	const cur = h.trick;
	if (!cur || cur.plays.length === 0) return log;
	if (log.some((t) => t.index === cur.index)) return log;
	return [...log, cur];
}

class Failures {
	items: string[] = [];
	seen = new Set<string>();
	check(cond: boolean, msg: string): void {
		if (cond) return;
		// Collapse identical repeats so one systemic bug doesn't drown the rest.
		const key = msg.replace(/seed=\S+/, '').slice(0, 120);
		if (this.seen.has(key)) return;
		this.seen.add(key);
		this.items.push(msg);
	}
}

/* ------------------------------------------------------------------ security */

/**
 * THE security invariant. Serialise what a seat is allowed to see and assert no
 * secret CardId appears anywhere in it, at any depth, under any key name.
 * Substring matching is intentional: it catches a leak through a field nobody
 * thought to audit.
 */
function checkRedaction(s: GameState, f: Failures, tag: string): void {
	const h = s.hand;
	const upCard = h.upCard; // public by design — excluded from every secret set

	for (const seat of SEATS) {
		const blob = JSON.stringify(project(s, seat));

		const secrets = new Map<CardId, string>();
		for (const other of SEATS) {
			if (other === seat) continue;
			for (const c of h.hands[other] ?? []) {
				if (c !== upCard) secrets.set(c, `seat ${other}'s hand`);
			}
		}
		for (const c of (h.kitty ?? []).slice(1)) secrets.set(c as CardId, 'buried kitty');
		if (h.dealerDiscard && h.dealerDiscard !== upCard) {
			secrets.set(h.dealerDiscard, "dealer's discard");
		}
		// A card already played to the table is public knowledge, so it cannot be
		// a secret even if a stale copy lingers in a hand array.
		for (const t of allTricks(h)) {
			for (const p of t.plays) secrets.delete(p.card);
		}

		for (const [c, where] of secrets) {
			if (blob.includes(`"${c}"`)) {
				f.check(false, `${tag}: seat ${seat}'s view leaks ${c} (${where}) in phase ${h.phase}`);
			}
		}
	}
}

/** Every one of the 24 cards accounted for exactly once, up-card aliasing aside. */
function checkCardConservation(s: GameState, f: Failures, tag: string): void {
	const h = s.hand;
	if (h.phase === 'lobby' || h.phase === 'cutting') return;

	const inHands = SEATS.flatMap((i) => [...(h.hands[i] ?? [])]);
	const played = allTricks(h).flatMap((t) => t.plays.map((p) => p.card));
	const kitty = [...(h.kitty ?? [])] as CardId[];
	const discard = h.dealerDiscard ? [h.dealerDiscard] : [];

	// A card is either still held, already played, buried, or discarded.
	const universe = new Set<CardId>([...inHands, ...played, ...kitty, ...discard]);
	f.check(universe.size === 24, `${tag}: ${universe.size} distinct cards tracked, expected 24`);

	// Nothing may be in two live places at once. The up-card is the one legal
	// alias (immutable kitty[0] + the dealer's hand after a pickup).
	const live = [...inHands, ...played, ...discard];
	const dupes = live.filter((c, i) => live.indexOf(c) !== i);
	for (const d of new Set(dupes)) {
		f.check(false, `${tag}: ${d} is in two live places at once (phase ${h.phase})`);
	}

	// Hand sizes: 5 each, except mid-pickup (dealer 6) and as tricks are played.
	if (h.phase === 'trick_play' || h.phase === 'trick_resolve') {
		const playedBySeat = new Map<Seat, number>();
		for (const t of allTricks(h)) {
			for (const p of t.plays) playedBySeat.set(p.seat, (playedBySeat.get(p.seat) ?? 0) + 1);
		}
		for (const seat of SEATS) {
			if (seat === h.sittingSeat) continue;
			const expect = 5 - (playedBySeat.get(seat) ?? 0);
			const actual = (h.hands[seat] ?? []).length;
			f.check(actual === expect, `${tag}: seat ${seat} holds ${actual}, expected ${expect}`);
		}
	}
}

/** Re-derive follow-suit legality from the rules, independently of `legal.ts`. */
function checkFollowSuit(s: GameState, f: Failures, tag: string): void {
	const h = s.hand;
	const trump = h.trump;
	if (!trump) return;

	// Reconstruct each seat's holdings at the moment of each play, by replaying
	// the hand forward from what they were dealt (still-held + everything played).
	const dealt = new Map<Seat, Set<CardId>>();
	for (const seat of SEATS) dealt.set(seat, new Set(h.hands[seat] ?? []));
	for (const t of allTricks(h)) {
		for (const p of t.plays) dealt.get(p.seat)!.add(p.card);
	}

	const remaining = new Map<Seat, Set<CardId>>();
	for (const seat of SEATS) remaining.set(seat, new Set(dealt.get(seat)!));

	for (const t of allTricks(h)) {
		const plays = t.plays;
		if (plays.length === 0) continue;
		const led = effectiveSuit(plays[0]!.card, trump);
		for (let i = 0; i < plays.length; i++) {
			const p = plays[i]!;
			const held = remaining.get(p.seat)!;
			if (i > 0 && effectiveSuit(p.card, trump) !== led) {
				const couldHaveFollowed = [...held].some((c) => effectiveSuit(c, trump) === led);
				f.check(
					!couldHaveFollowed,
					`${tag}: seat ${p.seat} played ${p.card} off-suit but still held the led suit ${led}`
				);
			}
			held.delete(p.card);
		}
	}
}

/** Recompute every trick winner with an independent comparator. */
function checkTrickWinners(s: GameState, f: Failures, tag: string): void {
	const h = s.hand;
	const trump = h.trump;
	if (!trump) return;
	for (const t of h.trickLog ?? []) {
		const plays = t.plays ?? [];
		if (plays.length === 0 || t.winnerSeat === null) continue;
		const led = effectiveSuit(plays[0]!.card, trump);
		let bestSeat = plays[0]!.seat;
		let best = -1;
		for (const p of plays) {
			const eff = effectiveSuit(p.card, trump);
			const v = eff === trump ? 100 + trumpRank(p.card, trump) : eff === led ? 50 + plainRank(p.card) : 0;
			if (v > best) {
				best = v;
				bestSeat = p.seat;
			}
		}
		f.check(
			t.winnerSeat === bestSeat,
			`${tag}: trick ${t.index} winner ${t.winnerSeat}, independent calc ${bestSeat} ` +
				`(led ${led}, trump ${trump}, ${plays.map((p) => `${p.seat}:${p.card}`).join(' ')})`
		);
		// The led suit the engine recorded must match the effective suit of play 0.
		f.check(t.ledSuit === led, `${tag}: trick ${t.index} ledSuit ${t.ledSuit}, expected ${led}`);
	}
}

/** A loner's hand has exactly 3 plays per trick; a normal hand has 4. */
function checkLonerShape(s: GameState, f: Failures, tag: string): void {
	const h = s.hand;
	const expect = h.sittingSeat === null ? 4 : 3;
	for (const t of h.trickLog ?? []) {
		f.check(
			t.plays.length === expect,
			`${tag}: trick ${t.index} had ${t.plays.length} plays, expected ${expect} (sitting=${h.sittingSeat})`
		);
		if (h.sittingSeat !== null) {
			f.check(
				!t.plays.some((p) => p.seat === h.sittingSeat),
				`${tag}: sitting seat ${h.sittingSeat} played in trick ${t.index}`
			);
		}
	}
}

/* -------------------------------------------------------------------- driver */

interface GameSummary {
	hands: number;
	finalScore: readonly [number, number];
	winner: number;
	steps: number;
	phasesSeen: Set<string>;
	moveIdsSeen: Set<string>;
	handResults: string[];
	loners: number;
	euchres: number;
}

function playGame(seed: string, rng: () => number, f: Failures, narrate: boolean): GameSummary {
	let s = createGame({ gameId: `g-${seed}`, seed });
	const tag = `seed=${seed}`;

	const phasesSeen = new Set<string>();
	const moveIdsSeen = new Set<string>();
	const handResults: string[] = [];
	let steps = 0;
	let hands = 0;
	let loners = 0;
	let euchres = 0;
	let lastScore = s.score;
	let lastHandNo = -1;
	let lastPhase = '';
	let lastTrickCount = 0;

	const MAX_STEPS = 6000;

	while (s.status === 'active' && steps < MAX_STEPS) {
		steps++;
		phasesSeen.add(s.hand.phase);

		// `hand.result` is written by advance() during the engine-only hand_score
		// phase, so sampling only after apply() misses every hand outcome.
		if (s.hand.result && !handResults.some((r) => r.startsWith(`${s.hand.handNo}:`))) {
			handResults.push(`${s.hand.handNo}:${s.hand.result}`);
			if (s.hand.result === 'euchre') euchres++;
			if (narrate) narrateHandEnd(s);
		}

		if (narrate) {
			if (s.hand.handNo !== lastHandNo && s.hand.phase === 'bid_round_1') {
				lastHandNo = s.hand.handNo;
				hands++;
				narrateHandStart(s);
			}
			if (lastPhase === 'bid_round_1' && s.hand.phase !== 'bid_round_1' && s.hand.trump) {
				narrateTrump(s);
			}
			const done = (s.hand.trickLog ?? []).length;
			if (done > lastTrickCount) {
				const t = s.hand.trickLog[done - 1]!;
				console.log(`        → ${seatName(t.winnerSeat!)} takes it.`);
				lastTrickCount = done;
			}
		} else if (s.hand.handNo !== lastHandNo && s.hand.phase === 'bid_round_1') {
			lastHandNo = s.hand.handNo;
			hands++;
		}
		lastPhase = s.hand.phase;

		let actor: Seat | null = null;
		let moves: LegalMove[] = [];
		for (const seat of SEATS) {
			const m = legalMoves(s, seat);
			if (m.length > 0) {
				actor = seat;
				moves = m;
				break;
			}
		}

		if (actor === null) {
			const r = advance(s);
			if (r.state === s) {
				f.check(false, `${tag}: advance() made no progress in phase ${s.hand.phase}`);
				break;
			}
			s = r.state;
			continue;
		}

		const alsoActing = SEATS.filter((x) => x !== actor && legalMoves(s, x).length > 0);
		f.check(alsoActing.length === 0, `${tag}: seats ${actor},${alsoActing.join(',')} can act at once`);
		f.check(actor === s.hand.turnSeat, `${tag}: legalMoves offered seat ${actor}, turnSeat is ${s.hand.turnSeat}`);

		const pick = moves[Math.floor(rng() * moves.length)]!;
		moveIdsSeen.add(pick.id);
		if (pick.id.endsWith('+alone')) loners++;

		if (rng() < 0.05) probeIllegal(s, actor, moves, f, tag);
		if (narrate) narrateMove(s, actor, pick);

		const before = s;
		s = apply(s, actor, pick.move).state;
		f.check(s !== before, `${tag}: apply(${pick.id}) returned the same state object`);

		checkCardConservation(s, f, tag);
		checkRedaction(s, f, tag);
		// Exercise the shared validator (spec §8) alongside our independent checks,
		// so a bug in one is not masked by agreement with the other.
		try {
			assertInvariants(s);
			assertLegalNonEmpty(s, legalMoves);
		} catch (e) {
			const msg = e instanceof InvariantError ? e.message : String(e);
			f.check(false, `${tag}: assertInvariants threw — ${msg}`);
		}

		const d0 = s.score[0] - lastScore[0];
		const d1 = s.score[1] - lastScore[1];
		f.check(d0 >= 0 && d1 >= 0, `${tag}: score went backwards ${lastScore} -> ${s.score}`);
		f.check([0, 1, 2, 4].includes(d0) && [0, 1, 2, 4].includes(d1), `${tag}: illegal delta ${d0}/${d1}`);
		f.check(!(d0 > 0 && d1 > 0), `${tag}: both teams scored on one hand`);
		lastScore = s.score;

		checkFollowSuit(s, f, tag);
		checkTrickWinners(s, f, tag);
		checkLonerShape(s, f, tag);
	}

	phasesSeen.add(s.hand.phase);
	if (s.hand.result && !handResults.some((r) => r.startsWith(`${s.hand.handNo}:`))) {
		handResults.push(`${s.hand.handNo}:${s.hand.result}`);
		if (s.hand.result === 'euchre') euchres++;
	}

	f.check(steps < MAX_STEPS, `${tag}: did not terminate within ${MAX_STEPS} steps`);
	f.check(s.status === 'complete', `${tag}: ended with status ${s.status}`);
	const target = s.cfg.gameTo;
	f.check(
		s.score[0] >= target || s.score[1] >= target,
		`${tag}: complete but neither team reached ${target} (${s.score})`
	);
	f.check(
		!(s.score[0] >= target && s.score[1] >= target),
		`${tag}: both teams at or past ${target} (${s.score})`
	);
	// legalMoves must close down once the match is over.
	for (const seat of SEATS) {
		f.check(legalMoves(s, seat).length === 0, `${tag}: seat ${seat} still has moves after game over`);
	}

	return {
		hands,
		finalScore: s.score,
		winner: s.score[0] >= target ? 0 : 1,
		steps,
		phasesSeen,
		moveIdsSeen,
		handResults,
		loners,
		euchres
	};
}

/** The engine must refuse a move outside the legal set, without mutating state. */
function probeIllegal(s: GameState, seat: Seat, legal: LegalMove[], f: Failures, tag: string): void {
	const legalIds = new Set(legal.map((m) => m.id));
	const bogus = DECK.find((c) => !legalIds.has(`play:${c}`));
	if (!bogus) return;
	const before = JSON.stringify(s);
	let rejected = false;
	try {
		apply(s, seat, { t: 'play', card: bogus });
	} catch {
		rejected = true;
	}
	f.check(rejected, `${tag}: illegal play:${bogus} was accepted for seat ${seat} in ${s.hand.phase}`);
	f.check(JSON.stringify(s) === before, `${tag}: rejected move mutated state in place`);

	// A seat that is not on turn must never be able to act.
	const off = SEATS.find((x) => x !== s.hand.turnSeat);
	if (off !== undefined) {
		f.check(legalMoves(s, off).length === 0, `${tag}: off-turn seat ${off} has legal moves`);
	}
}

/* ----------------------------------------------------------------- narration */

const GLYPH: Record<string, string> = { S: '♠', H: '♥', D: '♦', C: '♣' };
const card = (c: CardId) => `${c[0] === 'T' ? '10' : c[0]}${GLYPH[c[1]!]}`;
const seatName = (s: Seat) => ['South(you)', 'West', 'North', 'East'][s]!;
const pad = (s: Seat) => seatName(s).padEnd(10);

function narrateHandStart(s: GameState): void {
	const h = s.hand;
	console.log(
		`\n┌─ Hand ${h.handNo + 1} ── dealer ${seatName(h.dealerSeat)} ── up-card ${card(h.upCard!)} ── score ${s.score[0]}–${s.score[1]} ─┐`
	);
	for (const seat of SEATS) {
		console.log(`│ ${pad(seat)} ${(h.hands[seat] ?? []).map(card).join(' ')}`);
	}
}

function narrateTrump(s: GameState): void {
	const h = s.hand;
	const alone = h.aloneSeat !== null ? `  ALONE — ${seatName(h.sittingSeat!)} sits out` : '';
	console.log(`│ ▸ ${GLYPH[h.trump!]} is trump, called by ${seatName(h.makerSeat!)}${alone}`);
}

function narrateMove(s: GameState, seat: Seat, m: LegalMove): void {
	const h = s.hand;
	switch (m.move.t) {
		case 'cut':
			return;
		case 'pass':
			console.log(`│   ${pad(seat)} "Pass."`);
			return;
		case 'orderUp':
			console.log(`│   ${pad(seat)} "${m.label}"`);
			return;
		case 'call':
			console.log(`│   ${pad(seat)} "${m.label}"`);
			return;
		case 'discard':
			console.log(`│   ${pad(seat)} picks up, discards face down.`);
			return;
		case 'play': {
			const isLead = (h.trick?.plays ?? []).length === 0;
			const t = (h.trickLog ?? []).length + 1;
			console.log(
				`│     ${isLead ? `T${t} ` : '   '}${pad(seat)} ${card(m.move.card)}${isLead ? '  (leads)' : ''}`
			);
			return;
		}
	}
}

function narrateHandEnd(s: GameState): void {
	const h = s.hand;
	console.log(
		`│ ═ ${String(h.result).toUpperCase().padEnd(10)} tricks ${h.tricksWon[0]}–${h.tricksWon[1]}` +
			`  →  score ${s.score[0]}–${s.score[1]}`
	);
	console.log('└' + '─'.repeat(62));
}

/* ---------------------------------------------------------------------- main */

const N = Number(process.argv[2] ?? 300);
const QUIET = process.argv.includes('quiet');
const f = new Failures();

if (!QUIET) {
	console.log('═'.repeat(64));
	console.log('NARRATED GAME — seed "demo-7", random-but-legal play');
	console.log('═'.repeat(64));
	const g = playGame('demo-7', mulberry32(4242), f, true);
	console.log(
		`\nFINAL ${g.finalScore[0]}–${g.finalScore[1]} — team ${g.winner} wins ` +
			`after ${g.hands} hands, ${g.euchres} euchres, ${g.loners} loner calls.`
	);
}

console.log('\n' + '═'.repeat(64));
console.log(`FUZZ — ${N} complete games, every invariant checked at every step`);
console.log('═'.repeat(64));

const phases = new Set<string>();
const moveIds = new Set<string>();
const results = new Map<string, number>();
let hands = 0;
let steps = 0;
let loners = 0;
let euchres = 0;
const t0 = performance.now();

for (let i = 0; i < N; i++) {
	const g = playGame(`fuzz-${i}`, mulberry32(i * 7919 + 13), f, false);
	hands += g.hands;
	steps += g.steps;
	loners += g.loners;
	euchres += g.euchres;
	for (const p of g.phasesSeen) phases.add(p);
	for (const m of g.moveIdsSeen) moveIds.add(m);
	for (const r of g.handResults) {
		const k = r.split(':')[1]!;
		results.set(k, (results.get(k) ?? 0) + 1);
	}
}
const ms = performance.now() - t0;

const a = JSON.stringify(playGame('det', mulberry32(99), f, false));
const b = JSON.stringify(playGame('det', mulberry32(99), f, false));
f.check(a === b, 'determinism: identical seed + rng produced a different game');

console.log(`games            ${N}`);
console.log(`hands            ${hands}  (avg ${(hands / N).toFixed(1)}/game)`);
console.log(`engine steps     ${steps}`);
console.log(`loner calls      ${loners}`);
console.log(`euchres          ${euchres}`);
console.log(`elapsed          ${ms.toFixed(0)}ms  (${(ms / N).toFixed(2)}ms/game)`);
console.log(`phases exercised ${[...phases].sort().join(', ')}`);
console.log(`distinct moves   ${moveIds.size}`);
console.log(`hand results     ${[...results].map(([k, v]) => `${k}=${v}`).join('  ')}`);

console.log('\n' + '═'.repeat(64));
if (f.items.length === 0) {
	console.log('✓ ALL INVARIANTS HELD');
	for (const line of [
		'no secret card ever appeared in any seat\'s projected view',
		'all 24 cards conserved in every reachable state',
		'hand sizes correct for every seat at every trick',
		'follow-suit re-derived from replayed holdings, never violated',
		'trick winners recomputed independently, always agreed',
		'recorded ledSuit always the effective suit of the lead',
		'loner hands had exactly 3 plays/trick, sitting seat never played',
		'scores monotonic, only +1/+2/+4, never both teams in one hand',
		'illegal moves rejected without mutating state',
		'off-turn seats never had legal moves',
		'exactly one seat could act at any moment',
		'every game terminated with exactly one team at the target',
		'identical seed reproduced an identical game'
	]) {
		console.log(`  · ${line}`);
	}
	process.exit(0);
} else {
	console.log(`✗ ${f.items.length} DISTINCT FAILURES`);
	for (const m of f.items.slice(0, 30)) console.log(`  · ${m}`);
	process.exit(1);
}
