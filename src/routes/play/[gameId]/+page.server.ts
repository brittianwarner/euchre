import type { PageServerLoad } from './$types';
import { guestIdentity } from '#lib/server/guest.ts';
import { signGuest } from '#lib/actors/auth/guest.ts';
import { getRivetClient } from '#lib/server/rivet.ts';

export const load: PageServerLoad = async ({ params, cookies }) => ({
	gameId: params.gameId,
	actorId: await getRivetClient()
		.euchreTable.get(['table', params.gameId], {
			signal: AbortSignal.timeout(12_000)
		})
		.resolve(),
	token: await signGuest(await guestIdentity(cookies), 'euchre-actors')
});
