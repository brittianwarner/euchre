/**
 * The one door between the SvelteKit process and the `authStore` actor.
 *
 * Nothing outside this file may talk to `authStore` directly, because the
 * internal-token handshake (`./internal-token.ts`) is applied here and only
 * here. `src/routes/api/auth/[...all]/+server.ts` and `src/hooks.server.ts` both
 * go through it.
 *
 * The shape, from docs/06 §5.4:
 *
 * ```
 *   browser ──POST /api/auth/sign-in/magic-link──► SvelteKit route
 *                                                       │ .fetch(request)
 *                                                       ▼
 *                                          authStore ["auth"]  onRequest
 *                                            auth.handler(request)
 *                                            drizzle(c.db) ── SQLite
 *                                                       │
 *                                          Response (incl. Set-Cookie)
 *                                                       │
 *   browser ◄──────────── returned unmodified ──────────┘
 * ```
 */

import type { ActorHandle } from 'rivetkit/client';
import { getRivetClient } from '$lib/server/rivet';
import type { authStore } from './auth-store.actor';
import { AUTH_BASE_PATH, getAppUrl } from './env';
import { getAuthStoreInternalToken } from './internal-token';

/** The actor key. A singleton, as docs/06 §4.3 specifies. */
const AUTH_STORE_KEY = ['auth'];

/**
 * A handle to the singleton, carrying the internal-connection token.
 *
 * Explicitly annotated: 01 §5's circular-inference caveat applies to anything
 * that returns an actor handle, and it bites harder now that the registry and
 * the actors share SvelteKit's module graph.
 */
async function getAuthStoreHandle(): Promise<ActorHandle<typeof authStore>> {
	const internalToken = await getAuthStoreInternalToken();
	// authStore is not registered yet (M3). Cast keeps this module typechecking
	// while `/api/auth` remains unwired; runtime use still requires registry entry.
	const client = getRivetClient() as unknown as {
		authStore: {
			getOrCreate(
				key: string[],
				opts?: { params?: unknown }
			): ActorHandle<typeof authStore>;
		};
	};
	return client.authStore.getOrCreate(AUTH_STORE_KEY, {
		params: { internalToken }
	});
}

/**
 * Forward one `/api/auth/*` request to Better Auth inside the actor and return
 * its `Response` **unmodified**.
 *
 * Passing the whole `Request` (rather than a path string) matters: RivetKit
 * preserves the original URL through the
 * `x-rivet-internal-original-request-url` header only for `Request`/`URL`
 * inputs. With a bare string the actor would see `http://actor/request/...`,
 * Better Auth's `basePath` match would fail, and every route would 404.
 */
export async function forwardToAuthStore(request: Request): Promise<Response> {
	const handle = await getAuthStoreHandle();
	return handle.fetch(request);
}

/* -------------------------------------------------------------------------- */
/* Session lookup                                                             */
/* -------------------------------------------------------------------------- */

/** The user shape Better Auth returns from `GET /api/auth/get-session`. */
export interface AuthUser {
	readonly id: string;
	readonly email: string;
	readonly name: string;
	readonly emailVerified: boolean;
	readonly image?: string | null;
	/** ISO-8601 — JSON has no date type. */
	readonly createdAt: string;
	readonly updatedAt: string;
}

/**
 * The session shape, minus `token`.
 *
 * The raw session token is deliberately **not** part of this type. It is the
 * bearer credential for the account; it belongs in an `httpOnly` cookie and
 * nowhere else. Code that needs a credential for a Rivet connection mints a
 * short-lived, narrowly-scoped one instead — see `./actor-token.ts`.
 */
export interface AuthSession {
	readonly id: string;
	readonly userId: string;
	readonly expiresAt: string;
	readonly createdAt: string;
	readonly updatedAt: string;
	readonly ipAddress?: string | null;
	readonly userAgent?: string | null;
}

/** What `GET /api/auth/get-session` resolves to. */
export interface SessionResult {
	readonly session: AuthSession;
	readonly user: AuthUser;
}

/** A session lookup plus any cookies Better Auth wants refreshed. */
export interface SessionLookup {
	readonly result: SessionResult | null;
	/**
	 * `Set-Cookie` values from the lookup. Better Auth slides the session expiry
	 * once `updateAge` has elapsed and re-issues the cookie when it does; drop
	 * these and the browser's cookie quietly expires while the database row says
	 * it is still alive.
	 */
	readonly setCookie: readonly string[];
}

const EMPTY_LOOKUP: SessionLookup = { result: null, setCookie: [] };

/**
 * Resolve the session for an inbound request's headers.
 *
 * Never throws: a session lookup failing is a "you are signed out", not a 500
 * on an unrelated page. Failures are logged without any part of the cookie.
 *
 * @param headers The inbound request's headers. Only `cookie` and the
 *   forwarded-IP headers are meaningful, but all are passed so Better Auth's
 *   own `Origin`/IP logic behaves identically to a direct call.
 */
export async function lookupSession(headers: Headers): Promise<SessionLookup> {
	// Fast path: no cookie at all means no session, and skipping the hop keeps
	// the singleton (docs/06 §5.3) off the path of every anonymous page view.
	if (!headers.get('cookie')) return EMPTY_LOOKUP;

	const url = `${getAppUrl()}${AUTH_BASE_PATH}/get-session`;

	try {
		const handle = await getAuthStoreHandle();
		const response = await handle.fetch(new Request(url, { method: 'GET', headers }));

		if (!response.ok) return EMPTY_LOOKUP;

		const setCookie = readSetCookie(response.headers);
		const body: unknown = await response.json();

		return { result: asSessionResult(body), setCookie };
	} catch (cause) {
		console.error('[auth] session lookup failed', {
			message: cause instanceof Error ? cause.message : String(cause)
		});
		return EMPTY_LOOKUP;
	}
}

/** `Headers.getSetCookie()` where available, with a single-header fallback. */
function readSetCookie(headers: Headers): string[] {
	if (typeof headers.getSetCookie === 'function') return headers.getSetCookie();
	const single = headers.get('set-cookie');
	return single ? [single] : [];
}

/**
 * Narrow the JSON body without trusting it.
 *
 * Better Auth returns `null` for "no session". Anything that is not the exact
 * expected shape is treated as no session rather than partially believed.
 */
function asSessionResult(body: unknown): SessionResult | null {
	if (typeof body !== 'object' || body === null) return null;

	const candidate = body as { session?: unknown; user?: unknown };
	const session = candidate.session;
	const user = candidate.user;

	if (typeof session !== 'object' || session === null) return null;
	if (typeof user !== 'object' || user === null) return null;

	const userId = (user as { id?: unknown }).id;
	const sessionUserId = (session as { userId?: unknown }).userId;

	if (typeof userId !== 'string' || userId.length === 0) return null;
	if (typeof sessionUserId !== 'string' || sessionUserId !== userId) return null;

	return { session: session as AuthSession, user: user as AuthUser };
}
