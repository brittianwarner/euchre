/**
 * `/play/[gameId]` — pass the match id to the client. M2 skips ownership DB
 * checks; the table actor still enforces `ownerUserId` on connect.
 */

import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ params }) => {
	return {
		gameId: params.gameId,
		/** Local-dev magic token until `/api/rivet-token` lands. */
		token: 'dev' as const
	};
};
