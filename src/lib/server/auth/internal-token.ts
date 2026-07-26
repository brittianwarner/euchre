/**
 * The `authStore` internal-connection token.
 *
 * docs/06 §5.4 says: *"`onBeforeConnect` on `authStore` rejects every browser
 * connection. No browser ever holds a WebSocket to `authStore`; it is reached
 * only through `onRequest` from our own server code."*
 *
 * ---
 *
 * ### Spec note — how that rejection is actually implemented
 *
 * The spec sketch implies `onBeforeConnect` can simply `throw` unconditionally.
 * It cannot. Reading `rivetkit@2.3.9`, the native `onRequest` path calls
 * `runtime.actorConnectConn(...)` *before* invoking the handler, so a raw HTTP
 * `.fetch()` to the actor runs `onBeforeConnect` / `createConnState` exactly
 * like a WebSocket does. An unconditional throw would reject our own auth
 * traffic along with the browser's.
 *
 * So the guard is a shared-secret compare instead, which is the same shape as
 * the `internalToken` on every actor→actor queue (00 §3 invariant 6): the
 * caller must present a token derived from `BETTER_AUTH_SECRET`, compared in
 * constant time. A browser has no way to produce it — the secret is server-only
 * and the token is a one-way SHA-256 derivation of it, so possessing the token
 * does not yield the secret.
 *
 * The net effect matches the spec's intent exactly: only our own server code
 * can open a connection to `authStore`.
 */

import { sha256 } from './crypto';
import { requireAuthSecret } from './env';

/**
 * Domain-separation label. Changing it rotates every internal token without
 * touching `BETTER_AUTH_SECRET`, and guarantees this derivation can never
 * collide with the actor-connection tokens in `./actor-token.ts`.
 */
const LABEL = 'euchre:authStore:internal:v1';

/** Memoised per secret value, so a rotation in dev is picked up on reload. */
let cached: { secret: string; token: Promise<string> } | undefined;

/**
 * The token our server code presents as a connection param when opening a
 * connection to `authStore`.
 *
 * Never sent to a browser, never logged.
 */
export function getAuthStoreInternalToken(): Promise<string> {
	const secret = requireAuthSecret();
	if (!cached || cached.secret !== secret) {
		cached = { secret, token: sha256(`${LABEL}\n${secret}`) };
	}
	return cached.token;
}

/** The connection params `authStore` expects. Server-to-server only. */
export interface AuthStoreConnParams {
	readonly internalToken: string;
}

/** The connection state `authStore` derives. There is nothing to derive. */
export interface AuthStoreConnState {
	readonly internal: true;
}
