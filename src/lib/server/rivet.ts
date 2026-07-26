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

import { env } from '$env/dynamic/private';
import { createClient, type Client } from 'rivetkit/client';
import type { Registry } from '$lib/actors/registry';

/**
 * Local dev default. When `RIVET_ENDPOINT` is unset, RivetKit runs an engine on
 * this port with a filesystem driver. That fallback is fine locally and fatal
 * on Vercel (read-only filesystem), which is why the env var is required in
 * every deployed environment.
 */
const LOCAL_ENDPOINT = 'http://localhost:6420';

let client: Client<Registry> | undefined;

/**
 * Memoized client. Built lazily rather than at module scope so that importing
 * this file during build/prerender does not require the env var to be present.
 *
 * `RIVET_ENDPOINT` uses URL auth: `https://<namespace>:<sk_token>@api.rivet.dev`.
 */
export function getRivetClient(): Client<Registry> {
	if (!client) {
		client = createClient<Registry>(env.RIVET_ENDPOINT || LOCAL_ENDPOINT);
	}
	return client;
}

/**
 * The publishable endpoint handed to browsers (`pk_` token — safe to expose,
 * this is the same value `/api/rivet/metadata` already serves publicly).
 * `undefined` in local dev, where the client just talks to the local engine.
 */
export function getRivetPublicEndpoint(): string | undefined {
	return env.RIVET_PUBLIC_ENDPOINT || undefined;
}
