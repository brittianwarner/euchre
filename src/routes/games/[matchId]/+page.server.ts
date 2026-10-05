/**
 * `/games/[matchId]` — one page of the scrapbook: a single match, hand by
 * hand. `getReplay` already redacts everything a browser must never see
 * (seed, deck order, a buried dealer discard); this route just formats what
 * comes back.
 */

import { readGuestUserId } from '#lib/server/recent-tables.ts';
import { error, redirect } from '@sveltejs/kit';
import { connectProfile, readIdentity } from '../_lib/session.server';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ params, cookies, url }) => {
	const guestUserId =
		url.searchParams.get('profile') === 'browser' ? await readGuestUserId(cookies) : null;
	const identity = guestUserId ? { userId: guestUserId, email: '' } : await readIdentity(cookies);
	if (!identity) redirect(303, '/games');

	const profile = await connectProfile(identity);
	const replay = await profile.getReplay(params.matchId);
	if (!replay) error(404, 'No game like that in your scrapbook.');

	return { identity, replay };
};
