/**
 * `/games` — the scrapbook. Lifetime stats plus a paginated list of past
 * matches, both read from `playerProfile` through `/api/*` when the page
 * opens (no live connection needed for a page that never changes under you).
 *
 * Pagination is real keyset pagination (`listGames`'s own `played_at DESC,
 * match_id DESC` cursor), not a fake "page 2 of 2" — the whole back/forward
 * trail lives in the URL (`cursor` + `back`) so a link can be shared or
 * bookmarked and still land on the right page.
 */

import {
	getRecentTables,
	getSession,
	getStats,
	listGames,
	type RecentTables
} from '#lib/client/api.ts';
import type { MatchListPage } from '#lib/protocol/index.ts';
import type { ProfileStats } from '#lib/protocol/profile.ts';
import type { PageLoad } from './$types';

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

const NO_RECENT: RecentTables = { tables: [], unavailable: false };

export const load: PageLoad = async ({ fetch, url }) => {
	const [session, recent] = await Promise.all([
		getSession(fetch),
		getRecentTables(fetch).catch(() => NO_RECENT)
	]);
	const { identity, guestUserId } = session;
	const browserGamesRequest: Promise<MatchListPage | null> =
		guestUserId && guestUserId !== identity?.userId
			? // The saved-table links remain usable if the scrapbook is offline.
				listGames({ fetch, profile: 'browser', limit: PAGE_SIZE }).catch(() => null)
			: Promise.resolve(null);
	if (!identity) return { identity: null, recent, browserGames: await browserGamesRequest };

	const cursorParam = url.searchParams.get('cursor');
	const backStack = backStackFrom(url);

	const [browserGames, scrapbook] = await Promise.all([
		browserGamesRequest,
		Promise.all([
			getStats(fetch),
			listGames({ fetch, profile: 'email', limit: PAGE_SIZE, cursor: cursorParam })
		]).catch(() => null)
	]);
	if (!scrapbook) {
		return {
			recent,
			browserGames,
			identity,
			stats: null,
			page: null,
			olderHref: null,
			newerHref: null,
			loadError: true as const
		};
	}
	const [stats, page]: [ProfileStats, MatchListPage] = scrapbook;

	const olderHref = page.cursor ? hrefFor(page.cursor, [...backStack, cursorParam ?? '']) : null;
	const newerHref =
		backStack.length > 0
			? hrefFor(backStack[backStack.length - 1], backStack.slice(0, -1))
			: cursorParam
				? '/games'
				: null;

	return {
		recent,
		browserGames,
		identity,
		stats,
		page,
		olderHref,
		newerHref,
		loadError: false as const
	};
};
