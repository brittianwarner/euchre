import type { Cookies } from '@sveltejs/kit';
import { dev } from '$app/env';
import { signGuest, verifyGuest } from '#lib/actors/auth/guest.ts';

/** Signed anonymous browser identity; no shared player and no pretend account. */
export async function guestIdentity(cookies: Cookies): Promise<string> {
	const token = cookies.get('euchre_guest');
	if (token) {
		try {
			return await verifyGuest(token, 'euchre-session');
		} catch {
			/* An expired or tampered session is replaced with a new identity. */
		}
	}
	const userId = `guest-${crypto.randomUUID()}`;
	cookies.set('euchre_guest', await signGuest(userId, 'euchre-session'), {
		path: '/',
		httpOnly: true,
		sameSite: 'lax',
		secure: !dev,
		maxAge: 400 * 86400
	});
	return userId;
}
