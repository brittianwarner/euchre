/**
 * Auth environment.
 *
 * Every environment variable the auth subsystem reads is read *here* and
 * nowhere else, so that a missing variable produces one legible failure at one
 * call site instead of `undefined` leaking three modules deep.
 *
 * Read through `$env/dynamic/private` rather than `$env/static/private` so the
 * same Vercel build can run in preview and production without a rebuild. Every
 * consumer of this module runs on the server:
 *
 *   - SvelteKit hooks / routes (`src/hooks.server.ts`, `src/routes/api/auth/**`)
 *   - the `authStore` Rivet actor, which is mounted **inside this same
 *     deployment** at `/api/rivet/*` — so `$env/dynamic/private` resolves there
 *     too. Actors are not a separate process (docs/06 §2).
 *
 * Nothing in this file throws at module-evaluation time. Throwing at import
 * would take down `vite build`, prerendering and `svelte-kit sync`, which all
 * evaluate server modules with an empty environment.
 */

import { building, dev } from '$app/env';
import * as env from '$app/env/private';

/**
 * Development fallback for `BETTER_AUTH_SECRET`.
 *
 * Deliberately a fixed, obviously-fake string: it keeps `bun run dev` working
 * with a completely empty `.env`, and it is impossible to mistake for a real
 * secret in a log or a diff. {@link requireAuthSecret} refuses to hand it back
 * outside dev.
 */
const DEV_ONLY_SECRET = 'dev-only-insecure-better-auth-secret-do-not-deploy';

/** Local dev origin. Vite's default port, matching the rest of the repo. */
const DEV_ORIGIN = 'http://localhost:5173';

/**
 * Session lifetime. Seven days, refreshed once a day of use (see
 * {@link SESSION_UPDATE_AGE_SECONDS}).
 *
 * Not covered by invariant 22 ("no number frozen before it is measured") — that
 * invariant governs *tempo* and *latency* numbers, which are measured against a
 * running system. A session expiry is a policy choice, not a measurement.
 */
export const SESSION_EXPIRES_IN_SECONDS = 60 * 60 * 24 * 7;

/** Slide the session expiry at most once a day, to avoid a write per request. */
export const SESSION_UPDATE_AGE_SECONDS = 60 * 60 * 24;

/** Magic links are single-use and die in ten minutes. */
export const MAGIC_LINK_EXPIRES_IN_SECONDS = 60 * 10;

/**
 * The Better Auth mount path. Both the SvelteKit catch-all route and the
 * `betterAuth({ basePath })` inside the actor must agree on this exact string,
 * or every `Set-Cookie` path and every callback URL is subtly wrong.
 */
export const AUTH_BASE_PATH = '/api/auth';

/**
 * The canonical origin of this deployment, used as Better Auth's `baseURL` and
 * as the base for magic-link URLs.
 *
 * Precedence: `BETTER_AUTH_URL` → `PUBLIC_APP_URL` → Vercel's own
 * `VERCEL_PROJECT_PRODUCTION_URL` → the dev origin. The Vercel fallback is what
 * keeps preview deployments working without per-preview configuration.
 */
export function getAppUrl(): string {
	const explicit = env.BETTER_AUTH_URL || env.PUBLIC_APP_URL;
	if (explicit) return stripTrailingSlash(explicit);

	const vercelHost = env.VERCEL_PROJECT_PRODUCTION_URL || env.VERCEL_URL;
	if (vercelHost) return `https://${stripTrailingSlash(vercelHost)}`;

	return DEV_ORIGIN;
}

/**
 * Origins allowed to drive auth: CSRF `Origin` checks, magic-link redirect
 * targets, and (for the game actors) the WebSocket origin allowlist.
 *
 * `ALLOWED_ORIGINS` is a comma-separated list. {@link getAppUrl} is always
 * included, so the common case needs no configuration at all.
 */
export function getTrustedOrigins(): string[] {
	const configured = (env.ALLOWED_ORIGINS ?? '')
		.split(',')
		.map((o) => stripTrailingSlash(o.trim()))
		.filter((o) => o.length > 0);

	const origins = new Set<string>([getAppUrl(), ...configured]);

	// Vercel preview deployments get a fresh hostname per commit. Without this,
	// signing in on a preview URL fails the origin check.
	if (env.VERCEL_URL) origins.add(`https://${stripTrailingSlash(env.VERCEL_URL)}`);
	if (dev) origins.add(DEV_ORIGIN);

	return [...origins];
}

/**
 * The signing secret, for Better Auth *and* for the actor connection tokens in
 * `./actor-token.ts`.
 *
 * In dev with no `.env` it returns {@link DEV_ONLY_SECRET} so sign-in works out
 * of the box. In any non-dev runtime it throws, because a predictable secret
 * means forgeable sessions and forgeable connection tokens.
 */
export function requireAuthSecret(): string {
	const secret = env.BETTER_AUTH_SECRET;
	if (secret && secret.length > 0) return secret;

	if (dev || building) return DEV_ONLY_SECRET;

	throw new Error(
		'BETTER_AUTH_SECRET is unset. Sessions and Rivet connection tokens are signed ' +
			'with it; without it they are forgeable. Generate one with `openssl rand -base64 32`.'
	);
}

/** The Resend API key, or `undefined` — see `./email.ts` for the dev fallback. */
export function getResendApiKey(): string | undefined {
	return env.RESEND_API_KEY || undefined;
}

/** `From:` header for magic-link mail. Must be a Resend-verified sender. */
export function getEmailFrom(): string {
	return env.EMAIL_FROM || 'Euchre <onboarding@resend.dev>';
}

/**
 * True when it is safe to print a magic link to the server console.
 *
 * Console delivery is a *development* convenience. In production a link in the
 * log is a full account takeover for anyone with log access, so it is gated on
 * `dev` and never on "the API key happens to be missing".
 */
export function canLogMagicLink(): boolean {
	return dev;
}

function stripTrailingSlash(value: string): string {
	return value.endsWith('/') ? value.slice(0, -1) : value;
}
