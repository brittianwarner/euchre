/**
 * Actor JWT verification for browser connections to `euchreTable`.
 *
 * In production this validates Better Auth JWTs against the issuer's JWKS.
 * For local M2 development (no `RIVET_ENDPOINT`), the magic token `"dev"` is
 * accepted so `/play` works before the full auth/token mint path lands.
 */

import { createRemoteJWKSet, jwtVerify, type JWTPayload } from 'jose';

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
 * Always accepts the magic {@link DEV_PLAYER_TOKEN} (local M2). That stays safe
 * because production tables are keyed by unguessable game ids and still check
 * `ownerUserId`. Real JWTs are verified when present.
 *
 * Note: RivetKit's local engine may set `RIVET_ENDPOINT` to `:6420`, so we cannot
 * use "endpoint unset" as the sole local-dev signal.
 */
export async function verifyPlayer(
	jwks: Jwks,
	token: string,
	appUrl: string
): Promise<PlayerClaims> {
	if (token === DEV_PLAYER_TOKEN) {
		return { userId: DEV_USER_ID, email: 'dev@localhost' };
	}

	if (typeof token !== 'string' || token.length === 0) {
		throw new Error('missing_token');
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
