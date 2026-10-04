/**
 * Server-side Rivet client.
 *
 * Use this from SvelteKit `+page.server.ts` / `+server.ts` code that needs to
 * talk to an actor without going through the browser — e.g. reading a table's
 * state for an SSR'd page, or kicking off work from a form action.
 *
 * The browser does NOT use this. Browser code goes through `@rivetkit/svelte`,
 * which opens its WebSocket directly to Rivet Cloud after discovering the
 * endpoint via `GET /api/rivet/metadata`.
 *
 * This module is under `$lib/server/`, so SvelteKit will refuse to bundle it
 * into client code — `RIVET_ENDPOINT` carries a secret (`sk_`) token.
 */

import * as env from '$app/env/private';
import { createClient, type Client } from 'rivetkit/client';
import type { Registry } from '#lib/actors/registry.ts';

/**
 * Local dev default. When `RIVET_ENDPOINT` is unset, RivetKit runs an engine on
 * this port with a filesystem driver. That fallback is fine locally and fatal
 * on Vercel (read-only filesystem), which is why the env var is required in
 * every deployed environment.
 */
const LOCAL_ENDPOINT = 'http://127.0.0.1:6420';

let client: Client<Registry> | undefined;

/**
 * Memoized client. Built lazily rather than at module scope so that importing
 * this file during build/prerender does not require the env var to be present.
 *
 * Private endpoint, namespace and credential are configured separately.
 */
export function getRivetClient(): Client<Registry> {
	if (!client) {
		client = createClient<Registry>({
			endpoint: env.RIVET_ENDPOINT || LOCAL_ENDPOINT,
			namespace: env.RIVET_NAMESPACE || 'default',
			token: env.RIVET_TOKEN,
			devtools: false,
			headers: { Origin: process.env.APP_URL || 'http://localhost:5173' }
		});
	}
	return client;
}

/** Public same-origin relay URL, never an engine credential. */
export function getRivetPublicEndpoint(): string | undefined {
	return env.RIVET_PUBLIC_ENDPOINT || undefined;
}
