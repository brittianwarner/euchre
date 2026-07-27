/**
 * Lightweight per-browser identity for `playerProfile`.
 *
 * **Deliberately not a login.** The product decision (see the assignment this
 * shipped under) was to deprioritise magic-link auth for `/games` and
 * `/settings`: the user simply tells us the email they want their scrapbook
 * remembered under, we derive a stable id from it (`lowercase, trim, sha256`)
 * and use that as the `playerProfile` actor key `['user', userId]`. Nothing
 * here proves the email belongs to the person typing it — it is a nickname
 * for "this browser's history", not an account, and the copy shown to the
 * user says so.
 *
 * `playerProfile` still demands a verified HS256 token on every connection
 * (`verifyProfileToken` in `$lib/actors/player-profile/auth.ts`). This module
 * is that token's *signing* side. The secret lookup below mirrors that
 * module's private `jwtSecret()` byte for byte (same env vars, same
 * dev-only fallback) because there is no exported seam to share — the two
 * sides must simply agree, the way a signer and verifier always do.
 *
 * Every `playerProfile` call in `/games` and `/settings` goes through
 * {@link connectProfile}, which mints a fresh token and hands it to the
 * stateless Rivet client as connection params — there is no long-lived
 * WebSocket to these two pages, just short authenticated HTTP action calls,
 * which is all a scrapbook and a settings form ever need.
 */

import type { Cookies } from '@sveltejs/kit';
import { dev } from '$app/environment';
import { env } from '$env/dynamic/private';
import { getRivetClient } from '$lib/server/rivet';

/** Cookie carrying the normalized email. `httpOnly` — read only from `load`/actions. */
export const IDENTITY_COOKIE = 'euchre_email';

/** ~400 days: long enough that "remember me" never surprises anyone by expiring. */
const COOKIE_MAX_AGE_SECONDS = 60 * 60 * 24 * 400;

/** Comfortably under `verifyProfileToken`'s one-hour hard cap. */
const TOKEN_TTL_SECONDS = 30 * 60;

const JWT_AUDIENCE = 'euchre-actors';

/** Mirrors `player-profile/auth.ts`'s `DEV_ONLY_SECRET` exactly. */
const DEV_ONLY_SECRET = 'euchre-dev-insecure-secret';

/** True when there is no Rivet Cloud endpoint, i.e. local development. */
function isLocalDev(): boolean {
	return !env.RIVET_ENDPOINT;
}

/** Mirrors `player-profile/auth.ts`'s private `jwtSecret()` lookup order. */
function jwtSecret(): string {
	const secret = env.EUCHRE_ACTOR_JWT_SECRET || env.BETTER_AUTH_SECRET || env.AUTH_SECRET || '';
	if (secret.length >= 16) return secret;
	if (isLocalDev()) return DEV_ONLY_SECRET;
	throw new Error(
		'no signing secret configured for playerProfile tokens (set EUCHRE_ACTOR_JWT_SECRET)'
	);
}

const encoder = new TextEncoder();

function base64url(bytes: Uint8Array): string {
	let binary = '';
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** A pragmatic shape check — enough to catch a typo kindly, not RFC 5322. */
const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** `email.trim().toLowerCase()`, or `null` if what's left isn't email-shaped. */
export function normalizeEmail(raw: unknown): string | null {
	if (typeof raw !== 'string') return null;
	const normalized = raw.trim().toLowerCase();
	if (normalized.length === 0 || normalized.length > 254) return null;
	return EMAIL_PATTERN.test(normalized) ? normalized : null;
}

/** SHA-256 of the normalized email, hex-encoded. Stable, deterministic, opaque. */
export async function deriveUserId(normalizedEmail: string): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', encoder.encode(normalizedEmail));
	return Array.from(new Uint8Array(digest))
		.map((byte) => byte.toString(16).padStart(2, '0'))
		.join('');
}

/** Signs a short-lived HS256 token that `verifyProfileToken` will accept. */
export async function mintProfileToken(userId: string): Promise<string> {
	const now = Math.floor(Date.now() / 1000);
	const header = base64url(encoder.encode(JSON.stringify({ alg: 'HS256', typ: 'JWT' })));
	const payload = base64url(
		encoder.encode(
			JSON.stringify({ sub: userId, aud: JWT_AUDIENCE, iat: now, exp: now + TOKEN_TTL_SECONDS })
		)
	);
	const signingInput = `${header}.${payload}`;
	const key = await crypto.subtle.importKey(
		'raw',
		encoder.encode(jwtSecret()),
		{ name: 'HMAC', hash: 'SHA-256' },
		false,
		['sign']
	);
	const signature = await crypto.subtle.sign('HMAC', key, encoder.encode(signingInput));
	return `${signingInput}.${base64url(new Uint8Array(signature))}`;
}

/** The identity a browser is currently playing as. */
export interface Identity {
	readonly email: string;
	readonly userId: string;
}

/** Reads the identity cookie and derives its userId, or `null` if unset/unusable. */
export async function readIdentity(cookies: Cookies): Promise<Identity | null> {
	const email = normalizeEmail(cookies.get(IDENTITY_COOKIE));
	if (!email) return null;
	return { email, userId: await deriveUserId(email) };
}

export function setIdentityCookie(cookies: Cookies, email: string): void {
	cookies.set(IDENTITY_COOKIE, email, {
		path: '/',
		httpOnly: true,
		sameSite: 'lax',
		secure: !dev,
		maxAge: COOKIE_MAX_AGE_SECONDS
	});
}

export function clearIdentityCookie(cookies: Cookies): void {
	cookies.delete(IDENTITY_COOKIE, { path: '/' });
}

/**
 * A `playerProfile` handle authenticated as `identity`, good for exactly the
 * request that asked for it. Every action call on the returned handle carries
 * a freshly minted token as its connection params — `createConnState` verifies
 * it fresh each time, so there is nothing here that can go stale mid-request.
 */
export async function connectProfile(identity: Identity) {
	const token = await mintProfileToken(identity.userId);
	const client = getRivetClient();
	return client.playerProfile.getOrCreate(['user', identity.userId], { params: { token } });
}
