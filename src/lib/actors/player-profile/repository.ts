/**
 * Every SQL statement in `playerProfile`, and the only place rows become records.
 *
 * Three rules hold throughout this file:
 *
 * 1. **Values are always bound, never interpolated.** The only strings ever
 *    concatenated into a statement are column names, and those come from object
 *    literals in this file — a caller cannot reach them.
 * 2. **`seed` and `deck_order_json` never leave through a client-facing path.**
 *    {@link readClientReplay} and {@link listMatches} build types that have no
 *    field capable of holding them; {@link readServerReplay} is the one function
 *    that returns them, and its caller demands an internal token first.
 * 3. **Writes are idempotent.** `matches` conflicts on `match_id`, `hands` on
 *    `(match_id, hand_no)`, `episodes` on a deterministic `episode_id`. The table
 *    is free to retry after a serverless migration, and a retry is a no-op rather
 *    than a duplicate row.
 */

import type {
	EngineConfig,
	EpisodeKind,
	GameStatus,
	HandJournalEntry,
	HandResult,
	MatchListPage,
	MatchOutcome,
	MatchRecord,
	MatchStats,
	MatchSummary,
	PersonaAssignment,
	PlayerAction,
	Seat,
	Suit,
	Team,
	TokenUsage
} from '$lib/protocol';
import {
	EPISODE_SUMMARY_MAX_CHARS,
	PERSONA_NAME_MAX_CHARS,
	DEFAULT_ENGINE_CONFIG
} from './protocol-constants';
import {
	asArray,
	decodeJson,
	encodeJson,
	isRecord,
	pair,
	pick,
	protocolError,
	requireId,
	requireInt,
	requireOneOf,
	sanitizeText,
	toFloat,
	toInt,
	toIntOrNull,
	toStringColumn,
	toStringOrNull
} from './guards';
import type { ProfileDb } from './schema';
import type {
	ClientHandReplay,
	ClientMatchReplay,
	ProfileStats,
	ServerMatchReplay,
	StoredEpisode,
	StoredHandRow
} from './types';

/* ========================================================================== */
/* Enumerations, as runtime sets                                              */
/* ========================================================================== */

const GAME_STATUSES = ['active', 'complete', 'abandoned'] as const satisfies readonly GameStatus[];
const MATCH_OUTCOMES = ['won', 'lost', 'abandoned'] as const satisfies readonly MatchOutcome[];
const HAND_RESULTS = [
	'point',
	'march',
	'lone_point',
	'lone_march',
	'euchre',
	'throw_in'
] as const satisfies readonly HandResult[];
const SUITS = ['S', 'H', 'D', 'C'] as const satisfies readonly Suit[];
const EPISODE_KINDS = [
	'bid',
	'play',
	'loner',
	'euchre',
	'read'
] as const satisfies readonly EpisodeKind[];

/** Hard ceilings so one malformed message cannot write an unbounded batch. */
const MAX_HANDS_PER_CALL = 64;
const MAX_EPISODES_PER_CALL = 32;
const MAX_DECK_CARDS = 24;
const MAX_MOVES_PER_HAND = 128;
const MAX_LIST_LIMIT = 50;
const DEFAULT_LIST_LIMIT = 20;

/* ========================================================================== */
/* Small parsers                                                              */
/* ========================================================================== */

function parseSeat(raw: unknown, field: string): Seat {
	const value = requireInt(raw, field, 0, 3);
	return value as Seat;
}

function parseSeatOrNull(raw: unknown, field: string): Seat | null {
	return raw === null || raw === undefined ? null : parseSeat(raw, field);
}

function parseTeamOrNull(raw: unknown, field: string): Team | null {
	if (raw === null || raw === undefined) return null;
	return requireInt(raw, field, 0, 1) as Team;
}

function parsePair(raw: unknown, field: string): [number, number] {
	const list = asArray(raw);
	if (list.length !== 2) {
		throw protocolError('internal_error', `${field} must be a pair`);
	}
	return [requireInt(list[0], `${field}[0]`, -1000, 1000), requireInt(list[1], `${field}[1]`, -1000, 1000)];
}

function parsePairOrNull(raw: unknown, field: string): [number, number] | null {
	return raw === null || raw === undefined ? null : parsePair(raw, field);
}

function parseTimestamp(raw: unknown, field: string, fallback: number): number {
	if (raw === null || raw === undefined) return fallback;
	return requireInt(raw, field, 0, Number.MAX_SAFE_INTEGER);
}

function parseCount(raw: unknown): number {
	const value = toInt(raw, 0);
	return value < 0 ? 0 : value;
}

function parseStats(raw: unknown): MatchStats {
	return {
		euchresFor: parseCount(pick(raw, 'euchresFor')),
		euchresAgainst: parseCount(pick(raw, 'euchresAgainst')),
		lonersAttempted: parseCount(pick(raw, 'lonersAttempted')),
		lonersMade: parseCount(pick(raw, 'lonersMade')),
		marches: parseCount(pick(raw, 'marches')),
		throwIns: parseCount(pick(raw, 'throwIns')),
		tricks: [parseCount(pick(pick(raw, 'tricks'), '0')), parseCount(pick(pick(raw, 'tricks'), '1'))]
	};
}

function parseTokens(raw: unknown): TokenUsage {
	return {
		tokensIn: parseCount(pick(raw, 'tokensIn')),
		tokensOut: parseCount(pick(raw, 'tokensOut')),
		cacheReadTokens: parseCount(pick(raw, 'cacheReadTokens')),
		calls: parseCount(pick(raw, 'calls'))
	};
}

/**
 * Opponent display names in seat order 1, 2, 3.
 *
 * Sanitised here rather than trusted, even though the assignment came from this
 * profile's own settings: it made a round trip through another actor, and a name
 * is the one persona field that gets rendered.
 */
function opponentNames(personas: readonly PersonaAssignment[]): string[] {
	const bySeat = new Map<number, string>();
	for (const assignment of personas) {
		const seat = pick(assignment, 'seat');
		const name = pick(pick(assignment, 'persona'), 'name');
		if (typeof seat !== 'number' || typeof name !== 'string') continue;
		bySeat.set(
			seat,
			sanitizeText(name, {
				multiline: false,
				maxChars: PERSONA_NAME_MAX_CHARS,
				field: 'persona.name',
				code: 'internal_error'
			})
		);
	}
	return [1, 2, 3].map((seat) => bySeat.get(seat) ?? '');
}

/* ========================================================================== */
/* Inbound record validation                                                  */
/* ========================================================================== */

/**
 * Validates an inbound `MatchRecord`.
 *
 * `owner` is the actor's own key, not anything the message said. A record whose
 * `userId` disagrees is refused: the caller does not get to file history under
 * somebody else's profile even with a valid internal token.
 */
export function parseMatchRecord(raw: unknown, owner: string, now: number): MatchRecord {
	if (!isRecord(raw)) {
		throw protocolError('internal_error', 'record must be an object');
	}

	const userId = raw.userId;
	if (typeof userId !== 'string' || userId !== owner) {
		throw protocolError('forbidden', 'record does not belong to this profile');
	}

	const matchId = requireId(raw.matchId, 'record.matchId');
	const seed = raw.seed;
	if (typeof seed !== 'string' || seed.length === 0 || seed.length > 256) {
		throw protocolError('internal_error', 'record.seed must be a bounded non-empty string');
	}

	const startedAt = parseTimestamp(raw.startedAt, 'record.startedAt', now);
	const endedAt = raw.endedAt === null || raw.endedAt === undefined
		? null
		: parseTimestamp(raw.endedAt, 'record.endedAt', now);

	const personas = asArray(raw.personas).slice(0, 3) as readonly PersonaAssignment[];

	return {
		schema: 1,
		matchId,
		userId: owner,
		seed,
		cfg: (isRecord(raw.cfg) ? raw.cfg : DEFAULT_ENGINE_CONFIG) as EngineConfig,
		firstDealer: parseSeat(raw.firstDealer ?? 0, 'record.firstDealer'),
		startedAt,
		endedAt,
		status: requireOneOf(raw.status, GAME_STATUSES, 'record.status'),
		outcome: requireOneOf(raw.outcome, MATCH_OUTCOMES, 'record.outcome'),
		score: parsePair(raw.score, 'record.score'),
		winnerTeam: parseTeamOrNull(raw.winnerTeam, 'record.winnerTeam'),
		handsPlayed: requireInt(raw.handsPlayed ?? 0, 'record.handsPlayed', 0, 10_000),
		personas,
		stats: parseStats(raw.stats),
		tokens: parseTokens(raw.tokens)
	};
}

/** Validates an inbound `HandJournalEntry[]`. */
export function parseHands(raw: unknown, matchId: string): StoredHandRow[] {
	const list = asArray(raw);
	if (list.length > MAX_HANDS_PER_CALL) {
		throw protocolError('internal_error', `at most ${MAX_HANDS_PER_CALL} hands per call`);
	}

	return list.map((entry, index): StoredHandRow => {
		if (!isRecord(entry)) {
			throw protocolError('internal_error', `hands[${index}] must be an object`);
		}
		const entryMatchId = requireId(entry.matchId ?? matchId, `hands[${index}].matchId`);
		if (entryMatchId !== matchId) {
			throw protocolError('internal_error', `hands[${index}] belongs to a different match`);
		}

		const deckOrder = asArray(entry.deckOrder);
		if (deckOrder.length > MAX_DECK_CARDS) {
			throw protocolError('internal_error', `hands[${index}].deckOrder is too long`);
		}
		const moves = asArray(entry.moves);
		if (moves.length > MAX_MOVES_PER_HAND) {
			throw protocolError('internal_error', `hands[${index}].moves is too long`);
		}

		return {
			matchId: entryMatchId,
			handNo: requireInt(entry.handNo, `hands[${index}].handNo`, 0, 10_000),
			seed: typeof entry.seed === 'string' ? entry.seed.slice(0, 256) : '',
			dealerSeat: parseSeat(entry.dealerSeat ?? 0, `hands[${index}].dealerSeat`),
			deckOrder: deckOrder.filter((card): card is string => typeof card === 'string') as
				readonly StoredHandRow['deckOrder'][number][],
			moves: moves as readonly PlayerAction[],
			trump:
				entry.trump === null || entry.trump === undefined
					? null
					: requireOneOf(entry.trump, SUITS, `hands[${index}].trump`),
			makerSeat: parseSeatOrNull(entry.makerSeat, `hands[${index}].makerSeat`),
			aloneSeat: parseSeatOrNull(entry.aloneSeat, `hands[${index}].aloneSeat`),
			tricksWon: parsePair(entry.tricksWon ?? [0, 0], `hands[${index}].tricksWon`),
			result:
				entry.result === null || entry.result === undefined
					? null
					: requireOneOf(entry.result, HAND_RESULTS, `hands[${index}].result`),
			delta: parsePairOrNull(entry.delta, `hands[${index}].delta`),
			endedAt: parseTimestamp(entry.endedAt, `hands[${index}].endedAt`, 0)
		};
	});
}

/* ========================================================================== */
/* matches                                                                    */
/* ========================================================================== */

/** Column name → bound value. Keys are literals; values are always parameters. */
type ColumnMap = Record<string, unknown>;

function matchColumns(record: MatchRecord, now: number): ColumnMap {
	return {
		match_id: record.matchId,
		schema: record.schema,
		user_id: record.userId,
		seed: record.seed,
		cfg_json: encodeJson(record.cfg),
		first_dealer: record.firstDealer,
		started_at: record.startedAt,
		ended_at: record.endedAt,
		played_at: record.endedAt ?? record.startedAt,
		status: record.status,
		outcome: record.outcome,
		score_us: record.score[0],
		score_them: record.score[1],
		winner_team: record.winnerTeam,
		hands_played: record.handsPlayed,
		personas_json: encodeJson(record.personas),
		opponents_json: encodeJson(opponentNames(record.personas)),
		euchres_for: record.stats.euchresFor,
		euchres_against: record.stats.euchresAgainst,
		loners_attempted: record.stats.lonersAttempted,
		loners_made: record.stats.lonersMade,
		marches: record.stats.marches,
		throw_ins: record.stats.throwIns,
		tricks_us: record.stats.tricks[0],
		tricks_them: record.stats.tricks[1],
		tokens_in: record.tokens.tokensIn,
		tokens_out: record.tokens.tokensOut,
		cache_read_tokens: record.tokens.cacheReadTokens,
		llm_calls: record.tokens.calls,
		recorded_at: now,
		updated_at: now
	};
}

export interface RecordMatchOutcome {
	readonly matchId: string;
	/** `true` when this call created the row. */
	readonly inserted: boolean;
	/** `true` when it advanced an existing, still-live row. */
	readonly updated: boolean;
}

/**
 * Writes a match row, **idempotently**.
 *
 * The table may retry this call — a serverless function can be migrated between
 * the write and the acknowledgement — so a second delivery must not produce a
 * second row. Keyed on `match_id`:
 *
 * - no row → insert;
 * - a row that has already ended (`ended_at IS NOT NULL`) → **no-op**. A finished
 *   match is immutable history; a late retry cannot rewrite it;
 * - a row still in progress → advance it, which is what lets a match be recorded
 *   at a hand boundary and then completed at game over.
 *
 * The read and the write share one transaction, so two concurrent deliveries
 * cannot both see "no row" and both insert.
 */
export async function upsertMatch(
	db: ProfileDb,
	record: MatchRecord,
	now: number
): Promise<RecordMatchOutcome> {
	return db.transaction(async (tx) => {
		const existing = await tx.execute<{ ended_at: unknown }>(
			'SELECT ended_at FROM matches WHERE match_id = ? LIMIT 1',
			record.matchId
		);

		if (existing.length > 0) {
			if (toIntOrNull(existing[0].ended_at) !== null) {
				return { matchId: record.matchId, inserted: false, updated: false };
			}
			const columns = matchColumns(record, now);
			const names = Object.keys(columns).filter(
				(name) => name !== 'match_id' && name !== 'recorded_at'
			);
			const assignments = names.map((name) => `${name} = ?`).join(', ');
			await tx.execute(
				`UPDATE matches SET ${assignments} WHERE match_id = ? AND ended_at IS NULL`,
				...names.map((name) => columns[name]),
				record.matchId
			);
			return { matchId: record.matchId, inserted: false, updated: true };
		}

		const columns = matchColumns(record, now);
		const names = Object.keys(columns);
		const placeholders = names.map(() => '?').join(', ');
		await tx.execute(
			`INSERT INTO matches (${names.join(', ')}) VALUES (${placeholders})
				ON CONFLICT(match_id) DO NOTHING`,
			...names.map((name) => columns[name])
		);
		return { matchId: record.matchId, inserted: true, updated: false };
	});
}

/**
 * Journals hands, idempotently.
 *
 * `ON CONFLICT DO NOTHING` on `(match_id, hand_no)`: a hand is written once and
 * never rewritten, because a hand that has ended cannot change. Returns how many
 * rows were new.
 */
export async function insertHands(db: ProfileDb, hands: readonly StoredHandRow[]): Promise<number> {
	if (hands.length === 0) return 0;

	return db.transaction(async (tx) => {
		let written = 0;
		for (const hand of hands) {
			const columns: ColumnMap = {
				match_id: hand.matchId,
				hand_no: hand.handNo,
				seed: hand.seed,
				dealer_seat: hand.dealerSeat,
				deck_order_json: encodeJson(hand.deckOrder),
				moves_json: encodeJson(hand.moves),
				trump: hand.trump,
				maker_seat: hand.makerSeat,
				alone_seat: hand.aloneSeat,
				tricks_us: hand.tricksWon[0],
				tricks_them: hand.tricksWon[1],
				result: hand.result,
				delta_us: hand.delta === null ? null : hand.delta[0],
				delta_them: hand.delta === null ? null : hand.delta[1],
				ended_at: hand.endedAt
			};
			const names = Object.keys(columns);
			await tx.execute(
				`INSERT INTO hands (${names.join(', ')}) VALUES (${names.map(() => '?').join(', ')})
					ON CONFLICT(match_id, hand_no) DO NOTHING`,
				...names.map((name) => columns[name])
			);
			written += 1;
		}
		return written;
	});
}

/* ========================================================================== */
/* Pagination                                                                 */
/* ========================================================================== */

/**
 * A keyset cursor: `playedAt:matchId`.
 *
 * Keyset, not `OFFSET`: an offset drifts when a row is inserted between pages and
 * costs more the deeper you scroll. Both halves are needed because `played_at`
 * is not unique — two games can finish in the same millisecond, and `match_id` is
 * the tiebreak that makes the ordering total.
 *
 * The cursor is opaque to the client but attacker-supplied, so it is parsed
 * strictly and, being nothing but two bound parameters, cannot say anything
 * interesting to the database even if it is forged.
 */
function encodeCursor(playedAt: number, matchId: string): string {
	return `${playedAt}:${matchId}`;
}

interface DecodedCursor {
	readonly playedAt: number;
	readonly matchId: string;
}

function decodeCursor(raw: unknown): DecodedCursor | null {
	if (raw === null || raw === undefined || raw === '') return null;
	if (typeof raw !== 'string' || raw.length > 160) {
		throw protocolError('internal_error', 'cursor is not a string');
	}
	const separator = raw.indexOf(':');
	if (separator <= 0) {
		throw protocolError('internal_error', 'cursor is malformed');
	}
	const playedAtText = raw.slice(0, separator);
	if (!/^\d{1,15}$/.test(playedAtText)) {
		throw protocolError('internal_error', 'cursor timestamp is malformed');
	}
	return { playedAt: Number(playedAtText), matchId: requireId(raw.slice(separator + 1), 'cursor') };
}

/** Clamps a caller's page size into `[1, 50]`, defaulting to 20. */
export function clampLimit(raw: unknown): number {
	if (raw === null || raw === undefined) return DEFAULT_LIST_LIMIT;
	if (typeof raw !== 'number' || !Number.isFinite(raw)) return DEFAULT_LIST_LIMIT;
	return Math.min(MAX_LIST_LIMIT, Math.max(1, Math.trunc(raw)));
}

function rowToSummary(row: Record<string, unknown>): MatchSummary {
	return {
		matchId: toStringColumn(row.match_id),
		playedAt: toInt(row.played_at),
		status: requireOneOf(toStringColumn(row.status), GAME_STATUSES, 'status'),
		outcome: requireOneOf(toStringColumn(row.outcome), MATCH_OUTCOMES, 'outcome'),
		score: pair(row.score_us, row.score_them),
		handsPlayed: toInt(row.hands_played),
		opponents: decodeJson<string[]>(row.opponents_json, [])
	};
}

/**
 * One page of history, newest first.
 *
 * `SELECT`s exactly the columns {@link MatchSummary} needs — `seed` is not among
 * them, so it is not even read into memory on the path that ends at a browser.
 * Fetches `limit + 1` rows to decide whether a next cursor exists without a
 * second `COUNT(*)`.
 */
export async function listMatches(
	db: ProfileDb,
	limit: number,
	rawCursor: unknown
): Promise<MatchListPage> {
	const cursor = decodeCursor(rawCursor);
	const columns =
		'match_id, played_at, status, outcome, score_us, score_them, hands_played, opponents_json';

	const rows = cursor
		? await db.execute<Record<string, unknown>>(
				`SELECT ${columns} FROM matches
					WHERE played_at < ? OR (played_at = ? AND match_id < ?)
					ORDER BY played_at DESC, match_id DESC
					LIMIT ?`,
				cursor.playedAt,
				cursor.playedAt,
				cursor.matchId,
				limit + 1
			)
		: await db.execute<Record<string, unknown>>(
				`SELECT ${columns} FROM matches
					ORDER BY played_at DESC, match_id DESC
					LIMIT ?`,
				limit + 1
			);

	const page = rows.slice(0, limit).map((row) => rowToSummary(row));
	const nextCursor =
		rows.length > limit && page.length > 0
			? encodeCursor(page[page.length - 1].playedAt, page[page.length - 1].matchId)
			: null;

	return { rows: page, cursor: nextCursor };
}

/* ========================================================================== */
/* Replay                                                                     */
/* ========================================================================== */

function rowToHandJournal(row: Record<string, unknown>): HandJournalEntry {
	const trump = toStringOrNull(row.trump);
	const result = toStringOrNull(row.result);
	const deltaUs = toIntOrNull(row.delta_us);
	const deltaThem = toIntOrNull(row.delta_them);
	return {
		matchId: toStringColumn(row.match_id),
		handNo: toInt(row.hand_no),
		seed: toStringColumn(row.seed),
		dealerSeat: toInt(row.dealer_seat) as Seat,
		deckOrder: decodeJson<HandJournalEntry['deckOrder']>(row.deck_order_json, []),
		moves: decodeJson<readonly PlayerAction[]>(row.moves_json, []),
		trump: trump === null ? null : requireOneOf(trump, SUITS, 'hands.trump'),
		makerSeat: toIntOrNull(row.maker_seat) as Seat | null,
		aloneSeat: toIntOrNull(row.alone_seat) as Seat | null,
		tricksWon: pair(row.tricks_us, row.tricks_them),
		result: result === null ? null : requireOneOf(result, HAND_RESULTS, 'hands.result'),
		delta: deltaUs === null || deltaThem === null ? null : [deltaUs, deltaThem],
		endedAt: toInt(row.ended_at)
	};
}

/**
 * Drops a `discard` played by a seat other than the human.
 *
 * The dealer's discard is secret while the hand runs (`GameState.hand.dealerDiscard`
 * is marked SECRET in the engine's types) and the hand ending does not make it
 * public — the card was buried, never shown. Seat 0's own discard is kept: the
 * player chose it and already knows.
 */
function isClientVisibleMove(action: PlayerAction): boolean {
	const kind = pick(pick(action, 'move'), 't');
	if (kind !== 'discard') return true;
	return pick(action, 'seat') === 0;
}

/**
 * The **client-safe** replay of one match, or `null`.
 *
 * The returned type has no `seed` and no `deckOrder` field, so no future edit to
 * this function can leak them without a type error at the declaration.
 */
export async function readClientReplay(
	db: ProfileDb,
	matchId: string
): Promise<ClientMatchReplay | null> {
	const matches = await db.execute<Record<string, unknown>>(
		`SELECT match_id, started_at, ended_at, played_at, status, outcome, score_us, score_them,
			winner_team, hands_played, opponents_json, euchres_for, euchres_against,
			loners_attempted, loners_made, marches, throw_ins, tricks_us, tricks_them
			FROM matches WHERE match_id = ? LIMIT 1`,
		matchId
	);
	if (matches.length === 0) return null;
	const row = matches[0];

	const handRows = await db.execute<Record<string, unknown>>(
		`SELECT hand_no, dealer_seat, moves_json, trump, maker_seat, alone_seat,
			tricks_us, tricks_them, result, delta_us, delta_them, ended_at
			FROM hands WHERE match_id = ? ORDER BY hand_no ASC`,
		matchId
	);

	const hands: ClientHandReplay[] = handRows.map((hand) => {
		const trump = toStringOrNull(hand.trump);
		const result = toStringOrNull(hand.result);
		const deltaUs = toIntOrNull(hand.delta_us);
		const deltaThem = toIntOrNull(hand.delta_them);
		const moves = decodeJson<readonly PlayerAction[]>(hand.moves_json, []);
		return {
			handNo: toInt(hand.hand_no),
			dealerSeat: toInt(hand.dealer_seat) as Seat,
			trump: trump === null ? null : requireOneOf(trump, SUITS, 'hands.trump'),
			makerSeat: toIntOrNull(hand.maker_seat) as Seat | null,
			aloneSeat: toIntOrNull(hand.alone_seat) as Seat | null,
			tricksWon: pair(hand.tricks_us, hand.tricks_them),
			result: result === null ? null : requireOneOf(result, HAND_RESULTS, 'hands.result'),
			delta: deltaUs === null || deltaThem === null ? null : [deltaUs, deltaThem],
			endedAt: toInt(hand.ended_at),
			moves: moves.filter((move) => isClientVisibleMove(move))
		};
	});

	return {
		matchId: toStringColumn(row.match_id),
		startedAt: toInt(row.started_at),
		endedAt: toIntOrNull(row.ended_at),
		playedAt: toInt(row.played_at),
		status: requireOneOf(toStringColumn(row.status), GAME_STATUSES, 'status'),
		outcome: requireOneOf(toStringColumn(row.outcome), MATCH_OUTCOMES, 'outcome'),
		score: pair(row.score_us, row.score_them),
		winnerTeam: toIntOrNull(row.winner_team) as Team | null,
		handsPlayed: toInt(row.hands_played),
		opponents: decodeJson<string[]>(row.opponents_json, []),
		stats: {
			euchresFor: toInt(row.euchres_for),
			euchresAgainst: toInt(row.euchres_against),
			lonersAttempted: toInt(row.loners_attempted),
			lonersMade: toInt(row.loners_made),
			marches: toInt(row.marches),
			throwIns: toInt(row.throw_ins),
			tricks: pair(row.tricks_us, row.tricks_them)
		},
		hands
	};
}

/**
 * The **server-only** replay: the stored record verbatim, `seed` and every
 * `deckOrder` included, which together identify every hidden card.
 *
 * Its one legitimate consumer rebuilds the match server-side through
 * `createGame` + `apply` and calls `project(state, seat)` on each frame before
 * anything is sent anywhere. Reaching it requires the internal token.
 */
export async function readServerReplay(
	db: ProfileDb,
	matchId: string
): Promise<ServerMatchReplay | null> {
	const matches = await db.execute<Record<string, unknown>>(
		'SELECT * FROM matches WHERE match_id = ? LIMIT 1',
		matchId
	);
	if (matches.length === 0) return null;
	const row = matches[0];

	const handRows = await db.execute<Record<string, unknown>>(
		'SELECT * FROM hands WHERE match_id = ? ORDER BY hand_no ASC',
		matchId
	);

	const match: MatchRecord = {
		schema: 1,
		matchId: toStringColumn(row.match_id),
		userId: toStringColumn(row.user_id),
		seed: toStringColumn(row.seed),
		cfg: decodeJson<EngineConfig>(row.cfg_json, DEFAULT_ENGINE_CONFIG),
		firstDealer: toInt(row.first_dealer) as Seat,
		startedAt: toInt(row.started_at),
		endedAt: toIntOrNull(row.ended_at),
		status: requireOneOf(toStringColumn(row.status), GAME_STATUSES, 'status'),
		outcome: requireOneOf(toStringColumn(row.outcome), MATCH_OUTCOMES, 'outcome'),
		score: pair(row.score_us, row.score_them),
		winnerTeam: toIntOrNull(row.winner_team) as Team | null,
		handsPlayed: toInt(row.hands_played),
		personas: decodeJson<readonly PersonaAssignment[]>(row.personas_json, []),
		stats: {
			euchresFor: toInt(row.euchres_for),
			euchresAgainst: toInt(row.euchres_against),
			lonersAttempted: toInt(row.loners_attempted),
			lonersMade: toInt(row.loners_made),
			marches: toInt(row.marches),
			throwIns: toInt(row.throw_ins),
			tricks: pair(row.tricks_us, row.tricks_them)
		},
		tokens: {
			tokensIn: toInt(row.tokens_in),
			tokensOut: toInt(row.tokens_out),
			cacheReadTokens: toInt(row.cache_read_tokens),
			calls: toInt(row.llm_calls)
		}
	};

	return { match, hands: handRows.map((hand) => rowToHandJournal(hand)) };
}

/* ========================================================================== */
/* Stats                                                                      */
/* ========================================================================== */

/**
 * Lifetime statistics, computed by **one aggregate over the history table**.
 *
 * Deliberately not a set of counters kept in `c.state` and incremented on the
 * write path. A counter is a second source of truth: it drifts the first time a
 * write is retried, a row is deleted, or a deploy lands between the row and the
 * increment. This query cannot drift — it is a function of the table.
 *
 * `hands_played` comes from `matches`, not `COUNT(*)` on `hands`, because a hand
 * journal may be pruned while the match row remains.
 */
export async function computeStats(db: ProfileDb, now: number): Promise<ProfileStats> {
	const rows = await db.execute<Record<string, unknown>>(
		`SELECT
			COUNT(*) AS matches,
			COALESCE(SUM(CASE WHEN outcome = 'won' THEN 1 ELSE 0 END), 0) AS won,
			COALESCE(SUM(CASE WHEN outcome = 'lost' THEN 1 ELSE 0 END), 0) AS lost,
			COALESCE(SUM(CASE WHEN outcome = 'abandoned' THEN 1 ELSE 0 END), 0) AS abandoned,
			COALESCE(SUM(euchres_for), 0) AS euchres_for,
			COALESCE(SUM(euchres_against), 0) AS euchres_against,
			COALESCE(SUM(loners_attempted), 0) AS loners_attempted,
			COALESCE(SUM(loners_made), 0) AS loners_made,
			COALESCE(SUM(marches), 0) AS marches,
			COALESCE(SUM(throw_ins), 0) AS throw_ins,
			COALESCE(SUM(hands_played), 0) AS hands_played,
			COALESCE(SUM(tricks_us), 0) AS tricks_us,
			COALESCE(SUM(tricks_them), 0) AS tricks_them,
			COALESCE(SUM(tokens_in), 0) AS tokens_in,
			COALESCE(SUM(tokens_out), 0) AS tokens_out,
			COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
			COALESCE(SUM(llm_calls), 0) AS llm_calls
			FROM matches`
	);

	const row = rows[0] ?? {};
	const won = toInt(row.won);
	const lost = toInt(row.lost);
	const decided = won + lost;

	return {
		matches: toInt(row.matches),
		won,
		lost,
		abandoned: toInt(row.abandoned),
		winRate: decided === 0 ? 0 : won / decided,
		euchresFor: toInt(row.euchres_for),
		euchresAgainst: toInt(row.euchres_against),
		lonersAttempted: toInt(row.loners_attempted),
		lonersMade: toInt(row.loners_made),
		marches: toInt(row.marches),
		throwIns: toInt(row.throw_ins),
		handsPlayed: toInt(row.hands_played),
		tricks: pair(row.tricks_us, row.tricks_them),
		tokens: {
			tokensIn: toInt(row.tokens_in),
			tokensOut: toInt(row.tokens_out),
			cacheReadTokens: toInt(row.cache_read_tokens),
			calls: toInt(row.llm_calls)
		},
		updatedAt: now
	};
}

/* ========================================================================== */
/* Episodes                                                                   */
/* ========================================================================== */

/** Ninety days, the retention `EpisodeRecord.expiresAt` documents. */
const EPISODE_TTL_MS = 90 * 24 * 60 * 60 * 1000;

/**
 * A deterministic row id.
 *
 * `(role, matchId, handNo, kind)` is the natural key of a memory, so deriving the
 * primary key from it makes a re-delivered flush a primary-key conflict rather
 * than a duplicate grudge. Components are id-validated before they get here, so
 * the joiner cannot appear inside one of them.
 */
function episodeId(role: string, matchId: string, handNo: number, kind: EpisodeKind): string {
	return `${role}.${matchId}.${handNo}.${kind}`;
}

export function parseEpisodes(raw: unknown, now: number): StoredEpisode[] {
	const list = asArray(raw);
	if (list.length > MAX_EPISODES_PER_CALL) {
		throw protocolError('internal_error', `at most ${MAX_EPISODES_PER_CALL} episodes per call`);
	}

	return list.map((entry, index): StoredEpisode => {
		if (!isRecord(entry)) {
			throw protocolError('internal_error', `episodes[${index}] must be an object`);
		}
		const role = requireId(entry.role, `episodes[${index}].role`);
		const matchId = requireId(entry.matchId, `episodes[${index}].matchId`);
		const handNo = requireInt(entry.handNo, `episodes[${index}].handNo`, 0, 10_000);
		const kind = requireOneOf(entry.kind, EPISODE_KINDS, `episodes[${index}].kind`);
		const ts = parseTimestamp(entry.ts, `episodes[${index}].ts`, now);

		return {
			episodeId: episodeId(role, matchId, handNo, kind),
			role,
			matchId,
			handNo,
			ts,
			kind,
			// LLM-authored text that will re-enter a prompt as part of a dossier:
			// screened for invisible characters and clamped here, fenced later.
			summary: sanitizeText(entry.summary, {
				multiline: false,
				maxChars: EPISODE_SUMMARY_MAX_CHARS,
				field: `episodes[${index}].summary`,
				code: 'internal_error'
			}),
			salience: Math.min(1, Math.max(0, toFloat(entry.salience, 0))),
			expiresAt:
				entry.expiresAt === null || entry.expiresAt === undefined
					? ts + EPISODE_TTL_MS
					: parseTimestamp(entry.expiresAt, `episodes[${index}].expiresAt`, ts + EPISODE_TTL_MS)
		};
	});
}

export async function insertEpisodes(
	db: ProfileDb,
	episodes: readonly StoredEpisode[]
): Promise<number> {
	if (episodes.length === 0) return 0;
	return db.transaction(async (tx) => {
		for (const episode of episodes) {
			await tx.execute(
				`INSERT INTO episodes
					(episode_id, role, match_id, hand_no, ts, kind, summary, salience, expires_at)
					VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
					ON CONFLICT(episode_id) DO NOTHING`,
				episode.episodeId,
				episode.role,
				episode.matchId,
				episode.handNo,
				episode.ts,
				episode.kind,
				episode.summary,
				episode.salience,
				episode.expiresAt
			);
		}
		return episodes.length;
	});
}

function rowToEpisode(row: Record<string, unknown>): StoredEpisode {
	return {
		episodeId: toStringColumn(row.episode_id),
		role: toStringColumn(row.role),
		matchId: toStringColumn(row.match_id),
		handNo: toInt(row.hand_no),
		ts: toInt(row.ts),
		kind: requireOneOf(toStringColumn(row.kind), EPISODE_KINDS, 'episodes.kind'),
		summary: toStringColumn(row.summary),
		salience: toFloat(row.salience, 0),
		expiresAt: toInt(row.expires_at)
	};
}

/**
 * The memories a persona brings to a new match, most salient first.
 *
 * Ranked in SQL by `salience * exp(-ageDays / 14)` — a grudge fades unless it is
 * reinforced. SQLite has no `EXP()` in its core build, so the decay is expressed
 * as the equivalent power of `0.5` over a half-life; ordering is all that matters
 * and this is monotonic in the same argument.
 */
export async function selectEpisodesForRole(
	db: ProfileDb,
	role: string,
	limit: number,
	now: number
): Promise<StoredEpisode[]> {
	const halfLifeMs = 14 * 24 * 60 * 60 * 1000;
	const rows = await db.execute<Record<string, unknown>>(
		`SELECT * FROM episodes
			WHERE role = ? AND expires_at > ?
			ORDER BY salience * POWER(0.5, (? - ts) / ?) DESC, ts DESC
			LIMIT ?`,
		role,
		now,
		now,
		halfLifeMs,
		limit
	);
	return rows.map((row) => rowToEpisode(row));
}

/** Every live memory, newest first — what Settings lists. */
export async function listEpisodes(
	db: ProfileDb,
	limit: number,
	now: number
): Promise<StoredEpisode[]> {
	const rows = await db.execute<Record<string, unknown>>(
		'SELECT * FROM episodes WHERE expires_at > ? ORDER BY ts DESC LIMIT ?',
		now,
		limit
	);
	return rows.map((row) => rowToEpisode(row));
}

/** Settings' per-episode "forget". Returns `true` when a row went away. */
export async function deleteEpisode(db: ProfileDb, episodeId: string): Promise<boolean> {
	const before = await db.execute<Record<string, unknown>>(
		'SELECT episode_id FROM episodes WHERE episode_id = ? LIMIT 1',
		episodeId
	);
	if (before.length === 0) return false;
	await db.execute('DELETE FROM episodes WHERE episode_id = ?', episodeId);
	return true;
}

/**
 * Drops expired memories.
 *
 * Retention is a property of the record (`expiresAt`), not of a cron's opinion,
 * so this is a sweep of rows that have already said they are finished.
 */
export async function pruneExpiredEpisodes(db: ProfileDb, now: number): Promise<void> {
	await db.execute('DELETE FROM episodes WHERE expires_at <= ?', now);
}
