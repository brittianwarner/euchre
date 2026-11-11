/**
 * Shared setup for the auth fixture scripts: fixed secrets, a fixed clock, and the
 * SvelteKit virtual modules `session.server.ts` imports, so the real TS auth code
 * runs under bun outside SvelteKit.
 *
 * Import this module first, then import the TS sources dynamically (plugins and
 * env must be in place before those modules load).
 */

import { plugin } from 'bun';

/** `EUCHRE_ACTOR_JWT_SECRET`: guest and profile tokens (one secret in TS). */
export const ACTOR_SECRET = 'fixture-actor-secret-0123456789abcdef-guest';
/** `EUCHRE_INTERNAL_TOKEN`. */
export const INTERNAL_TOKEN = 'fixture-internal-token-0123456789abcdef';
/** `EUCHRE_IDENTITY_SECRET` (D2; Rust only, verified here with jose). */
export const IDENTITY_SECRET = 'fixture-identity-secret-0123456789abcdef';
/** Fixed clock: 2027-01-15T08:00:00Z. */
export const NOW_SECS = 1_800_000_000;

process.env.EUCHRE_ACTOR_JWT_SECRET = ACTOR_SECRET;
process.env.EUCHRE_INTERNAL_TOKEN = INTERNAL_TOKEN;
delete process.env.BETTER_AUTH_SECRET;
delete process.env.AUTH_SECRET;
delete process.env.RIVET_ENDPOINT;
delete process.env.EUCHRE_JWT_ISSUER;

const RealDate = Date;
let clockMs = NOW_SECS * 1000;

/** Set the fixed clock (milliseconds). */
export function setClockMs(ms: number): void {
	clockMs = ms;
}

/** Set the fixed clock (seconds). */
export function setClockSecs(secs: number): void {
	clockMs = secs * 1000;
}

class FixedDate extends RealDate {
	constructor(...args: unknown[]) {
		if (args.length === 0) super(clockMs);
		else super(...(args as [string | number | Date]));
	}
	static now(): number {
		return clockMs;
	}
}
globalThis.Date = FixedDate as unknown as DateConstructor;

plugin({
	name: 'euchre-auth-fixture-shims',
	setup(build) {
		build.module('$app/env', () => ({ exports: { dev: true }, loader: 'object' }));
		build.module('$app/env/private', () => ({
			exports: {
				EUCHRE_ACTOR_JWT_SECRET: ACTOR_SECRET,
				BETTER_AUTH_SECRET: '',
				AUTH_SECRET: '',
				RIVET_ENDPOINT: ''
			},
			loader: 'object'
		}));
		build.onResolve({ filter: /server\/rivet(\.ts)?$/ }, () => ({
			path: 'euchre-fixture-rivet-stub',
			namespace: 'euchre-fixture'
		}));
		build.onLoad({ filter: /.*/, namespace: 'euchre-fixture' }, () => ({
			contents: 'export function getRivetClient() { throw new Error("not available in fixtures"); }',
			loader: 'js'
		}));
	}
});

/** Repo-relative path helpers (scripts run from the repo root). */
export const WEB = `${import.meta.dir}/../../../apps/web/src`;
export const FIXTURES = `${import.meta.dir}/../tests/fixtures/auth`;

const enc = new TextEncoder();

/** base64url without padding. */
export function b64url(bytes: Uint8Array | string): string {
	const b = typeof bytes === 'string' ? enc.encode(bytes) : bytes;
	let s = '';
	for (const x of b) s += String.fromCharCode(x);
	return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** HMAC-SHA256 over `input` with `secret`, base64url. */
export async function hmac(secret: string, input: string, hash = 'SHA-256'): Promise<string> {
	const key = await crypto.subtle.importKey('raw', enc.encode(secret), { name: 'HMAC', hash }, false, [
		'sign'
	]);
	return b64url(new Uint8Array(await crypto.subtle.sign('HMAC', key, enc.encode(input))));
}

/** A token with the given raw header/payload JSON text, signed (or not) with `secret`. */
export async function forge(
	secret: string | null,
	header: string,
	payload: string | Uint8Array,
	hash = 'SHA-256'
): Promise<string> {
	const input = `${b64url(header)}.${b64url(payload)}`;
	return `${input}.${secret === null ? '' : await hmac(secret, input, hash)}`;
}

/** Run `f` and return `{ ok: true, value }` or `{ ok: false, error: message }`. */
export async function outcome<T>(
	f: () => Promise<T>
): Promise<{ ok: true; value: T } | { ok: false; error: string }> {
	try {
		return { ok: true, value: await f() };
	} catch (e) {
		return { ok: false, error: e instanceof Error ? e.message : String(e) };
	}
}
