/**
 * `playerProfile`'s SQLite schema and its migration path.
 *
 * ### Why `c.db` and not `c.state` or `c.kv`
 *
 * Match history is **unbounded** — one row per game, forever — and every read of
 * it is either a range scan (`/games`, newest first, paged) or an aggregate
 * (lifetime stats). That rules out `c.state`, which is deserialised in full on
 * every wake and every serverless migration: a player with two thousand games
 * would pay for all of them to open Settings.
 *
 * It also rules out `c.kv`. `c.kv` could store the rows, but every one of the
 * three things this actor actually does with history — "the twenty most recent,
 * newest first", "this one match with its hands", "sum euchres over everything" —
 * would become a client-side scan of the whole key space, re-implemented by hand,
 * in JavaScript, in an actor. SQLite does all three with an index and a `WHERE`
 * clause, and `c.db` is actor-local so there is no network round trip and no
 * external database (`docs/06-REVISED-ARCHITECTURE.md`: there is no Postgres in
 * this project, anywhere).
 *
 * Settings go the other way for the same reason inverted: three personas and five
 * flags are small, bounded and wanted in full every time, so they live in
 * `c.state`.
 *
 * ### Idempotence
 *
 * `onMigrate` runs on **every actor start**, not once per actor lifetime — a
 * serverless deployment wakes and sleeps this actor constantly. So migration is a
 * ledger, not a script: `_profile_migrations` records which ids have been applied,
 * and {@link applyMigrations} runs only the ones missing. Every statement is
 * additionally written `IF NOT EXISTS`, so even a ledger lost to corruption
 * replays without error. RivetKit wraps the whole hook in a SQLite savepoint, so a
 * migration that throws leaves the database exactly as it was.
 *
 * Adding a migration means appending to {@link MIGRATIONS}. Never edit an entry
 * that has shipped: its id is already in somebody's ledger and it will not run
 * again.
 */

import type { RawAccess } from 'rivetkit/db';

/** The database handle this module speaks to — RivetKit's raw SQLite client. */
export type ProfileDb = RawAccess;

interface Migration {
	/** Stable, never reused, never edited after shipping. */
	readonly id: string;
	/** One statement per entry. Executed in order. */
	readonly statements: readonly string[];
}

const LEDGER_TABLE = '_profile_migrations';

/**
 * Every migration, oldest first.
 *
 * `0001_initial` carries the whole v1 schema:
 *
 * - **`matches`** — one row per game. `played_at` (= `ended_at ?? started_at`) is
 *   denormalised so the history index is a single covering key rather than a
 *   `COALESCE` the planner cannot use. Every counter needed by lifetime stats is a
 *   column, so the stats query is one aggregate over one table with no joins.
 * - **`hands`** — the journal, `(match_id, hand_no)` as the primary key so a
 *   re-delivered hand collides instead of duplicating. `seed` and `deck_order_json`
 *   are the server-only fields that make a replay possible; nothing in this module
 *   returns them to a connection.
 * - **`episodes`** — cross-game persona memory. `episode_id` is derived
 *   deterministically from `(role, match_id, hand_no, kind)`, so a re-delivered
 *   flush is a primary-key conflict, not a second grudge.
 *
 * No foreign key from `hands` to `matches`: hands are journalled at every hand
 * boundary and the match row is written at game over, so for most of a match the
 * child exists without the parent. That ordering is the point — an interrupted
 * game still has history.
 */
const MIGRATIONS: readonly Migration[] = [
	{
		id: '0001_initial',
		statements: [
			`CREATE TABLE IF NOT EXISTS matches (
				match_id TEXT PRIMARY KEY,
				schema INTEGER NOT NULL,
				user_id TEXT NOT NULL,
				seed TEXT NOT NULL,
				cfg_json TEXT NOT NULL,
				first_dealer INTEGER NOT NULL,
				started_at INTEGER NOT NULL,
				ended_at INTEGER,
				played_at INTEGER NOT NULL,
				status TEXT NOT NULL,
				outcome TEXT NOT NULL,
				score_us INTEGER NOT NULL,
				score_them INTEGER NOT NULL,
				winner_team INTEGER,
				hands_played INTEGER NOT NULL,
				personas_json TEXT NOT NULL,
				opponents_json TEXT NOT NULL,
				euchres_for INTEGER NOT NULL DEFAULT 0,
				euchres_against INTEGER NOT NULL DEFAULT 0,
				loners_attempted INTEGER NOT NULL DEFAULT 0,
				loners_made INTEGER NOT NULL DEFAULT 0,
				marches INTEGER NOT NULL DEFAULT 0,
				throw_ins INTEGER NOT NULL DEFAULT 0,
				tricks_us INTEGER NOT NULL DEFAULT 0,
				tricks_them INTEGER NOT NULL DEFAULT 0,
				tokens_in INTEGER NOT NULL DEFAULT 0,
				tokens_out INTEGER NOT NULL DEFAULT 0,
				cache_read_tokens INTEGER NOT NULL DEFAULT 0,
				llm_calls INTEGER NOT NULL DEFAULT 0,
				recorded_at INTEGER NOT NULL,
				updated_at INTEGER NOT NULL
			)`,
			`CREATE INDEX IF NOT EXISTS idx_matches_history
				ON matches (played_at DESC, match_id DESC)`,
			`CREATE INDEX IF NOT EXISTS idx_matches_outcome ON matches (outcome)`,
			`CREATE TABLE IF NOT EXISTS hands (
				match_id TEXT NOT NULL,
				hand_no INTEGER NOT NULL,
				seed TEXT NOT NULL,
				dealer_seat INTEGER NOT NULL,
				deck_order_json TEXT NOT NULL,
				moves_json TEXT NOT NULL,
				trump TEXT,
				maker_seat INTEGER,
				alone_seat INTEGER,
				tricks_us INTEGER NOT NULL,
				tricks_them INTEGER NOT NULL,
				result TEXT,
				delta_us INTEGER,
				delta_them INTEGER,
				ended_at INTEGER NOT NULL,
				PRIMARY KEY (match_id, hand_no)
			)`,
			`CREATE TABLE IF NOT EXISTS episodes (
				episode_id TEXT PRIMARY KEY,
				role TEXT NOT NULL,
				match_id TEXT NOT NULL,
				hand_no INTEGER NOT NULL,
				ts INTEGER NOT NULL,
				kind TEXT NOT NULL,
				summary TEXT NOT NULL,
				salience REAL NOT NULL,
				expires_at INTEGER NOT NULL
			)`,
			`CREATE INDEX IF NOT EXISTS idx_episodes_role
				ON episodes (role, salience DESC, ts DESC)`,
			`CREATE INDEX IF NOT EXISTS idx_episodes_expiry ON episodes (expires_at)`
		]
	}
];

/**
 * Creates the ledger, then applies whatever is missing.
 *
 * Safe to call on a fresh database, on an up-to-date one, and on one whose last
 * migration was interrupted — RivetKit's savepoint means a partial migration
 * never commits, so the ledger and the schema cannot disagree.
 */
export async function applyMigrations(db: ProfileDb): Promise<void> {
	await db.execute(
		`CREATE TABLE IF NOT EXISTS ${LEDGER_TABLE} (
			id TEXT PRIMARY KEY,
			applied_at INTEGER NOT NULL
		)`
	);

	const applied = await db.execute<{ id: unknown }>(`SELECT id FROM ${LEDGER_TABLE}`);
	const done = new Set<string>();
	for (const row of applied) {
		if (typeof row.id === 'string') done.add(row.id);
	}

	for (const migration of MIGRATIONS) {
		if (done.has(migration.id)) continue;
		for (const statement of migration.statements) {
			await db.execute(statement);
		}
		await db.execute(
			`INSERT INTO ${LEDGER_TABLE} (id, applied_at) VALUES (?, ?)
				ON CONFLICT(id) DO NOTHING`,
			migration.id,
			Date.now()
		);
	}
}

/** The ids of every migration this build knows about, for diagnostics. */
export const MIGRATION_IDS: readonly string[] = MIGRATIONS.map((migration) => migration.id);
