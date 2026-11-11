/**
 * `/games/[matchId]` — one page of the scrapbook: a single match, hand by
 * hand. `GET /api/games/{matchId}` already redacts everything a browser must
 * never see (seed, deck order, a buried dealer discard); this route just
 * formats what comes back.
 */

import { error, redirect } from '@sveltejs/kit';
import { ApiError, getReplay, getSession } from '#lib/client/api.ts';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params, url }) => {
	const session = await getSession(fetch);
	const guestUserId = url.searchParams.get('profile') === 'browser' ? session.guestUserId : null;
	const identity = guestUserId ? { userId: guestUserId, email: '' } : session.identity;
	if (!identity) redirect(303, '/games');

	try {
		const replay = await getReplay({
			fetch,
			matchId: params.matchId,
			profile: guestUserId ? 'browser' : 'email'
		});
		return { identity, replay };
	} catch (cause) {
		if (cause instanceof ApiError) {
			if (cause.status === 401) redirect(303, '/games');
			error(cause.status, cause.message);
		}
		throw cause;
	}
};
