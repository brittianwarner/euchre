/**
 * `aiSeat` — key `["table", gameId, "seat", "1" | "2" | "3"]`. **One actor per AI
 * opponent.** This file is assembly, exactly like `player-profile/index.ts`:
 * every rung of the decision ladder lives in `./decide.ts` (`runLadder`), the
 * SDK seam is `./llm-bridge.ts`, card-counting and cross-hand notes are
 * `./memory.ts`, difficulty narrowing is `./difficulty.ts`, output screening is
 * `./screen.ts`, and every durable shape and cap is `./types.ts`. Nothing here
 * re-derives a rule those modules already own.
 *
 * ## Why a real actor and not an inline call
 *
 * Before this file existed, `euchreTable` parked a heuristic move, armed the
 * watchdog, persisted, and then called `decide()` from `$lib/ai` **directly**,
 * in-process. That worked, but it meant three opponents shared the table's own
 * failure domain, memory and token budget. Now each chair is its own actor with
 * its own persona, its own bounded per-hand memory, its own circuit breaker and
 * its own crash domain: a seat that wedges on a hung model call takes exactly
 * that one chair down, never the table, and the table's watchdog auto-plays for
 * it exactly as before.
 *
 * ## The durability ordering this seat must never disturb
 *
 * The table still parks a legal fallback move, arms the watchdog and reveal
 * schedules, and persists — **all before** sending this actor anything. That
 * ordering lives in `euchreTable`'s dispatch and is unchanged: this actor's
 * reply is a pure upgrade of an already-safe decision, never a dependency the
 * table waits on. Concretely, that is why `decide` is a **queue**, not a
 * blocking action — `c.queue.send(...)` returns as soon as the message is
 * durable, and the table moves on immediately. This seat's reply travels back
 * over the table's own `aiDecision` queue, which independently re-checks the
 * token, re-derives `legalMoves()`, and takes the acting seat from
 * `state.hand.turnSeat` — never from anything this actor claims.
 *
 * ## What this actor is trusted with, and what it structurally cannot see
 *
 * `AIDecideRequest.view` is `project(state, seat)` — the same redaction a human
 * in this chair would get. Nothing in `./decide.ts`, `./memory.ts` or
 * `./difficulty.ts` imports `GameState`, so an opponent seeing another hand is a
 * compile error rather than a policy this file has to remember to enforce.
 *
 * @see docs/03-AI-AGENTS.md §1, §7, §10 · docs/06-REVISED-ARCHITECTURE.md §4
 */

import { actor, queue } from 'rivetkit';
import { teamOf } from '$lib/euchre';
import {
	MODEL_BID,
	MODEL_PLAY,
	type AIDecideRequest,
	type AIDecision,
	type AiSeatLifecycleMessage,
	type PersonaConfig,
	type PersonaView,
	type Seat
} from '$lib/protocol';
import { runLadder, type LadderOutcome } from './decide';
import { absorbPublicState, episodeFor, rememberHand, resetMemory } from './memory';
import {
	BUDGET_CALLS,
	BUDGET_INPUT_TOKENS,
	BUDGET_OUTPUT_TOKENS,
	DECISION_LOG_CAP,
	EPISODE_BUFFER_CAP,
	LATENCY_SAMPLE_CAP,
	type AiSeatCreateInput,
	type AiSeatState,
	type AiStatus,
	type DecisionRecord
} from './types';

/* ========================================================================== */
/* Small pure helpers                                                        */
/* ========================================================================== */

/** True when no Rivet Cloud endpoint is configured, i.e. `bun run dev`. */
function isLocalDev(): boolean {
	return !process.env.RIVET_ENDPOINT;
}

/**
 * The deployment-wide secret for calls into `playerProfile`.
 *
 * Deliberately **not** `state.internalToken` — that is the per-match secret the
 * table minted and handed this seat, and `playerProfile` outlives every match
 * and has no way to know it. Mirrors `player-profile/auth.ts`'s
 * `internalSecret()` exactly, dev fallback included, because both sides must
 * agree on the same value without importing from each other.
 */
function profileInternalToken(): string | null {
	const secret = process.env.EUCHRE_INTERNAL_TOKEN;
	if (typeof secret === 'string' && secret.length >= 16) return secret;
	if (isLocalDev()) return 'euchre-dev-insecure-secret';
	return null;
}

/** Constant-time compare against this match's own `internalToken`. */
function internalTokenOk(expected: string, got: unknown): boolean {
	if (typeof got !== 'string' || got.length !== expected.length) return false;
	let diff = 0;
	for (let i = 0; i < expected.length; i++) diff |= expected.charCodeAt(i) ^ got.charCodeAt(i);
	return diff === 0;
}

/** Every chair this actor may occupy. Seat 0 is the human and never reaches here. */
function seatFromKey(key: readonly string[]): Seat {
	const raw = key[3];
	if (raw === '1' || raw === '2' || raw === '3') return Number(raw) as Seat;
	throw new Error(`aiSeat must be keyed ['table', gameId, 'seat', '1'|'2'|'3']; got ${JSON.stringify(key)}`);
}

/** The placeholder persona for a seat woken with no `createWithInput`. Never decides. */
function blankPersona(): PersonaConfig {
	return {
		id: 'unprovisioned',
		version: 0,
		name: 'Seat',
		blurb: '',
		prompt: '',
		housePrompt: '',
		difficulty: 'casual',
		aggression: 0.5,
		risk: 0.5,
		chattiness: 0,
		temperature: 0.5,
		modelId: MODEL_PLAY,
		bidModelId: MODEL_BID
	};
}

function average(samples: readonly number[]): number {
	if (samples.length === 0) return 0;
	let sum = 0;
	for (const s of samples) sum += s;
	return sum / samples.length;
}

function p95(samples: readonly number[]): number {
	if (samples.length === 0) return 0;
	const sorted = [...samples].sort((a, b) => a - b);
	const idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * 0.95) - 1));
	return sorted[idx] ?? 0;
}

function pushSample(samples: number[], value: number): void {
	samples.push(value);
	while (samples.length > LATENCY_SAMPLE_CAP) samples.shift();
}

/* ========================================================================== */
/* The actor↔actor authorization boundary (queues)                           */
/* ========================================================================== */

interface Guard {
	readonly conn?: unknown;
}

/**
 * Every queue here is actor-to-actor only; no browser ever connects to a seat
 * directly. Fails open for the stateless HTTP handle exactly like
 * `euchre-table/queues.ts`'s `externalDenied` — defence in depth on top of
 * {@link internalTokenOk}, never instead of it.
 */
function externalDenied(c: Guard): boolean {
	return c.conn === undefined;
}

const aiSeatQueues = {
	/** `euchreTable` → here. The only message that produces an `AIDecision`. */
	decide: queue<AIDecideRequest, undefined, Guard>({ canPublish: externalDenied }),
	/** `euchreTable` → here, at every deal. Clears per-hand card-counting memory. */
	resetHand: queue<AiSeatLifecycleMessage, undefined, Guard>({ canPublish: externalDenied }),
	/** `euchreTable` → here, at every hand boundary. Buffers a cross-hand episode. */
	handEnd: queue<AiSeatLifecycleMessage, undefined, Guard>({ canPublish: externalDenied }),
	/** `euchreTable` → here, once. Flushes episodes and today's token spend. */
	gameEnd: queue<AiSeatLifecycleMessage, undefined, Guard>({ canPublish: externalDenied })
};

/* ========================================================================== */
/* The context and the sibling client, structurally                          */
/* ========================================================================== */

/**
 * The narrow slice of the actor context the handlers below use.
 *
 * Hand-declared rather than `ActionContextOf<typeof aiSeat>` for the same reason
 * `euchreTable` does it: these helpers are called from inside `run`, which is
 * part of the definition such a derived type would have to reference.
 */
interface SeatCtx {
	state: AiSeatState;
	readonly key: readonly string[];
	readonly log: {
		info(...args: unknown[]): void;
		warn(...args: unknown[]): void;
		error(...args: unknown[]): void;
	};
	saveState(opts?: { immediate?: boolean }): Promise<void>;
	client(): unknown;
}

/** A handle on another actor, narrowed to the one verb this seat uses. */
interface ActorSendHandle {
	send(name: string, body: unknown): Promise<unknown>;
}

/**
 * The two siblings this seat talks to, declared structurally to avoid the
 * `registry.ts ↔ aiSeat` type cycle documented for `euchreTable`'s identical
 * `TableClient`.
 */
interface SeatClient {
	readonly euchreTable: { getOrCreate(key: readonly string[]): ActorSendHandle };
	readonly playerProfile: { getOrCreate(key: readonly string[]): ActorSendHandle };
}

function seatClient(c: SeatCtx): SeatClient {
	return c.client() as SeatClient;
}

/* ========================================================================== */
/* The `decide` queue — the whole point of this actor                        */
/* ========================================================================== */

function recordDecision(
	state: AiSeatState,
	req: AIDecideRequest,
	outcome: LadderOutcome,
	latencyMs: number
): void {
	const meter = state.meter;
	meter.decisions += 1;
	meter.llmCalls += outcome.llmCalls;
	meter.illegalAttempts += outcome.illegalAttempts;
	if (outcome.source === 'llm') meter.llmWins += 1;
	else if (outcome.source === 'fallback') meter.fallbacks += 1;
	else if (outcome.source === 'forced') meter.forced += 1;
	if (outcome.firstLegalRescue) meter.firstLegalRescues += 1;

	pushSample(meter.samples, latencyMs);
	meter.avgLatencyMs = average(meter.samples);
	meter.p95LatencyMs = p95(meter.samples);

	const entry: DecisionRecord = {
		turnId: req.turnId,
		handNo: req.view.handNo,
		kind: req.kind,
		moveId: outcome.moveId,
		source: outcome.source,
		latencyMs,
		candidates: outcome.candidates,
		rationale: outcome.rationale
	};
	state.decisionLog.push(entry);
	while (state.decisionLog.length > DECISION_LOG_CAP) state.decisionLog.shift();
}

/** Fold model usage into the match budget and trip the circuit breaker if it is spent. */
function applyUsage(
	state: AiSeatState,
	usage: { readonly inputTokens: number; readonly outputTokens: number; readonly cacheReadTokens: number }
): void {
	state.budget.tokensIn += usage.inputTokens;
	state.budget.tokensOut += usage.outputTokens;
	state.budget.cacheReadTokens += usage.cacheReadTokens;
	if (usage.inputTokens > 0 || usage.outputTokens > 0) state.budget.calls += 1;
	if (
		state.budget.tokensIn >= BUDGET_INPUT_TOKENS ||
		state.budget.tokensOut >= BUDGET_OUTPUT_TOKENS ||
		state.budget.calls >= BUDGET_CALLS
	) {
		state.budget.degraded = true;
	}
}

/**
 * Answer one `AIDecideRequest`.
 *
 * Every check here is a tripwire against a message this seat should not act on,
 * never an instruction: the token is compared before any state read, the
 * request's own `seat` is compared against `state.seat` (never trusted as an
 * instruction), and a redelivered `turnId` is dropped rather than re-billed to
 * the model. The reply is sent — never awaited by the table — over
 * `euchreTable`'s own `aiDecision` queue, which re-derives everything anyway.
 */
async function onDecide(c: SeatCtx, body: AIDecideRequest): Promise<void> {
	const state = c.state;

	if (!internalTokenOk(state.internalToken, body.internalToken)) {
		state.meter.tokenRejections += 1;
		c.log.warn('dropped forged decide request', { seat: state.seat });
		return;
	}
	if (body.gameId !== state.gameId) return;
	if (body.seat !== state.seat) {
		state.meter.seatRejections += 1;
		c.log.warn('dropped decide request for a different seat', {
			mine: state.seat,
			claimed: body.seat
		});
		return;
	}
	if (body.turnId === state.lastTurnId) {
		// A redelivery of a turn this seat already answered. The table has moved
		// on (or is about to); re-running the ladder here would just re-bill a
		// model call for an answer nobody is waiting on.
		state.meter.staleTurns += 1;
		return;
	}

	absorbPublicState(state.memory, body.view);

	const startedAt = Date.now();
	const outcome = await runLadder(
		{
			persona: state.persona,
			dossier: state.dossier,
			nonce: state.fenceNonce,
			degraded: state.budget.degraded,
			now: () => Date.now(),
			log: (event, fields) => c.log.info(event, fields)
		},
		body
	);

	if (outcome === null) {
		// The table asked a seat with nothing to decide. A dispatch bug, not a
		// model failure — there is no legal move to answer with.
		c.log.error('decide requested with an empty legal set', { seat: state.seat, turnId: body.turnId });
		return;
	}

	const latencyMs = Date.now() - startedAt;
	state.lastTurnId = body.turnId;
	recordDecision(state, body, outcome, latencyMs);
	applyUsage(state, outcome.usage);
	await c.saveState({ immediate: true });

	const decision: AIDecision = {
		internalToken: state.internalToken,
		gameId: state.gameId,
		seat: state.seat,
		turnId: body.turnId,
		moveId: outcome.moveId,
		source: outcome.source,
		rationale: outcome.rationale,
		latencyMs
	};
	await seatClient(c).euchreTable.getOrCreate(['table', state.gameId]).send('aiDecision', decision);
}

/* ========================================================================== */
/* Lifecycle — resetHand / handEnd / gameEnd                                 */
/* ========================================================================== */

function onResetHand(c: SeatCtx, body: AiSeatLifecycleMessage): void {
	const state = c.state;
	if (!internalTokenOk(state.internalToken, body.internalToken)) return;
	if (body.gameId !== state.gameId) return;
	resetMemory(state.memory, body.handNo ?? state.memory.handNo + 1);
}

/** Flush the episode buffer to `playerProfile`, or drop it if there is nowhere to send it. */
async function flushEpisodes(c: SeatCtx): Promise<void> {
	const state = c.state;
	if (state.episodeBuffer.length === 0) return;

	if (state.profileKey === null) {
		state.episodeBuffer = [];
		return;
	}
	const token = profileInternalToken();
	if (token === null) return; // no deployment secret configured; retry next boundary

	const episodes = state.episodeBuffer.map((e) => ({
		role: state.persona.id,
		matchId: state.matchId,
		handNo: e.handNo,
		ts: e.ts,
		kind: e.kind,
		summary: e.summary,
		salience: e.salience
	}));

	try {
		await seatClient(c)
			.playerProfile.getOrCreate(state.profileKey)
			.send('recordEpisodes', { internalToken: token, episodes });
		state.episodeBuffer = [];
	} catch (err) {
		// Bounded by EPISODE_BUFFER_CAP either way, so a permanent failure here
		// drops the oldest notes rather than growing without limit.
		c.log.warn('flushEpisodes failed; will retry at the next boundary', {
			err: String(err).slice(0, 200)
		});
	}
}

/** Flush today's token spend to `playerProfile`'s daily bucket, once, best-effort. */
async function flushTokens(c: SeatCtx): Promise<void> {
	const state = c.state;
	if (state.profileKey === null) return;
	const { tokensIn, tokensOut, cacheReadTokens, calls } = state.budget;
	if (tokensIn === 0 && tokensOut === 0 && calls === 0) return;
	const token = profileInternalToken();
	if (token === null) return;

	try {
		await seatClient(c)
			.playerProfile.getOrCreate(state.profileKey)
			.send('recordTokens', { internalToken: token, tokensIn, tokensOut, cacheReadTokens, calls });
	} catch (err) {
		c.log.warn('flushTokens failed', { err: String(err).slice(0, 200) });
	}
}

async function onHandEnd(c: SeatCtx, body: AiSeatLifecycleMessage): Promise<void> {
	const state = c.state;
	if (!internalTokenOk(state.internalToken, body.internalToken)) return;
	if (body.gameId !== state.gameId) return;

	rememberHand(state.notes, state.notedHands, state.memory.snapshot, state.seat);
	const episode = episodeFor(state.memory.snapshot, state.seat, Date.now());
	if (episode !== null) {
		state.episodeBuffer.push(episode);
		while (state.episodeBuffer.length > EPISODE_BUFFER_CAP) state.episodeBuffer.shift();
	}
	await flushEpisodes(c);
	await c.saveState({ immediate: true });
}

async function onGameEnd(c: SeatCtx, body: AiSeatLifecycleMessage): Promise<void> {
	const state = c.state;
	if (!internalTokenOk(state.internalToken, body.internalToken)) return;
	if (body.gameId !== state.gameId) return;

	await flushEpisodes(c);
	await flushTokens(c);
	await c.saveState({ immediate: true });
}

/** One message off the durable queue. Mirrors `euchre-table`'s `TableMessage`. */
type AiSeatMessage =
	| { readonly name: 'decide'; readonly body: AIDecideRequest; complete(): Promise<void> }
	| { readonly name: 'resetHand'; readonly body: AiSeatLifecycleMessage; complete(): Promise<void> }
	| { readonly name: 'handEnd'; readonly body: AiSeatLifecycleMessage; complete(): Promise<void> }
	| { readonly name: 'gameEnd'; readonly body: AiSeatLifecycleMessage; complete(): Promise<void> };

/* ========================================================================== */
/* The actor                                                                  */
/* ========================================================================== */

export const aiSeat = actor({
	options: {
		name: 'AI Seat',
		icon: 'robot'
	},

	queues: aiSeatQueues,

	/**
	 * Snapshot the persona for the whole match. `input` is read once, here, and
	 * never again — editing a persona in Settings affects the next match.
	 *
	 * A missing `input` (a bare `getOrCreate` with no `createWithInput`, which
	 * should not happen on the hot path but must not crash the actor) produces an
	 * `unprovisioned` seat: a persona that can never be chosen by
	 * `resolveDecider`'s override seam, and an `internalToken` of `''` that no
	 * real message can ever match, so every `decide` is safely dropped rather
	 * than answered in character as nobody.
	 */
	createState: (c, input: AiSeatCreateInput | undefined): AiSeatState => {
		const seat = seatFromKey(c.key);
		// The key is the real authority (per `AiSeatCreateInput.seat`'s own doc):
		// a disagreeing `input.seat` is silently overruled, never trusted.
		const gameId = input?.gameId ?? c.key[1] ?? '';
		return {
			schema: 1,
			provisioned: input !== undefined,
			gameId,
			matchId: input?.matchId ?? gameId,
			seat,
			teamId: teamOf(seat),
			internalToken: input?.internalToken ?? '',
			fenceNonce: input?.fenceNonce ?? '',
			persona: input?.persona ?? blankPersona(),
			dossier: input?.dossier ?? '',
			profileKey: input?.profileKey !== undefined ? [...input.profileKey] : null,
			memory: {
				handNo: 0,
				trumpSeen: [],
				voids: { 0: [], 1: [], 2: [], 3: [] },
				leadHistory: [],
				bidHistory: [],
				snapshot: null
			},
			notes: [],
			notedHands: [],
			budget: { tokensIn: 0, tokensOut: 0, cacheReadTokens: 0, calls: 0, degraded: false },
			meter: {
				decisions: 0,
				llmCalls: 0,
				llmWins: 0,
				fallbacks: 0,
				forced: 0,
				illegalAttempts: 0,
				firstLegalRescues: 0,
				tokenRejections: 0,
				seatRejections: 0,
				staleTurns: 0,
				banterEmitted: 0,
				banterRejected: 0,
				avgLatencyMs: 0,
				p95LatencyMs: 0,
				samples: []
			},
			banter: { lastHandNo: -1, lastTrickIndex: -1, inTrick: 0, inHand: 0, lastAt: 0 },
			lastTurnId: null,
			episodeBuffer: [],
			decisionLog: []
		};
	},

	/**
	 * The single serialized writer, exactly like `euchreTable`'s and
	 * `playerProfile`'s: every mutation is a queue message, processed strictly in
	 * order, so two `decide` messages racing cannot interleave their model calls
	 * and corrupt the meter or the budget.
	 */
	run: async (c): Promise<void> => {
		const messages = c.queue.iter({ completable: true }) as AsyncIterable<AiSeatMessage>;
		for await (const message of messages) {
			try {
				switch (message.name) {
					case 'decide':
						await onDecide(c, message.body);
						break;
					case 'resetHand':
						onResetHand(c, message.body);
						await c.saveState({ immediate: true });
						break;
					case 'handEnd':
						await onHandEnd(c, message.body);
						break;
					case 'gameEnd':
						await onGameEnd(c, message.body);
						break;
				}
			} catch (err) {
				c.log.error('aiSeat queue handler failed', {
					name: message.name,
					err: String(err).slice(0, 200)
				});
			}
			await message.complete();
		}
	},

	actions: {
		/** Client-safe status for a HUD chip. No card identity, no prompt text. */
		getStatus: (c): AiStatus => ({
			seat: c.state.seat,
			gameId: c.state.gameId,
			provisioned: c.state.provisioned,
			personaId: c.state.persona.id,
			personaVersion: c.state.persona.version,
			degraded: c.state.budget.degraded,
			decisions: c.state.meter.decisions,
			llmCalls: c.state.meter.llmCalls,
			fallbacks: c.state.meter.fallbacks,
			forced: c.state.meter.forced,
			illegalAttempts: c.state.meter.illegalAttempts,
			avgLatencyMs: c.state.meter.avgLatencyMs,
			p95LatencyMs: c.state.meter.p95LatencyMs,
			tokensIn: c.state.budget.tokensIn,
			tokensOut: c.state.budget.tokensOut,
			notes: c.state.notes.length
		}),

		/** The client-safe face of this seat's persona. Omits the prompt and model ids. */
		getPersona: (c): PersonaView => ({
			id: c.state.persona.id,
			version: c.state.persona.version,
			name: c.state.persona.name,
			blurb: c.state.persona.blurb,
			seat: c.state.seat,
			difficulty: c.state.persona.difficulty,
			chattiness: c.state.persona.chattiness,
			degraded: c.state.budget.degraded
		}),

		/** Debug surface only: no card identity, no prompt text. */
		getDecisionLog: (c): DecisionRecord[] => [...c.state.decisionLog]
	}
});
