/**
 * `/settings` — the AI persona editor. Reads and writes `playerProfile`'s
 * `ProfileSettings` (house prompt, three personas, table prefs) through the
 * same `updateSettings` action the actor already validates, clamps and
 * sanitises every field against — this route trusts none of its own input
 * and lets that action be the one place caps are enforced.
 */

import { fail, redirect } from '@sveltejs/kit';
import { AI_SEATS } from '$lib/actors/player-profile/types';
import {
	clearIdentityCookie,
	connectProfile,
	normalizeEmail,
	readIdentity,
	setIdentityCookie
} from '../games/_lib/session.server';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ cookies }) => {
	const identity = await readIdentity(cookies);
	if (!identity) return { identity: null };

	try {
		const profile = await connectProfile(identity);
		const settings = await profile.getSettings();
		return { identity, settings, loadError: false as const };
	} catch {
		return { identity, settings: null, loadError: true as const };
	}
};

function fieldName(seat: number, field: string): string {
	return `seat${seat}_${field}`;
}

export const actions: Actions = {
	identify: async ({ request, cookies, url }) => {
		const data = await request.formData();
		const email = normalizeEmail(data.get('email'));
		if (!email) {
			return fail(400, { error: 'That doesn’t look like an email address yet.' });
		}
		setIdentityCookie(cookies, email);
		redirect(303, url.pathname);
	},

	signout: async ({ cookies, url }) => {
		clearIdentityCookie(cookies);
		redirect(303, url.pathname);
	},

	save: async ({ request, cookies }) => {
		const identity = await readIdentity(cookies);
		if (!identity) {
			return fail(401, { error: 'Tell us who you are before saving.' });
		}

		const data = await request.formData();
		const patch = {
			housePrompt: String(data.get('housePrompt') ?? ''),
			table: {
				pace: String(data.get('pace') ?? 'normal'),
				banter: data.get('banter') === 'on',
				showRationale: data.get('showRationale') === 'on',
				sound: data.get('sound') === 'on',
				reduceMotion: data.get('reduceMotion') === 'on'
			},
			personas: AI_SEATS.map((seat) => ({
				seat,
				patch: {
					name: String(data.get(fieldName(seat, 'name')) ?? ''),
					blurb: String(data.get(fieldName(seat, 'blurb')) ?? ''),
					prompt: String(data.get(fieldName(seat, 'prompt')) ?? ''),
					difficulty: String(data.get(fieldName(seat, 'difficulty')) ?? 'casual'),
					aggression: Number(data.get(fieldName(seat, 'aggression')) ?? 0.5),
					risk: Number(data.get(fieldName(seat, 'risk')) ?? 0.5),
					chattiness: Number(data.get(fieldName(seat, 'chattiness')) ?? 0.5)
				}
			}))
		};

		try {
			const profile = await connectProfile(identity);
			const settings = await profile.updateSettings(patch);
			return { settings, saved: true as const };
		} catch {
			return fail(400, {
				error: 'Couldn’t save that — one of the fields may be too long or an odd value.'
			});
		}
	}
};
