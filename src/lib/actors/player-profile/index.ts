/**
 * `playerProfile` — key `['user', userId]`. **One actor per human**, and the only
 * durable home for anything that outlives a single match.
 *
 * This file is pure assembly: every SQL statement lives in `./repository`, every
 * migration in `./schema`, every settings validator in `./settings`, every runtime
 * guard and text hygiene rule in `./guards`, every token check in `./auth`, and
 * every local shape in `./types`. Nothing here talks to SQLite directly and
 * nothing here re-derives a rule those modules already own — this actor decides
 * *when* those functions run and *who* is allowed to trigger them, and that is all.
 *
 * ## Partitioning
 *
 * The actor's key **is** the access-control list: `ownerOf(c.key)` is the only
 * source of a user id anywhere in this module, so "only the owning user may read
 * or write" is a property of the key, not a check that can be forgotten on one
 * action. A connection's JWT `sub` must equal the key's user id or `createConnState`
 * throws before anything else runs.
 *
 * ## Two authorization boundaries, never confused
 *
 * 1. **A human connection.** `ProfileConnectParams.token` is a short-lived JWT,
 *    verified in `createConnState` against `verifyProfileToken`. Every
 *    browser-facing action below demands `c.conn.state.authenticated`.
 * 2. **An actor-to-actor call.** `euchreTable` and `aiSeat` do not hold a JWT for
 *    this profile and cannot get one — they carry the deployment-wide
 *    `EUCHRE_INTERNAL_TOKEN` instead, checked by `assertInternalToken` before any
 *    state or database read. The five internal queues below are guarded the same
 *    way `euchreTable`'s are: `canPublish` fails open for a forged, connectionless
 *    publish (`docs/01-ARCHITECTURE.md` §3.2), so the token compare — not the
 *    guard — is what actually stops a forgery.
 *
 * ## Why writes are queues and reads are actions
 *
 * Every mutation below is idempotent by construction in `./repository` (`ON
 * CONFLICT DO NOTHING`, or a guarded `UPDATE ... WHERE ended_at IS NULL`), so
 * ordering is not a correctness requirement the way it is for `euchreTable`'s
 * trick-by-trick state machine. It is still routed through queues rather than
 * actions: a queue message is durable and redelivered if this actor dies mid
 * write, an action is not. Reads (`getSettings`, `listGames`, `getReplay`, …) have
 * nothing to redeliver and stay actions, which also keeps them free of the run
 * loop's single-lane serialization.
 */

import { actor, queue } from 'rivetkit';
import { db } from 'rivetkit/db';
import { PROTOCOL_VERSION } from '$lib/protocol';
import { assertInternalToken, ownerOf, verifyProfileToken } from './auth';
import { protocolError, requireId } from './guards';
import {
	clampLimit,
	computeStats,
	deleteEpisode,
	insertEpisodes,
	insertHands,
	listEpisodes,
	listMatches,
	parseEpisodes,
	parseHands,
	parseMatchRecord,
	pruneExpiredEpisodes,
	readClientReplay,
	readServerReplay,
	selectEpisodesForRole,
	upsertMatch
} from './repository';
import { applyMigrations, type ProfileDb } from './schema';
import {
	applySettingsPatch,
	cloneSettings,
	defaultSettings,
	normalizeSettings,
	personaView
} from './settings';
import { AI_SEATS } from './types';
import type {
	ClientMatchReplay,
	DailyTokenUsage,
	LifetimeStatsCache,
	ListGamesRequest,
	ProfileActorState,
	ProfileConnState,
	ProfileConnectParams,
	ProfileSettings,
	ProfileStats,
	RecordEpisodesMessage,
	RecordGameMessage,
	RecordHandsMessage,
	RecordTokensMessage,
	ServerMatchReplay,
	SetActiveGameMessage,
	StoredEpisode
} from './types';
import type { MatchListPage, PersonaView } from '$lib/protocol';

/* ========================================================================== */
/* Small pure helpers                                                        */
/* ========================================================================== */

/** `YYYY-MM-DD` in UTC — the daily token bucket's rollover key. */
function utcDay(now: number): string {
	return new Date(now).toISOString().slice(0, 10);
}

function emptyDailyTokens(now: number): DailyTokenUsage {
	return { tokensIn: 0, tokensOut: 0, cacheReadTokens: 0, calls: 0, day: utcDay(now) };
}

function emptyLifetime(now: number): LifetimeStatsCache {
	return {
		matches: 0,
		won: 0,
		lost: 0,
		abandoned: 0,
		euchresFor: 0,
		euchresAgainst: 0,
		lonersMade: 0,
		handsPlayed: 0,
		updatedAt: now
	};
}

/** The cached lifetime slice of a freshly computed {@link ProfileStats}. */
function lifetimeFromStats(stats: ProfileStats): LifetimeStatsCache {
	return {
		matches: stats.matches,
		won: stats.won,
		lost: stats.lost,
		abandoned: stats.abandoned,
		euchresFor: stats.euchresFor,
		euchresAgainst: stats.euchresAgainst,
		lonersMade: stats.lonersMade,
		handsPlayed: stats.handsPlayed,
		updatedAt: stats.updatedAt
	};
}

/** A non-negative integer, or `0` for anything else. Guards the token counters. */
function nonNegativeInt(raw: unknown): number {
	return typeof raw === 'number' && Number.isFinite(raw) && raw > 0 ? Math.trunc(raw) : 0;
}

/** Constant-time token check that reports pass/fail instead of throwing. */
async function tokenOk(token: unknown): Promise<boolean> {
	try {
		await assertInternalToken(token);
		return true;
	} catch {
		return false;
	}
}

/** Every browser-facing action demands this. `c.conn.state` is the sole source. */
function requireAuthenticated(state: ProfileConnState): void {
	if (!state.authenticated) {
		throw protocolError('forbidden', 'this connection is not authenticated');
	}
}

/* ========================================================================== */
/* The actor↔actor authorization boundary (queues)                           */
/* ========================================================================== */

/** The shape a `canPublish` hook receives. Declared structurally; see `queues.ts` in `euchre-table`. */
interface Guard {
	readonly conn?: { readonly state: ProfileConnState };
}

/**
 * Reject anything arriving over a live, connected browser.
 *
 * Fails open for the stateless HTTP handle every actor-to-actor call actually
 * uses (`c.conn === undefined`) — defence in depth on top of
 * {@link assertInternalToken}, never instead of it. See
 * `euchre-table/queues.ts`'s identical `externalDenied` for the full rationale.
 */
function externalDenied(c: Guard): boolean {
	return c.conn === undefined;
}

const profileQueues = {
	/** `euchreTable` → here: upsert a match row, and its hands if bundled. */
	recordGame: queue<RecordGameMessage, undefined, Guard>({ canPublish: externalDenied }),
	/** `euchreTable` → here: journal hands at a hand boundary. */
	recordHands: queue<RecordHandsMessage, undefined, Guard>({ canPublish: externalDenied }),
	/** `aiSeat` → here: flush a buffered episode set at hand end. */
	recordEpisodes: queue<RecordEpisodesMessage, undefined, Guard>({ canPublish: externalDenied }),
	/** `euchreTable` → here: which match `/play` should resume. */
	setActiveGame: queue<SetActiveGameMessage, undefined, Guard>({ canPublish: externalDenied }),
	/** Any seat's meter → here: add today's spend. */
	recordTokens: queue<RecordTokensMessage, undefined, Guard>({ canPublish: externalDenied })
};

/* ========================================================================== */
/* The context, structurally (helpers called from the run loop)              */
/* ========================================================================== */

/**
 * The narrow slice of the actor context the queue handlers below use.
 *
 * Hand-declared rather than `ActionContextOf<typeof playerProfile>` because that
 * would be circular here too: these helpers are called from inside `run`, which
 * is part of the definition this type would have to reference.
 */
interface ProfileCtx {
	state: ProfileActorState;
	readonly db: ProfileDb;
	readonly key: readonly string[];
	readonly log: {
		info(...args: unknown[]): void;
		warn(...args: unknown[]): void;
		error(...args: unknown[]): void;
	};
	saveState(opts?: { immediate?: boolean }): Promise<void>;
}

/* ========================================================================== */
/* Queue handlers — the only writers in this actor                           */
/* ========================================================================== */

async function onRecordGame(c: ProfileCtx, body: RecordGameMessage): Promise<void> {
	if (!(await tokenOk(body.internalToken))) {
		c.log.warn('dropped forged recordGame');
		return;
	}
	const owner = ownerOf(c.key);
	const now = Date.now();
	const record = parseMatchRecord(body.record, owner, now);
	await upsertMatch(c.db, record, now);
	if (body.hands !== undefined) {
		await insertHands(c.db, parseHands(body.hands, record.matchId));
	}
	c.state.lifetime = lifetimeFromStats(await computeStats(c.db, now));
	await c.saveState({ immediate: true });
}

async function onRecordHands(c: ProfileCtx, body: RecordHandsMessage): Promise<void> {
	if (!(await tokenOk(body.internalToken))) {
		c.log.warn('dropped forged recordHands');
		return;
	}
	const matchId = requireId(body.matchId, 'matchId');
	await insertHands(c.db, parseHands(body.hands, matchId));
}

async function onRecordEpisodes(c: ProfileCtx, body: RecordEpisodesMessage): Promise<void> {
	if (!(await tokenOk(body.internalToken))) {
		c.log.warn('dropped forged recordEpisodes');
		return;
	}
	await insertEpisodes(c.db, parseEpisodes(body.episodes, Date.now()));
}

async function onSetActiveGame(c: ProfileCtx, body: SetActiveGameMessage): Promise<void> {
	if (!(await tokenOk(body.internalToken))) {
		c.log.warn('dropped forged setActiveGame');
		return;
	}
	c.state.activeGameId = body.gameId;
	await c.saveState({ immediate: true });
}

async function onRecordTokens(c: ProfileCtx, body: RecordTokensMessage): Promise<void> {
	if (!(await tokenOk(body.internalToken))) {
		c.log.warn('dropped forged recordTokens');
		return;
	}
	const now = Date.now();
	const day = utcDay(now);
	if (c.state.dailyTokens.day !== day) {
		c.state.dailyTokens = emptyDailyTokens(now);
	}
	c.state.dailyTokens.tokensIn += nonNegativeInt(body.tokensIn);
	c.state.dailyTokens.tokensOut += nonNegativeInt(body.tokensOut);
	c.state.dailyTokens.cacheReadTokens += nonNegativeInt(body.cacheReadTokens);
	c.state.dailyTokens.calls += nonNegativeInt(body.calls);
	await c.saveState({ immediate: true });
}

/** One message off the durable queue. Mirrors `euchre-table`'s `TableMessage`. */
type ProfileMessage =
	| { readonly name: 'recordGame'; readonly body: RecordGameMessage; complete(): Promise<void> }
	| { readonly name: 'recordHands'; readonly body: RecordHandsMessage; complete(): Promise<void> }
	| { readonly name: 'recordEpisodes'; readonly body: RecordEpisodesMessage; complete(): Promise<void> }
	| { readonly name: 'setActiveGame'; readonly body: SetActiveGameMessage; complete(): Promise<void> }
	| { readonly name: 'recordTokens'; readonly body: RecordTokensMessage; complete(): Promise<void> };

/* ========================================================================== */
/* The actor                                                                  */
/* ========================================================================== */

export const playerProfile = actor({
	options: {
		name: 'Player Profile',
		icon: 'id-badge'
	},

	db: db({ onMigrate: applyMigrations }),
	queues: profileQueues,

	/**
	 * A brand-new profile. `ownerOf(c.key)` is called for its side effect alone —
	 * a malformed key (anything but `['user', userId]`) fails at creation rather
	 * than on the first read.
	 */
	createState: (c): ProfileActorState => {
		ownerOf(c.key);
		const now = Date.now();
		return {
			schema: 1,
			activeGameId: null,
			dailyTokens: emptyDailyTokens(now),
			lifetime: emptyLifetime(now),
			settings: defaultSettings(now)
		};
	},

	/**
	 * Authenticate, and authorize against the actor's own key.
	 *
	 * `authenticated: false` (no token supplied) is not rejected here — it is the
	 * shape of a connectionless internal action call, which authorizes itself with
	 * an explicit `internalToken` argument instead (see `getServerReplay`,
	 * `getEpisodesForRole`). Every *browser-facing* action separately demands
	 * `authenticated: true`.
	 */
	createConnState: async (
		c,
		params: ProfileConnectParams | undefined
	): Promise<ProfileConnState> => {
		if (params?.protocolVersion !== undefined && params.protocolVersion !== PROTOCOL_VERSION) {
			throw protocolError('protocol_version_mismatch', 'unsupported protocol version');
		}
		const owner = ownerOf(c.key);
		const token = params?.token;
		if (typeof token !== 'string' || token.length === 0) {
			return { authenticated: false, userId: null, since: Date.now() };
		}
		const claims = await verifyProfileToken(token, Date.now());
		if (claims.userId !== owner) {
			throw protocolError('forbidden', 'token does not belong to this profile');
		}
		return { authenticated: true, userId: owner, since: Date.now() };
	},

	/**
	 * The single serialized writer, exactly like `euchreTable`'s. Settings are
	 * repaired on every wake — `c.state` is deserialized whole, including state a
	 * previous deployment wrote, and {@link normalizeSettings} is idempotent on an
	 * already-valid value.
	 */
	run: async (c): Promise<void> => {
		c.state.settings = normalizeSettings(c.state.settings, Date.now());

		const messages = c.queue.iter({ completable: true }) as AsyncIterable<ProfileMessage>;
		for await (const message of messages) {
			try {
				switch (message.name) {
					case 'recordGame':
						await onRecordGame(c, message.body);
						break;
					case 'recordHands':
						await onRecordHands(c, message.body);
						break;
					case 'recordEpisodes':
						await onRecordEpisodes(c, message.body);
						break;
					case 'setActiveGame':
						await onSetActiveGame(c, message.body);
						break;
					case 'recordTokens':
						await onRecordTokens(c, message.body);
						break;
				}
			} catch (err) {
				c.log.error('playerProfile queue handler failed', {
					name: message.name,
					err: String(err).slice(0, 200)
				});
			}
			await message.complete();
		}
	},

	actions: {
		/* -- Browser-facing reads and writes. All demand an authenticated conn. -- */

		getSettings: (c): ProfileSettings => {
			requireAuthenticated(c.conn.state);
			return cloneSettings(c.state.settings);
		},

		updateSettings: (c, patch: unknown): ProfileSettings => {
			requireAuthenticated(c.conn.state);
			c.state.settings = applySettingsPatch(c.state.settings, patch, Date.now());
			return cloneSettings(c.state.settings);
		},

		getPersonas: (c): PersonaView[] => {
			requireAuthenticated(c.conn.state);
			return c.state.settings.personas.map((p, i) => personaView(p, AI_SEATS[i]));
		},

		getActiveGame: (c): string | null => {
			requireAuthenticated(c.conn.state);
			return c.state.activeGameId;
		},

		getStats: async (c): Promise<ProfileStats> => {
			requireAuthenticated(c.conn.state);
			return computeStats(c.db, Date.now());
		},

		listGames: async (c, req?: ListGamesRequest): Promise<MatchListPage> => {
			requireAuthenticated(c.conn.state);
			return listMatches(c.db, clampLimit(req?.limit), req?.cursor ?? null);
		},

		getReplay: async (c, matchId: unknown): Promise<ClientMatchReplay | null> => {
			requireAuthenticated(c.conn.state);
			return readClientReplay(c.db, requireId(matchId, 'matchId'));
		},

		listEpisodes: async (c, limit?: unknown): Promise<StoredEpisode[]> => {
			requireAuthenticated(c.conn.state);
			return listEpisodes(c.db, clampLimit(limit), Date.now());
		},

		forgetEpisode: async (c, episodeId: unknown): Promise<boolean> => {
			requireAuthenticated(c.conn.state);
			return deleteEpisode(c.db, requireId(episodeId, 'episodeId'));
		},

		/* -- Internal-only. Guarded by an explicit `internalToken` argument, --   */
		/* -- never by `c.conn`, because these are always connectionless calls. -- */

		getServerReplay: async (
			c,
			internalToken: unknown,
			matchId: unknown
		): Promise<ServerMatchReplay | null> => {
			await assertInternalToken(internalToken);
			return readServerReplay(c.db, requireId(matchId, 'matchId'));
		},

		getEpisodesForRole: async (
			c,
			internalToken: unknown,
			role: unknown,
			limit?: unknown
		): Promise<StoredEpisode[]> => {
			await assertInternalToken(internalToken);
			return selectEpisodesForRole(c.db, requireId(role, 'role'), clampLimit(limit), Date.now());
		},

		pruneEpisodes: async (c, internalToken: unknown): Promise<void> => {
			await assertInternalToken(internalToken);
			await pruneExpiredEpisodes(c.db, Date.now());
		}
	}
});
