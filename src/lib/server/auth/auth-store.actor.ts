/**
 * `authStore` — the singleton actor keyed `["auth"]` that owns every Better
 * Auth table.
 *
 * This is docs/06 §5.3's decision, candidate (a2): **Better Auth runs inside
 * the actor**, over the stock Drizzle adapter bound to this actor's `c.db`
 * SQLite. There is no Postgres, no Turso, no `DATABASE_URL`, and no custom
 * database adapter. Users, sessions, magic-link verifications and rate-limit
 * counters all live in Rivet's durable storage, so they survive a serverless
 * function migration — which is the property docs/06 §5.2 candidate (c)
 * (`memoryAdapter`) fails, and fails silently.
 *
 * ## Storage policy (docs/06 §4.3)
 *
 * | tier | contents |
 * |---|---|
 * | `c.state` | `{ schema: 1 }` and nothing else. |
 * | `c.kv` | nothing. |
 * | `c.db` | `user`, `session`, `account`, `verification`, `rateLimit`. |
 *
 * ## Reaching it
 *
 * Only from our own server code, via `./gateway.ts`:
 *
 * ```
 * browser → /api/auth/*  →  SvelteKit route  →  client.authStore
 *                                                .getOrCreate(['auth'], { params })
 *                                                .fetch(request)
 *                                            →  onRequest → auth.handler(request)
 * ```
 *
 * The `Response` — `Set-Cookie` and all — is returned to the browser unmodified.
 * `svelteKitHandler` and `sveltekitCookies` are deliberately absent; see
 * `./auth.ts`.
 *
 * ## The singleton
 *
 * Named honestly, as docs/06 §5.3 requires: this is the one global hot spot in
 * a system whose scaling story is "every key is sharded by `gameId` or
 * `userId`". It is tolerable because it is never on the realtime path — a
 * browser connecting to `euchreTable` verifies a token with local crypto
 * (`./verify-session.ts`) and touches this actor not at all.
 */

import { actor, UserError } from 'rivetkit';
import { db as drizzleDatabase } from 'rivetkit/db/drizzle';
import { createAuth, type Auth } from './auth';
import { timingSafeEqual } from './crypto';
import {
	getAuthStoreInternalToken,
	type AuthStoreConnParams,
	type AuthStoreConnState
} from './internal-token';
import { runAuthMigrations } from './migrations';
import { authSchema } from './schema';

/**
 * One Better Auth instance per database handle.
 *
 * Not `c.vars`: `createVars` runs before the database client is available, and
 * a `WeakMap` keyed on the client object gets the same per-wake lifetime for
 * free — a new wake produces a new client, so a stale instance can never be
 * reused against a closed database.
 */
const instances = new WeakMap<object, Auth>();

function getAuth(db: object): Auth {
	const existing = instances.get(db);
	if (existing) return existing;
	const created = createAuth(db);
	instances.set(db, created);
	return created;
}

export const authStore = actor({
	options: {
		name: 'AuthStore',
		icon: 'shield-check'
	},

	/** Deliberately tiny. Everything real is in `c.db`. */
	state: { schema: 1 },

	db: drizzleDatabase({
		schema: authSchema,
		// Idempotent `CREATE TABLE IF NOT EXISTS` DDL, run inside RivetKit's
		// migration savepoint. See the spec note at the top of `./migrations.ts`
		// for why this is not a `drizzle-kit` bundle.
		onMigrate: async (client) => {
			await runAuthMigrations(client);
		}
	}),

	/**
	 * The only gate on this actor.
	 *
	 * `authStore` is browser-*addressable* through the Rivet gateway even though
	 * no browser has any business talking to it, so per 00 §3 invariant 16 it
	 * declares the hook and refuses. The refusal is a constant-time compare
	 * against a token derived from `BETTER_AUTH_SECRET` — see the spec note in
	 * `./internal-token.ts` for why an unconditional `throw` would not work.
	 */
	onBeforeConnect: async (_c, params: unknown): Promise<void> => {
		const supplied =
			typeof params === 'object' && params !== null && 'internalToken' in params
				? (params as AuthStoreConnParams).internalToken
				: '';

		const expected = await getAuthStoreInternalToken();

		if (typeof supplied !== 'string' || !timingSafeEqual(supplied, expected)) {
			// No detail in the message: a caller that does not already hold the
			// token learns nothing about why it failed.
			throw new UserError('Forbidden', { code: 'forbidden' });
		}
	},

	createConnState: (): AuthStoreConnState => ({ internal: true }),

	/**
	 * Every `/api/auth/*` request, handled by Better Auth against `c.db`.
	 *
	 * `request` arrives with its **original** URL and headers intact — the
	 * client's `.fetch()` preserves them via RivetKit's
	 * `x-rivet-internal-original-request-url` header — which is what lets Better
	 * Auth match its own `basePath`, run its `Origin` check, and read the client
	 * IP for rate limiting.
	 */
	onRequest: async (c, request: Request): Promise<Response> => {
		return getAuth(c.db).handler(request);
	},

	/**
	 * No browser-facing actions. Better Auth's HTTP surface is the whole API;
	 * adding a bespoke action would be a second, unaudited way into the session
	 * table.
	 */
	actions: {}
});
