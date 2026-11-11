/**
 * JSON golden generator for the `euchre-protocol` Rust port.
 *
 * Runs the real TS engine (read-only, imported by absolute path) through a small
 * harness that mirrors the `euchreTable` actor's `commit` / `settle` / tempo
 * advance, and writes the wire payloads of spec 02 section 12 to
 * `tests/fixtures/*.json`. Payloads that the TS app builds inside actor code
 * (which is not importable without rivetkit) are built here with the exact
 * object-literal key order the actor uses; the line each copies is cited.
 *
 * Usage (from the repo root):  bun run crates/euchre-protocol/scripts/gen-fixtures.ts
 *
 * Override the TS source location with EUCHRE_TS_SRC (default ~/goods/euchre).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

const SRC = process.env.EUCHRE_TS_SRC ?? join(homedir(), 'goods/euchre');
const E = await import(join(SRC, 'src/lib/euchre/index.ts'));
const P = await import(join(SRC, 'src/lib/protocol/pacing.ts'));
const Personas = await import(join(SRC, 'src/lib/server/personas.ts'));

const OUT = join(dirname(new URL(import.meta.url).pathname), '..', 'tests', 'fixtures');
mkdirSync(OUT, { recursive: true });

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

/** PersonaConfig in its interface key order (protocol/index.ts:659-688). */
function canonPersona(p: Any): Any {
	return {
		id: p.id,
		version: p.version,
		name: p.name,
		blurb: p.blurb,
		prompt: p.prompt,
		housePrompt: p.housePrompt,
		difficulty: p.difficulty,
		aggression: p.aggression,
		risk: p.risk,
		chattiness: p.chattiness,
		temperature: p.temperature,
		modelId: p.modelId,
		bidModelId: p.bidModelId
	};
}

const personas = (Personas.defaultPersonas() as Any[]).map((a) => ({
	seat: a.seat,
	persona: canonPersona(a.persona),
	dossier: a.dossier
}));

/* ------------------------------------------------------------------------ */
/* A miniature euchreTable: commit / settle / observe / tempo                */
/* ------------------------------------------------------------------------ */

const GAME_ID = '6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b';
const cfg = { gameTo: 10, stickTheDealer: false, requireNaturalTrump: false };
const createInput = {
	ownerUserId: 'guest-0b5f2a64-5d1e-4c33-9a8e-2f7c1d0e9b11',
	profileUserId: 'guest-0b5f2a64-5d1e-4c33-9a8e-2f7c1d0e9b11',
	seed: '9e3779b97f4a7c15',
	cfg,
	firstDealer: 3,
	personas
};

let game: Any = E.createGame({
	gameId: GAME_ID,
	seed: createInput.seed,
	internalToken: '1c6f0a52-8d3e-4b7a-9f21-6e5d4c3b2a10',
	cfg,
	firstDealer: 3
});
let observations: Any[] = [];
const reviewTricks = false;
let reviewReadyTurnId: string | null = null;
let journal: Any[] = [];

/** index.ts:540 */
function commit(state: Any): void {
	game = { ...state, v: game.v + 1 };
}

/** index.ts settle(): advance through lobby / deal only. */
function settle(steps: Any[]): Any[] {
	while (game.hand.phase === 'lobby' || game.hand.phase === 'deal') {
		const out = E.advance(game);
		if (out.state === game) break;
		commit(out.state);
		steps.push(...out.steps);
	}
	return steps;
}

function decisionKindFor(phase: string): string | null {
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

/** index.ts:551-580 observeMove, with a fixed 1740 ms human think time. */
function observeMove(before: Any, seat: number, move: Any): void {
	const kind = decisionKindFor(before.hand.phase);
	if (!kind) return;
	const human = seat === 0;
	const elapsed = 1740;
	const reliable = human;
	const action =
		move.move.t === 'play' || ['pass', 'orderUp', 'call'].includes(move.move.t)
			? move.label
			: kind === 'discard'
				? 'Dealer discarded a private card'
				: 'Cut decision';
	observations = [
		...observations.filter((o) => o.handNo === before.hand.handNo),
		{
			handNo: before.hand.handNo,
			seat,
			kind,
			action,
			elapsedMs: reliable ? Math.round(elapsed / 250) * 250 : null,
			timing: human ? (reliable ? 'human-observed' : 'unreliable') : 'computer-paced'
		}
	].slice(-32);
}

/** index.ts:453-473 tableView. */
function tableView(seat: number): Any {
	return {
		...E.project(game, seat),
		players: personas.map(({ seat, persona }) => ({
			seat,
			name: persona.name,
			blurb: persona.blurb,
			style:
				persona.risk >= 0.7 ? 'Bold bidder' : persona.risk <= 0.3 ? 'Card counter' : 'Steady player'
		})),
		observations: observations.filter((o) => o.handNo === game.hand.handNo),
		reviewTricks: reviewTricks ?? false,
		rules: {
			stickTheDealer: game.cfg.stickTheDealer,
			requireNaturalTrump: game.cfg.requireNaturalTrump ?? false
		},
		awaitingTrickReview: game.hand.phase === 'trick_resolve' && reviewReadyTurnId === game.turnId
	};
}

/** index.ts:475 pushSync payload for seat 0. */
function syncFor(steps: Any[]): Any {
	return { v: game.v, view: tableView(0), steps: E.projectSteps(steps, 0) };
}

/** index.ts:1033-1052 archiveHand. */
function archive(hand: Any): Any {
	return {
		handNo: hand.handNo,
		dealerSeat: hand.dealerSeat,
		tricksWon: hand.tricksWon,
		upCard: hand.upCard,
		trump: hand.trump,
		makerSeat: hand.makerSeat,
		aloneSeat: hand.aloneSeat,
		tricks: hand.trickLog,
		buried: [
			...hand.kitty.slice(1),
			...(hand.dealerDiscard ? [hand.dealerDiscard] : hand.upCard ? [hand.upCard] : [])
		],
		unplayed: Object.values(hand.hands).flat(),
		result: hand.result,
		delta: hand.delta
	};
}

/** index.ts:1055-1073 recordHand entry. */
function journalEntry(hand: Any, endedAt: number): Any {
	return {
		matchId: GAME_ID,
		handNo: hand.handNo,
		seed: game.seed,
		dealerSeat: hand.dealerSeat,
		deckOrder: hand.deckOrder,
		moves: journal,
		trump: hand.trump,
		makerSeat: hand.makerSeat,
		aloneSeat: hand.aloneSeat,
		tricksWon: hand.tricksWon,
		result: hand.result,
		delta: hand.delta,
		endedAt
	};
}

/* ------------------------------------------------------------------------ */
/* Play one deterministic match, capturing payloads as they occur            */
/* ------------------------------------------------------------------------ */

function pickMove(legal: Any[]): Any {
	const h = game.hand;
	const ids = legal.map((m: Any) => m.id);
	const by = (id: string) => legal.find((m: Any) => m.id === id);
	if (h.phase === 'cutting') return by(h.handNo % 2 === 0 ? 'cut:no' : 'cut:yes') ?? legal[0];
	if (h.handNo === 0 && (h.phase === 'bid_round_1' || h.phase === 'bid_round_2')) {
		return by('pass') ?? legal[0];
	}
	if (h.phase === 'bid_round_1') {
		if (h.handNo % 2 === 1 && ids.includes('orderUp')) {
			return h.handNo % 4 === 3 && ids.includes('orderUp+alone') ? by('orderUp+alone') : by('orderUp');
		}
		return by('pass') ?? legal[0];
	}
	if (h.phase === 'bid_round_2') {
		const calls = legal.filter((m: Any) => m.move.t === 'call');
		const alone = h.handNo % 4 === 2;
		return calls.find((m: Any) => m.move.alone === alone) ?? calls[0] ?? legal[0];
	}
	return legal[0];
}

const captured: Record<string, Any> = {};
const stepByType: Record<string, Any> = {};
let leadView: Any = null;
let syncAfterPlay: Any = null;
let dealSync: Any = null;
let aiRequest: Any = null;
let ruleErrorAck: Any = null;
let firstSync: Any = null;
let scoredReview: Any = null;
let throwInReview: Any = null;
let scoredHand: Any = null;
let throwInHand: Any = null;
let entry: Any = null;
let endedAt = 1767996000000;

firstSync = (() => {
	const steps = settle([]);
	return syncFor(steps);
})();

for (let guard = 0; guard < 2000 && game.status === 'active'; guard++) {
	const phase = game.hand.phase;
	if (phase === 'trick_resolve' || phase === 'hand_score') {
		const before = game;
		if (phase === 'hand_score' && scoredReview === null) {
			scoredReview = archive(before.hand);
			scoredHand = before.hand;
			entry = journalEntry(before.hand, (endedAt += 60_000));
		}
		const out = E.advance(before);
		commit(out.state);
		const steps = settle([...out.steps]);
		for (const s of E.projectSteps(steps, 0)) stepByType[s.t] ??= s;
		if (game.hand.phase === 'cutting' && game.hand.handNo !== before.hand.handNo) journal = [];
		continue;
	}
	const seat = game.hand.turnSeat;
	if (seat === null) throw new Error(`stuck in ${phase}`);
	const legal = E.legalMoves(game, seat);
	const move = pickMove(legal);

	if (seat === 0 && phase === 'trick_play' && leadView === null && game.hand.trick.plays.length === 0) {
		leadView = tableView(0);
	}
	if (seat !== 0 && phase === 'trick_play' && aiRequest === null && legal.length > 1) {
		aiRequest = {
			internalToken: game.internalToken,
			gameId: game.gameId,
			seat,
			turnId: game.turnId,
			kind: decisionKindFor(phase),
			view: tableView(seat),
			legal,
			deadlineAt: 1767996004000
		};
	}
	if (seat === 0 && ruleErrorAck === null && legal.length > 1) {
		try {
			E.apply(game, 0, move.move, { turnId: 'stale-turn' });
		} catch (err: Any) {
			if (!E.isRuleError(err)) throw err;
			ruleErrorAck = { ok: false, code: err.code, legal: err.legal };
		}
	}

	const before = game;
	const out = E.apply(before, seat, move.move, { turnId: before.turnId });
	commit(out.state);
	observeMove(before, seat, move);
	journal = [...journal, { seat, id: move.id, move: move.move, kind: seat === 0 ? 'human' : 'ai', ...(seat === 0 ? {} : { source: 'fallback' }) }].slice(-40);
	const steps = settle([...out.steps]);
	for (const s of E.projectSteps(steps, 0)) stepByType[s.t] ??= s;

	if (steps.some((s: Any) => s.t === 'throwIn') && throwInReview === null) {
		throwInReview = archive(before.hand);
		throwInHand = before.hand;
	}
	if (game.hand.handNo !== before.hand.handNo) journal = [];
	if (phase === 'cutting' && dealSync === null) dealSync = syncFor(steps);
	if (seat === 0 && phase === 'trick_play' && syncAfterPlay === null) syncAfterPlay = syncFor(steps);
}
if (game.status !== 'complete') throw new Error('match did not finish');

const required = [
	'cut',
	'dealt',
	'upCardTurned',
	'bid',
	'turnedDown',
	'trumpSet',
	'dealerDiscarded',
	'cardPlayed',
	'trickWon',
	'handScored',
	'throwIn',
	'gameWon'
];
for (const t of required) if (!stepByType[t]) throw new Error(`no ${t} step captured`);
for (const [k, v] of Object.entries({ leadView, syncAfterPlay, dealSync, aiRequest, ruleErrorAck, scoredReview, throwInReview, entry })) {
	if (v === null) throw new Error(`nothing captured for ${k}`);
}

/* ------------------------------------------------------------------------ */
/* Fixtures                                                                  */
/* ------------------------------------------------------------------------ */

const outcome = game.winnerTeam === 0 ? 'won' : game.winnerTeam === 1 ? 'lost' : 'abandoned';
const stats = {
	euchresFor: 1,
	euchresAgainst: 0,
	lonersAttempted: 2,
	lonersMade: 1,
	marches: 3,
	throwIns: 1,
	tricks: [31, 29]
};
const tokens = { tokensIn: 0, tokensOut: 0, cacheReadTokens: 0, calls: 0 };

const write = (name: string, value: unknown) => {
	writeFileSync(join(OUT, name), JSON.stringify(value, null, '\t') + '\n');
	captured[name] = value;
};

write('view.json', {
	tableView: leadView,
	pureProject: E.project(game, 0),
	storeTest: E.project(E.createGame({ gameId: 'store-test', seed: 'store-test' }), 0)
});

write('sync.json', {
	first: firstSync,
	afterPlay: syncAfterPlay,
	deal: dealSync,
	// onReview `set`: commit (v+1, same turnId) then pushSync(c, []) with reviewTricks on.
	reviewGate: {
		v: syncAfterPlay.v + 1,
		view: { ...syncAfterPlay.view, v: syncAfterPlay.v + 1, reviewTricks: true },
		steps: []
	}
});

write('steps.json', required.map((t) => stepByType[t]));

write('events.json', {
	thinking: { seat: 1, on: true, extended: false },
	chatCall: { msgId: `${GAME_ID}:58:0`, seat: 1, kind: 'call', text: 'I assist.', final: true },
	chatSystem: {
		msgId: `${GAME_ID}:12:1`,
		seat: 0,
		kind: 'system',
		text: 'Throw it in. Nobody wanted it.',
		final: true
	},
	chatBanter: { msgId: `${GAME_ID}:n7p0c3:2`, seat: 2, kind: 'banter', text: 'Nice lead.', final: true },
	chatDelta: { msgId: `${GAME_ID}:n7p0c3:2`, seat: 2, delta: 'Nice ' },
	presence: { seat: 0, online: true },
	tableEventNames: ['sync', 'thinking', 'chat', 'chatDelta', 'presence'],
	perConnectionEvents: ['sync']
});

write('actions.json', {
	submitMove: {
		moveId: leadView.legal[0].id,
		turnId: leadView.turnId,
		clientMoveId: '2d9a3c40-6f1b-4e8a-b0c2-7d5e3f9a1b64'
	},
	ackOk: { ok: true, v: syncAfterPlay.v },
	ackErr: ruleErrorAck,
	ackGameComplete: { ok: false, code: 'game_complete', legal: [] },
	reviewSet: { action: 'set', enabled: true },
	reviewContinue: { action: 'continue', turnId: leadView.turnId },
	reviewAck: { queued: true },
	connectParams: { token: 'eyJhbGciOiJIUzI1NiJ9.e30.sig', protocolVersion: 1 },
	connectParamsInternal: { internalToken: game.internalToken, token: '' },
	connectParamsFull: {
		internalToken: game.internalToken,
		token: 'eyJhbGciOiJIUzI1NiJ9.e30.sig',
		seat: 0,
		protocolVersion: 1
	},
	connState: { userId: createInput.ownerUserId, seat: 0, role: 'player', since: 1767996000123 },
	connStateInternal: { userId: 'internal', seat: 0, role: 'internal', since: 1767996000456 },
	createInput,
	createInputEmpty: {},
	userError: {
		code: 'illegal_move',
		message: 'That move is not legal right now',
		metadata: { legal: leadView.legal }
	},
	protocolError: { code: 'forbidden', message: 'Not your game', retryable: false },
	protocolErrorBare: { code: 'stale_turn', message: 'stale' },
	tableSession: {
		actorId: 'a1b2c3d4e5',
		token: 'eyJhbGciOiJIUzI1NiJ9.e30.sig',
		gatewayToken: 'gw.token',
		expiresAt: 1767996900000
	},
	tableSessionNoGateway: {
		actorId: 'a1b2c3d4e5',
		token: 'eyJhbGciOiJIUzI1NiJ9.e30.sig',
		expiresAt: 1767996900000
	},
	moveMsg: {
		seq: 4,
		turnId: leadView.turnId,
		moveId: leadView.legal[0].id,
		clientMoveId: '2d9a3c40-6f1b-4e8a-b0c2-7d5e3f9a1b64',
		userId: createInput.ownerUserId
	},
	reviewMsgSet: {
		internalToken: game.internalToken,
		gameId: GAME_ID,
		action: 'set',
		enabled: true,
		turnId: leadView.turnId
	},
	reviewMsgContinue: {
		internalToken: game.internalToken,
		gameId: GAME_ID,
		action: 'continue',
		turnId: leadView.turnId
	},
	tickMsg: { internalToken: game.internalToken, gameId: GAME_ID, kind: 'tempo', turnId: leadView.turnId },
	tickMsgTimeout: {
		internalToken: game.internalToken,
		gameId: GAME_ID,
		kind: 'aiTimeout',
		turnId: leadView.turnId,
		seat: 2
	}
});

/** Mirrors aiSeat's `table.send('aiSay', …)` literal (ai-seat/index.ts:400-409). */
function aiSayFinal(o: { internalToken: string; gameId: string; seat: number; turnId: string; handNo: number; text: string }): Any {
	const state = { internalToken: o.internalToken, gameId: o.gameId, seat: o.seat };
	const body = { turnId: o.turnId, view: { handNo: o.handNo } };
	const text = o.text;
	return {
		internalToken: state.internalToken,
		gameId: state.gameId,
		seat: state.seat,
		turnId: body.turnId,
		handNo: body.view.handNo,
		msgId: `${body.turnId}:talk:${state.seat}`,
		text,
		final: true
	};
}

write('ai.json', {
	decideRequest: aiRequest,
	decideRequestRanked: {
		internalToken: aiRequest.internalToken,
		gameId: aiRequest.gameId,
		seat: aiRequest.seat,
		turnId: aiRequest.turnId,
		kind: aiRequest.kind,
		view: aiRequest.view,
		legal: aiRequest.legal,
		ranking: aiRequest.legal.map((m: Any, i: number) => ({
			id: m.id,
			score: i === 0 ? 12 : 3.25 - i,
			why: i === 0 ? 'Takes the trick with the lowest winner.' : 'Saves a winner.'
		})),
		deadlineAt: aiRequest.deadlineAt
	},
	decision: {
		internalToken: game.internalToken,
		gameId: GAME_ID,
		seat: aiRequest.seat,
		turnId: aiRequest.turnId,
		moveId: aiRequest.legal[0].id,
		source: 'llm',
		rationale: 'Low trump keeps the bower for later.',
		latencyMs: 812
	},
	decisionFull: {
		internalToken: game.internalToken,
		gameId: GAME_ID,
		seat: aiRequest.seat,
		turnId: aiRequest.turnId,
		moveId: aiRequest.legal[0].id,
		source: 'fallback',
		banter: 'Your lead.',
		rationale: 'Heuristic.',
		latencyMs: 0
	},
	// The literal aiSeat sends (ai-seat/index.ts:400-409), key order included.
	sayFinal: aiSayFinal({
		internalToken: game.internalToken,
		gameId: GAME_ID,
		seat: 2,
		turnId: aiRequest.turnId,
		handNo: 3,
		text: 'Nice lead.'
	}),
	// No TS producer streams deltas; same order with `delta` after `msgId`.
	sayDelta: {
		internalToken: game.internalToken,
		gameId: GAME_ID,
		seat: 2,
		turnId: aiRequest.turnId,
		msgId: `${aiRequest.turnId}:talk:2`,
		delta: 'Nice ',
		final: false
	},
	lifecycleResetHand: { internalToken: game.internalToken, gameId: GAME_ID, kind: 'resetHand', handNo: 4 },
	lifecycleGameEnd: { internalToken: game.internalToken, gameId: GAME_ID, kind: 'gameEnd' },
	personas,
	personaView: {
		id: personas[0].persona.id,
		version: personas[0].persona.version,
		name: personas[0].persona.name,
		blurb: personas[0].persona.blurb,
		seat: 1,
		difficulty: personas[0].persona.difficulty,
		chattiness: personas[0].persona.chattiness,
		degraded: false
	}
});

write('review.json', {
	scored: scoredReview,
	throwIn: throwInReview,
	legacy: (() => {
		const { dealerSeat: _d, tricksWon: _t, upCard: _u, ...rest } = scoredReview;
		return rest;
	})(),
	nullUpCard: { ...scoredReview, upCard: null },
	// The HandState each archive was built from (server-only; fixture input for build_hand_review).
	scoredHand,
	throwInHand
});

write('records.json', {
	matchRecord: {
		schema: 1,
		matchId: GAME_ID,
		userId: createInput.profileUserId,
		seed: game.seed,
		cfg: game.cfg,
		firstDealer: game.firstDealer,
		startedAt: 1767995000000,
		endedAt,
		status: game.status,
		outcome,
		score: game.score,
		winnerTeam: game.winnerTeam,
		handsPlayed: game.handNo,
		personas,
		stats,
		tokens
	},
	liveMatchRecord: {
		schema: 1,
		matchId: GAME_ID,
		userId: createInput.profileUserId,
		seed: game.seed,
		cfg: game.cfg,
		firstDealer: game.firstDealer,
		startedAt: 1767995000000,
		endedAt: null,
		status: 'active',
		outcome: 'abandoned',
		score: [0, 0],
		winnerTeam: null,
		handsPlayed: 0,
		personas,
		stats,
		tokens
	},
	handJournalEntry: entry,
	matchSummary: {
		matchId: GAME_ID,
		playedAt: endedAt,
		status: game.status,
		outcome,
		score: game.score,
		handsPlayed: game.handNo,
		opponents: personas.map((p) => p.persona.name)
	},
	matchListPage: { rows: [], cursor: null },
	lifetimeStats: {
		matches: 4,
		won: 2,
		lost: 1,
		abandoned: 1,
		euchresFor: 3,
		euchresAgainst: 2,
		lonersMade: 1,
		handsPlayed: 41,
		updatedAt: endedAt
	},
	playerProfileState: {
		schema: 1,
		activeGameId: null,
		dailyTokens: { tokensIn: 1200, tokensOut: 340, cacheReadTokens: 0, calls: 7 },
		lifetime: {
			matches: 0,
			won: 0,
			lost: 0,
			abandoned: 0,
			euchresFor: 0,
			euchresAgainst: 0,
			lonersMade: 0,
			handsPlayed: 0,
			updatedAt: 1767995000000
		}
	},
	episode: {
		role: 'east-grandpa',
		matchId: GAME_ID,
		handNo: 3,
		ts: 1767995500000,
		kind: 'loner',
		summary: 'Went alone on hearts and made it.',
		salience: 0.9,
		expiresAt: 1767995500000 + 90 * 86_400_000
	}
});

write('pacing.json', {
	tableMotion: P.TABLE_MOTION,
	trickPresentationMs: P.TRICK_PRESENTATION_MS
});

/* JS number text (`JSON.stringify`) for `js_f64`: [f64 bits as hex, JS text]. */
{
	let seed = 0x9e3779b9;
	const rand32 = () => {
		seed = (seed + 0x6d2b79f5) | 0;
		let t = seed;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return (t ^ (t >>> 14)) >>> 0;
	};
	const buf = new DataView(new ArrayBuffer(8));
	const bitsOf = (v: number) => {
		buf.setFloat64(0, v);
		return buf.getBigUint64(0).toString(16).padStart(16, '0');
	};
	const values: number[] = [
		0.000001, 0.0000015, 0.00000999, 0.0000001, 1e-7, 2 ** 53, 2 ** 53 + 2, 2 ** 63, 1e19, 1e20,
		1e21, 1e21 + 2 ** 17, 5e-324, 1.7976931348623157e308, 0.1 + 0.2, 1 / 3, 2 / 3, -1.5e-6,
		123.456, 0.55, 0.85, 0.7, 0.35, 0.9, 0.25, -0.75
	];
	for (let i = 0; i < 120; i++) {
		// Random bit patterns: every exponent range.
		buf.setUint32(0, rand32());
		buf.setUint32(4, rand32());
		const v = buf.getFloat64(0);
		if (Number.isFinite(v)) values.push(v);
	}
	for (let i = 0; i < 60; i++) values.push(rand32() / 2 ** 32); // persona dials live in [0,1]
	for (let i = 0; i < 40; i++) values.push((rand32() / 2 ** 32) * 10 ** ((rand32() % 30) - 12));
	write(
		'numbers.json',
		values.map((v) => [bitsOf(v), JSON.stringify(v)])
	);
}

let total = 0;
for (const v of Object.values(captured)) total += JSON.stringify(v, null, '\t').length;
console.log(`wrote ${Object.keys(captured).length} fixtures (${(total / 1024).toFixed(1)} KB) to ${OUT}`);
