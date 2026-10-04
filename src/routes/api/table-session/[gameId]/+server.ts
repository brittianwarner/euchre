import { error, json } from '@sveltejs/kit';
import { signGuest, verifyGuest } from '#lib/actors/auth/guest.ts';
import { getRivetClient } from '#lib/server/rivet.ts';
import type { RequestHandler } from './$types';

/** Mint renewable, actor-scoped gateway access only after the actor verifies ownership. */
export const POST: RequestHandler = async ({ request, url, cookies, params }) => {
	if (request.headers.get('origin') !== url.origin) error(403, 'Origin not allowed');
	if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(params.gameId)) {
		error(400, 'Invalid table');
	}
	let userId: string;
	try {
		userId = await verifyGuest(cookies.get('euchre_guest') ?? '', 'euchre-session');
	} catch {
		error(401, 'Please start a game first');
	}
	const token = await signGuest(userId, 'euchre-actors');
	const table = getRivetClient().euchreTable.get(['table', params.gameId], {
		params: { token },
		signal: AbortSignal.timeout(12_000)
	});
	try {
		// snapshot passes the same origin and owner checks as a browser connection.
		await table.snapshot();
	} catch {
		error(403, 'This table is not available to this browser');
	}
	const issued = process.env.RIVET_TOKEN
		? await table.issueToken({ subject: userId, expiresIn: 900 })
		: undefined;
	return json(
		{
			token,
			gatewayToken: issued?.token,
			expiresAt: issued?.expiresAt ?? Date.now() + 900_000
		},
		{ headers: { 'Cache-Control': 'no-store' } }
	);
};
