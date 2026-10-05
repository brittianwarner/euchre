import type { PageServerLoad } from './$types';
import { guestIdentity } from '#lib/server/guest.ts';
import { rememberTable } from '#lib/server/recent-tables.ts';

// Actor resolution is retried by the client. A temporary engine outage must not
// replace a saved game with an HTTP 500 page.
export const load: PageServerLoad = async ({ params, cookies }) => {
	await guestIdentity(cookies);
	rememberTable(cookies, params.gameId);
	return { gameId: params.gameId };
};
