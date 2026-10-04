/**
 * `$lib/protocol` — the shared contract.
 *
 * Every wire-visible shape in this application is declared exactly once, here:
 * the realtime events the table broadcasts, what a browser is allowed to send on
 * connect versus what the server derives and trusts, the two actor-to-actor
 * messages that make an AI seat decide, the user-editable persona settings, the
 * durable history records, and the error codes everyone shares.
 *
 * **This module is types and small frozen constants only.** No runtime logic, no
 * `rivetkit` import, no `zod`, no `$env`, no clock, no randomness. It is imported
 * unchanged by the actors, by the SvelteKit server, and by the browser bundle, so
 * anything with a dependency does not belong in it. Validators (`zod` schemas)
 * belong next to the boundary that validates; they are *derived from* these types,
 * never the other way round.
 *
 * Three rules govern everything below, and every questionable field was decided by
 * them:
 *
 * 1. **The table is authoritative.** Nothing a client or a model sends is trusted.
 *    Types that carry untrusted data say so in their name or their TSDoc; where a
 *    server-derived twin exists ({@link TableConnectParams} → {@link TableConnState}),
 *    the two are deliberately different types so they cannot be confused.
 * 2. **`PublicGameView` is the only shape that leaves the server.** Produced solely
 *    by `project(state, seat)` in `$lib/euchre`. It appears in {@link SyncEvent} for
 *    the browser and in {@link AIDecideRequest} for an AI seat — an AI opponent is
 *    given exactly what a human in that chair could know, so cheating is a type
 *    error rather than a policy. **No message type in this file may ever carry
 *    `GameState`.**
 * 3. **The acting seat is never read from a payload.** Where a `seat` field exists
 *    on an inbound message it is present only so a mismatch can be *detected* and
 *    logged; the authority is `c.conn.state.seat` for a human and
 *    `state.hand.turnSeat` for anything internal.
 *
 * Normative sources: `docs/06-REVISED-ARCHITECTURE.md` (hosting, persistence),
 * `docs/01-ARCHITECTURE.md` §3 and §6 (actors, events, redaction),
 * `docs/03-AI-AGENTS.md` (personas, the decision ladder, prompt security).
 */

import type {
	AIDecisionSource,
	CardId,
	EngineConfig,
	GameStatus,
	HandResult,
	LegalMove,
	LegalMoveId,
	PlayerAction,
	PublicGameView,
	RuleCode,
	Seat,
	Step,
	Suit,
	Team
} from '#lib/euchre/index.ts';

/* ========================================================================== */
/* Engine re-exports                                                          */
/* ========================================================================== */

/**
 * The engine vocabulary every consumer of this protocol needs, re-exported so
 * that nothing outside `src/lib/euchre` ever imports a module *inside* it.
 *
 * Import **types** from `$lib/protocol`; import **functions** (`project`,
 * `legalMoves`, `apply`, …) from `$lib/euchre`, which is the engine's own public
 * surface. Deep imports such as `$lib/euchre/types` are a review reject in both
 * directions — one rulebook, one door.
 */
export type {
	ActionKind,
	AIDecisionSource,
	AutoReason,
	Bid,
	BidRecord,
	CardId,
	DealPacket,
	DealtPacket,
	EngineConfig,
	GamePhase,
	GameStatus,
	HandResult,
	LegalMove,
	LegalMoveId,
	Move,
	PendingDecision,
	Play,
	PlayerAction,
	PublicGameView,
	Rank,
	RuleCode,
	Seat,
	SeatRole,
	Step,
	Suit,
	Team,
	Trick
} from '#lib/euchre/index.ts';

/**
 * `GameState` is deliberately **not** re-exported. It holds all four hands, the
 * buried kitty and the dealer's discard, and it exists only inside
 * `euchreTable`'s `c.state`. If a file that imports `$lib/protocol` needs it,
 * that file is on the wrong side of the redaction boundary.
 */

/* ========================================================================== */
/* Protocol version                                                           */
/* ========================================================================== */

/**
 * The wire-contract version. Bump it when a payload in this file changes shape in
 * a way an older client cannot read.
 *
 * A client may send it in {@link TableConnectParams}; the table may reject or
 * degrade on mismatch. It is *not* `GameState.v` — that is a per-match monotonic
 * state counter and has nothing to do with this.
 */
export const PROTOCOL_VERSION = 1 as const;

/* ========================================================================== */
/* 1. Connection: what the browser sends, what the server derives              */
/* ========================================================================== */

/**
 * **UNTRUSTED.** The connection params a browser supplies when it opens its
 * WebSocket to Rivet Cloud for `euchreTable`, delivered to `onBeforeConnect` and
 * `createConnState` as the second argument.
 *
 * WebSockets carry no custom headers and `c.request.headers` does not work for
 * `.connect()`, so *all* connection auth arrives here. Supplied on the client by
 * `withActorParams()` (never a static `params` object, which would capture one
 * token forever and start failing fifteen minutes later).
 *
 * Every field is attacker-controlled. The only one with any authority is
 * {@link token}, and only after `jose` has verified its signature, issuer,
 * audience (`euchre-actors`) and expiry. {@link seat} is a *request*, not a fact:
 * the server assigns the seat and returns it in {@link TableConnState}.
 *
 * @see TableConnState for the server-derived twin.
 */
export interface TableConnectParams {
	/** Server-only per-match credential for stateless actor messages. */
	readonly internalToken?: string;
	/**
	 * A short-lived (~15 min) JWT minted by `POST /api/rivet-token`, audience
	 * `euchre-actors`. Verified in `createConnState` against the JWKS cached in
	 * `c.vars`; a failure throws `UserError` with code
	 * {@link PROTOCOL_ERROR_CODES.invalid_token}.
	 */
	readonly token: string;
	/**
	 * The seat the client would *like*. Advisory only, and in v1 always `0` — the
	 * human is seat 0 and the other three chairs are AI. The server ignores a
	 * disagreeing value rather than honouring it; a claim of a non-zero seat is
	 * logged as an attack.
	 */
	readonly seat?: Seat;
	/** The client's {@link PROTOCOL_VERSION}. Informational; the server decides. */
	readonly protocolVersion?: number;
}

/**
 * The one role a connection may hold in v1.
 *
 * Kept as a single-member union rather than a bare string so that widening it is
 * a deliberate, greppable act: a second role would mean a connection whose
 * projection seat is not its owner's seat, which requires re-deriving the whole of
 * `docs/01-ARCHITECTURE.md` §6.2. Spectators do not exist.
 */
export type ConnRole = 'player' | 'internal';

/**
 * **SERVER-DERIVED. TRUSTED.** What `createConnState` computed after verifying
 * {@link TableConnectParams}, stored by Rivet as `c.conn.state` and available on
 * every subsequent action, queue guard and fan-out.
 *
 * Nothing here is echoed from the client. `userId` comes out of the verified JWT
 * claims; `seat` is assigned by the table; `role` is decided by the table. This is
 * the *only* legitimate source of the acting seat for a human move, and the only
 * legitimate input to per-connection redaction:
 *
 * ```ts
 * conn.send('sync', { v, view: project(state, conn.state.seat), steps: projectSteps(steps, conn.state.seat) });
 * ```
 */
export interface TableConnState {
	/** From the verified JWT `sub`. Checked against the match's owner. */
	readonly userId: string;
	/** Assigned by the server. The sole authority for a human move's acting seat. */
	readonly seat: Seat;
	readonly role: ConnRole;
	/** Server clock at connect, for presence and idle accounting. */
	readonly since: number;
}

/* ========================================================================== */
/* 2. The realtime event protocol                                             */
/* ========================================================================== */

/**
 * The authoritative state push, and **the only transport in the system that
 * carries card identity**.
 *
 * Sent **per connection** with `conn.send('sync', …)`, never `c.broadcast` —
 * because both fields are redacted for one specific seat and a broadcast has no
 * seat. `pushSync()` is the only function permitted to emit it.
 *
 * `view` is `project(state, conn.state.seat)`; `steps` is
 * `projectSteps(steps, conn.state.seat)`. Steps are decoration — what just
 * happened, so the client can animate and announce — and are never truth. The
 * client applies `view` regardless of whether it can animate the steps, and drops
 * any event whose `v` it has already seen.
 *
 * Missed events are never replayed by the transport. A genuine network drop loses
 * them, which is why `v` is monotonic and why every (re)connect calls `snapshot()`
 * and snaps without animating (`steps: []`).
 */
export interface SyncEvent {
	/** `state.v`, monotonic per match. The client drops `v <= seen`. */
	readonly v: number;
	/** Redacted for the receiving connection's seat, by `project()`. */
	readonly view: PublicGameView;
	/** Transitions since this connection's last `v`, gated by `projectSteps()`. */
	readonly steps: readonly Step[];
}

/**
 * A seat is deliberating. Broadcast — it carries no card identity, so no redaction
 * is needed.
 *
 * `extended: true` is raised once a decision has outlived the model's first abort
 * and the gate re-armed, so the UI escalates ("still thinking…") instead of
 * appearing frozen. It is a *presentation* signal: the hard cap that guarantees
 * the game never stalls is a server schedule, not this flag.
 */
export interface ThinkingEvent {
	readonly seat: Seat;
	readonly on: boolean;
	readonly extended: boolean;
}

/**
 * What a chat line is, which decides how it is screened and where it is rendered.
 *
 * - `call` — engine-authored table script, verbatim and unscreened ("Order it
 *   up.", "I assist.", "March!", "Euchre!", "We're in the barn."). Generated from
 *   `sayForBid()` and the scoring result, never by a model.
 * - `system` — engine-authored table notice (throw-in, reshuffle, nudge).
 * - `banter` — model-authored persona chatter, generated from public state only
 *   and screened by `screenBanter()` before it is emitted. Routed to the separate
 *   `#live-banter` aria-live region so it can never interleave with a trump call.
 */
export type ChatKind = 'call' | 'banter' | 'system';

/**
 * A complete chat line. Broadcast.
 *
 * `msgId` is stable across the {@link ChatDeltaEvent}s that streamed it and this
 * final message, so the client can replace an accumulating buffer in place — and
 * so a line rejected by the output screen mid-stream can be *retracted* by id.
 */
export interface ChatEvent {
	readonly msgId: string;
	readonly seat: Seat;
	readonly kind: ChatKind;
	/**
	 * Already screened and clamped to {@link BANTER_MAX_CHARS} for `banter`.
	 * Rendered as text, never as markup.
	 */
	readonly text: string;
	/** Always `true`: this event *is* the terminal message for `msgId`. */
	readonly final: true;
}

/**
 * A streaming fragment of a `banter` line. Broadcast.
 *
 * Batched by the table (~80 ms / ~40 chars) so that three chatty seats at once
 * stay an order of magnitude under Rivet's 1000-message queue cap. The same output
 * screen is applied to the accumulated buffer at every flush *and* at the final
 * message; a rejection retracts the whole `msgId` client-side.
 */
export interface ChatDeltaEvent {
	readonly msgId: string;
	readonly seat: Seat;
	readonly delta: string;
}

/** A seat's connection came or went. Broadcast; carries no card identity. */
export interface PresenceEvent {
	readonly seat: Seat;
	readonly online: boolean;
}

/**
 * The complete event surface of `euchreTable`, name → payload.
 *
 * The actor's `events: { … }` block and the client's `onEvent()` handlers are both
 * checked against this map, so adding an event in one place without the other is a
 * type error. There are five, and there is no sixth: any new payload that could
 * carry a `CardId` must go through {@link SyncEvent}, because `sync` is the only
 * per-connection (and therefore the only redactable) channel.
 */
export interface TableEventMap {
	readonly sync: SyncEvent;
	readonly thinking: ThinkingEvent;
	readonly chat: ChatEvent;
	readonly chatDelta: ChatDeltaEvent;
	readonly presence: PresenceEvent;
}

/** The name of any event `euchreTable` emits. */
export type TableEventName = keyof TableEventMap;

/** Every event name, for exhaustive subscription and for tests. */
export const TABLE_EVENT_NAMES = [
	'sync',
	'thinking',
	'chat',
	'chatDelta',
	'presence'
] as const satisfies readonly TableEventName[];

/**
 * Events delivered per connection rather than broadcast, and therefore the only
 * ones that may contain redacted data.
 *
 * In v1 this set is exactly `{ sync }`. If it ever grows, the `no-broadcast-state`
 * lint rule and the redaction fuzz gate must both be extended before the new event
 * ships — every other channel is a broadcast and has no seat to redact for.
 */
export const PER_CONNECTION_EVENTS = ['sync'] as const satisfies readonly TableEventName[];

/* ========================================================================== */
/* 3. The human mutation path                                                 */
/* ========================================================================== */

/**
 * What the browser sends to `submitMove` — the **only** connection-reachable
 * mutation in the system.
 *
 * Note what is absent: there is no `seat`. The acting seat is `c.conn.state.seat`
 * and nothing else. A payload carrying a seat would be a field an attacker could
 * fill in, so the field does not exist.
 *
 * `turnId` is echoed back from the view the client acted on; a mismatch is
 * rejected as `stale_turn` rather than applied late. `clientMoveId` is an
 * idempotency key for double-click and reconnect-retry dedupe, matched against the
 * table's 32-entry `recentMoveIds` ring.
 */
export interface SubmitMoveRequest {
	readonly moveId: LegalMoveId;
	readonly turnId: string;
	readonly clientMoveId: string;
}

/**
 * The answer to a `submitMove`.
 *
 * On rejection the table returns the *current* legal set, so a client that has
 * drifted can re-render truth immediately instead of guessing. `code` distinguishes
 * a rule rejection (`must_follow_suit`) from a protocol rejection
 * (`game_complete`), and drives the copy shown to the player.
 *
 * **Gotcha, from `docs/06-REVISED-ARCHITECTURE.md` §6.3:** with `@rivetkit/svelte`'s
 * default `throwOnError: false`, a failed action *resolves `undefined`* rather than
 * throwing. Optimistic snap-back must branch on a falsy result or on
 * `lastActionError`, never on a `catch` block, or a rejected move sticks on screen.
 */
export type MoveAck =
	| { readonly ok: true; readonly v: number }
	| {
			readonly ok: false;
			readonly code: MoveRejectionCode;
			readonly legal: readonly LegalMove[];
	  };

/** Why a `submitMove` was refused: a rules verdict or a table-level refusal. */
export type MoveRejectionCode = RuleCode | ProtocolErrorCode;

/* ========================================================================== */
/* 4. Actor-to-actor: euchreTable ↔ aiSeat                                    */
/* ========================================================================== */

/**
 * The common envelope on every actor-to-actor queue message.
 *
 * `internalToken` — minted by `crypto.randomUUID()` in the table's `createState`
 * and handed to each `aiSeat` as create input — **is the authorization boundary**,
 * not the queues' `canPublish` guard. `canPublish` fails open for a forged publish
 * over the stateless HTTP handle (no `c.conn`), which is why the token is compared
 * in constant time *before any state read*, on both sides, on every message.
 */
export interface InternalEnvelope {
	/** Constant-time compared before any state read. Never logged. */
	readonly internalToken: string;
	/** The match this message belongs to. */
	readonly gameId: string;
}

/**
 * Which decision is being asked for. Derived from `state.hand.phase` by the table;
 * the AI seat uses it to pick a model tier, a step timeout and a pacing floor.
 *
 * `cut`, `bid1`, `bid2` and `discard` take the deliberate (Opus) path; `play` takes
 * the fast (Haiku) path.
 */
export type AIDecisionKind = 'cut' | 'bid1' | 'discard' | 'bid2' | 'play';

/**
 * One scored candidate from the deterministic heuristic engine.
 *
 * The heuristic itself is **not** part of this module — it is euchre judgement, not
 * euchre rules, and it lives with the AI seat. Only the shape is shared, because
 * the ranking crosses an actor boundary.
 *
 * Ties break on `id`, so a ranking is fully deterministic and reproducible in
 * replay. `why` is engine-authored and is the fallback for a rejected model
 * rationale.
 */
export interface RankedMove {
	readonly id: LegalMoveId;
	/** Higher is better. Comparable only within one ranking. */
	readonly score: number;
	/** Engine-authored, short, safe to show a human. */
	readonly why: string;
}

/**
 * **REQUEST: `euchreTable` → `aiSeat`, queue `decide`.**
 *
 * Everything an AI opponent is permitted to know, and nothing else.
 *
 * The seat receives {@link PublicGameView} — the same redacted projection a human
 * in that chair would get — so it cannot see another hand, the three buried kitty
 * cards, or the dealer's discard. That is structural: there is no field on this
 * type capable of carrying them. **Never widen this message to `GameState`, not
 * for debugging, not for a "stronger" opponent, not behind a flag.**
 *
 * The reply is a separate one-way message ({@link AIDecision}), never `wait: true`.
 * A blocking send would park the table's run loop on the very call that a watchdog
 * would have had to rescue.
 */
export interface AIDecideRequest extends InternalEnvelope {
	/**
	 * The seat this request is *for*. The receiving actor compares it against its
	 * own `c.state.seat` and drops the message on mismatch — it is a tripwire, not
	 * an instruction.
	 */
	readonly seat: Seat;
	/** The server turn nonce. Echoed back so a stale reply can be discarded. */
	readonly turnId: string;
	readonly kind: AIDecisionKind;
	/**
	 * `project(state, seat)`. The redaction boundary, and the only state the model
	 * ever sees. Also the sole input to the seat's card-counting memory — memory is
	 * absorbed from the public view, never from privileged state.
	 */
	readonly view: PublicGameView;
	/**
	 * The complete legal set, in engine order and already computed by the table.
	 *
	 * `legal.map((m) => m.id)` is exactly the `z.enum` the model's output schema is
	 * built from, rebuilt per decision — which is what makes an illegal AI move
	 * unrepresentable rather than merely rejected. The table independently
	 * re-derives `legalMoves()` before applying anything anyway, so a version skew
	 * degrades to a logged fallback rather than an illegal move.
	 */
	readonly legal: readonly LegalMove[];
	/**
	 * The heuristic ranking over the same set, used to narrow candidates by
	 * difficulty and as the guaranteed fallback. Optional so the AI seat can rank
	 * for itself; when present it must cover `legal` and nothing outside it.
	 */
	readonly ranking?: readonly RankedMove[];
	/**
	 * Server-clock deadline. The seat must have replied by then; past it the table's
	 * watchdog fires and auto-plays the top-ranked move regardless. The seat clamps
	 * its own model timeouts against this so the cap holds by construction rather
	 * than by arithmetic luck.
	 */
	readonly deadlineAt: number;
}

/**
 * **REPLY: `aiSeat` → `euchreTable`, queue `aiDecision`.**
 *
 * One chosen move id and the metadata the table needs to journal, pace and narrate
 * it. It is a *proposal*: before this touches state the table checks the token,
 * checks `turnId`, takes the acting seat from `state.hand.turnSeat` (never from
 * {@link seat} here), and re-derives `legalMoves()` to confirm membership.
 *
 * The decision is then parked behind the persisted pacing floor if it arrived early
 * — a 380 ms answer reads as a robot — and released by a schedule, so a crash
 * during the pacing window still delivers the move on wake.
 */
export interface AIDecision extends InternalEnvelope {
	/** Which seat believes it is answering. Compared against `hand.turnSeat`. */
	readonly seat: Seat;
	/** Must equal the request's `turnId`; a mismatch is a stale reply and is dropped. */
	readonly turnId: string;
	/** Must be a member of the request's `legal` ids, re-verified by the table. */
	readonly moveId: LegalMoveId;
	/**
	 * How the choice was reached: `llm` (the model picked it), `fallback` (the model
	 * failed, was illegal, or the budget is degraded, so the heuristic picked it), or
	 * `forced` (only one legal move existed — no API call was made).
	 */
	readonly source: AIDecisionSource;
	/**
	 * Optional persona line for the chat channel. Already screened and clamped to
	 * {@link BANTER_MAX_CHARS}. Generated from public state only, so it cannot name
	 * a card the speaker should not know.
	 */
	readonly banter?: string;
	/**
	 * Optional short justification, journalled and shown in the post-game "why did it
	 * do that" panel. It reaches a human, so it passes the same output screen and is
	 * clamped to {@link RATIONALE_MAX_CHARS}; on rejection it falls back to the
	 * engine's own {@link RankedMove.why}.
	 */
	readonly rationale?: string;
	/** Wall time from receiving the request to producing this reply, for metering. */
	readonly latencyMs: number;
}

/** The non-decision messages the table sends an AI seat. */
export type AiSeatLifecycleKind = 'resetHand' | 'handEnd' | 'gameEnd';

/**
 * **`euchreTable` → `aiSeat`, queues `resetHand` / `handEnd` / `gameEnd`.**
 *
 * `resetHand` clears the seat's per-hand card-counting memory at every deal;
 * `handEnd` flushes its episode buffer to `playerProfile`; `gameEnd` flushes the
 * meter and breaks the workflow loop. Each is its own queue; this is the body they
 * share.
 */
export interface AiSeatLifecycleMessage extends InternalEnvelope {
	readonly kind: AiSeatLifecycleKind;
	/** The hand this message refers to. Absent for `gameEnd`. */
	readonly handNo?: number;
}

/* ========================================================================== */
/* 5. Personas — user-authored, therefore untrusted                           */
/* ========================================================================== */

/** The two Anthropic models this application uses. */
export type AnthropicModelId = 'claude-haiku-4-5' | 'claude-opus-5';

/** Fast tier: card plays and banter. */
export const MODEL_PLAY: AnthropicModelId = 'claude-haiku-4-5';

/** Deliberate tier: the cut, both bidding rounds, the dealer's discard, going alone. */
export const MODEL_BID: AnthropicModelId = 'claude-opus-5';

/**
 * Models that **reject `temperature`, `top_p` and `top_k` with HTTP 400**.
 *
 * The whole Opus-5 family is a no-sampling family: a request carrying any sampling
 * parameter fails outright. The call-site helper that builds model params branches
 * on membership of this set, and a test asserts no sampling parameter is ever sent
 * to a member.
 */
export const NO_SAMPLING_MODELS = ['claude-opus-5'] as const satisfies readonly AnthropicModelId[];

/**
 * How wide a candidate set the model gets to choose from.
 *
 * Difficulty narrows *candidates*, never legality, latency or token count — all
 * three settings issue exactly one API call with the same prompt size:
 * - `expert` — the full legal set.
 * - `casual` — the heuristic's top three.
 * - `rookie` — the top three after seeded noise, so a beginner opponent is still
 *   reproducible in replay.
 */
export type Difficulty = 'rookie' | 'casual' | 'expert';

/** Every difficulty, for settings UI and exhaustiveness checks. */
export const DIFFICULTIES = ['rookie', 'casual', 'expert'] as const satisfies readonly Difficulty[];

/** Maximum characters in a persona's display name. */
export const PERSONA_NAME_MAX_CHARS = 32;

/** Maximum characters in the one-line character blurb shown in the HUD. */
export const PERSONA_BLURB_MAX_CHARS = 120;

/**
 * Maximum characters of user-authored persona prompt.
 *
 * Enforced in the SvelteKit form action **and again** when the prompt is fenced
 * into a system message, because the form action is not the trust boundary.
 */
export const PERSONA_PROMPT_MAX_CHARS = 2000;

/** Maximum characters of the shared house-dynamics prompt applied to every seat. */
export const HOUSE_PROMPT_MAX_CHARS = 1000;

/**
 * Maximum characters of rendered cross-game dossier (≈400 tokens).
 *
 * The dossier is LLM-authored text from earlier matches that re-enters a prompt, so
 * it is treated as semi-trusted: fenced like the user prompt, and clamped here.
 */
export const PERSONA_DOSSIER_MAX_CHARS = 1600;

/** Hex characters in the per-game fence nonce that delimits untrusted blocks. */
export const PERSONA_FENCE_NONCE_CHARS = 12;

/** Maximum characters of a banter line after screening. */
export const BANTER_MAX_CHARS = 140;

/** Maximum characters of a journalled decision rationale. */
export const RATIONALE_MAX_CHARS = 120;

/** Maximum characters of a stored episode summary. */
export const EPISODE_SUMMARY_MAX_CHARS = 140;

/** Lower bound of every persona dial (`aggression`, `risk`, `chattiness`, `temperature`). */
export const DIAL_MIN = 0;

/** Upper bound of every persona dial. */
export const DIAL_MAX = 1;

/**
 * The sampling temperature actually sent to a sampling-capable model is clamped
 * into `[0.2, 0.8]`, whatever the persona dial says: 0 makes a persona lifeless and
 * 1 makes it incoherent, and neither is a setting worth exposing the consequences
 * of. Members of {@link NO_SAMPLING_MODELS} receive no temperature at all.
 */
export const LLM_TEMPERATURE_MIN = 0.2;

/** @see LLM_TEMPERATURE_MIN */
export const LLM_TEMPERATURE_MAX = 0.8;

/**
 * A user-editable opponent.
 *
 * **{@link prompt} and {@link housePrompt} are untrusted input.** They are free
 * text, written by a human, that reaches a model choosing moves in an authoritative
 * game. They may shape tone, risk appetite and which of the legal moves is
 * preferred. They may not change the rules of euchre, the output format, what the
 * seat can see, or which moves are legal — and by construction they cannot, because
 * the output schema is a `z.enum` of the table-supplied legal ids and the table
 * re-derives legality before applying anything.
 *
 * The defences are layered, and each field's cap above is only the fourth of them:
 * structural output typing, independent re-derivation, information starvation
 * (the seat holds only a {@link PublicGameView}, and banter is generated from an
 * even narrower public-only slice), nonce-fenced delimiting with control-character
 * stripping and hard length clamps, and output screening of every generated line.
 *
 * A persona is **snapshotted into the match at creation and never re-read
 * mid-game**, so editing it in Settings affects the next match, not the live one.
 */
export interface PersonaConfig {
	/** Stable id: the preset key or a user-generated id. Also the episode `role`. */
	readonly id: string;
	/** Increments on every save; the match records which version it played against. */
	readonly version: number;
	/** ≤ {@link PERSONA_NAME_MAX_CHARS}. Shown at the seat and spoken in announcements. */
	readonly name: string;
	/** ≤ {@link PERSONA_BLURB_MAX_CHARS}. One line of character, shown in the HUD. */
	readonly blurb: string;
	/** **UNTRUSTED.** ≤ {@link PERSONA_PROMPT_MAX_CHARS}. Style only, never legality. */
	readonly prompt: string;
	/** **UNTRUSTED.** ≤ {@link HOUSE_PROMPT_MAX_CHARS}. Shared table dynamics. */
	readonly housePrompt: string;
	readonly difficulty: Difficulty;
	/** `0`–`1`. How readily this seat bids and reaches for a march. */
	readonly aggression: number;
	/** `0`–`1`. Tolerance for being euchred in exchange for upside. */
	readonly risk: number;
	/**
	 * `0`–`1`. A banter line is attempted only when a moment's salience clears
	 * `1 − chattiness`, so `0` is a silent seat and `1` narrates everything.
	 */
	readonly chattiness: number;
	/** `0`–`1` as authored; clamped to `[0.2, 0.8]` at the call and dropped entirely for no-sampling models. */
	readonly temperature: number;
	/** Fast tier, used for card plays and banter. */
	readonly modelId: AnthropicModelId;
	/** Deliberate tier, used for the cut, bidding, the discard and going alone. */
	readonly bidModelId: AnthropicModelId;
}

/**
 * One persona, bound to one chair, with its cross-game memory rendered — the exact
 * object snapshotted at match creation.
 *
 * Three of these are written to the table's `match_meta` row in `onCreate` and are
 * the create input for the three `aiSeat` actors. Storing them lets a cold-path
 * dispatch rebuild a seat that was destroyed or never created, without putting
 * personas in the whole-loaded `c.state` and without re-reading (and so changing)
 * a persona mid-match.
 */
export interface PersonaAssignment {
	/** `1`, `2` or `3` — never `0`, which is the human. */
	readonly seat: Seat;
	readonly persona: PersonaConfig;
	/**
	 * Rendered cross-game episodes, ≤ {@link PERSONA_DOSSIER_MAX_CHARS}. Semi-trusted
	 * LLM-authored text; fenced exactly like the user prompt.
	 */
	readonly dossier: string;
}

/**
 * The client-safe face of a persona, returned by `aiSeat.getPersona()`.
 *
 * Deliberately omits the model ids and the raw prompts: the browser renders the
 * seat, and Settings — which is where a prompt is legitimately shown — reads it
 * from `playerProfile`, not from a live match.
 */
export interface PersonaView {
	readonly id: string;
	readonly version: number;
	readonly name: string;
	readonly blurb: string;
	readonly seat: Seat;
	readonly difficulty: Difficulty;
	readonly chattiness: number;
	/** `true` once the token circuit breaker has tripped: "playing on instinct". */
	readonly degraded: boolean;
}

/* ========================================================================== */
/* 6. Stored history — playerProfile                                          */
/* ========================================================================== */

/**
 * How a match ended, from the human's point of view. `abandoned` is a real and
 * common outcome: hands are journalled at every hand boundary, not only at game
 * over, so an interrupted match still has history worth showing.
 */
export type MatchOutcome = 'won' | 'lost' | 'abandoned';

/** Per-match counters, rolled up nightly into {@link LifetimeStats}. */
export interface MatchStats {
	/** Hands where the human's team euchred the makers. */
	readonly euchresFor: number;
	/** Hands where the human's team was euchred. */
	readonly euchresAgainst: number;
	readonly lonersAttempted: number;
	readonly lonersMade: number;
	readonly marches: number;
	readonly throwIns: number;
	/** Tricks taken by each team across the whole match. */
	readonly tricks: readonly [number, number];
}

/** Model spend for one match, accumulated from the three seats' meters. */
export interface TokenUsage {
	readonly tokensIn: number;
	readonly tokensOut: number;
	readonly cacheReadTokens: number;
	readonly calls: number;
}

/**
 * **The stored game-history record**, one row per match in `playerProfile`'s
 * `c.db`, written by the `recordMatch` queue at game over (and retried by the
 * `flushProfile` schedule).
 *
 * **Server-only.** {@link seed} and the per-hand `deckOrder` are what make every
 * hidden card recoverable — that is exactly what makes a replay possible, and
 * exactly why neither may reach a browser. `getReplay` re-derives frames
 * *server-side* through the engine and projects every frame before it is sent; the
 * client-facing row is {@link MatchSummary}.
 *
 * A replay costs ≈3 KB per game because it is `{ seed, moves[] }`, not frames.
 */
export interface MatchRecord {
	readonly schema: 1;
	readonly matchId: string;
	/** Owner. Also `playerProfile`'s key, asserted against the connection's claims. */
	readonly userId: string;
	/** **SERVER-ONLY.** Reproduces every shuffle. Never leaves the actor. */
	readonly seed: string;
	readonly cfg: EngineConfig;
	readonly firstDealer: Seat;
	readonly startedAt: number;
	/** `null` while the match is still live. */
	readonly endedAt: number | null;
	readonly status: GameStatus;
	readonly outcome: MatchOutcome;
	readonly score: readonly [number, number];
	readonly winnerTeam: Team | null;
	readonly handsPlayed: number;
	/** The three opponents exactly as played, versioned. */
	readonly personas: readonly PersonaAssignment[];
	readonly stats: MatchStats;
	readonly tokens: TokenUsage;
}

/**
 * One journalled hand, written by `recordHand` at every hand boundary.
 *
 * **Server-only**, for the same reason as {@link MatchRecord}: `seed` plus
 * `deckOrder` plus `moves` is the complete hand, hidden cards included. Replaying
 * `moves` through `apply()` from a state built on `seed` reproduces the hand
 * exactly, which is what both the replay viewer and the fuzz gate rely on.
 */
export interface HandJournalEntry {
	readonly matchId: string;
	/** 0-based within the match. */
	readonly handNo: number;
	/** **SERVER-ONLY.** */
	readonly seed: string;
	readonly dealerSeat: Seat;
	/** **SERVER-ONLY.** The 24-card permutation; identifies every hidden card. */
	readonly deckOrder: readonly CardId[];
	/** Every attributed action of the hand, in order, replayable through `apply()`. */
	readonly moves: readonly PlayerAction[];
	readonly trump: Suit | null;
	readonly makerSeat: Seat | null;
	readonly aloneSeat: Seat | null;
	readonly tricksWon: readonly [number, number];
	/** `null` only if the hand was abandoned before it scored. */
	readonly result: HandResult | null;
	readonly delta: readonly [number, number] | null;
	readonly endedAt: number;
}

/**
 * The client-facing history row, returned by `listMatches(cursor)` and rendered by
 * `/games`.
 *
 * It has **no seed and no deck order** — structurally, not by omission at
 * serialization time. Adding either to this type would put the whole deck of every
 * past hand in a browser.
 */
export interface MatchSummary {
	readonly matchId: string;
	readonly playedAt: number;
	readonly status: GameStatus;
	readonly outcome: MatchOutcome;
	readonly score: readonly [number, number];
	readonly handsPlayed: number;
	/** Opponent display names, in seat order 1, 2, 3. */
	readonly opponents: readonly string[];
}

/** One page of history. `cursor` is `null` when there is nothing more to load. */
export interface MatchListPage {
	readonly rows: readonly MatchSummary[];
	readonly cursor: string | null;
}

/** Rolled up nightly by the `rollup` cron; drives the `/games` header stats. */
export interface LifetimeStats {
	readonly matches: number;
	readonly won: number;
	readonly lost: number;
	readonly abandoned: number;
	readonly euchresFor: number;
	readonly euchresAgainst: number;
	readonly lonersMade: number;
	readonly handsPlayed: number;
	/** Server clock of the last rollup. */
	readonly updatedAt: number;
}

/**
 * `playerProfile`'s whole `c.state` — deliberately tiny, because `c.state` is
 * deserialized in full on every wake and every serverless migration. Everything
 * paginated or queried lives in `c.db`.
 */
export interface PlayerProfileState {
	readonly schema: 1;
	/** How `/play` resumes an interrupted match. `null` when there is none. */
	readonly activeGameId: string | null;
	/** Today's model spend, for the per-user budget. */
	readonly dailyTokens: TokenUsage;
	readonly lifetime: LifetimeStats;
}

/** What an episode is *about*; the v1 set, extended only with a migration. */
export type EpisodeKind = 'bid' | 'play' | 'loner' | 'euchre' | 'read';

/**
 * One cross-game memory, flushed from an AI seat's buffer at hand end via the
 * `remember` queue and stored in `playerProfile`'s `c.db`.
 *
 * **Semi-trusted.** `summary` is LLM-authored text that later re-enters a prompt as
 * part of a dossier, so it is screened before storage, clamped to
 * {@link EPISODE_SUMMARY_MAX_CHARS}, and fenced when rendered. Selection at match
 * creation ranks by `salience × exp(−ageDays / 14)`, so a grudge fades unless it is
 * reinforced. Settings lists them and offers a per-episode `forget`.
 */
export interface EpisodeRecord {
	/** The persona this memory belongs to — {@link PersonaConfig.id}. */
	readonly role: string;
	readonly matchId: string;
	readonly handNo: number;
	readonly ts: number;
	readonly kind: EpisodeKind;
	/** ≤ {@link EPISODE_SUMMARY_MAX_CHARS}, screened. */
	readonly summary: string;
	/** `0`–`1`. How much this is worth remembering. */
	readonly salience: number;
	/** Hard expiry, `ts + 90d`. Retention is a property of the record, not a cron's opinion. */
	readonly expiresAt: number;
}

/* ========================================================================== */
/* 7. Error codes                                                             */
/* ========================================================================== */

/**
 * Actor- and transport-level failures, shared by every actor and by the client.
 *
 * These are the `code` on a RivetKit `UserError` and on a failed {@link MoveAck}.
 * They are **machine codes**: the client maps each to its own copy, and nothing
 * ever parses a message string. Rules verdicts are `RuleCode` from the engine and
 * are deliberately a separate union — a `must_follow_suit` is a fact about euchre,
 * a `forbidden` is a fact about this system.
 */
export type ProtocolErrorCode =
	/** `onBeforeConnect`: the `Origin` header is not in the allowlist. The socket terminates at Rivet Cloud, so this check is the only CORS there is. */
	| 'origin_not_allowed'
	/** The JWT is missing, malformed, expired, or fails signature/issuer/audience. The client refreshes and calls `reconnect()`. */
	| 'invalid_token'
	/** Authenticated, but not the owner of this match (or not this profile's user). */
	| 'forbidden'
	/** A payload claimed a seat that disagrees with the connection's. Logged as an attack, then dropped. */
	| 'seat_mismatch'
	/** An actor-to-actor message failed the constant-time `internalToken` compare. Logged as an attack, then dropped. */
	| 'internal_token_mismatch'
	/** No such match, or it has been reaped. */
	| 'game_not_found'
	/** The match is over or abandoned; no move will be accepted. */
	| 'game_complete'
	/** A `clientMoveId` already in the dedupe ring. Idempotent no-op, not an error to surface. */
	| 'duplicate_move'
	/** Too many submits from one connection in the window. */
	| 'rate_limited'
	/** The client's {@link PROTOCOL_VERSION} is not one this deployment speaks. */
	| 'protocol_version_mismatch'
	/** Persona text failed validation (length, or nothing left after sanitisation). */
	| 'invalid_persona'
	/** The AI seat could not be reached or did not reply; the table auto-played. Informational. */
	| 'ai_unavailable'
	/** An unexpected server fault. The only code that is not actionable by the client. */
	| 'internal_error';

/** Every {@link ProtocolErrorCode}, keyed by itself, for exhaustive mapping to copy. */
export const PROTOCOL_ERROR_CODES = {
	origin_not_allowed: 'origin_not_allowed',
	invalid_token: 'invalid_token',
	forbidden: 'forbidden',
	seat_mismatch: 'seat_mismatch',
	internal_token_mismatch: 'internal_token_mismatch',
	game_not_found: 'game_not_found',
	game_complete: 'game_complete',
	duplicate_move: 'duplicate_move',
	rate_limited: 'rate_limited',
	protocol_version_mismatch: 'protocol_version_mismatch',
	invalid_persona: 'invalid_persona',
	ai_unavailable: 'ai_unavailable',
	internal_error: 'internal_error'
} as const satisfies Record<ProtocolErrorCode, ProtocolErrorCode>;

/** Any error code this system produces: a rules verdict or a protocol failure. */
export type ErrorCode = RuleCode | ProtocolErrorCode;

/**
 * Connection-guard codes raised by `@rivetkit/svelte` before a call reaches an
 * actor at all. Re-declared here so the client can branch on them without parsing
 * message strings: `ACTOR_NOT_YET_CONNECTED` is a retryable startup race,
 * `ACTOR_DISCONNECTED` is a real drop that should surface a reconnect banner.
 */
export type ConnectionErrorCode = 'ACTOR_NOT_YET_CONNECTED' | 'ACTOR_DISCONNECTED';

/**
 * The serialized shape of a failure that crosses the wire.
 *
 * `message` is for logs and developers. The client renders copy chosen by `code`,
 * because a message may be an unlocalised server string and must never be shown
 * verbatim.
 */
export interface ProtocolError {
	readonly code: ErrorCode;
	readonly message: string;
	/** `true` when the same call may succeed unchanged on a retry. */
	readonly retryable?: boolean;
}
