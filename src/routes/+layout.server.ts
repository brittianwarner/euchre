/** Publishes only the public relay URL, never the private engine or its token. */
import { getRivetPublicEndpoint } from '#lib/server/rivet.ts';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = () => {
	return {
		// `undefined` in local dev, where RivetKit runs an engine on :6420 and
		// the browser falls back to this app's own /api/rivet mount.
		rivetPublicEndpoint: getRivetPublicEndpoint()
	};
};
