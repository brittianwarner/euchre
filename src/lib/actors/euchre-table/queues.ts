/**
 * `euchre-table/queues.ts` — the table's durable inbox and its realtime outbox.
 *
 * Two things live here and nothing else: the four **queues** that are the only
 * way state ever changes, and the five **events** the table emits. Both are
 * declaration-only so that `index.ts` reads as game plumbing rather than as
 * schema, and so the `canPublish` / `canSubscribe` guards can be reviewed on one
 * screen.
 *
 * ## Why every mutation is a queue message
 *
 * Actions run **concurrently and are not durable**. Two `submitMove` actions
 * racing each other would interleave their `await`s and corrupt the trick, and a
 * crash mid-action loses the move with no redelivery. Queue messages are
 * persisted before ack, are processed strictly in order by the single `run`
 * loop, and are redelivered if the loop dies before `message.complete()`. So the
 * table's actions are read-only or enqueue-and-wait, and the run loop is the
 * only writer. See `docs/01-ARCHITECTURE.md` §8.
 *
 * ## `canPublish` is not the security boundary, and this file says so out loud
 *
 * Rivet 2.3 stateless HTTP publishes also have connection state. Internal
 * callers authenticate with the per-match secret at connection setup; browser
 * connections never receive the internal role. Message handlers independently
 * verify the envelope token before reading or changing state.
 *
 * The real boundary on every actor→actor queue is the unguessable
 * `internalToken` minted in the table's `createState`, handed to each `aiSeat` as
 * create input, and constant-time compared by {@link tokenOk} **before any state
 * read**, on every message. `docs/01-ARCHITECTURE.md` §3.2 requires a test that
 * proves this: deleting the `tokenOk` call must make a forged-publish test pass.
 *
 * @see docs/01-ARCHITECTURE.md §3.2 · docs/06-REVISED-ARCHITECTURE.md §4
 */

import { event, queue } from 'rivetkit';
import { HUMAN_SEAT } from '#lib/euchre/index.ts';
import type {
	AIDecision,
	ChatDeltaEvent,
	ChatEvent,
	InternalEnvelope,
	LegalMoveId,
	MoveAck,
	PresenceEvent,
	Seat,
	SyncEvent,
	TableConnState,
	TableEventName,
	ThinkingEvent
} from '#lib/protocol/index.ts';

/* ========================================================================== */
/* The guard context                                                          */
/* ========================================================================== */

/**
 * The shape a `canPublish` / `canSubscribe` hook actually receives.
 *
 * Declared structurally rather than imported from the actor definition, which
 * would be circular (the actor's `queues` block is what we are typing). Supplying
 * it as the third generic of `queue<TMessage, TComplete, TContext>` is what gives
 * these hooks a typed `c.conn` at all.
 */
export interface Guard {
	readonly request?: Request;
	readonly conn?: { readonly state: TableConnState };
}

/**
 * Reject anything that arrived over a live browser connection.
 *
 * Read the module header before trusting this: it fails open for a stateless
 * HTTP publish. It is defence in depth on top of {@link tokenOk}, never instead
 * of it.
 */
function externalDenied(c: Guard): boolean {
	return c.conn === undefined || c.conn.state.role === 'internal';
}

/** Only the human's own player connection may publish a move. */
function humanPlayerOnly(c: Guard): boolean {
	if (c.conn === undefined) return false;
	return c.conn.state.role === 'player' && c.conn.state.seat === HUMAN_SEAT;
}

/** Only a player connection may subscribe to a redacted per-connection channel. */
function playersOnly(c: Guard): boolean {
	return c.conn?.state.role === 'player';
}

/* ========================================================================== */
/* The actor↔actor authorization boundary                                     */
/* ========================================================================== */

/**
 * Constant-time comparison of a message's `internalToken` against the match's.
 *
 * Called before **any** state read on every actor→actor message. Length is
 * compared first and short-circuits, which leaks only the token's length — the
 * token is a v4 UUID, so its length is public anyway.
 *
 * Deliberately hand-rolled rather than `node:crypto`'s `timingSafeEqual`: this
 * module is imported by the SvelteKit build, and pulling a node builtin into that
 * graph for a 36-byte compare is a worse trade than eight lines of XOR.
 */
export function tokenOk(expected: string, got: unknown): boolean {
	if (!expected || typeof got !== 'string') return false;
	if (got.length !== expected.length) return false;
	let diff = 0;
	for (let i = 0; i < expected.length; i++) {
		diff |= expected.charCodeAt(i) ^ got.charCodeAt(i);
	}
	return diff === 0;
}

/* ========================================================================== */
/* Queue message bodies                                                       */
/* ========================================================================== */

/**
 * A human move on its way into the run loop.
 *
 * **There is no `seat` field, and there must never be one.** The acting seat is
 * `HUMAN_SEAT`, a server constant; `submitMove` reads `c.conn.state.seat` only to
 * *verify* the connection is the human's, and the verified `userId` rides along
 * purely so the loop can log who acted. A seat in this payload would be a field
 * an attacker could fill in (binding rule 3).
 */
export interface MoveMsg {
	/** Server idempotency: `state.appliedSeq + 1` at enqueue time. */
	readonly seq: number;
	/** The nonce of the view the client acted on. Re-checked by `apply()`. */
	readonly turnId: string;
	readonly moveId: LegalMoveId;
	/** Client-supplied dedupe key, matched against the 32-entry `recentMoveIds` ring. */
	readonly clientMoveId: string;
	/** From the verified JWT, for logs only. Never an authorization input. */
	readonly userId: string;
}

/** Synthesized by authenticated actions; browsers cannot publish internal envelopes. */
export interface ReviewMsg extends InternalEnvelope {
	readonly action: 'set' | 'continue';
	readonly enabled?: boolean;
	readonly turnId: string;
}

/** Every reason a timer wakes the mutation loop. */
export type TickKind =
	'tempo' | 'releaseAi' | 'aiTimeout' | 'nudge' | 'abandon' | 'flushProfile' | 'reap';

/**
 * A schedule firing, converted into a durable mutation.
 *
 * Schedules fire **actions**, and actions are not durable — so every schedule
 * target does nothing but a stale-nonce check followed by `c.queue.send("tick",
 * …)`. Schedule → action → queue is the only legal path from a timer into the
 * mutation loop.
 */
export interface TickMsg extends InternalEnvelope {
	readonly kind: TickKind;
	/** The nonce the timer was armed against. A mismatch means the turn moved on: drop. */
	readonly turnId: string;
	/** Present for `aiTimeout`; a tripwire compared against `hand.turnSeat`, never an instruction. */
	readonly seat?: Seat;
}

/**
 * A streamed or completed persona line relayed from an `aiSeat`.
 *
 * `$lib/protocol` declares {@link ChatDeltaEvent} but no actor→actor message
 * capable of producing one, because the protocol module was written before the
 * table existed. This is that message, and it is deliberately the narrowest
 * shape that can feed the two chat events: a seat, a stable `msgId`, and either a
 * `delta` or a final `text`. It carries no card identity and cannot: banter is
 * generated by the seat from a public-only slice of its own view.
 */
export interface AiSayMsg extends InternalEnvelope {
	readonly seat: Seat;
	/** Stable across the deltas and the final message, so a rejected line can be retracted. */
	readonly msgId: string;
	/** The turn this line belongs to; a stale line is dropped rather than spoken late. */
	readonly turnId: string;
	readonly delta?: string;
	readonly text?: string;
	readonly final: boolean;
}

/* ========================================================================== */
/* The queues                                                                 */
/* ========================================================================== */

/**
 * The table's complete durable inbox. Four queues, one writer.
 *
 * `move` is the only one reachable from a browser, and it is the only one that
 * completes with a payload — the client needs its `MoveAck` synchronously so a
 * rejected optimistic play can snap back.
 */
export const tableQueues = {
	/** **The** human mutation path. Completes with a {@link MoveAck}. */
	move: queue<MoveMsg, MoveAck, Guard>({ canPublish: humanPlayerOnly }),
	review: queue<ReviewMsg, undefined, Guard>({ canPublish: externalDenied }),
	/** An `aiSeat`'s chosen move. Authorized by {@link tokenOk}, not by the guard. */
	aiDecision: queue<AIDecision, undefined, Guard>({ canPublish: externalDenied }),
	/** An `aiSeat`'s persona chatter, relayed to the `chat` / `chatDelta` events. */
	aiSay: queue<AiSayMsg, undefined, Guard>({ canPublish: externalDenied }),
	/** Every timer, funnelled through the loop so timing is durable too. */
	tick: queue<TickMsg, undefined, Guard>({ canPublish: externalDenied })
};

/** The name of any queue the table accepts. */
export type TableQueueName = keyof typeof tableQueues;

/* ========================================================================== */
/* The events                                                                 */
/* ========================================================================== */

/**
 * The table's complete outbox, checked against `TableEventMap` by the
 * `satisfies` clause so adding an event here without adding it to
 * `$lib/protocol` (or vice versa) is a type error.
 *
 * Exactly one of these is per-connection: `sync`. It is the only channel that
 * carries card identity, which is why it is the only one that can be redacted —
 * `conn.send('sync', …)` has a seat to project for, and `c.broadcast` does not.
 * The other four are broadcasts and are structurally incapable of leaking a card.
 */
export const tableEvents = {
	sync: event<SyncEvent, Guard>({ canSubscribe: playersOnly }),
	thinking: event<ThinkingEvent>(),
	chat: event<ChatEvent>(),
	chatDelta: event<ChatDeltaEvent>(),
	presence: event<PresenceEvent>()
} satisfies Record<TableEventName, unknown>;
