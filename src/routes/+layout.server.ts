/**
 * Root server layout load.
 *
 * Reads `RIVET_PUBLIC_ENDPOINT` (server-only env) and publishes it as page
 * data. It carries a **publishable** `pk_` token, not the secret `sk_` one —
 * Rivet's own `GET /api/rivet/metadata` already serves this value to anyone
 * who asks, so putting it in page data exposes nothing new. It is here so the
 * browser can skip the metadata round trip and point `@rivetkit/svelte`
 * straight at Rivet Cloud.
 *
 * `$env/dynamic/private` (not `static`) so the value is read at request time on
 * Vercel rather than baked in at build time — the same build can then run in
 * preview and production namespaces.
 */

import { getRivetPublicEndpoint } from '$lib/server/rivet';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = () => {
	return {
		// `undefined` in local dev, where RivetKit runs an engine on :6420 and
		// the browser falls back to this app's own /api/rivet mount.
		rivetPublicEndpoint: getRivetPublicEndpoint()
	};
};
