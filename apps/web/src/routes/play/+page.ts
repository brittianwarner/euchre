/**
 * `/play` — create a table, then go to it. `POST /api/tables` mints this
 * browser's guest cookie if needed, creates the table with the player's
 * personas and records it in the recent-tables cookie. The query string
 * carries the house rules from `/new`.
 */

import { error, redirect } from '@sveltejs/kit';
import { ApiError, createTable } from '#lib/client/api.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	let gameId: string;
	try {
		({ gameId } = await createTable({
			fetch,
			stick: url.searchParams.get('stick') === 'true',
			natural: url.searchParams.get('natural') === 'true'
		}));
	} catch (cause) {
		if (cause instanceof ApiError) error(cause.status, cause.message);
		throw cause;
	}
	redirect(302, `/play/${gameId}`);
};
