/**
 * Root universal layout load.
 *
 * Deliberately does NOT set `export const ssr = false`.
 *
 * The reference implementation (rivet-game-svelte) disables SSR globally
 * because every one of its routes is a Threlte canvas. This app is different:
 * auth pages, the lobby, and game history are ordinary server-rendered routes
 * and need SSR for correct redirects, SEO, and first paint. Killing SSR at the
 * root would take all of that down to serve one 3D route.
 *
 * The 3D table route scopes `ssr = false` to itself in its own `+layout.ts`
 * / `+page.ts` (later milestone). Do not hoist it here.
 *
 * This load just widens the server layout's data with the client-side mount
 * path, so nothing downstream has to hardcode the string.
 */

import type { LayoutLoad } from './$types';

/**
 * Where this deployment mounts the Rivet registry. Used as the client endpoint
 * in local dev (and as a fallback in production), where `@rivetkit/svelte`
 * fetches `${rivetMountPath}/metadata` to discover Rivet Cloud.
 *
 * Underscore-prefixed because SvelteKit only permits `load`, `prerender`,
 * `csr`, `ssr`, `trailingSlash`, `config`, and `_`-prefixed names as exports
 * from a route module.
 */
export const _RIVET_MOUNT_PATH = '/api/rivet';

export const load: LayoutLoad = ({ data }) => {
	return {
		rivetPublicEndpoint: data.rivetPublicEndpoint,
		rivetMountPath: _RIVET_MOUNT_PATH
	};
};
