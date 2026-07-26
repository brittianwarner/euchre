/**
 * The authorisation boundary of `playerProfile`.
 *
 * There are exactly two ways to be allowed into this actor, and no third:
 *
 * 1. **A connection whose JWT verifies and whose `sub` equals the actor's own
 *    key.** The actor is keyed `['user', userId]`, so the key *is* the access
 *    control list. A token for someone else's profile fails on the key compare,
 *    not on anything the caller said about itself — which is why no action in
 *    this module takes a `userId` argument.
 * 2. **An internal call carrying the deployment's server secret**, compared in
 *    constant time before any state or database read.
 *
 * ### Why HS256 and a shared secret
 *
 * `docs/01-ARCHITECTURE.md` describes a JWKS-verified asymmetric token. That
 * verifier belongs to whoever owns `/api/rivet-token`, and it does not exist yet;
 * this module cannot depend on a file another agent may or may not name the same
 * thing, and it will not add a dependency to a lockfile four agents are sharing.
 * So the verification here is written against the Web Crypto API that both Node
 * and Bun ship, with a symmetric key. {@link verifyProfileToken} is the entire
 * seam: when the asymmetric verifier lands, that one function's body changes and
 * nothing else in this actor moves.
 *
 * Everything *around* the signature check — audience, expiry, not-before, clock
 * skew, `sub` shape, and the key compare — is verifier-independent and stays.
 */

import { protocolError } from './guards';

/** Audience every actor-bound token must carry (`docs/01-ARCHITECTURE.md` §6.1). */
export const JWT_AUDIENCE = 'euchre-actors';

/** Tolerated clock skew between the token minter and this actor, in ms. */
const CLOCK_SKEW_MS = 60_000;

/** A token older than this is refused even if `exp` says otherwise. */
const MAX_TOKEN_LIFETIME_MS = 60 * 60_000;

/**
 * Well-known development secret, used **only** when no Rivet Cloud endpoint is
 * configured — i.e. the same `bun run dev` signal `registry.ts` already keys off.
 *
 * In any deployed environment `RIVET_ENDPOINT` is set, this constant is
 * unreachable, and a missing secret fails closed.
 */
const DEV_ONLY_SECRET = 'euchre-dev-insecure-secret';

/** True when there is no Rivet Cloud endpoint, i.e. local development. */
function isLocalDev(): boolean {
	return !process.env.RIVET_ENDPOINT;
}

/**
 * The symmetric key the connection JWT is signed with.
 *
 * Read from the environment on every call rather than cached at module scope: a
 * serverless instance may be recycled with a rotated secret, and this is not a
 * hot path.
 */
function jwtSecret(): string | null {
	const secret =
		process.env.EUCHRE_ACTOR_JWT_SECRET ??
		process.env.BETTER_AUTH_SECRET ??
		process.env.AUTH_SECRET ??
		null;
	if (secret && secret.length >= 16) return secret;
	if (isLocalDev()) return DEV_ONLY_SECRET;
	return null;
}

/**
 * The deployment-wide secret on actor-to-actor calls into this profile.
 *
 * Distinct from `InternalEnvelope.internalToken` in `$lib/protocol`, which is a
 * *per-match* secret minted in `euchreTable.createState`. A profile outlives
 * every match and cannot know a per-match value, so the token compared here is
 * the server secret both processes read from the environment.
 */
function internalSecret(): string | null {
	const secret = process.env.EUCHRE_INTERNAL_TOKEN ?? null;
	if (secret && secret.length >= 16) return secret;
	if (isLocalDev()) return DEV_ONLY_SECRET;
	return null;
}

/* ========================================================================== */
/* Constant-time comparison                                                   */
/* ========================================================================== */

const encoder = new TextEncoder();

async function sha256(value: string): Promise<Uint8Array> {
	const digest = await crypto.subtle.digest('SHA-256', encoder.encode(value));
	return new Uint8Array(digest);
}

/**
 * Constant-time string equality.
 *
 * Compares SHA-256 digests rather than the strings themselves, so the comparison
 * is over two fixed 32-byte buffers and leaks neither the length nor the position
 * of the first differing byte.
 */
export async function constantTimeEqual(a: string, b: string): Promise<boolean> {
	const [da, db] = await Promise.all([sha256(a), sha256(b)]);
	let diff = 0;
	for (let i = 0; i < da.length; i += 1) diff |= da[i] ^ db[i];
	return diff === 0;
}

/* ========================================================================== */
/* Internal token                                                             */
/* ========================================================================== */

/**
 * Verifies the internal token on an actor-to-actor message.
 *
 * Deliberately takes the token alone: the caller must extract it *before* it
 * touches anything else on the message, so that an unauthenticated payload is
 * never parsed, never logged and never written.
 *
 * @throws `internal_token_mismatch` — the only code it will ever report, whether
 * the token was absent, malformed or wrong. Distinguishing those for a caller
 * would be a free oracle.
 */
export async function assertInternalToken(token: unknown): Promise<void> {
	const expected = internalSecret();
	if (expected === null) {
		throw protocolError(
			'internal_token_mismatch',
			'EUCHRE_INTERNAL_TOKEN is not configured; internal writes are refused'
		);
	}
	if (typeof token !== 'string' || token.length === 0) {
		throw protocolError('internal_token_mismatch', 'internal token missing');
	}
	if (!(await constantTimeEqual(token, expected))) {
		throw protocolError('internal_token_mismatch', 'internal token rejected');
	}
}

/* ========================================================================== */
/* JWT                                                                        */
/* ========================================================================== */

/** What a verified connection token establishes. Nothing else is taken from it. */
export interface VerifiedClaims {
	/** The JWT `sub`. Compared against the actor key; never used to select one. */
	readonly userId: string;
	readonly issuedAt: number;
	readonly expiresAt: number;
}

function base64UrlToBytes(segment: string): Uint8Array {
	const padded = segment.replace(/-/g, '+').replace(/_/g, '/');
	const padding = padded.length % 4 === 0 ? '' : '='.repeat(4 - (padded.length % 4));
	const binary = atob(padded + padding);
	const bytes = new Uint8Array(binary.length);
	for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

function decodeJwtSegment(segment: string): unknown {
	const text = new TextDecoder().decode(base64UrlToBytes(segment));
	return JSON.parse(text) as unknown;
}

function claimNumber(claims: Record<string, unknown>, key: string): number | null {
	const value = claims[key];
	return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function audienceMatches(value: unknown): boolean {
	if (typeof value === 'string') return value === JWT_AUDIENCE;
	if (Array.isArray(value)) return value.some((entry) => entry === JWT_AUDIENCE);
	return false;
}

/**
 * Verifies a connection token and returns the claims it establishes.
 *
 * Checks, in order and all of them: three-segment shape, `alg` is exactly the one
 * algorithm we accept (never the token's choice of any algorithm — that is the
 * classic `alg: none` hole), signature over `header.payload`, `aud`, `iss` when
 * one is configured, `exp`, `nbf`, and a bounded lifetime. Only then is `sub`
 * read.
 *
 * Every failure reports `invalid_token` with a message meant for a server log.
 * The client's only correct response to any of them is to refresh and reconnect.
 */
export async function verifyProfileToken(token: string, now: number): Promise<VerifiedClaims> {
	const secret = jwtSecret();
	if (secret === null) {
		throw protocolError('invalid_token', 'no signing secret configured');
	}

	const parts = token.split('.');
	if (parts.length !== 3) {
		throw protocolError('invalid_token', 'malformed token');
	}
	const [headerSegment, payloadSegment, signatureSegment] = parts;

	let header: unknown;
	let payload: unknown;
	try {
		header = decodeJwtSegment(headerSegment);
		payload = decodeJwtSegment(payloadSegment);
	} catch {
		throw protocolError('invalid_token', 'token segments are not JSON');
	}

	if (typeof header !== 'object' || header === null || Array.isArray(header)) {
		throw protocolError('invalid_token', 'token header is not an object');
	}
	if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) {
		throw protocolError('invalid_token', 'token payload is not an object');
	}
	const headerClaims = header as Record<string, unknown>;
	const claims = payload as Record<string, unknown>;

	// The algorithm is ours, not the token's. A token asking for `none` — or for
	// anything else — is rejected before any crypto happens.
	if (headerClaims.alg !== 'HS256') {
		throw protocolError('invalid_token', 'unsupported token algorithm');
	}

	const key = await crypto.subtle.importKey(
		'raw',
		encoder.encode(secret),
		{ name: 'HMAC', hash: 'SHA-256' },
		false,
		['verify']
	);
	const signed = encoder.encode(`${headerSegment}.${payloadSegment}`);
	let signature: Uint8Array;
	try {
		signature = base64UrlToBytes(signatureSegment);
	} catch {
		throw protocolError('invalid_token', 'malformed signature');
	}
	const signatureBuffer = signature.buffer.slice(
		signature.byteOffset,
		signature.byteOffset + signature.byteLength
	) as ArrayBuffer;
	const valid = await crypto.subtle.verify('HMAC', key, signatureBuffer, signed);
	if (!valid) {
		throw protocolError('invalid_token', 'signature rejected');
	}

	if (!audienceMatches(claims.aud)) {
		throw protocolError('invalid_token', 'wrong audience');
	}

	const expectedIssuer = process.env.EUCHRE_JWT_ISSUER;
	if (expectedIssuer && claims.iss !== expectedIssuer) {
		throw protocolError('invalid_token', 'wrong issuer');
	}

	const exp = claimNumber(claims, 'exp');
	if (exp === null) {
		throw protocolError('invalid_token', 'token has no expiry');
	}
	const expiresAt = exp * 1000;
	if (expiresAt + CLOCK_SKEW_MS < now) {
		throw protocolError('invalid_token', 'token expired');
	}

	const nbf = claimNumber(claims, 'nbf');
	if (nbf !== null && nbf * 1000 - CLOCK_SKEW_MS > now) {
		throw protocolError('invalid_token', 'token not yet valid');
	}

	const iat = claimNumber(claims, 'iat');
	const issuedAt = iat === null ? now : iat * 1000;
	if (iat !== null) {
		if (issuedAt - CLOCK_SKEW_MS > now) {
			throw protocolError('invalid_token', 'token issued in the future');
		}
		if (expiresAt - issuedAt > MAX_TOKEN_LIFETIME_MS) {
			throw protocolError('invalid_token', 'token lifetime exceeds the maximum');
		}
	}

	const sub = claims.sub;
	if (typeof sub !== 'string' || sub.length === 0 || sub.length > 128) {
		throw protocolError('invalid_token', 'token subject is missing or unusable');
	}

	return { userId: sub, issuedAt, expiresAt };
}

/* ========================================================================== */
/* The actor key                                                              */
/* ========================================================================== */

/** The namespace every `playerProfile` key starts with: `['user', userId]`. */
export const PROFILE_KEY_NAMESPACE = 'user';

/**
 * The user this actor instance belongs to, read from its own key.
 *
 * This is the *only* source of a user id in this module. No action accepts one,
 * and no stored `userId` is ever used to decide who may read a row — it is stored
 * so that a mismatched write can be rejected, which is a different thing.
 */
export function ownerOf(key: readonly string[]): string {
	const [namespace, userId] = key;
	if (namespace !== PROFILE_KEY_NAMESPACE || typeof userId !== 'string' || userId.length === 0) {
		throw protocolError(
			'internal_error',
			`playerProfile must be keyed ['${PROFILE_KEY_NAMESPACE}', userId]`
		);
	}
	return userId;
}
