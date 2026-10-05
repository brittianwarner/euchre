import type { Cookies } from '@sveltejs/kit';
import { dev } from '$app/env';
import { verifyGuest, signGuest } from '#lib/actors/auth/guest.ts';
import { getRivetClient } from './rivet';

const COOKIE = 'euchre_recent_tables';
const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export function recentTableIds(cookies: Cookies): string[] {
	return (cookies.get(COOKIE) ?? '')
		.split(',')
		.filter((id) => ID.test(id))
		.slice(0, 12);
}
export function rememberTable(cookies: Cookies, gameId: string): void {
	if (!ID.test(gameId)) return;
	cookies.set(
		COOKIE,
		[gameId, ...recentTableIds(cookies).filter((id) => id !== gameId)].slice(0, 12).join(','),
		{
			path: '/',
			httpOnly: true,
			sameSite: 'lax',
			secure: !dev,
			maxAge: 400 * 86400
		}
	);
}
export async function loadRecentTables(cookies: Cookies) {
	const guest = cookies.get('euchre_guest');
	if (!guest) return { tables: [], unavailable: false };
	let userId: string;
	try {
		userId = await verifyGuest(guest, 'euchre-session');
	} catch {
		return { tables: [], unavailable: false };
	}
	const token = await signGuest(userId, 'euchre-actors');
	const ids = recentTableIds(cookies);
	const results = await Promise.allSettled(
		ids.map(async (gameId) => {
			// A cookie is a bookmark, never authority. Each table rechecks ownership.
			const view = await getRivetClient()
				.euchreTable.get(['table', gameId], {
					params: { token },
					signal: AbortSignal.timeout(5000)
				})
				.snapshot();
			return { gameId, status: view.status, score: view.score, handNo: view.handNo };
		})
	);
	return {
		tables: results.map((r, index) =>
			r.status === 'fulfilled'
				? r.value
				: { gameId: ids[index]!, status: null, score: null, handNo: null }
		),
		unavailable: results.some((r) => r.status === 'rejected')
	};
}

/** The signed browser identity also owns matches completed before choosing an email. */
export async function readGuestUserId(cookies: Cookies): Promise<string | null> {
	try {
		return await verifyGuest(cookies.get('euchre_guest') ?? '', 'euchre-session');
	} catch {
		return null;
	}
}
