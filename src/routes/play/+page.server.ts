/**
 * `/play` — server creates (or would resume) a table, then redirects.
 * The browser never creates a game; this load is the only CREATE path for M2.
 */

import * as env from '$app/env/private';
import { redirect } from '@sveltejs/kit';
import { guestIdentity } from '#lib/server/guest.ts';
import { getRivetClient } from '#lib/server/rivet.ts';
import { defaultPersonas, personasFromSettings } from '#lib/server/personas.ts';
import { rememberTable } from '#lib/server/recent-tables.ts';
import { connectProfile, readIdentity } from '../games/_lib/session.server';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ fetch, cookies, url }) => {
	// Cold local starts must initialize the engine before the first actor create.
	if (!env.RIVET_ENDPOINT) {
		const response = await fetch('/api/rivet/metadata');
		if (!response.ok) throw new Error('The local table service could not start');
	}
	const gameId = crypto.randomUUID();
	const client = getRivetClient();
	const identity = await readIdentity(cookies);
	let personas = defaultPersonas();
	if (identity) {
		let timer: ReturnType<typeof setTimeout> | undefined;
		try {
			const settings = await Promise.race([
				connectProfile(identity).then((profile) => profile.getSettings()),
				new Promise<never>((_, reject) => {
					timer = setTimeout(() => reject(new Error('Persona settings timed out')), 2000);
				})
			]);
			personas = personasFromSettings(settings);
		} catch {
			console.warn('Persona settings unavailable; starting with kitchen-table players');
		} finally {
			clearTimeout(timer);
		}
	}

	await client.euchreTable.create(['table', gameId], {
		input: {
			ownerUserId: await guestIdentity(cookies),
			personas,
			profileUserId: identity?.userId,
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
