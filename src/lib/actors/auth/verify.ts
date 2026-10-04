/**
 * Actor JWT verification for browser connections to `euchreTable`.
 *
 * In production this validates Better Auth JWTs against the issuer's JWKS.
 * Anonymous play uses signed, audience-bound guest tokens.
 */

import { verifyGuest } from './guest';
import { createRemoteJWKSet, decodeProtectedHeader, jwtVerify, type JWTPayload } from 'jose';

/** JWKS handle rebuilt on every actor wake via `createVars`. */
export type Jwks = ReturnType<typeof createRemoteJWKSet>;

/** Claims every player connection must carry after verification. */
export interface PlayerClaims {
	readonly userId: string;
	readonly email: string;
}

/** Audience stamped on actor-connection JWTs (and checked here). */
export const ACTOR_JWT_AUDIENCE = 'euchre-actors';

/** Local-dev magic token accepted when Rivet Cloud is not configured. */
export const DEV_PLAYER_TOKEN = 'dev';

/** Stable local user id paired with {@link DEV_PLAYER_TOKEN}. */
export const DEV_USER_ID = 'dev-user';

/**
 * Build a JWKS fetcher pointed at Better Auth's JWKS route.
 * Unused on the local-dev token path, but still constructed so createVars is uniform.
 */
export function makeJwks(appUrl: string): Jwks {
	const base = appUrl.replace(/\/$/, '');
	return createRemoteJWKSet(new URL(`${base}/api/auth/jwks`));
}

/**
 * Verify a connection token and return the player claims.
 *
 * Guest and account tokens both require a valid signature and expiry.
 */
export async function verifyPlayer(
	jwks: Jwks,
	token: string,
	appUrl: string
): Promise<PlayerClaims> {
	if (typeof token !== 'string' || token.length === 0 || token === DEV_PLAYER_TOKEN) {
		throw new Error('missing_or_invalid_token');
	}
	if (decodeProtectedHeader(token).alg === 'HS256') {
		const userId = await verifyGuest(token, 'euchre-actors');
		return { userId, email: '' };
	}

	const { payload } = await jwtVerify(token, jwks, {
		issuer: appUrl.replace(/\/$/, ''),
		audience: ACTOR_JWT_AUDIENCE,
		clockTolerance: 5
	});

	return claimsFromPayload(payload);
}

/** Pull `userId` + `email` out of a verified JWT payload. */
function claimsFromPayload(payload: JWTPayload): PlayerClaims {
	const userId =
		(typeof payload.id === 'string' && payload.id) ||
		(typeof payload.sub === 'string' && payload.sub) ||
		null;
	const email = typeof payload.email === 'string' ? payload.email : null;
	if (!userId || !email) {
		throw new Error('jwt_missing_claims');
	}
	return { userId, email };
}
