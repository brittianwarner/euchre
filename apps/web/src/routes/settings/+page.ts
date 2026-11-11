/**
 * `/settings` — the AI persona editor. Reads `playerProfile`'s
 * `ProfileSettings` (house prompt, three personas, table prefs) through
 * `GET /api/profile/settings`. Saving goes through `PUT` on the same path,
 * where the actor validates, clamps and sanitises every field — the page
 * trusts none of its own input.
 */

import { getSession, getSettings } from '#lib/client/api.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch }) => {
	const { identity } = await getSession(fetch);
	if (!identity) return { identity: null };

	try {
		const settings = await getSettings(fetch);
		return { identity, settings, loadError: false as const };
	} catch {
		return { identity, settings: null, loadError: true as const };
	}
};
