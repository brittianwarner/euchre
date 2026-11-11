/**
 * `playerProfile` shapes the browser reads and writes through `/api/profile/*`
 * and `/api/games/*`.
 *
 * The profile actor lives in the Rust server (`crates/euchre-profile`). These
 * are the client-safe halves of its contract, kept from the TypeScript actor's
 * `types.ts`: settings, the settings patch, stats, and the redacted replay.
 * Anything whose name starts with `Client` may cross to a browser; the
 * unredacted replay (with `seed` and `deckOrder`) never does, so it has no type
 * here.
 */

import type {
	GameStatus,
	HandResult,
	MatchOutcome,
	MatchStats,
	PersonaConfig,
	PlayerAction,
	Seat,
	Suit,
	Team,
	TokenUsage
} from './index.ts';

/** The three chairs a persona may occupy. Seat 0 is the human and is never one. */
export type AiSeat = 1 | 2 | 3;

/** The AI seats in ascending order. `personas[i]` in settings is `AI_SEATS[i]`. */
export const AI_SEATS = [1, 2, 3] as const satisfies readonly AiSeat[];

/** Table-level preferences. Small, bounded, enumerated — no free text. */
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
 * prompt. `housePrompt` and every `personas[i].prompt` are untrusted free text.
 * `personas` is exactly three entries, index `i` binding {@link AI_SEATS}`[i]`.
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
 * The whole of a settings save. Absent keys are left untouched; a present key is
 * validated, clamped and written by the actor. `schema`, `version`, `updatedAt`
 * and a persona's `id` are server-owned and cannot be patched.
 */
export interface SettingsPatch {
	readonly housePrompt?: string;
	readonly table?: Partial<TablePrefs>;
	/** At most one entry per AI seat. */
	readonly personas?: readonly { readonly seat: number; readonly patch: PersonaPatch }[];
}

/**
 * One hand of a replay, safe for a browser: no `seed`, no `deckOrder`, and an AI
 * dealer's buried discard removed from `moves`.
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

/** A whole replay, safe for a browser. */
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

/** Everything the `/games` header shows, computed from the `matches` table at read time. */
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
