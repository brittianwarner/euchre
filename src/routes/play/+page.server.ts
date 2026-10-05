/**
 * `/play` — server creates (or would resume) a table, then redirects.
 * The browser never creates a game; this load is the only CREATE path for M2.
 */

import * as env from '$app/env/private';
import { redirect } from '@sveltejs/kit';
import { guestIdentity } from '#lib/server/guest.ts';
import { getRivetClient } from '#lib/server/rivet.ts';
import { defaultPersonas } from '#lib/server/personas.ts';
import { rememberTable } from '#lib/server/recent-tables.ts';
import { readIdentity } from '../games/_lib/session.server';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ fetch, cookies, url }) => {
	// Cold local starts must initialize the engine before the first actor create.
	if (!env.RIVET_ENDPOINT) {
		const response = await fetch('/api/rivet/metadata');
		if (!response.ok) throw new Error('The local table service could not start');
	}
	const gameId = crypto.randomUUID();
	const client = getRivetClient();

	await client.euchreTable.create(['table', gameId], {
		input: {
			ownerUserId: await guestIdentity(cookies),
			seed: gameId.replace(/-/g, '').slice(0, 32),
			personas: defaultPersonas(),
			profileUserId: (await readIdentity(cookies))?.userId,
			// Standard euchre match.
			cfg: {
				gameTo: 10,
				stickTheDealer: url.searchParams.get('stick') === 'true',
				requireNaturalTrump: url.searchParams.get('natural') === 'true'
			}
		}
	});

	rememberTable(cookies, gameId);
	redirect(302, `/play/${gameId}`);
};
