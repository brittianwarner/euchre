/**
 * `playerProfile` — local types.
 *
 * Everything **wire-visible** that already exists in `$lib/protocol` is imported
 * from there and never redeclared. What lives here is the handful of shapes the
 * protocol deliberately leaves to this actor: the mutable `c.state` twin of
 * {@link PlayerProfileState}, the settings object (which the protocol does not
 * model because it is this actor's private business), the envelopes that carry
 * an `internalToken` on actor-to-actor calls, and the **client-safe** replay
 * shapes that exist so that `MatchRecord` / `HandJournalEntry` — both of which
 * carry `seed` and `deckOrder` — can never be returned to a browser by accident.
 *
 * Two conventions used throughout:
 *
 * 1. Protocol types are `readonly` everywhere. `c.state` must be writable, so the
 *    state-resident twins here are mutable. A mutable object is assignable to its
 *    readonly counterpart, so actions can still declare protocol return types;
 *    {@link _AssertStateMatchesProtocol} pins that down at compile time.
 * 2. Anything whose name starts with `Client` is a shape that may cross to a
 *    browser. Anything that embeds `seed` or `deckOrder` structurally cannot be
 *    one, which is why the client replay types below are separate declarations
 *    rather than `Omit<>`s — an `Omit` silently re-admits a field when the source
 *    type gains one.
 */

import type {
	CardId,
	EngineConfig,
	EpisodeKind,
	GameStatus,
	HandResult,
	MatchOutcome,
	MatchStats,
	PersonaConfig,
	PlayerAction,
	PlayerProfileState,
	Seat,
	Suit,
	Team,
	TokenUsage
} from '$lib/protocol';

/* ========================================================================== */
/* Seats                                                                      */
/* ========================================================================== */

/** The three chairs a persona may occupy. Seat 0 is the human and is never one. */
export type AiSeat = 1 | 2 | 3;

/** The AI seats in ascending order. `personas[i]` in settings is `AI_SEATS[i]`. */
export const AI_SEATS = [1, 2, 3] as const satisfies readonly AiSeat[];

/* ========================================================================== */
/* Connection                                                                 */
/* ========================================================================== */

/**
 * **UNTRUSTED.** What a browser supplies when it opens a connection to this
 * actor. Only {@link token} carries any authority, and only after verification.
 *
 * There is deliberately no `userId` field: the owning user is the actor's own key
 * (`['user', userId]`) and the JWT `sub`, never something a caller states.
 */
export interface ProfileConnectParams {
	readonly token: string;
	readonly protocolVersion?: number;
}

/**
 * **SERVER-DERIVED. TRUSTED.** `c.conn.state`.
 *
 * `authenticated: false` is not a failure mode — it is what an internal,
 * connection-less call (actor-to-actor, or a stateless HTTP action invocation)
 * looks like. Such a connection can read and write nothing; every internal action
 * demands a valid `internalToken` instead. A *bad* token, by contrast, never gets
 * this far: `createConnState` throws.
 */
export type ProfileConnState =
	| { readonly authenticated: true; readonly userId: string; readonly since: number }
	| { readonly authenticated: false; readonly userId: null; readonly since: number };

/* ========================================================================== */
/* Settings — small, bounded, therefore `c.state`                             */
/* ========================================================================== */

/**
 * Table-level preferences. Small, bounded, enumerated — no free text.
 */
export interface TablePrefs {
	/** How long the table lingers between beats. Presentation only. */
	pace: 'brisk' | 'normal' | 'relaxed';
	/** Whether AI seats are allowed to chatter at all, overriding `chattiness`. */
	banter: boolean;
	/** Show each AI seat's one-line rationale in the post-game panel. */
	showRationale: boolean;
	sound: boolean;
	reduceMotion: boolean;
}

/**
 * The user-authored half of a profile: three personas and the shared house
 * prompt.
 *
 * **`housePrompt` and every `personas[i].prompt` are untrusted free text.** They
 * are stored verbatim after sanitisation and length-clamping and are *never*
 * interpolated into SQL, into a template, into a log line, or into anything
 * evaluated. This actor only stores and returns them; the fencing that lets them
 * reach a model safely is `aiSeat`'s job.
 *
 * Mutable because it lives in `c.state`. `personas` is exactly three entries,
 * index `i` binding {@link AI_SEATS}`[i]`.
 */
export interface ProfileSettings {
	schema: 1;
	/** Bumped by the server on every accepted save. Never supplied by a caller. */
	version: number;
	updatedAt: number;
	/** ≤ `HOUSE_PROMPT_MAX_CHARS`. Untrusted. */
	housePrompt: string;
	personas: PersonaConfig[];
	table: TablePrefs;
}

/** A single persona edit. Every field is optional; every present field is validated. */
export interface PersonaPatch {
	readonly name?: string;
	readonly blurb?: string;
	readonly prompt?: string;
	readonly difficulty?: string;
	readonly aggression?: number;
	readonly risk?: number;
	readonly chattiness?: number;
	readonly temperature?: number;
	readonly modelId?: string;
	readonly bidModelId?: string;
}

/**
 * The whole of an `updateSettings` call. Absent keys are left untouched; a
 * present key is validated, clamped and written.
 *
 * Note what cannot be patched: `schema`, `version`, `updatedAt`, and a persona's
 * `id`. Those are server-owned.
 */
export interface SettingsPatch {
	readonly housePrompt?: string;
	readonly table?: Partial<TablePrefs>;
	/** At most one entry per AI seat. */
	readonly personas?: readonly { readonly seat: number; readonly patch: PersonaPatch }[];
}

/* ========================================================================== */
/* Actor state                                                                */
/* ========================================================================== */

/** {@link TokenUsage}, mutable, plus the UTC day the bucket belongs to. */
export interface DailyTokenUsage {
	tokensIn: number;
	tokensOut: number;
	cacheReadTokens: number;
	calls: number;
	/** `YYYY-MM-DD` in UTC. When the day rolls over the bucket resets. */
	day: string;
}

/** {@link LifetimeStats}, mutable. Recomputed by SQL — never incremented. */
export interface LifetimeStatsCache {
	matches: number;
	won: number;
	lost: number;
	abandoned: number;
	euchresFor: number;
	euchresAgainst: number;
	lonersMade: number;
	handsPlayed: number;
	updatedAt: number;
}

/**
 * The actor's entire `c.state`. Bounded by construction: three personas, one
 * house prompt, five preference flags, one token bucket, one stats cache.
 *
 * `lifetime` is a **cache of a SQL aggregate**, not a counter. It is only ever
 * written by assigning the full result of {@link computeLifetime}; there is no
 * `+= 1` anywhere in this module. It exists because
 * `PlayerProfileState.lifetime` is part of the shared contract and because the
 * `/games` header wants it without a query.
 */
export interface ProfileActorState {
	schema: 1;
	activeGameId: string | null;
	dailyTokens: DailyTokenUsage;
	lifetime: LifetimeStatsCache;
	settings: ProfileSettings;
}

/**
 * Compile-time proof that the mutable state above still satisfies the shared
 * contract's `PlayerProfileState`. If someone renames a field, this breaks here
 * rather than at the `/games` page.
 */
export type _AssertStateMatchesProtocol =
	ProfileActorState extends PlayerProfileState ? true : never;
const _assertStateMatchesProtocol: _AssertStateMatchesProtocol = true;
void _assertStateMatchesProtocol;

/* ========================================================================== */
/* Internal (actor-to-actor) envelopes                                        */
/* ========================================================================== */

/**
 * The authorisation envelope on every internal call.
 *
 * This is *not* `InternalEnvelope` from `$lib/protocol`: that one's token is the
 * per-match secret `euchreTable` mints in its own `createState` and shares with
 * its three `aiSeat`s. `playerProfile` outlives any match and has no way to know
 * a per-match secret, so the token compared here is the deployment-wide server
 * secret (`EUCHRE_INTERNAL_TOKEN`). Same discipline — constant-time compare,
 * before any state read, never logged — different key.
 */
export interface ProfileInternalEnvelope {
	readonly internalToken: string;
}

/** `euchreTable` → `playerProfile`: record (or advance) a match row. */
export interface RecordGameMessage extends ProfileInternalEnvelope {
	/** A `MatchRecord`, validated on arrival. Its `userId` must equal the key's. */
	readonly record: unknown;
	/** Optional `HandJournalEntry[]` recorded in the same call. */
	readonly hands?: unknown;
}

/** `euchreTable` → `playerProfile`: journal hands at a hand boundary. */
export interface RecordHandsMessage extends ProfileInternalEnvelope {
	readonly matchId: string;
	readonly hands: unknown;
}

/** `aiSeat` → `playerProfile`: flush an episode buffer at hand end. */
export interface RecordEpisodesMessage extends ProfileInternalEnvelope {
	readonly episodes: unknown;
}

/** `euchreTable` → `playerProfile`: which match `/play` should resume. */
export interface SetActiveGameMessage extends ProfileInternalEnvelope {
	readonly gameId: string | null;
}

/** Any seat's meter → `playerProfile`: add spend to today's bucket. */
export interface RecordTokensMessage extends ProfileInternalEnvelope {
	readonly tokensIn: number;
	readonly tokensOut: number;
	readonly cacheReadTokens: number;
	readonly calls: number;
}

/* ========================================================================== */
/* Replay — the redaction boundary of this actor                              */
/* ========================================================================== */

/**
 * One hand of a replay, **safe for a browser**.
 *
 * Compare `HandJournalEntry`: no `seed`, and no `deckOrder`. Those two fields are
 * what make every hidden card recoverable, which is the whole point of the stored
 * record and exactly why this type cannot contain them.
 *
 * `moves` is filtered, not copied: a `discard` by an AI dealer is removed,
 * because the buried card was never public and the hand being over does not make
 * it so. Seat 0's own discard is kept — the human chose it.
 */
export interface ClientHandReplay {
	readonly handNo: number;
	readonly dealerSeat: Seat;
	readonly trump: Suit | null;
	readonly makerSeat: Seat | null;
	readonly aloneSeat: Seat | null;
	readonly tricksWon: readonly [number, number];
	readonly result: HandResult | null;
	readonly delta: readonly [number, number] | null;
	readonly endedAt: number;
	readonly moves: readonly PlayerAction[];
}

/** A whole replay, **safe for a browser**. */
export interface ClientMatchReplay {
	readonly matchId: string;
	readonly startedAt: number;
	readonly endedAt: number | null;
	readonly playedAt: number;
	readonly status: GameStatus;
	readonly outcome: MatchOutcome;
	readonly score: readonly [number, number];
	readonly winnerTeam: Team | null;
	readonly handsPlayed: number;
	readonly opponents: readonly string[];
	readonly stats: MatchStats;
	readonly hands: readonly ClientHandReplay[];
}

/**
 * The full, unredacted replay: `MatchRecord` plus every `HandJournalEntry`,
 * `seed` and `deckOrder` included.
 *
 * Reachable **only** through `getReplay`, which demands a valid internal token.
 * Its consumer is server-side: rebuild the match through `createGame` + `apply`
 * and `project()` each frame before anything is sent anywhere.
 */
export interface ServerMatchReplay {
	readonly match: import('$lib/protocol').MatchRecord;
	readonly hands: readonly import('$lib/protocol').HandJournalEntry[];
}

/* ========================================================================== */
/* Stats                                                                      */
/* ========================================================================== */

/**
 * Everything the `/games` header shows, computed by a single SQL aggregate over
 * the `matches` table at read time.
 *
 * There is no incremental counter anywhere in this actor, so there is nothing to
 * drift: delete a row, re-record a match, restore from a backup — the numbers
 * follow the table.
 */
export interface ProfileStats {
	readonly matches: number;
	readonly won: number;
	readonly lost: number;
	readonly abandoned: number;
	/** `won / (won + lost)`, `0` when no match has been decided. Abandons excluded. */
	readonly winRate: number;
	readonly euchresFor: number;
	readonly euchresAgainst: number;
	readonly lonersAttempted: number;
	readonly lonersMade: number;
	readonly marches: number;
	readonly throwIns: number;
	readonly handsPlayed: number;
	readonly tricks: readonly [number, number];
	readonly tokens: TokenUsage;
	readonly updatedAt: number;
}

/* ========================================================================== */
/* Episodes                                                                   */
/* ========================================================================== */

/** An {@link EpisodeRecord} as returned to Settings, with its stable row id. */
export interface StoredEpisode {
	readonly episodeId: string;
	readonly role: string;
	readonly matchId: string;
	readonly handNo: number;
	readonly ts: number;
	readonly kind: EpisodeKind;
	readonly summary: string;
	readonly salience: number;
	readonly expiresAt: number;
}

/* ========================================================================== */
/* Miscellaneous request shapes                                               */
/* ========================================================================== */

/** Arguments to `listGames`. Both fields are attacker-supplied and validated. */
export interface ListGamesRequest {
	readonly limit?: number;
	readonly cursor?: string | null;
}

/** A row of `hands` as it is written; the JSON columns stay opaque here. */
export interface StoredHandRow {
	readonly matchId: string;
	readonly handNo: number;
	readonly seed: string;
	readonly dealerSeat: Seat;
	readonly deckOrder: readonly CardId[];
	readonly moves: readonly PlayerAction[];
	readonly trump: Suit | null;
	readonly makerSeat: Seat | null;
	readonly aloneSeat: Seat | null;
	readonly tricksWon: readonly [number, number];
	readonly result: HandResult | null;
	readonly delta: readonly [number, number] | null;
	readonly endedAt: number;
}

/** `cfg` is round-tripped as an opaque JSON blob; this names it for readers. */
export type StoredEngineConfig = EngineConfig;
