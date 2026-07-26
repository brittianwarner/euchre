/**
 * `/play` — server creates (or would resume) a table, then redirects.
 * The browser never creates a game; this load is the only CREATE path for M2.
 */

import { redirect } from '@sveltejs/kit';
import { DEV_USER_ID } from '$lib/actors/auth/verify';
import { getRivetClient } from '$lib/server/rivet';
import { defaultPersonas } from '$lib/server/personas';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async () => {
	const gameId = crypto.randomUUID();
	const client = getRivetClient();

	await client.euchreTable.create(['table', gameId], {
		input: {
			ownerUserId: DEV_USER_ID,
			seed: gameId.replace(/-/g, '').slice(0, 32),
			personas: defaultPersonas(),
			// Local M2: shorter matches while tempo/AI pacing is still being tuned.
			cfg: { gameTo: 5 }
		}
	});

	redirect(302, `/play/${gameId}`);
};
