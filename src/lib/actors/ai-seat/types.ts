/**
 * `aiSeat` — internal types, caps and tunables.
 *
 * **One actor per AI opponent.** Key `["table", gameId, "seat", "1" | "2" | "3"]`,
 * so each of the three chairs is an isolated actor instance with its own persona,
 * its own memory, its own token budget, its own circuit breaker and its own
 * failure domain. A seat that wedges takes exactly one chair down with it, and the
 * table's watchdog auto-plays for it.
 *
 * Two structural properties are load-bearing and are asserted by the types here
 * rather than by review:
 *
 * 1. **This actor cannot see the game.** Nothing in this file — and nothing in
 *    `src/lib/actors/ai-seat/**` — imports `GameState`. The only state that ever
 *    reaches an AI seat is the `PublicGameView` inside an `AIDecideRequest`, which
 *    is exactly what a human sitting in that chair would be shown. Cheating is a
 *    compile error, not a policy.
 * 2. **Everything durable is bounded.** `c.state` is deserialized whole on every
 *    wake and every serverless migration, so every collection below carries an
 *    explicit cap, named as a constant and enforced in one place.
 *
 * Normative source: `docs/03-AI-AGENTS.md` §1, §10.
 */

import type {
	AIDecisionKind,
	AIDecisionSource,
	CardId,
	EpisodeKind,
	LegalMoveId,
	PersonaConfig,
	Seat,
	Suit,
	Team
} from '$lib/protocol';

/* ========================================================================== */
/* Caps — every durable collection has one                                     */
/* ========================================================================== */

/** Trump cards remembered per hand. There are only seven, so this is complete. */
export const MEM_TRUMP_SEEN_CAP = 7;

/** Suits remembered per seat as publicly shown voids. There are only three off-suits. */
export const MEM_VOID_SUITS_CAP = 3;

/** Leads remembered per hand. A hand has five tricks. */
export const MEM_LEAD_CAP = 5;

/** Auction lines remembered per hand. Round 1 plus round 2 is at most eight. */
export const MEM_BID_CAP = 8;

/**
 * **Cross-hand memory cap.** Eight notes about the human, each clamped to
 * {@link NOTE_MAX_CHARS}, so this whole store is bounded at ~1.1 KiB and never
 * grows with the length of a match. Oldest notes are evicted first; a note is
 * never appended twice for the same hand.
 */
export const MEM_NOTE_CAP = 8;

/** Maximum characters of one cross-hand note. Mirrors `EPISODE_SUMMARY_MAX_CHARS`. */
export const NOTE_MAX_CHARS = 140;

/** Episodes buffered between hand boundaries before they are flushed. */
export const EPISODE_BUFFER_CAP = 12;

/** Latency samples kept for the p95 estimate. */
export const LATENCY_SAMPLE_CAP = 32;

/** Decisions kept for `getDecisionLog()`. Debug surface only. */
export const DECISION_LOG_CAP = 24;

/**
 * Per-match, per-seat token ceiling. On exceeding it the seat sets
 * `budget.degraded` and plays heuristic-only for the rest of the match behind the
 * "playing on instinct" HUD chip.
 *
 * **Provisional.** Invariant 22 applies: this number is not frozen until the
 * metering harness reports measured spend over a real match.
 */
export const BUDGET_INPUT_TOKENS = 250_000;

/** @see BUDGET_INPUT_TOKENS */
export const BUDGET_OUTPUT_TOKENS = 20_000;

/** @see BUDGET_INPUT_TOKENS */
export const BUDGET_CALLS = 300;

/* ========================================================================== */
/* Timing — every value is clamped against the request's own deadline          */
/* ========================================================================== */

/**
 * Wall-clock budget for the first model attempt. The real bound is always
 * `min(this, deadlineAt - now - RESERVE_MS)`, so the table's hard cap holds by
 * construction rather than by arithmetic luck.
 *
 * **Provisional** (invariant 22).
 */
export const LLM_FIRST_ATTEMPT_MS = 2200;

/** Wall-clock budget for the single escalation attempt. Bid paths only. */
export const LLM_ESCALATION_MS = 1400;

/** Never start an attempt with less than this much budget left. */
export const LLM_MIN_ATTEMPT_MS = 400;

/** Time reserved before `deadlineAt` for replying, metering and transport. */
export const DEADLINE_RESERVE_MS = 300;

/** Wall-clock budget for a banter line, which is optional and never blocks. */
export const BANTER_BUDGET_MS = 1800;

/* ========================================================================== */
/* Banter rate limiting                                                        */
/* ========================================================================== */

/** At most one banter line per trick from this seat. */
export const BANTER_MAX_PER_TRICK = 1;

/** At most this many banter lines from this seat per hand. */
export const BANTER_MAX_PER_HAND = 4;

/** Minimum milliseconds between two banter lines from this seat. */
export const BANTER_COOLDOWN_MS = 2500;

/* ========================================================================== */
/* Create input                                                                */
/* ========================================================================== */

/**
 * What the table hands an AI seat at `getOrCreate(..., { createWithInput })`.
 *
 * The persona is **snapshotted here and never re-read**: editing it in Settings
 * affects the next match, not the live one, and the cached prompt prefix stays
 * stable for the whole match.
 */
export interface AiSeatCreateInput {
	readonly gameId: string;
	/** `1`, `2` or `3`. Must agree with the actor key, which is the real authority. */
	readonly seat: Seat;
	/** The table's `internalToken`. The actor-to-actor authorization boundary. */
	readonly internalToken: string;
	/** Per-game nonce that fences untrusted persona text. 12 hex chars. */
	readonly fenceNonce: string;
	readonly persona: PersonaConfig;
	/** Rendered cross-game episodes. Semi-trusted; fenced exactly like the prompt. */
	readonly dossier?: string;
	/** `playerProfile`'s key, when episode flushing is wired. Optional. */
	readonly profileKey?: readonly string[];
	/** The match id used when journalling episodes. Defaults to `gameId`. */
	readonly matchId?: string;
}

/* ========================================================================== */
/* Durable state                                                               */
/* ========================================================================== */

/**
 * Per-hand card-counting memory, absorbed **from the redacted view only** and
 * cleared by the `resetHand` message at every deal.
 */
export interface AiSeatHandMemory {
	handNo: number;
	/** Trump seen this hand, seeded with the up-card. Cap {@link MEM_TRUMP_SEEN_CAP}. */
	trumpSeen: CardId[];
	/** Publicly shown voids. Cap {@link MEM_VOID_SUITS_CAP} suits per seat. */
	voids: Record<Seat, Suit[]>;
	/** Cards led this hand. Cap {@link MEM_LEAD_CAP}. */
	leadHistory: CardId[];
	/** Auction lines this hand. Cap {@link MEM_BID_CAP}. */
	bidHistory: string[];
	/** Highest-information snapshot of the hand seen so far; the note source. */
	snapshot: HandSnapshot | null;
}

/**
 * The public facts about the hand in progress, refreshed on every view. Small,
 * flat and fixed-size — this is what a cross-hand note is derived from, because
 * the `handEnd` message carries only a hand number.
 */
export interface HandSnapshot {
	readonly handNo: number;
	readonly trump: Suit | null;
	readonly makerSeat: Seat | null;
	readonly aloneSeat: Seat | null;
	readonly tricksWon: readonly [number, number];
	readonly result: string | null;
}

/** One buffered cross-game memory, flushed at hand end. */
export interface AiSeatEpisode {
	readonly kind: EpisodeKind;
	/** ≤ {@link NOTE_MAX_CHARS}, screened before it is ever stored or rendered. */
	readonly summary: string;
	/** `0`–`1`. How much this is worth remembering. */
	readonly salience: number;
	readonly handNo: number;
	readonly ts: number;
}

/** Model spend for this seat in this match. */
export interface AiSeatBudget {
	tokensIn: number;
	tokensOut: number;
	cacheReadTokens: number;
	calls: number;
	/** `true` once a ceiling is crossed: heuristic-only for the rest of the match. */
	degraded: boolean;
}

/** Decision counters, surfaced by `getStatus()` and used to police the ladder. */
export interface AiSeatMeter {
	decisions: number;
	llmCalls: number;
	llmWins: number;
	fallbacks: number;
	forced: number;
	/** Times the model named an id outside the candidate set. Should be zero. */
	illegalAttempts: number;
	/** Times even the heuristic produced nothing and the first legal move was taken. */
	firstLegalRescues: number;
	/** Messages dropped because the `internalToken` did not match. Attack signal. */
	tokenRejections: number;
	/** Messages dropped because the claimed seat was not ours. Attack signal. */
	seatRejections: number;
	staleTurns: number;
	banterEmitted: number;
	banterRejected: number;
	avgLatencyMs: number;
	p95LatencyMs: number;
	/** Rolling latency samples. Cap {@link LATENCY_SAMPLE_CAP}. */
	samples: number[];
}

/** Banter rate-limiter state, so a chatty persona still cannot flood the table. */
export interface AiSeatBanterGate {
	lastHandNo: number;
	lastTrickIndex: number;
	inTrick: number;
	inHand: number;
	lastAt: number;
}

/** One entry of the debug decision log. Public-safe: no cards, no prompt text. */
export interface DecisionRecord {
	readonly turnId: string;
	readonly handNo: number;
	readonly kind: AIDecisionKind;
	readonly moveId: LegalMoveId;
	readonly source: AIDecisionSource;
	readonly latencyMs: number;
	readonly candidates: number;
	readonly rationale: string;
}

/**
 * The whole of `aiSeat`'s `c.state`.
 *
 * Nothing important lives in `c.vars`: `c.vars` is wiped on sleep, on restart, on
 * crash and on every serverless migration, and a seat that forgot its persona
 * mid-match would be a different opponent after a cold start.
 *
 * Size: bounded by the caps above plus the persona's own text (≤ 2000 + 1000
 * chars, capped at the protocol boundary) plus the dossier (≤ 1600 chars) — a few
 * KiB, all of it fixed for the life of the match.
 */
export interface AiSeatState {
	readonly schema: 1;
	/** `false` until the table has supplied create input or the first valid message. */
	provisioned: boolean;
	gameId: string;
	matchId: string;
	seat: Seat;
	teamId: Team;
	/** Constant-time compared before any other state is read. Never logged. */
	internalToken: string;
	fenceNonce: string;
	persona: PersonaConfig;
	/** Semi-trusted, snapshotted at create, stable for the whole match. */
	dossier: string;
	profileKey: string[] | null;

	memory: AiSeatHandMemory;
	/** **Cross-hand.** Survives `resetHand`. Cap {@link MEM_NOTE_CAP}. */
	notes: string[];
	/** Hands a note has already been written for, so notes are never duplicated. */
	notedHands: number[];

	budget: AiSeatBudget;
	meter: AiSeatMeter;
	banter: AiSeatBanterGate;

	/** The most recent turn this seat answered; the stale-reply tripwire. */
	lastTurnId: string | null;
	/** Cap {@link EPISODE_BUFFER_CAP}. Flushed at `handEnd`. */
	episodeBuffer: AiSeatEpisode[];
	/** Cap {@link DECISION_LOG_CAP}. */
	decisionLog: DecisionRecord[];
}

/** What `getStatus()` returns. Carries no card identity and no prompt text. */
export interface AiStatus {
	readonly seat: Seat;
	readonly gameId: string;
	readonly provisioned: boolean;
	readonly personaId: string;
	readonly personaVersion: number;
	readonly degraded: boolean;
	readonly decisions: number;
	readonly llmCalls: number;
	readonly fallbacks: number;
	readonly forced: number;
	readonly illegalAttempts: number;
	readonly avgLatencyMs: number;
	readonly p95LatencyMs: number;
	readonly tokensIn: number;
	readonly tokensOut: number;
	readonly notes: number;
}

/** The answer to the `decide` action: an acknowledgement, never a move. */
export interface DecideAck {
	readonly accepted: boolean;
	/** Present when `accepted` is `false`. A {@link import('$lib/protocol').ProtocolErrorCode}. */
	readonly code?: string;
}
