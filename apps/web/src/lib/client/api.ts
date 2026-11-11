/**
 * Typed calls to the Rust server's `/api/*`.
 *
 * The web app is a static SPA (D4): every former `+page.server.ts` load and form
 * action is one of these calls, made from the browser with the browser's own
 * cookies. Loads pass SvelteKit's `fetch`; components use the global one.
 *
 * Every error body is `{ message, error: { code, message } }`. A failed call
 * throws {@link ApiError} carrying the status and the server's message.
 */
import type { GameStatus, MatchListPage } from '#lib/protocol/index.ts';
import type {
	ClientMatchReplay,
	ProfileSettings,
	ProfileStats,
	SettingsPatch
} from '#lib/protocol/profile.ts';

type Fetch = typeof fetch;

/** The email nickname this browser plays as. */
export interface Identity {
	readonly email: string;
	readonly userId: string;
}

/** `GET /api/session`. The cookies are HttpOnly, so this is how the page learns them. */
export interface SessionInfo {
	readonly guestUserId: string | null;
	readonly identity: Identity | null;
}

/** One saved table from the recent-tables cookie, with its live summary when it answered. */
export interface RecentTable {
	readonly gameId: string;
	readonly status: GameStatus | null;
	readonly score: readonly [number, number] | null;
	readonly handNo: number | null;
}

/** `GET /api/tables/recent`. */
export interface RecentTables {
	readonly tables: readonly RecentTable[];
	readonly unavailable: boolean;
}

/** Which profile a scrapbook read uses: the browser guest, or the email identity. */
export type ProfileChoice = 'browser' | 'email';

/** A non-2xx answer from `/api/*`. */
export class ApiError extends Error {
	readonly status: number;
	readonly code: string | undefined;

	constructor(params: { status: number; message: string; code?: string }) {
		super(params.message);
		this.name = 'ApiError';
		this.status = params.status;
		this.code = params.code;
	}
}

interface CallOptions {
	readonly fetch?: Fetch;
	readonly method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
	readonly body?: unknown;
}

async function call<T>(path: string, options: CallOptions = {}): Promise<T> {
	const run = options.fetch ?? fetch;
	const response = await run(path, {
		method: options.method ?? 'GET',
		credentials: 'same-origin',
		headers:
			options.body === undefined
				? { Accept: 'application/json' }
				: { Accept: 'application/json', 'Content-Type': 'application/json' },
		body: options.body === undefined ? undefined : JSON.stringify(options.body)
	});
	if (!response.ok) {
		let message = response.statusText || 'Request failed';
		let code: string | undefined;
		try {
			const body = (await response.json()) as {
				message?: string;
				error?: { code?: string; message?: string };
			};
			message = body.error?.message ?? body.message ?? message;
			code = body.error?.code;
		} catch {
			/* A plain-text body (for example `421 Unknown host`) keeps the status text. */
		}
		throw new ApiError({ status: response.status, message, code });
	}
	if (response.status === 204) return undefined as T;
	return (await response.json()) as T;
}

function query(params: Record<string, string | number | null | undefined>): string {
	const search = new URLSearchParams();
	for (const [key, value] of Object.entries(params)) {
		if (value !== null && value !== undefined && value !== '') search.set(key, String(value));
	}
	const text = search.toString();
	return text ? `?${text}` : '';
}

/** Who this browser is: its guest id (if it has played) and its email identity (if set). */
export function getSession(fetchFn?: Fetch): Promise<SessionInfo> {
	return call('/api/session', { fetch: fetchFn });
}

/** "This is me": remember `email` as this browser's identity (sets the HttpOnly cookie). */
export function identify(email: string): Promise<{ identity: Identity }> {
	return call('/api/identity/identify', { method: 'POST', body: { email } });
}

/** "Not you? Switch profile": forget this browser's identity. */
export function signOut(): Promise<void> {
	return call('/api/identity', { method: 'DELETE' });
}

/** Create a table for this browser (minting its guest cookie if needed). */
export function createTable(params: {
	fetch?: Fetch;
	stick: boolean;
	natural: boolean;
}): Promise<{ gameId: string }> {
	return call('/api/tables', {
		fetch: params.fetch,
		method: 'POST',
		body: { stick: params.stick, natural: params.natural }
	});
}

/** The tables this browser has open, newest first. */
export function getRecentTables(fetchFn?: Fetch): Promise<RecentTables> {
	return call('/api/tables/recent', { fetch: fetchFn });
}

/** Lifetime stats for the email identity. */
export function getStats(fetchFn?: Fetch): Promise<ProfileStats> {
	return call('/api/profile/stats', { fetch: fetchFn });
}

/** One page of a scrapbook. */
export function listGames(params: {
	fetch?: Fetch;
	profile: ProfileChoice;
	limit: number;
	cursor?: string | null;
}): Promise<MatchListPage> {
	return call(
		`/api/games${query({ profile: params.profile, limit: params.limit, cursor: params.cursor })}`,
		{ fetch: params.fetch }
	);
}

/** One match, redacted for a browser. A missing match is `ApiError` 404. */
export function getReplay(params: {
	fetch?: Fetch;
	matchId: string;
	profile: ProfileChoice;
}): Promise<ClientMatchReplay> {
	return call(
		`/api/games/${encodeURIComponent(params.matchId)}${query({ profile: params.profile })}`,
		{ fetch: params.fetch }
	);
}

/** The email identity's settings. */
export function getSettings(fetchFn?: Fetch): Promise<ProfileSettings> {
	return call('/api/profile/settings', { fetch: fetchFn });
}

/** Save a settings patch; the actor validates and clamps every field and returns the result. */
export function saveSettings(patch: SettingsPatch): Promise<ProfileSettings> {
	return call('/api/profile/settings', { method: 'PUT', body: patch });
}
