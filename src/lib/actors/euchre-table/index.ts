/**
 * `euchreTable` — key `["table", gameId]`. The single authority on one match.
 *
 * This actor owns the deck order, all four hands, the kitty, the cut, both
 * bidding rounds, the dealer's discard, going alone, follow-suit legality, trick
 * resolution, the server-held read pauses, scoring, turn sequencing, AI dispatch
 * and per-connection redaction — and it contains **zero euchre logic**. Every
 * mutation is exactly one call into the pure reducer in `$lib/euchre`, so this
 * file is plumbing around a rulebook it is not allowed to duplicate. It holds no
 * model client, no API key, no user record and no cross-match data.
 *
 * Five properties hold here, and every awkward line below exists to keep one of
 * them:
 *
 * 1. **One rulebook.** `apply`, `advance`, `legalMoves` and `project` are the
 *    only functions that know what euchre is. If a rule appears to need
 *    re-implementing in this file, it does not: read the reducer again.
 * 2. **The table is authoritative.** Neither the browser nor a model is trusted.
 *    A move id from either is looked up in `legalMoves()` *computed here*, and
 *    the reducer re-validates it a second time.
 * 3. **The acting seat is never read from a payload.** A human move acts as
 *    `HUMAN_SEAT`, verified against `c.conn.state.seat`; every internal message
 *    acts as `state.hand.turnSeat`. A `seat` field on an inbound message exists
 *    only so a disagreement can be *detected*, logged as an attack, and dropped.
 * 4. **Redaction is absolute.** `project(state, seat)` and `projectSteps(steps,
 *    seat)` are the only things that ever reach a connection, and they are
 *    called with the seat taken from `conn.state` — never from a variable that
 *    could be another seat. `sync` is per-connection precisely because it is the
 *    only channel carrying card identity.
 * 5. **Durability ordering, with no exceptions:** mutate → `saveState({
 *    immediate: true })` → fan out → ack. Plain `saveState()` waits for a
 *    throttled flush of up to ten seconds and is never what you want here.
 *
 * ## Why a run loop and not actions
 *
 * Actions run concurrently and are not durable. Two `submitMove` calls racing
 * would interleave their `await`s and corrupt the trick, and a crash mid-action
 * loses the move. So every action here is read-only or enqueue-and-wait, and the
 * single `run` loop is the only writer in the actor. See `./queues.ts`.
 *
 * ## Why the timers are queue messages too
 *
 * `c.schedule` fires **actions**, which are not durable. Every schedule target in
 * this file therefore does nothing but a stale-nonce check followed by
 * `c.queue.send("tick", …)`. Schedule → action → queue is the only path from a
 * timer into the mutation loop, which is what makes the AI pacing floor, the
 * watchdog and the server-held read pauses survive a restart.
 *
 * ## Serverless
 *
 * We run in Rivet's serverless mode: `c.vars` dies on every migration and a
 * migration is routine, not exceptional. Everything durable is `c.state`; the
 * seed lives in state and the RNG closure is rebuilt from it by the engine, which
 * is why a restart reproduces the same shuffle. {@link reconcile} runs on every
 * wake and is a routine code path, not a safety net.
 *
 * @see docs/01-ARCHITECTURE.md §3.2, §8, §9
 * @see docs/06-REVISED-ARCHITECTURE.md §3, §4
 */

import { UserError, actor } from 'rivetkit';
import {
	HUMAN_SEAT,
	advance,
	apply,
	isRuleError,
	legalMoves,
	partnerOf,
	project,
	projectSteps,
	teamOf,
	createGame,
	type AutoReason,
	type EngineConfig,
	type GameState,
	type HandState,
	type LegalMove,
	type PlayerAction,
	type Seat,
	type Step,
	type Trick
} from '$lib/euchre';
import {
	BANTER_MAX_CHARS,
	PROTOCOL_ERROR_CODES,
	PROTOCOL_VERSION,
	type AIDecideRequest,
	type AIDecision,
	type AIDecisionKind,
	type AIDecisionSource,
	type AiSeatLifecycleKind,
	type AiSeatLifecycleMessage,
	type ChatKind,
	type HandJournalEntry,
	type LegalMoveId,
	type MatchOutcome,
	type MatchRecord,
	type MatchStats,
	type MoveAck,
	type PersonaAssignment,
	type PublicGameView,
	type SubmitMoveRequest,
	type SyncEvent,
	type TableConnState,
	type TableConnectParams,
	type TableEventName
} from '$lib/protocol';
// Owned by the auth agent; see `docs/06-REVISED-ARCHITECTURE.md` §5.5. Imported by
// its documented name so this file compiles the moment that module lands.
import { makeJwks, verifyPlayer, type Jwks } from '$lib/actors/auth/verify';
import { topMove } from '$lib/ai/heuristic';
// `aiSeat` owns the LLM call now (`$lib/ai` is its dependency, not this table's);
// only the create-input shape crosses the boundary, and only as a type.
import type { AiSeatCreateInput } from '$lib/actors/ai-seat/types';
import {
	tableEvents,
	tableQueues,
	tokenOk,
	type AiSayMsg,
	type MoveMsg,
	type TableQueueName,
	type TickMsg
} from './queues';
import { TEMPO, thinkFloorMs, trickResolveMs } from './tempo';

/* ========================================================================== */
/* Configuration read from the environment                                    */
/* ========================================================================== */

/**
 * The origin allowlist. Rivet has **no WebSocket CORS** and the socket terminates
 * at Rivet Cloud, so Vercel's own CORS configuration is not in the request path
 * at all: this check is the only one there is.
 *
 * Unset is treated as local development (the same signal `registry.ts` uses for
 * `RIVET_ENDPOINT`) and allows any origin. In a deployed environment an unset
 * allowlist with `RIVET_ENDPOINT` present is a hard deny, because failing open on
 * origin in production is how a table gets driven from someone else's page.
 */
function allowedOrigins(): readonly string[] {
	const raw = process.env.ALLOWED_ORIGINS ?? '';
	return raw
		.split(',')
		.map((s) => s.trim())
		.filter((s) => s.length > 0);
}

/** True when no Rivet Cloud endpoint is configured, i.e. `bun run dev`. */
function isLocalDev(): boolean {
	return !process.env.RIVET_ENDPOINT;
}

/** Issuer/audience base for JWT verification. */
function appUrl(): string {
	return process.env.APP_URL ?? process.env.PUBLIC_APP_URL ?? 'http://localhost:5173';
}

/* ========================================================================== */
/* State                                                                      */
/* ========================================================================== */

/** What `/play` (or a fixture) supplies when the table is first created. */
export interface TableCreateInput {
	/**
	 * The Better Auth user id of the one human. When omitted the first
	 * authenticated connection claims the table — see {@link claimOwner}.
	 */
	readonly ownerUserId?: string;
	/** Overrides the cryptographically random seed. Fixtures and replays only. */
	readonly seed?: string;
	readonly cfg?: Partial<EngineConfig>;
	readonly firstDealer?: Seat;
	/** The three opponents, snapshotted at creation and never re-read mid-match. */
	readonly personas?: readonly PersonaAssignment[];
}

/**
 * The table's whole `c.state`: the engine's `GameState` verbatim, plus the
 * bookkeeping that is the *actor's* job rather than the reducer's.
 *
 * `GameState` already carries the durability-bearing fields the architecture
 * names — `v`, `turnId`, `turnDeadlineAt`, `timerId`, `watchdogId`, `revealId`,
 * `pending`, `recentMoveIds`, `appliedSeq` — and the reducer never touches any of
 * them, so they survive every `apply()` by ordinary spread. What is added here is
 * everything the reducer has no opinion about: who owns the match, who is sitting
 * in the other three chairs, the current hand's replay journal, and the four
 * schedule ids that have no home in `GameState`.
 *
 * Every collection is bounded, because `c.state` is deserialized **whole** on
 * every wake and a serverless migration is a wake: `personas` is exactly three
 * entries whose text fields are capped by `$lib/protocol`, and `journal` is one
 * hand — at most one cut, eight bids, a discard and twenty plays.
 */
export interface TableState {
	readonly schema: 1;
	/** Same value as `game.gameId`; the id history is filed under. */
	readonly matchId: string;
	/** Empty until claimed. The authorization check in `createConnState`. */
	ownerUserId: string;
	readonly startedAt: number;
	/** The authoritative match state. Replaced wholesale, never patched in place. */
	game: GameState;
	/** Snapshotted at creation. Read only on the cold path, to rebuild a lost `aiSeat`. */
	personas: readonly PersonaAssignment[];
	/** The current hand's attributed actions, replayable through `apply()`. Cap 40. */
	journal: readonly PlayerAction[];
	stats: MatchStats;
	handsPlayed: number;
	/** Guards the once-per-hand journal flush against a redelivered message. */
	lastJournaledHandNo: number;
	/** Guards the once-per-hand `resetHand` fan-out to the AI seats. */
	lastResetHandNo: number;
	/** Set once the durable copy exists in `playerProfile`. Gates self-reap. */
	recordedAt: number | null;
	/** Two consecutive abandon fires flip `status` to `abandoned`. */
	abandonStrikes: number;
	/** Schedule ids with no home in `GameState`. Cancelled on every turn advance. */
	tempoId: string | null;
	abandonId: string | null;
	profileRetryId: string | null;
	reapId: string | null;
}

/**
 * Rebuilt on every wake and deliberately not durable.
 *
 * Only the JWKS lives here: it is a non-serializable `jose` object, it is a pure
 * cache of something fetchable, and losing it costs one key fetch. Nothing that
 * must survive a migration may be added — under serverless, `c.vars` dies far
 * more often than it did under a long-lived runner.
 */
export interface TableVars {
	readonly jwks: Jwks;
}

/* ========================================================================== */
/* The context, structurally                                                  */
/* ========================================================================== */

/** One live browser connection, as much of it as this file needs. */
interface TableConn {
	readonly id: string;
	readonly state: TableConnState;
	send(name: TableEventName, payload: unknown): void;
}

/**
 * The slice of the actor context the helpers below use.
 *
 * Deriving this with `ActionContextOf<typeof euchreTable>` would be circular —
 * the actor's own definition is what we are typing, and every helper is called
 * from inside it. This is the narrowest structural alternative: the real context
 * is assignable to it, so helpers take `c` with no cast, and `state` stays fully
 * typed as {@link TableState} rather than degrading to `any`.
 */
interface TableCtx {
	state: TableState;
	readonly vars: TableVars;
	readonly key: string[];
	readonly conns: ReadonlyMap<string, TableConn>;
	readonly log: {
		info(...args: unknown[]): void;
		warn(...args: unknown[]): void;
		error(...args: unknown[]): void;
	};
	readonly schedule: {
		after(duration: number, action: string, ...args: unknown[]): Promise<string>;
		cancel(id: string): Promise<boolean>;
	};
	readonly queue: { send(name: TableQueueName, body: unknown): Promise<unknown> };
	broadcast(name: TableEventName, payload: unknown): void;
	saveState(opts?: { immediate?: boolean }): Promise<void>;
	client(): unknown;
	destroy(): void;
}

/** A handle on another actor, narrowed to the one verb this table uses. */
interface ActorSendHandle {
	send(name: string, body: unknown): Promise<unknown>;
}

/**
 * The two sibling actors this table talks to, declared structurally.
 *
 * Importing the registry's type here would make `registry.ts → euchre-table →
 * registry.ts` a cycle that `rivetkit`'s client inference resolves by collapsing
 * `c.state` to `unknown` (the documented TypeScript caveat). Declaring the two
 * verbs we actually use costs one cast at {@link tableClient} and keeps every
 * other type in this file honest. The *messages* are still typed: they are
 * `AIDecideRequest`, `AiSeatLifecycleMessage`, `MatchRecord` and
 * `HandJournalEntry` from `$lib/protocol`.
 */
interface TableClient {
	readonly aiSeat: {
		getOrCreate(
			key: readonly string[],
			opts?: { createWithInput?: unknown }
		): ActorSendHandle;
	};
	readonly playerProfile: {
		getOrCreate(key: readonly string[]): ActorSendHandle;
	};
}

function tableClient(c: TableCtx): TableClient {
	return c.client() as TableClient;
}

/* ========================================================================== */
/* Small pure helpers                                                         */
/* ========================================================================== */

/** Cryptographically random hex. Used for the match seed and nothing else. */
function randomHex(bytes: number): string {
	const buf = new Uint8Array(bytes);
	crypto.getRandomValues(buf);
	let out = '';
	for (const b of buf) out += b.toString(16).padStart(2, '0');
	return out;
}

/** The three AI chairs. Seat 0 is always the human and is never in this list. */
const AI_SEATS: readonly Seat[] = [1, 2, 3];

function isAiSeat(seat: Seat): boolean {
	return seat !== HUMAN_SEAT;
}

/** Which decision the seat at `phase` is being asked for, or `null` if none is. */
function decisionKindFor(phase: HandState['phase']): AIDecisionKind | null {
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

/** A blank stats row. Every counter starts at zero; nothing is derived later. */
function emptyStats(): MatchStats {
	return {
		euchresFor: 0,
		euchresAgainst: 0,
		lonersAttempted: 0,
		lonersMade: 0,
		marches: 0,
		throwIns: 0,
		tricks: [0, 0]
	};
}

/**
 * Fold one finished hand into the match counters.
 *
 * "For" and "against" are from the human's point of view: team 0 is `{0, 2}` and
 * is always the human's team (`teamOf(seat) === seat & 1`).
 */
function accumulateStats(prev: MatchStats, hand: HandState): MatchStats {
	const result = hand.result;
	const makerTeam = hand.makerSeat === null ? null : teamOf(hand.makerSeat);
	const euchre = result === 'euchre';
	const lone = hand.aloneSeat !== null;
	return {
		euchresFor: prev.euchresFor + (euchre && makerTeam === 1 ? 1 : 0),
		euchresAgainst: prev.euchresAgainst + (euchre && makerTeam === 0 ? 1 : 0),
		lonersAttempted: prev.lonersAttempted + (lone ? 1 : 0),
		lonersMade: prev.lonersMade + (lone && (result === 'lone_point' || result === 'lone_march') ? 1 : 0),
		marches: prev.marches + (result === 'march' || result === 'lone_march' ? 1 : 0),
		throwIns: prev.throwIns + (result === 'throw_in' ? 1 : 0),
		tricks: [prev.tricks[0] + hand.tricksWon[0], prev.tricks[1] + hand.tricksWon[1]]
	};
}

/**
 * Strip a generated line down to something safe to render.
 *
 * Control characters are removed rather than escaped (a persona has no business
 * emitting them), whitespace is collapsed so a model cannot paint the chat pane
 * with newlines, and the result is hard-clamped. The client renders the result as
 * text and never as markup — this is the second of those two defences, not the
 * only one.
 */
function screenLine(text: unknown, max: number): string {
	if (typeof text !== 'string') return '';
	let out = '';
	for (const ch of text) {
		const code = ch.codePointAt(0) ?? 0;
		if (code < 0x20 || code === 0x7f) out += ' ';
		else out += ch;
	}
	return out.replace(/\s+/g, ' ').trim().slice(0, max);
}

/* ========================================================================== */
/* Redaction — the only place state leaves the actor                          */
/* ========================================================================== */

/**
 * Push the authoritative view to every connection, redacted for *that*
 * connection's seat.
 *
 * The **only** producer of the `sync` event, and the reason `sync` is
 * per-connection rather than a broadcast: `c.broadcast` has no seat, so it has
 * nothing to redact for. The seat comes from `conn.state.seat`, which
 * `createConnState` derived from a verified JWT — never from a payload, never
 * from a loop variable that could hold another seat.
 *
 * `steps` are decoration (what just happened, so the client can animate) and are
 * never truth: `projectSteps` blanks the `dealt` packets not addressed to this
 * seat, which is the one step that carries card identity for anybody else.
 */
function pushSync(c: TableCtx, steps: readonly Step[]): void {
	const game = c.state.game;
	for (const conn of c.conns.values()) {
		const seat = conn.state.seat;
		const payload: SyncEvent = {
			v: game.v,
			view: project(game, seat),
			steps: projectSteps(steps, seat)
		};
		conn.send('sync', payload);
	}
}

/** Broadcast one screened chat line. Carries no card identity, so no redaction. */
function emitChat(c: TableCtx, seat: Seat, kind: ChatKind, text: string, msgId: string): void {
	const clean = screenLine(text, BANTER_MAX_CHARS);
	if (clean.length === 0) return;
	c.broadcast('chat', { msgId, seat, kind, text: clean, final: true });
}

/**
 * Speak the engine-authored table script for a transition.
 *
 * These lines are **not** model output: `sayForBid()` produced them inside the
 * reducer and they are carried on the `bid` step verbatim, which is why they are
 * emitted unscreened in spirit (they still pass {@link emitChat}'s clamp, which is
 * a no-op for a fixed phrase) and why they are `kind: "call"` rather than
 * `"banter"`. The client routes the two kinds to different aria-live regions so a
 * persona's chatter can never interleave with a trump call.
 */
function emitScript(c: TableCtx, steps: readonly Step[]): void {
	const game = c.state.game;
	let n = 0;
	for (const step of steps) {
		const msgId = `${game.gameId}:${game.v}:${n++}`;
		if (step.t === 'bid') {
			emitChat(c, step.seat, 'call', step.say, msgId);
		} else if (step.t === 'throwIn') {
			emitChat(c, game.hand.dealerSeat, 'system', 'Throw it in. Nobody wanted it.', msgId);
		} else if (step.t === 'handScored') {
			const line =
				step.result === 'euchre'
					? 'Euchre!'
					: step.result === 'march' || step.result === 'lone_march'
						? 'March!'
						: null;
			if (line !== null && step.makerSeat !== null) {
				emitChat(c, step.makerSeat, 'call', line, msgId);
			}
		}
	}
}

/* ========================================================================== */
/* Committing engine output                                                   */
/* ========================================================================== */

/**
 * Adopt a reducer result as the new authoritative state and bump the version.
 *
 * `v` is the actor's counter, not the reducer's: `apply()` and `advance()` carry
 * it through untouched precisely so that this is the only line in the codebase
 * that increments it. The client drops any `sync` whose `v` it has already seen,
 * so monotonicity here is what makes a reconnect race harmless.
 */
function commit(c: TableCtx, state: GameState): void {
	c.state.game = { ...state, v: c.state.game.v + 1 };
}

/** Append to the current hand's replay journal, bounded at 40 entries. */
function journal(c: TableCtx, action: PlayerAction): void {
	c.state.journal = [...c.state.journal, action].slice(-40);
}

/** Remember a client's dedupe key. Ring buffer, cap 32 (V18). */
function rememberMoveId(c: TableCtx, clientMoveId: string): void {
	const ring = [...c.state.game.recentMoveIds, clientMoveId];
	c.state.game.recentMoveIds = ring.slice(-32);
}

/**
 * Run the engine forward through every phase that has no seat to act in, and
 * stop the instant one does.
 *
 * `advance()` also knows how to leave `trick_resolve` and `hand_score`, and this
 * function deliberately does **not** let it: those two are server-held read
 * pauses armed by {@link armTurn}, and they are a security property rather than
 * decoration. A hacked client that skips every local animation still cannot see
 * the next card early, because while the table sits in `trick_resolve` the next
 * card has not been computed (`docs/01-ARCHITECTURE.md` §9.4).
 *
 * `lobby` and `deal`, by contrast, are transitions rather than resting phases:
 * the deal is atomic and there is nothing to hold.
 */
function settle(c: TableCtx, steps: Step[]): Step[] {
	for (;;) {
		const phase = c.state.game.hand.phase;
		if (phase !== 'lobby' && phase !== 'deal') return steps;
		const out = advance(c.state.game);
		if (out.state === c.state.game) return steps;
		commit(c, out.state);
		steps.push(...out.steps);
	}
}

/** Cancel every per-turn timer. Called before arming the next turn's. */
async function cancelTurnSchedules(c: TableCtx): Promise<void> {
	const game = c.state.game;
	const ids: [string | null, (v: null) => void][] = [
		[game.timerId, () => (game.timerId = null)],
		[game.watchdogId, () => (game.watchdogId = null)],
		[game.revealId, () => (game.revealId = null)],
		[c.state.tempoId, () => (c.state.tempoId = null)],
		[c.state.abandonId, () => (c.state.abandonId = null)]
	];
	for (const [id, clear] of ids) {
		if (id !== null) await c.schedule.cancel(id);
		clear(null);
	}
}

/* ========================================================================== */
/* Turn sequencing                                                            */
/* ========================================================================== */

/**
 * Decide what happens next now that the state has settled, and arm exactly one
 * timer for it.
 *
 * Five outcomes, and they are exhaustive over the phase machine:
 * `game_over`/non-active → record and reap; `trick_resolve`/`hand_score` → the
 * server-held read pause; the human's turn → the nudge/abandon ladder; an AI
 * seat's turn → dispatch with a watchdog; no seat at all → nothing (which only
 * happens if the engine is mid-transition, and {@link settle} has already run).
 *
 * `steps` is used only to choose the length of the read pause: a trick that seals
 * a euchre earns a longer beat.
 */
async function armTurn(c: TableCtx, steps: readonly Step[]): Promise<void> {
	await cancelTurnSchedules(c);
	const game = c.state.game;

	if (game.status !== 'active') {
		await recordMatchIfNeeded(c);
		return;
	}

	const phase = game.hand.phase;
	if (phase === 'trick_resolve' || phase === 'hand_score') {
		const seals = steps.some((s) => s.t === 'trickWon' && s.sealsEuchre);
		const ms = phase === 'hand_score' ? TEMPO.handScoreMs : trickResolveMs(seals);
		c.state.tempoId = await c.schedule.after(ms, 'onTempoGate', game.turnId);
		await c.saveState({ immediate: true });
		return;
	}

	const seat = game.hand.turnSeat;
	if (seat === null) return;
	if (seat === HUMAN_SEAT) {
		await armHumanLadder(c);
		return;
	}
	await dispatchAi(c, seat);
}

/**
 * Arm the two human timers.
 *
 * There is **no 30-second human turn timer**, and this comment exists so that one
 * is not re-added: there is one human at this table and nobody is waiting on
 * them, so a turn timer would mean the game plays itself while the player reads
 * the rules panel. What is armed instead is a 90 s partner nudge that changes no
 * state, and a 240 s auto-play that exists only so a walked-away solo match
 * terminates instead of pinning an actor forever.
 */
async function armHumanLadder(c: TableCtx): Promise<void> {
	const game = c.state.game;
	game.turnDeadlineAt = null;
	game.timerId = await c.schedule.after(TEMPO.nudgeMs, 'onNudge', game.turnId);
	c.state.abandonId = await c.schedule.after(TEMPO.abandonMs, 'onAbandon', game.turnId);
	await c.saveState({ immediate: true });
}

/* ========================================================================== */
/* AI dispatch                                                                */
/* ========================================================================== */

/**
 * A stable, per-match fence nonce for the persona prompt.
 *
 * Derived from the match id so it survives a restart without being stored, and so
 * a replay reproduces it. It only has to be unguessable *to the model*, whose
 * entire view of the world is the prompt it is handed.
 */
function fenceNonce(matchId: string): string {
	let h = 0x811c9dc5;
	for (let i = 0; i < matchId.length; i++) {
		h ^= matchId.charCodeAt(i);
		h = Math.imul(h, 0x01000193) >>> 0;
	}
	return h.toString(16).padStart(8, '0') + ((h * 2654435761) >>> 0).toString(16).padStart(8, '0');
}

/** This seat's snapshotted opponent, or `undefined` if the table was never given one. */
function personaFor(c: TableCtx, seat: Seat): PersonaAssignment | undefined {
	return c.state.personas.find((p) => p.seat === seat);
}

/**
 * What an `aiSeat` needs the first time it is woken for this match.
 *
 * Built fresh on every call rather than cached: it is cheap (a handful of field
 * reads), and `getOrCreate`'s `createWithInput` is only ever consulted on the
 * message that actually creates the actor, so recomputing it here costs nothing
 * and keeps this function honest about what a cold-started seat will see.
 */
function aiSeatCreateInput(c: TableCtx, seat: Seat, assignment: PersonaAssignment): AiSeatCreateInput {
	const game = c.state.game;
	const base: AiSeatCreateInput = {
		gameId: game.gameId,
		seat,
		internalToken: game.internalToken,
		fenceNonce: fenceNonce(c.state.matchId),
		persona: assignment.persona,
		dossier: assignment.dossier,
		matchId: c.state.matchId
	};
	return c.state.ownerUserId === '' ? base : { ...base, profileKey: ['user', c.state.ownerUserId] };
}

/**
 * A handle on the actor for one AI chair, or `null` when this table was never
 * given a persona for it (a bare table, or a fixture) — the caller's cue to fall
 * back to the heuristic without spending a round trip on an actor that would
 * only answer "unprovisioned".
 *
 * **Both** callers below — `dispatchAi` and `notifyAiSeats` — pass the same
 * `createWithInput` on every call, not only the first. Whichever of them
 * actually creates the actor is the one `rivetkit` honours; the other's input is
 * silently ignored for an actor that already exists. If only one call site
 * supplied it, a lifecycle message arriving before this match's first decision
 * (routine: `resetHand` fires at the very first deal, before any seat has acted)
 * would permanently cold-create an unprovisioned seat that no later
 * `createWithInput` could ever reach.
 */
function aiSeatHandle(c: TableCtx, seat: Seat): ActorSendHandle | null {
	const assignment = personaFor(c, seat);
	if (assignment === undefined) return null;
	const key = ['table', c.state.game.gameId, 'seat', String(seat)];
	return tableClient(c).aiSeat.getOrCreate(key, {
		createWithInput: aiSeatCreateInput(c, seat, assignment)
	});
}

/**
 * Decide an AI seat's move, then park it behind the think floor for
 * `releaseAiMove`.
 *
 * **Ordering is the whole design.** The heuristic answer is parked and the
 * watchdog armed and persisted *before* the seat is asked anything, so a crash, a
 * serverless migration or a hung upstream mid-inference still leaves a legal move
 * on disk ready to fire. `aiSeat`'s reply then *upgrades* that parked decision if
 * it answers in time and the turn has not moved on — see `onAiDecision`, which is
 * unchanged by this dispatch living in another actor: it still re-derives
 * `legalMoves()`, still takes the acting seat from `hand.turnSeat`, and still
 * drops a reply for a `turnId` that is no longer current. There is no window in
 * which the table is waiting on a model with nothing to fall back to.
 *
 * A single legal move short-circuits entirely: no request, no seat wake, no
 * latency.
 */
async function dispatchAi(c: TableCtx, seat: Seat): Promise<void> {
	const game = c.state.game;
	const kind = decisionKindFor(game.hand.phase);
	if (kind === null) return;

	const legal = legalMoves(game, seat);
	if (legal.length === 0) {
		c.log.error('dispatchAi with an empty legal set', { seat, phase: game.hand.phase });
		return;
	}

	const now = Date.now();
	const revealAt = now + thinkFloorMs(kind, game.turnId);
	const view = project(game, seat);
	const forced = legal.length === 1;
	const fallbackId = forced ? legal[0]!.id : (topMove(view, view.hand, legal) ?? legal[0]!.id);
	const turnId = game.turnId;

	game.pending = {
		seat,
		turnId,
		requestedAt: now,
		revealAt,
		decision: fallbackId,
		source: forced ? 'forced' : 'fallback'
	};
	const deadlineAt = now + TEMPO.aiHardCapMs;
	game.turnDeadlineAt = deadlineAt;
	game.watchdogId = await c.schedule.after(TEMPO.aiHardCapMs, 'onAiTimeout', turnId, seat);
	game.revealId = await c.schedule.after(Math.max(0, revealAt - now), 'releaseAiMove', turnId);
	await c.saveState({ immediate: true }); // PERSIST — before the seat is asked anything.
	c.broadcast('thinking', { seat, on: true, extended: false });

	// A forced move is already decided; nothing to ask the seat for.
	if (forced) return;

	const handle = aiSeatHandle(c, seat);
	if (handle === null) {
		c.log.warn('no persona for AI seat; keeping heuristic', { seat });
		return;
	}

	const req: AIDecideRequest = {
		internalToken: game.internalToken,
		gameId: game.gameId,
		seat,
		turnId,
		kind,
		view,
		legal,
		deadlineAt
	};

	try {
		// Fire-and-forget: `aiSeat` replies over `aiDecision`, never over this call.
		// A blocking wait here would park the run loop on the very request the
		// watchdog above exists to survive.
		await handle.send('decide', req);
	} catch (err) {
		// Never fatal: the heuristic decision is already parked and scheduled.
		c.log.warn('dispatch to aiSeat failed; heuristic stands', { seat, e: String(err).slice(0, 200) });
	}
}

/**
 * Fan out a lifecycle event to every AI chair that has a persona.
 *
 * Each seat is notified independently and a failure on one is logged and
 * skipped rather than aborting the rest — a wedged seat 2 must not stop seat 1
 * and seat 3 from clearing their own per-hand memory or flushing their own
 * episodes.
 */
async function notifyAiSeats(
	c: TableCtx,
	kind: AiSeatLifecycleKind,
	handNo?: number
): Promise<void> {
	const game = c.state.game;
	const message: AiSeatLifecycleMessage = {
		internalToken: game.internalToken,
		gameId: game.gameId,
		kind,
		...(handNo === undefined ? {} : { handNo })
	};
	for (const seat of AI_SEATS) {
		const handle = aiSeatHandle(c, seat);
		if (handle === null) continue;
		try {
			await handle.send(kind, message);
		} catch (err) {
			c.log.warn('notifyAiSeats failed', { seat, kind, e: String(err).slice(0, 200) });
		}
	}
}

/* ========================================================================== */
/* Applying a move                                                            */
/* ========================================================================== */

/** How a move is attributed in the replay journal. */
type Attribution =
	| { readonly kind: 'human' }
	| { readonly kind: 'ai'; readonly source: AIDecisionSource }
	| { readonly kind: 'auto'; readonly reason: AutoReason };

/**
 * Apply one seat's move and carry the table all the way to the next open turn.
 *
 * The single choke point every mutation path funnels through — human, AI, parked
 * AI release, watchdog force-play and abandon auto-play all end up here — so the
 * durability ordering is written once: apply → commit → journal → settle →
 * **persist** → fan out → hand boundary. Arming the next turn is the caller's
 * job, because the human path has to slot its ack in between the fan-out and the
 * arm.
 *
 * `apply()` re-validates `seat` against `hand.turnSeat` and the move against the
 * legal set. It throws `RuleError` and nothing else for a rules violation; the run
 * loop turns that into a typed rejection rather than an unacked message.
 */
async function applyMove(
	c: TableCtx,
	seat: Seat,
	move: LegalMove,
	attribution: Attribution
): Promise<readonly Step[]> {
	const before = c.state.game;
	const out = apply(before, seat, move.move, { turnId: before.turnId });

	commit(c, out.state);
	c.state.game.pending = null;
	journal(c, { seat, id: move.id, move: move.move, ...attribution });

	const steps = settle(c, [...out.steps]);

	await c.saveState({ immediate: true }); // 1. PERSIST
	if (isAiSeat(seat)) c.broadcast('thinking', { seat, on: false, extended: false });
	emitScript(c, steps);
	pushSync(c, steps); // 2. FAN OUT
	await handBoundary(c, before, steps);
	return steps;
}

/**
 * Auto-play on the table's own authority when a seat has stopped answering.
 *
 * The fallback is `legalMoves()[0]` — the engine's own ordering. The table
 * deliberately holds **no heuristic**: ranking is euchre judgement rather than
 * euchre rules and lives with the AI seat, and importing it here would put a
 * second opinion about play quality inside the authority. A legal move that is
 * merely mediocre is the correct failure mode; a stalled game is not.
 */
async function autoPlay(c: TableCtx, seat: Seat, reason: AutoReason): Promise<void> {
	const legal = legalMoves(c.state.game, seat);
	const move = legal[0];
	if (move === undefined) {
		c.log.error('autoPlay found no legal move', { seat, phase: c.state.game.hand.phase });
		return;
	}
	c.log.warn('auto-playing for a silent seat', { seat, reason, moveId: move.id });
	const steps = await applyMove(c, seat, move, { kind: 'auto', reason });
	await armTurn(c, steps);
}

/* ========================================================================== */
/* Hand and match boundaries                                                  */
/* ========================================================================== */

/**
 * Flush the finished hand to durable history and reset the AI seats' memory.
 *
 * Guarded by `lastJournaledHandNo` / `lastResetHandNo` rather than by a flag,
 * because a redelivered queue message replays this function and the guard has to
 * live in the same `c.state` as the mutation it protects — that makes the second
 * run a no-op **by construction, not by luck**.
 *
 * `before` is the pre-transition state, needed only for a throw-in: a misdeal
 * rolls straight into the next hand's `cutting` phase without ever resting in
 * `hand_score`, so by the time we are called the thrown-in hand is gone from
 * `c.state.game`. (With `stickTheDealer` on — the default — a throw-in cannot
 * occur at all; the branch exists so that turning the variant off is not a
 * silent data-loss bug.)
 */
async function handBoundary(
	c: TableCtx,
	before: GameState,
	steps: readonly Step[]
): Promise<void> {
	const game = c.state.game;
	const thrownIn = steps.some((s) => s.t === 'throwIn');

	if (game.hand.phase === 'hand_score' && c.state.lastJournaledHandNo !== game.hand.handNo) {
		c.state.lastJournaledHandNo = game.hand.handNo;
		c.state.handsPlayed += 1;
		c.state.stats = accumulateStats(c.state.stats, game.hand);
		await recordHand(c, game.hand);
		await notifyAiSeats(c, 'handEnd', game.hand.handNo);
	} else if (thrownIn && c.state.lastJournaledHandNo !== before.hand.handNo) {
		c.state.lastJournaledHandNo = before.hand.handNo;
		c.state.stats = { ...c.state.stats, throwIns: c.state.stats.throwIns + 1 };
		await recordHand(c, before.hand);
	}

	if (game.hand.phase === 'cutting' && c.state.lastResetHandNo !== game.hand.handNo) {
		c.state.lastResetHandNo = game.hand.handNo;
		c.state.journal = [];
		await notifyAiSeats(c, 'resetHand', game.hand.handNo);
	}

	await c.saveState({ immediate: true });
}

/**
 * Write one hand to `playerProfile`.
 *
 * **Server-only data crosses here and stops there.** `seed` plus `deckOrder` plus
 * `moves` is the complete hand, hidden cards included — which is exactly what
 * makes a replay possible and exactly why neither may reach a browser. The
 * client-facing row is `MatchSummary`, which structurally has no seed and no deck
 * order.
 */
async function recordHand(c: TableCtx, hand: HandState): Promise<void> {
	// M2: playerProfile is not registered — skip durable journal flush.
	void c;
	void hand;
}

/** M2 stub: never wake an unregistered playerProfile actor. */
function profileHandle(_c: TableCtx): ActorSendHandle | null {
	return null;
}

/** Which way the match went, from the human's point of view. */
function outcomeOf(game: GameState): MatchOutcome {
	if (game.winnerTeam === 0) return 'won';
	if (game.winnerTeam === 1) return 'lost';
	return 'abandoned';
}

/**
 * Write the finished match to `playerProfile`, once, and only then allow the
 * table to be reaped.
 *
 * The retry is a schedule rather than a loop because the run loop must not block
 * on another actor: a `playerProfile` that is briefly unreachable would otherwise
 * hold this table awake and unresponsive. `recordedAt` is the idempotency guard
 * and lives in the same `c.state` as the write it protects.
 */
async function recordMatchIfNeeded(c: TableCtx): Promise<void> {
	if (c.state.recordedAt !== null) return;
	if (c.state.ownerUserId === '') return;

	const game = c.state.game;
	const record: MatchRecord = {
		schema: 1,
		matchId: c.state.matchId,
		userId: c.state.ownerUserId,
		seed: game.seed,
		cfg: game.cfg,
		firstDealer: game.firstDealer,
		startedAt: c.state.startedAt,
		endedAt: Date.now(),
		status: game.status,
		outcome: outcomeOf(game),
		score: game.score,
		winnerTeam: game.winnerTeam,
		handsPlayed: c.state.handsPlayed,
		personas: c.state.personas,
		stats: c.state.stats,
		// The table meters nothing: token spend is accumulated per seat and
		// flushed by each `aiSeat` on `gameEnd`. Zeroes here are honest.
		tokens: { tokensIn: 0, tokensOut: 0, cacheReadTokens: 0, calls: 0 }
	};

	const profile = profileHandle(c);
	if (profile === null) {
		// M2: mark recorded locally so self-reap / retries do not spin.
		c.state.recordedAt = Date.now();
		await notifyAiSeats(c, 'gameEnd');
		await c.saveState({ immediate: true });
		return;
	}

	try {
		await profile.send('recordMatch', record);
		c.state.recordedAt = Date.now();
		await notifyAiSeats(c, 'gameEnd');
	} catch (err) {
		c.log.warn('recordMatch failed; will retry', { err: String(err) });
		if (c.state.profileRetryId === null) {
			c.state.profileRetryId = await c.schedule.after(TEMPO.profileRetryMs, 'flushProfile');
		}
		await c.saveState({ immediate: true });
		return;
	}

	if (c.state.reapId === null) {
		c.state.reapId = await c.schedule.after(TEMPO.reapMs, 'onReap');
	}
	await c.saveState({ immediate: true });
}

/* ========================================================================== */
/* Queue handlers                                                             */
/* ========================================================================== */

/**
 * The human's move.
 *
 * The acting seat is `HUMAN_SEAT` — a server constant — and is not read from
 * `body`, which has no seat field to read. `submitMove` already checked that the
 * connection is the human's; this is the authority, not that check.
 *
 * Two different mechanisms guard two different problems and are never conflated:
 * `turnId` is the server nonce that rejects a second tab acting on a stale view,
 * and `recentMoveIds` is a 32-entry ring that makes a double-click or a
 * reconnect-retry an idempotent no-op.
 */
async function onMove(c: TableCtx, body: MoveMsg): Promise<MoveAck> {
	const game = c.state.game;

	if (game.status !== 'active') {
		return { ok: false, code: PROTOCOL_ERROR_CODES.game_complete, legal: [] };
	}
	if (body.seq <= game.appliedSeq || game.recentMoveIds.includes(body.clientMoveId)) {
		// A redelivered message or a double-click. Idempotent by construction.
		return { ok: true, v: game.v };
	}

	const seat = HUMAN_SEAT;
	const legal = legalMoves(game, seat);
	const move = legal.find((m) => m.id === body.moveId);
	if (move === undefined) {
		return { ok: false, code: 'illegal_move', legal };
	}

	// Throws RuleError on a stale nonce or a rules violation; the loop converts it.
	const steps = await applyMoveAsHuman(c, seat, move, body);
	await armTurn(c, steps);
	return { ok: true, v: c.state.game.v };
}

/** {@link applyMove} plus the two idempotency writes that only the human path has. */
async function applyMoveAsHuman(
	c: TableCtx,
	seat: Seat,
	move: LegalMove,
	body: MoveMsg
): Promise<readonly Step[]> {
	const before = c.state.game;
	const out = apply(before, seat, move.move, { turnId: body.turnId });

	commit(c, out.state);
	c.state.game.appliedSeq = body.seq;
	c.state.game.pending = null;
	rememberMoveId(c, body.clientMoveId);
	journal(c, { seat, id: move.id, move: move.move, kind: 'human' });

	const steps = settle(c, [...out.steps]);

	await c.saveState({ immediate: true }); // 1. PERSIST
	emitScript(c, steps);
	pushSync(c, steps); // 2. FAN OUT
	await handBoundary(c, before, steps);
	return steps;
}

/**
 * An `aiSeat`'s answer. A **proposal**, not an instruction.
 *
 * Everything is re-derived: the token is compared in constant time before any
 * state read, the acting seat is taken from `hand.turnSeat` (the `seat` on the
 * message is a tripwire compared against it, never an input), the turn nonce must
 * match, and the chosen id must appear in a freshly computed `legalMoves()`. An
 * id that does not falls back to the engine's first legal move and is logged —
 * never retried as the same message, because retrying a poisoned message is how
 * a poison pill gets built.
 *
 * A decision that arrives faster than the pacing floor is **parked**, not played:
 * `pending.revealAt` is persisted and a schedule releases it, so a crash during
 * the pacing window still delivers the move on wake. The move lands late, never
 * lost.
 */
async function onAiDecision(c: TableCtx, body: AIDecision): Promise<void> {
	const game = c.state.game;

	if (!tokenOk(game.internalToken, body.internalToken)) {
		c.log.warn('dropped forged aiDecision', { code: PROTOCOL_ERROR_CODES.internal_token_mismatch });
		return;
	}
	if (body.gameId !== game.gameId) return;
	if (game.status !== 'active') return;
	if (body.turnId !== game.turnId) return; // stale reply for a turn that moved on

	const seat = game.hand.turnSeat; // THE authority
	if (seat === null || !isAiSeat(seat)) return;
	if (body.seat !== seat) {
		c.log.warn('dropped aiDecision with a mismatched seat', {
			claimed: body.seat,
			actual: seat,
			code: PROTOCOL_ERROR_CODES.seat_mismatch
		});
		return;
	}

	const pending = game.pending;
	if (pending === null || pending.turnId !== game.turnId) return; // nothing outstanding
	if (pending.decision !== null) return; // duplicate reply; first one wins

	const legal = legalMoves(game, seat);
	const chosen = legal.find((m) => m.id === body.moveId);
	const fallback = legal[0];
	if (chosen === undefined && fallback === undefined) return;
	if (chosen === undefined) {
		c.log.warn('aiSeat returned a move outside the legal set', { seat, moveId: body.moveId });
	}
	const move = chosen ?? fallback;
	if (move === undefined) return;
	const source: AIDecisionSource = chosen === undefined ? 'fallback' : body.source;

	if (typeof body.banter === 'string') {
		emitChat(c, seat, 'banter', body.banter, `${game.gameId}:${game.turnId}:${seat}`);
	}

	pending.decision = move.id;
	pending.source = source;

	const wait = pending.revealAt - Date.now();
	if (wait > 0) {
		if (game.watchdogId !== null) {
			await c.schedule.cancel(game.watchdogId);
			game.watchdogId = null;
		}
		game.revealId = await c.schedule.after(wait, 'releaseAiMove', game.turnId);
		await c.saveState({ immediate: true });
		return;
	}
	await releasePending(c);
}

/** Play a parked decision now that its pacing floor has elapsed. */
async function releasePending(c: TableCtx): Promise<void> {
	const game = c.state.game;
	const pending = game.pending;
	if (pending === null || pending.turnId !== game.turnId || pending.decision === null) return;

	const seat = game.hand.turnSeat;
	if (seat === null || seat !== pending.seat) {
		game.pending = null;
		await c.saveState({ immediate: true });
		return;
	}

	const legal = legalMoves(game, seat);
	const move = legal.find((m) => m.id === pending.decision) ?? legal[0];
	if (move === undefined) {
		game.pending = null;
		await c.saveState({ immediate: true });
		return;
	}

	const source: AIDecisionSource = pending.source ?? 'fallback';
	const steps = await applyMove(c, seat, move, { kind: 'ai', source });
	await armTurn(c, steps);
}

/** A persona line relayed from an AI seat. Carries no card identity by construction. */
function onAiSay(c: TableCtx, body: AiSayMsg): void {
	const game = c.state.game;
	if (!tokenOk(game.internalToken, body.internalToken)) {
		c.log.warn('dropped forged aiSay', { code: PROTOCOL_ERROR_CODES.internal_token_mismatch });
		return;
	}
	if (body.gameId !== game.gameId) return;
	if (!isAiSeat(body.seat)) return;
	if (body.turnId !== game.turnId) return; // a line for a turn that has passed

	if (body.final) {
		emitChat(c, body.seat, 'banter', body.text ?? '', body.msgId);
		return;
	}
	const delta = screenLine(body.delta, BANTER_MAX_CHARS);
	if (delta.length === 0) return;
	c.broadcast('chatDelta', { msgId: body.msgId, seat: body.seat, delta });
}

/** Every timer, after it has been made durable by passing through the queue. */
async function onTick(c: TableCtx, body: TickMsg): Promise<void> {
	const game = c.state.game;
	if (!tokenOk(game.internalToken, body.internalToken)) {
		c.log.warn('dropped forged tick', { code: PROTOCOL_ERROR_CODES.internal_token_mismatch });
		return;
	}
	if (body.gameId !== game.gameId) return;

	// `flushProfile` and `reap` are match-scoped, not turn-scoped: they must
	// survive the nonce check that every other tick has to pass.
	if (body.kind === 'flushProfile') {
		c.state.profileRetryId = null;
		await recordMatchIfNeeded(c);
		return;
	}
	if (body.kind === 'reap') {
		c.state.reapId = null;
		if (c.state.recordedAt === null) {
			// The durable copy does not exist yet. Never destroy over history.
			c.state.reapId = await c.schedule.after(TEMPO.reapMs, 'onReap');
			await c.saveState({ immediate: true });
			return;
		}
		c.destroy();
		return;
	}

	if (body.turnId !== game.turnId) return; // the turn moved on; this timer is stale
	if (game.status !== 'active') return;

	switch (body.kind) {
		case 'tempo': {
			c.state.tempoId = null;
			const before = c.state.game;
			const out = advance(before);
			if (out.state === before) return;
			commit(c, out.state);
			const steps = settle(c, [...out.steps]);
			await c.saveState({ immediate: true }); // 1. PERSIST
			emitScript(c, steps);
			pushSync(c, steps); // 2. FAN OUT
			await handBoundary(c, before, steps);
			await armTurn(c, steps);
			return;
		}
		case 'releaseAi': {
			c.state.game.revealId = null;
			await releasePending(c);
			return;
		}
		case 'aiTimeout': {
			c.state.game.watchdogId = null;
			const seat = game.hand.turnSeat;
			if (seat === null || !isAiSeat(seat)) return;
			if (body.seat !== undefined && body.seat !== seat) return;
			c.broadcast('thinking', { seat, on: false, extended: false });
			await autoPlay(c, seat, 'ai_timeout');
			return;
		}
		case 'nudge': {
			c.state.game.timerId = null;
			// Presentation only: the partner says something, no state changes.
			emitChat(
				c,
				partnerOf(HUMAN_SEAT),
				'system',
				'Your call, partner.',
				`${game.gameId}:${game.turnId}:nudge`
			);
			return;
		}
		case 'abandon': {
			c.state.abandonId = null;
			const seat = game.hand.turnSeat;
			if (seat !== HUMAN_SEAT) return;
			c.state.abandonStrikes += 1;
			if (c.state.abandonStrikes >= 2) {
				commit(c, { ...c.state.game, status: 'abandoned' });
				await c.saveState({ immediate: true });
				pushSync(c, []);
				await recordMatchIfNeeded(c);
				return;
			}
			await autoPlay(c, seat, 'abandon');
			return;
		}
	}
}

/* ========================================================================== */
/* Wake reconciliation                                                        */
/* ========================================================================== */

/**
 * Close the one genuinely lossy window: a crash after a message was acked but
 * before the next turn's timer was armed.
 *
 * Under serverless this is a **routine** code path, not a rarely-exercised safety
 * net — every migration is a wake, and a wake runs this. It re-derives the
 * expected pending action from `(phase, turnSeat, turnId, pending)` and arms it,
 * which is why the design keeps all four of those in the same durable blob.
 *
 * A parked AI decision is handled first and returns early, because it is the one
 * case where the correct action is *not* to re-open the turn: the decision already
 * exists and only its pacing floor is outstanding.
 */
async function reconcile(c: TableCtx): Promise<void> {
	const game = c.state.game;
	if (game.status !== 'active') {
		await recordMatchIfNeeded(c);
		return;
	}

	const pending = game.pending;
	if (pending !== null && pending.turnId === game.turnId && pending.decision !== null) {
		const wait = pending.revealAt - Date.now();
		if (wait <= 0) {
			await releasePending(c);
			return;
		}
		game.revealId = await c.schedule.after(wait, 'releaseAiMove', game.turnId);
		await c.saveState({ immediate: true });
		return;
	}

	const before = c.state.game;
	const steps = settle(c, []);
	await c.saveState({ immediate: true });
	if (steps.length > 0) {
		emitScript(c, steps);
		pushSync(c, steps);
	}
	await handBoundary(c, before, steps);
	await armTurn(c, steps);
}

/* ========================================================================== */
/* The message envelope                                                       */
/* ========================================================================== */

/**
 * One message off the durable queue.
 *
 * `c.queue.iter()` is typed `AsyncIterable<any>` in `rivetkit@2.3.9`, so the loop
 * is annotated with the discriminated union the four queue declarations imply.
 * The cast is the only one in this file and is confined to the `for await` head.
 */
type TableMessage =
	| { readonly name: 'move'; readonly body: MoveMsg; complete(response: MoveAck): Promise<void> }
	| { readonly name: 'aiDecision'; readonly body: AIDecision; complete(): Promise<void> }
	| { readonly name: 'aiSay'; readonly body: AiSayMsg; complete(): Promise<void> }
	| { readonly name: 'tick'; readonly body: TickMsg; complete(): Promise<void> };

/* ========================================================================== */
/* The actor                                                                  */
/* ========================================================================== */

export const euchreTable = actor({
	options: {
		name: 'Euchre Table',
		icon: 'diamond'
	},

	events: tableEvents,
	queues: tableQueues,

	/**
	 * Mint the match.
	 *
	 * `internalToken` is a v4 UUID from the platform CSPRNG, not derived from the
	 * seed: it is the actor↔actor authorization boundary, and a token a replay can
	 * reproduce is not a secret. `seed` is 32 hex characters of CSPRNG output for
	 * the same reason a shuffle should not be guessable from the game id.
	 *
	 * The engine returns a `lobby` state — nothing is dealt and no seat is to act.
	 * `run` calls {@link reconcile}, which settles it into `cutting` and opens the
	 * first hand. Creation therefore deals nothing, which is what lets a table be
	 * created ahead of the player arriving.
	 */
	createState: (c, input: TableCreateInput | undefined): TableState => {
		const gameId = c.key[1] ?? `g-${randomHex(8)}`;
		const game = createGame({
			gameId,
			seed: input?.seed ?? randomHex(16),
			internalToken: crypto.randomUUID(),
			cfg: input?.cfg,
			firstDealer: input?.firstDealer
		});
		return {
			schema: 1,
			matchId: gameId,
			ownerUserId: input?.ownerUserId ?? '',
			startedAt: Date.now(),
			game,
			personas: input?.personas ?? [],
			journal: [],
			stats: emptyStats(),
			handsPlayed: 0,
			lastJournaledHandNo: -1,
			lastResetHandNo: -1,
			recordedAt: null,
			abandonStrikes: 0,
			tempoId: null,
			abandonId: null,
			profileRetryId: null,
			reapId: null
		};
	},

	/** Rebuilt on every wake; see {@link TableVars}. */
	createVars: (): TableVars => ({ jwks: makeJwks(appUrl()) }),

	/**
	 * The origin allowlist, and the only CORS this system has.
	 *
	 * The browser's WebSocket terminates at **Rivet Cloud**, not at this
	 * deployment, so Vercel's CORS configuration is not in the path and cannot
	 * help. In production an empty allowlist is a hard deny rather than a
	 * permissive default: failing open here would let any page drive a table.
	 */
	onBeforeConnect: (c): void => {
		const origin = c.request?.headers.get('origin') ?? '';
		const allowed = allowedOrigins();
		if (allowed.length === 0) {
			if (isLocalDev()) return;
			throw new UserError('Origin not allowed', {
				code: PROTOCOL_ERROR_CODES.origin_not_allowed
			});
		}
		if (!allowed.includes(origin)) {
			throw new UserError('Origin not allowed', {
				code: PROTOCOL_ERROR_CODES.origin_not_allowed
			});
		}
	},

	/**
	 * Authenticate, authorize, and assign the seat.
	 *
	 * WebSockets carry no custom headers and `c.request.headers` does not work for
	 * `.connect()`, so every credential arrives in connection params — all of it
	 * attacker-controlled. The only field with any authority is `token`, and only
	 * after `jose` has checked its signature, issuer, audience and expiry.
	 *
	 * **The seat is assigned, never accepted.** `params.seat` is a *request*; the
	 * server answers `HUMAN_SEAT` unconditionally, and a claim on seats 1–3 is
	 * rejected outright rather than silently corrected, because a client that asks
	 * for an AI chair is not a client with a stale constant — it is trying to be
	 * dealt someone else's cards.
	 */
	createConnState: async (c, params: TableConnectParams): Promise<TableConnState> => {
		if (params?.protocolVersion !== undefined && params.protocolVersion !== PROTOCOL_VERSION) {
			throw new UserError('Unsupported protocol version', {
				code: PROTOCOL_ERROR_CODES.protocol_version_mismatch
			});
		}
		if (params?.seat !== undefined && params.seat !== HUMAN_SEAT) {
			c.log.warn('connection claimed a non-human seat', {
				claimed: params.seat,
				code: PROTOCOL_ERROR_CODES.seat_mismatch
			});
			throw new UserError('Seat 0 is the only seat a player may occupy', {
				code: PROTOCOL_ERROR_CODES.seat_mismatch
			});
		}

		let userId: string;
		try {
			const claims = await verifyPlayer(c.vars.jwks, params?.token ?? '', appUrl());
			userId = claims.userId;
		} catch {
			throw new UserError('Invalid or expired token', {
				code: PROTOCOL_ERROR_CODES.invalid_token
			});
		}

		// Authorization, not just authentication. An unclaimed table adopts its
		// first authenticated visitor — a table is created by its owner through
		// the server-side client, so the window is one round trip wide and the
		// gameId is unguessable; a claimed table admits nobody else, ever.
		if (c.state.ownerUserId === '') {
			c.state.ownerUserId = userId;
			await c.saveState({ immediate: true });
		} else if (c.state.ownerUserId !== userId) {
			throw new UserError('Not your game', { code: PROTOCOL_ERROR_CODES.forbidden });
		}

		return { userId, seat: HUMAN_SEAT, role: 'player', since: Date.now() };
	},

	/**
	 * A fresh connection is resynchronised immediately and unconditionally.
	 *
	 * Missed events are never replayed by the transport, so a genuine drop loses
	 * them: `steps: []` tells the client to **snap, not animate**. That is the whole
	 * reconnect protocol, and it is why `v` is monotonic.
	 */
	onConnect: (c, conn): void => {
		const seat = conn.state.seat;
		const payload: SyncEvent = { v: c.state.game.v, view: project(c.state.game, seat), steps: [] };
		conn.send('sync', payload);
		c.broadcast('presence', { seat, online: true });
	},

	onDisconnect: (c, conn): void => {
		c.broadcast('presence', { seat: conn.state.seat, online: false });
	},

	/**
	 * The single serialized writer.
	 *
	 * A `RuleError` is a *rules event* — expected, recoverable, shown to a player —
	 * and becomes a typed rejection carrying the current legal set so a drifted
	 * client can re-render truth without another round trip. Anything else is a
	 * genuine fault and is rethrown **unacked**, so the message is redelivered
	 * rather than silently dropped.
	 *
	 * Every branch completes its message. A `completable: true` message that is
	 * never completed is redelivered forever, which turns one bug into a hot loop.
	 */
	run: async (c): Promise<void> => {
		c.log.info('euchreTable run loop starting', { matchId: c.state.matchId });
		await reconcile(c);
		c.log.info('euchreTable reconciled', {
			phase: c.state.game.hand.phase,
			turnSeat: c.state.game.hand.turnSeat,
			status: c.state.game.status
		});

		const messages = c.queue.iter({ completable: true }) as AsyncIterable<TableMessage>;
		for await (const message of messages) {
			c.log.info('euchreTable queue', { name: message.name });
			try {
				switch (message.name) {
					case 'move': {
						const ack = await onMove(c, message.body);
						await message.complete(ack); // 3. ACK, after persist and fan-out
						break;
					}
					case 'aiDecision': {
						await onAiDecision(c, message.body);
						await message.complete();
						break;
					}
					case 'aiSay': {
						onAiSay(c, message.body);
						await message.complete();
						break;
					}
					case 'tick': {
						await onTick(c, message.body);
						await message.complete();
						break;
					}
				}
			} catch (err) {
				if (!isRuleError(err)) throw err;

				// Narrow by queue name so `move.complete` keeps its MoveAck arity.
				if (message.name === 'move') {
					await message.complete({ ok: false, code: err.code, legal: err.legal });
					continue;
				}

				// An internal path produced a rules violation: a bug in dispatch,
				// not in the player. Ack it so it is not redelivered forever.
				c.log.error('internal path raised a RuleError', {
					name: message.name,
					code: err.code
				});
				await message.complete();
				continue;
			}
		}
	},

	actions: {
		/**
		 * The reconnect resync, and a read-only action like every other one here
		 * except `submitMove`.
		 *
		 * Redacted from `c.conn.state.seat` — the seat this connection was assigned,
		 * not a seat it asked for.
		 */
		snapshot: (c): PublicGameView => project(c.state.game, c.conn.state.seat),

		/**
		 * The last completed trick, for the last-trick viewer.
		 *
		 * During `trick_resolve`, `hand.trick` and the final entry of `hand.trickLog`
		 * are **the same trick**; reading only the log avoids showing it twice.
		 */
		lastTrick: (c): Trick | null => {
			const log = c.state.game.hand.trickLog;
			return log.length === 0 ? null : (log[log.length - 1] ?? null);
		},

		/**
		 * The only connection-reachable mutation, and it mutates nothing itself.
		 *
		 * It validates, then enqueues and waits: the durable run loop is what
		 * actually applies the move. The acting seat is `c.conn.state.seat`,
		 * verified here and then discarded — {@link MoveMsg} has no seat field, so
		 * nothing downstream can be misled by one.
		 *
		 * An unknown move id is refused here with a typed `UserError` rather than
		 * being passed to the reducer, so the client gets `illegal_move` plus the
		 * current legal set and can snap back immediately.
		 */
		submitMove: async (c, request: SubmitMoveRequest): Promise<MoveAck> => {
			const conn = c.conn.state;
			if (conn.role !== 'player' || conn.seat !== HUMAN_SEAT) {
				throw new UserError('Not a player connection', {
					code: PROTOCOL_ERROR_CODES.forbidden
				});
			}

			const game = c.state.game;
			if (game.status !== 'active') {
				throw new UserError('This match is over', {
					code: PROTOCOL_ERROR_CODES.game_complete
				});
			}

			const legal = legalMoves(game, HUMAN_SEAT);
			const move = legal.find((m) => m.id === request?.moveId);
			if (move === undefined) {
				throw new UserError('That move is not legal right now', {
					code: 'illegal_move',
					metadata: { legal }
				});
			}
			if (typeof request.clientMoveId !== 'string' || request.clientMoveId.length === 0) {
				throw new UserError('Missing clientMoveId', {
					code: PROTOCOL_ERROR_CODES.internal_error
				});
			}

			const body: MoveMsg = {
				seq: game.appliedSeq + 1,
				turnId: request.turnId,
				moveId: move.id,
				clientMoveId: request.clientMoveId.slice(0, 64),
				userId: conn.userId
			};

			// M2/serverless workaround: `enqueueAndWait` from an action deadlocks
			// against the single-lane `run` loop under local serverless wakes
			// (action holds the lane while waiting for run to `complete`). Apply
			// inline with the same durability ordering as `onMove`. AI/timer
			// paths still enter via queue `tick` messages from schedule actions
			// that use non-blocking `queue.send`.
			try {
				return await onMove(c as unknown as TableCtx, body);
			} catch (err) {
				if (isRuleError(err)) {
					return { ok: false, code: err.code, legal: err.legal };
				}
				throw err;
			}
		},

		/* -- Schedule targets. Each does a stale-nonce check and enqueues. ------ */

		onTempoGate: async (c, turnId: string): Promise<void> => {
			if (c.state.game.turnId !== turnId) return;
			await c.queue.send('tick', tick(c.state.game, 'tempo', turnId));
		},

		releaseAiMove: async (c, turnId: string): Promise<void> => {
			if (c.state.game.turnId !== turnId) return;
			await c.queue.send('tick', tick(c.state.game, 'releaseAi', turnId));
		},

		onAiTimeout: async (c, turnId: string, seat: Seat): Promise<void> => {
			if (c.state.game.turnId !== turnId) return;
			await c.queue.send('tick', { ...tick(c.state.game, 'aiTimeout', turnId), seat });
		},

		onNudge: async (c, turnId: string): Promise<void> => {
			if (c.state.game.turnId !== turnId) return;
			await c.queue.send('tick', tick(c.state.game, 'nudge', turnId));
		},

		onAbandon: async (c, turnId: string): Promise<void> => {
			if (c.state.game.turnId !== turnId) return;
			await c.queue.send('tick', tick(c.state.game, 'abandon', turnId));
		},

		flushProfile: async (c): Promise<void> => {
			await c.queue.send('tick', tick(c.state.game, 'flushProfile', c.state.game.turnId));
		},

		onReap: async (c): Promise<void> => {
			await c.queue.send('tick', tick(c.state.game, 'reap', c.state.game.turnId));
		}
	}
});

/** The shared body of every schedule-originated tick. */
function tick(game: GameState, kind: TickMsg['kind'], turnId: string): TickMsg {
	return { internalToken: game.internalToken, gameId: game.gameId, kind, turnId };
}
