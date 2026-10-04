/**
 * `/games` — the scrapbook. Lifetime stats plus a paginated list of past
 * matches, both read straight from `playerProfile` at request time (SSR,
 * no live connection needed for a page that never changes under you).
 *
 * Pagination is real keyset pagination (`listMatches`'s own `played_at DESC,
 * match_id DESC` cursor), not a fake "page 2 of 2" — the whole back/forward
 * trail lives in the URL (`cursor` + `back`) so a link can be shared or
 * bookmarked and still land on the right page.
 */

import { fail, redirect } from '@sveltejs/kit';
import type { MatchListPage } from '#lib/protocol/index.ts';
import type { ProfileStats } from '#lib/actors/player-profile/types.ts';
import {
	clearIdentityCookie,
	connectProfile,
	normalizeEmail,
	readIdentity,
	setIdentityCookie
} from './_lib/session.server';
import type { Actions, PageServerLoad } from './$types';

/** Cards per scrapbook page. Small enough that "Older games" feels like a real turn of a page. */
const PAGE_SIZE = 10;

function backStackFrom(url: URL): string[] {
	const raw = url.searchParams.get('back');
	return raw ? raw.split(',').filter((entry) => entry.length > 0 || entry === '') : [];
}

function hrefFor(cursor: string, back: readonly string[]): string {
	const params = new URLSearchParams();
	if (cursor) params.set('cursor', cursor);
	if (back.length > 0) params.set('back', back.join(','));
	const qs = params.toString();
	return qs ? `?${qs}` : '/games';
}

export const load: PageServerLoad = async ({ cookies, url }) => {
	const identity = await readIdentity(cookies);
	if (!identity) return { identity: null };

	const cursorParam = url.searchParams.get('cursor');
	const backStack = backStackFrom(url);

	try {
		const profile = await connectProfile(identity);
		const [stats, page]: [ProfileStats, MatchListPage] = await Promise.all([
			profile.getStats(),
			profile.listGames({ limit: PAGE_SIZE, cursor: cursorParam })
		]);

		const olderHref = page.cursor ? hrefFor(page.cursor, [...backStack, cursorParam ?? '']) : null;
		const newerHref =
			backStack.length > 0
				? hrefFor(backStack[backStack.length - 1], backStack.slice(0, -1))
				: cursorParam
					? '/games'
					: null;

		return {
			identity,
			stats,
			page,
			olderHref,
			newerHref,
			loadError: false as const
		};
	} catch {
		return {
			identity,
			stats: null,
			page: null,
			olderHref: null,
			newerHref: null,
			loadError: true as const
		};
	}
};

export const actions: Actions = {
	/** "This is me" — set the browser's identity, then reload this same page. */
	identify: async ({ request, cookies, url }) => {
		const data = await request.formData();
		const email = normalizeEmail(data.get('email'));
		if (!email) {
			return fail(400, { error: 'That doesn’t look like an email address yet.' });
		}
		setIdentityCookie(cookies, email);
		redirect(303, url.pathname);
	},

	/** "Not you? Switch profile" — forget this browser's identity. */
	signout: async ({ cookies, url }) => {
		clearIdentityCookie(cookies);
		redirect(303, url.pathname);
	}
};
