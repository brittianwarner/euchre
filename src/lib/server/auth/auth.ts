/**
 * The Better Auth instance.
 *
 * **This runs inside the `authStore` Rivet actor, not in the SvelteKit request
 * path.** docs/06 §5.3 chose candidate (a2): Better Auth executes *inside* the
 * actor, over the stock Drizzle adapter bound to that actor's `c.db` SQLite, so
 * there is zero custom database-adapter code. Candidate (a1) — a hand-written
 * adapter that re-implements Better Auth's where-clause semantics — was
 * rejected because a subtly wrong `findOne` on `session` is a silent
 * authentication bug.
 *
 * The consequence, spelled out in docs/06 §5.4 and worth repeating because it
 * inverts the usual SvelteKit integration:
 *
 *   - `svelteKitHandler` is **not** used.
 *   - `sveltekitCookies(getRequestEvent)` is **not** used, and including it
 *     would throw: it exists to bridge cookies onto a SvelteKit request event,
 *     and there is no request event inside an actor.
 *   - `Set-Cookie` rides back on the actor's `Response`, which
 *     `src/routes/api/auth/[...all]/+server.ts` returns unmodified.
 *
 * `createAuth` is a factory rather than a module-level singleton because the
 * database only exists once an actor context does. `./auth-store.actor.ts`
 * memoises one instance per `c.db`.
 */

import { betterAuth } from 'better-auth';
import { drizzleAdapter, type DB } from 'better-auth/adapters/drizzle';
import { bearer, magicLink } from 'better-auth/plugins';
import { dev } from '$app/env';
import { sendMagicLinkEmail } from './email';
import {
	AUTH_BASE_PATH,
	MAGIC_LINK_EXPIRES_IN_SECONDS,
	SESSION_EXPIRES_IN_SECONDS,
	SESSION_UPDATE_AGE_SECONDS,
	getAppUrl,
	getTrustedOrigins,
	requireAuthSecret
} from './env';
import { authSchema } from './schema';

/**
 * Headers trusted to carry the client IP, most-trustworthy first.
 *
 * `x-vercel-forwarded-for` is set by Vercel's edge and cannot be spoofed by a
 * client; the other two are accepted only as a fallback for non-Vercel runs.
 * This matters because the rate limiter's bucket key is the IP — an attacker
 * who can choose their own bucket is not rate limited at all.
 */
const IP_ADDRESS_HEADERS = ['x-vercel-forwarded-for', 'x-forwarded-for', 'x-real-ip'];

/**
 * Build the Better Auth instance for one actor database.
 *
 * @param db The Drizzle client from `c.db` inside the `authStore` actor.
 */
export function createAuth(db: DB) {
	return betterAuth({
		appName: 'Euchre',
		secret: requireAuthSecret(),
		baseURL: getAppUrl(),
		basePath: AUTH_BASE_PATH,

		// CSRF: Better Auth rejects state-changing requests whose Origin is not
		// listed here, and refuses to redirect to an untrusted callbackURL.
		trustedOrigins: getTrustedOrigins(),

		database: drizzleAdapter(db, {
			provider: 'sqlite',
			schema: authSchema,
			// `c.db` is a single SQLite file behind an async proxy. Better Auth's
			// drizzle adapter only needs transactions on MySQL (where it emulates
			// `RETURNING`); on SQLite every write already uses `RETURNING`, so
			// wrapping each operation in a transaction buys nothing and adds a
			// round trip per call.
			transaction: false
		}),

		// Magic links only. No passwords to leak, no reset flow to abuse.
		emailAndPassword: { enabled: false },

		session: {
			expiresIn: SESSION_EXPIRES_IN_SECONDS,
			updateAge: SESSION_UPDATE_AGE_SECONDS,
			// Deliberately off. A signed cache cookie would save a SQLite read but
			// not the actor hop (only the actor holds the secret that verifies it),
			// so it buys almost nothing and costs up to `maxAge` of staleness after
			// a revocation.
			cookieCache: { enabled: false }
		},

		advanced: {
			// Vercel is HTTPS everywhere; `dev` is plain http on localhost, where a
			// `Secure` cookie would simply never be stored.
			useSecureCookies: !dev,
			defaultCookieAttributes: {
				httpOnly: true,
				sameSite: 'lax',
				path: '/'
			},
			ipAddress: { ipAddressHeaders: IP_ADDRESS_HEADERS }
		},

		/**
		 * Rate limiting, stored in the actor's SQLite.
		 *
		 * `storage: 'database'` rather than the default `'memory'` for two
		 * independent reasons: memory counters live in one function instance's
		 * heap (docs/06 §5.2 rejects that shape outright), and even inside the
		 * singleton actor they would reset on every serverless migration — which
		 * is precisely when a burst would sail through.
		 *
		 * The magic-link rules are the tight ones. Everything else gets a loose
		 * global bucket so that a signed-in browser polling `get-session` on
		 * navigation is never throttled.
		 */
		rateLimit: {
			enabled: true,
			storage: 'database',
			window: 60,
			max: 120,
			customRules: {
				// Five link requests per five minutes per IP. Above the plugin's own
				// default (5/60s) in window length so that retry-spamming a typo'd
				// address costs real time.
				'/sign-in/magic-link': { window: 300, max: 5 },
				// Verification is a GET a mail client may prefetch, so it is looser
				// than the request side but still bounded.
				'/magic-link/verify': { window: 300, max: 20 }
			}
		},

		// Better Auth phones home with anonymous usage data by default.
		telemetry: { enabled: false },

		plugins: [
			magicLink({
				expiresIn: MAGIC_LINK_EXPIRES_IN_SECONDS,
				// A database leak should not yield usable sign-in links.
				storeToken: 'hashed',
				// Plugin-level limit, applied in addition to `rateLimit.customRules`
				// above. Keeping both is deliberate: the plugin rule survives someone
				// later editing the global `rateLimit` block.
				rateLimit: { window: 60, max: 5 },
				/**
				 * NOTE — SafeLinks / Proofpoint.
				 *
				 * `url` here is the direct `…/api/auth/magic-link/verify?token=…`
				 * endpoint. Some corporate mail scanners prefetch links, which
				 * consumes a single-use token before the human clicks it. docs/06
				 * §5.1 records the fix — an `/auth/continue` interstitial that
				 * requires a real interaction and only then hits `verify`. That page
				 * lives under `src/routes/auth/**`, which this module does not own,
				 * so it is not wired here: rewriting `url` to point at a route that
				 * does not exist yet would break sign-in outright. When the
				 * interstitial lands, change exactly one thing — build
				 * `${appUrl}/auth/continue?to=${encodeURIComponent(url)}` and send
				 * that instead.
				 */
				sendMagicLink: async ({ email, url }) => {
					await sendMagicLinkEmail({ email, url });
				}
			}),

			/**
			 * Accept `Authorization: Bearer <session-token>` as well as the cookie.
			 *
			 * This is what lets `verifySessionToken()` in `./verify-session.ts`
			 * validate a raw session token from inside an actor, where there is no
			 * cookie jar. It does not weaken the browser path: the cookie is still
			 * `httpOnly`, and a bearer token is only useful to someone who already
			 * has it.
			 */
			bearer()
		]
	});
}

/** The concrete Better Auth instance type, for annotating cross-module values. */
export type Auth = ReturnType<typeof createAuth>;
