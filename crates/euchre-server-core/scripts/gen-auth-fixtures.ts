/**
 * Generate the cross-language auth fixtures in `crates/euchre-server-core/tests/fixtures/auth/`.
 *
 *     bun crates/euchre-server-core/scripts/gen-auth-fixtures.ts        # from the repo root
 *
 * Imports the real TS auth code (`apps/web/src/lib/actors/auth/guest.ts`,
 * `player-profile/auth.ts`, `euchre-table/queues.ts`, `routes/games/_lib/session.server.ts`)
 * with fixed secrets and a fixed clock (see `auth-fixture-env.ts`), and records each
 * TS verdict. `tests/foundation_auth.rs` replays every case against the Rust port.
 *
 * `rust_ok` is the verdict the Rust port must give; it equals the TS verdict except
 * for the deviations listed in `DEVIATIONS` (documented in docs/port/reports/server-foundation.md).
 */

import {
	ACTOR_SECRET,
	FIXTURES,
	IDENTITY_SECRET,
	INTERNAL_TOKEN,
	NOW_SECS,
	WEB,
	b64url,
	forge,
	outcome,
	setClockMs,
	setClockSecs
} from './auth-fixture-env.ts';

const { signGuest, verifyGuest } = await import(`${WEB}/lib/actors/auth/guest.ts`);
const { verifyProfileToken, assertInternalToken } = await import(
	`${WEB}/lib/actors/player-profile/auth.ts`
);
const { tokenOk } = await import(`${WEB}/lib/actors/euchre-table/queues.ts`);
const { mintProfileToken, normalizeEmail, deriveUserId } = await import(
	`${WEB}/routes/games/_lib/session.server.ts`
);
const { SignJWT } = await import(`${WEB}/../node_modules/jose/dist/webapi/index.js`);
// The cookie package SvelteKit's `cookies.get` decodes with (resolved from the kit itself).
const kitEntry = Bun.resolveSync('@sveltejs/kit', WEB);
const { parseCookie } = await import(Bun.resolveSync('cookie', kitEntry.slice(0, kitEntry.lastIndexOf('/'))));

const enc = new TextEncoder();
const UID = 'guest-6f9619ff-8b86-4d01-b42d-00cf4fc964ff';
const NOW = NOW_SECS;

/** Cases where Rust deliberately differs from TS (name -> Rust verdict). */
const DEVIATIONS: Record<string, { rust_ok: boolean; why: string }> = {
	guest_missing_exp: {
		rust_ok: false,
		why: 'Rust requires exp on guest tokens; jose treats it as optional. Every minted token has one.'
	},
	guest_sig_noncanonical_trailing_bits: {
		rust_ok: false,
		why: 'jsonwebtoken rejects non-canonical base64url; jose decodes loosely. Never produced by a signer.'
	}
};

// ------------------------------------------------------------------ guest

interface GuestCase {
	name: string;
	token: string;
	aud: 'euchre-session' | 'euchre-actors';
	now: number;
	ts_ok: boolean;
	ts_sub?: string;
	ts_error?: string;
	rust_ok: boolean;
	deviation?: string;
}

setClockSecs(NOW);
const session: string = await signGuest(UID, 'euchre-session');
const actors: string = await signGuest(UID, 'euchre-actors');
const guestMinted = [
	{ user_id: UID, aud: 'euchre-session', iat: NOW, token: session },
	{ user_id: UID, aud: 'euchre-actors', iat: NOW, token: actors }
];

const h = '{"alg":"HS256"}';
const payload = (extra: Record<string, unknown>): string =>
	JSON.stringify({
		kind: 'guest',
		sub: UID,
		iss: 'euchre-guest',
		aud: 'euchre-actors',
		iat: NOW,
		exp: NOW + 7200,
		...extra
	});
const [aHead, aPayload, aSig] = actors.split('.');
const flip = (c: string): string => (c === 'A' ? 'B' : 'A');
// Last char of a 32-byte signature carries 4 data bits + 2 zero bits; set the low bits.
const lastSigChar = aSig[aSig.length - 1];
const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
const noncanonical = alphabet[alphabet.indexOf(lastSigChar) | 1];

const guestInputs: Array<Omit<GuestCase, 'ts_ok' | 'ts_sub' | 'ts_error' | 'rust_ok'>> = [
	{ name: 'guest_session_ok', token: session, aud: 'euchre-session', now: NOW + 10 },
	{ name: 'guest_actors_ok_last_second', token: actors, aud: 'euchre-actors', now: NOW + 7199 },
	{ name: 'guest_actors_expired_at_exp', token: actors, aud: 'euchre-actors', now: NOW + 7200 },
	{ name: 'guest_session_as_actors', token: session, aud: 'euchre-actors', now: NOW },
	{ name: 'guest_actors_as_session', token: actors, aud: 'euchre-session', now: NOW },
	{
		name: 'guest_tampered_payload',
		token: `${aHead}.${b64url(payload({ sub: 'guest-someone-else' }))}.${aSig}`,
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_tampered_signature',
		token: `${aHead}.${aPayload}.${flip(aSig[0])}${aSig.slice(1)}`,
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_sig_noncanonical_trailing_bits',
		token: `${aHead}.${aPayload}.${aSig.slice(0, -1)}${noncanonical}`,
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_wrong_issuer',
		token: await forge(ACTOR_SECRET, h, payload({ iss: 'euchre-other' })),
		aud: 'euchre-actors',
		now: NOW
	},
	{ name: 'guest_alg_none', token: await forge(null, '{"alg":"none"}', payload({})), aud: 'euchre-actors', now: NOW },
	{
		name: 'guest_alg_hs512',
		token: await forge(ACTOR_SECRET, '{"alg":"HS512"}', payload({}), 'SHA-512'),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_wrong_key',
		token: await forge('another-secret-another-secret-another', h, payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{ name: 'guest_kind_not_guest', token: await forge(ACTOR_SECRET, h, payload({ kind: 'user' })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_sub_without_prefix', token: await forge(ACTOR_SECRET, h, payload({ sub: 'user-1' })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_nbf_future', token: await forge(ACTOR_SECRET, h, payload({ nbf: NOW + 100 })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_nbf_now_ok', token: await forge(ACTOR_SECRET, h, payload({ nbf: NOW })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_iat_not_number', token: await forge(ACTOR_SECRET, h, payload({ iat: 'x' })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_exp_not_number', token: await forge(ACTOR_SECRET, h, payload({ exp: 'x' })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_missing_exp', token: await forge(ACTOR_SECRET, h, payload({ exp: undefined })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_missing_iss', token: await forge(ACTOR_SECRET, h, payload({ iss: undefined })), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_missing_aud', token: await forge(ACTOR_SECRET, h, payload({ aud: undefined })), aud: 'euchre-actors', now: NOW },
	{
		name: 'guest_missing_iss_and_aud',
		token: await forge(ACTOR_SECRET, h, payload({ iss: undefined, aud: undefined })),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_unknown',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":["zzz"],"zzz":1}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_empty',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":[]}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_not_array',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":"b64","b64":true}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_b64_true_ok',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":["b64"],"b64":true}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_b64_missing',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":["b64"]}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_b64_not_bool',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":["b64"],"b64":"no"}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_crit_b64_false',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","crit":["b64"],"b64":false}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_aud_array_ok',
		token: await forge(ACTOR_SECRET, h, payload({ aud: ['x', 'euchre-actors'] })),
		aud: 'euchre-actors',
		now: NOW
	},
	{
		name: 'guest_typ_jwt_header_ok',
		token: await forge(ACTOR_SECRET, '{"alg":"HS256","typ":"JWT"}', payload({})),
		aud: 'euchre-actors',
		now: NOW
	},
	{ name: 'guest_payload_array', token: await forge(ACTOR_SECRET, h, '[1]'), aud: 'euchre-actors', now: NOW },
	{ name: 'guest_two_parts', token: `${aHead}.${aPayload}`, aud: 'euchre-actors', now: NOW },
	{ name: 'guest_empty', token: '', aud: 'euchre-actors', now: NOW }
];

const guestCases: GuestCase[] = [];
for (const c of guestInputs) {
	setClockSecs(c.now);
	const r = await outcome(() => verifyGuest(c.token, c.aud));
	const dev = DEVIATIONS[c.name];
	guestCases.push({
		...c,
		ts_ok: r.ok,
		...(r.ok ? { ts_sub: r.value as string } : { ts_error: r.error }),
		rust_ok: dev ? dev.rust_ok : r.ok,
		...(dev ? { deviation: dev.why } : {})
	});
}

// ------------------------------------------------------------------ profile

interface ProfileCase {
	name: string;
	token: string;
	now_ms: number;
	issuer: string | null;
	ok: boolean;
	user_id?: string;
	issued_at_ms?: number;
	expires_at_ms?: number;
	error?: string;
}

setClockSecs(NOW);
const profileUser = '4f2a9c1e0b7d8a6c5e3f1a2b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d';
const profileToken: string = await mintProfileToken(profileUser);
const ph = '{"alg":"HS256","typ":"JWT"}';
const pp = (o: Record<string, unknown>): string => JSON.stringify({ sub: 'a', aud: 'euchre-actors', ...o });
const [pHead, pPayload, pSig] = profileToken.split('.');
const pNoncanonical = alphabet[alphabet.indexOf(pSig[pSig.length - 1]) | 1];
const invalidUtf8 = new Uint8Array([
	...enc.encode('{"sub":"a'),
	0xff,
	0xc3,
	...enc.encode('","aud":"euchre-actors","exp":1800001800}')
]);
const bomPayload = new Uint8Array([0xef, 0xbb, 0xbf, ...enc.encode(pp({ exp: NOW + 1800 }))]);
const NOW_MS = NOW * 1000 + 5_000;

const profileInputs: Array<Omit<ProfileCase, 'ok' | 'user_id' | 'issued_at_ms' | 'expires_at_ms' | 'error'>> = [
	{ name: 'profile_minted_ok', token: profileToken, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_minted_wrong_issuer', token: profileToken, now_ms: NOW_MS, issuer: 'euchre-test' },
	{
		name: 'profile_issuer_ok',
		token: await forge(ACTOR_SECRET, ph, pp({ iss: 'euchre-test', iat: NOW, exp: NOW + 1800 })),
		now_ms: NOW_MS,
		issuer: 'euchre-test'
	},
	{ name: 'profile_expired_past_skew', token: profileToken, now_ms: (NOW + 1800) * 1000 + 60_001, issuer: null },
	{ name: 'profile_expired_within_skew', token: profileToken, now_ms: (NOW + 1800) * 1000 + 60_000, issuer: null },
	{ name: 'profile_alg_none', token: await forge(null, '{"alg":"none"}', pp({ exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_alg_hs512', token: await forge(ACTOR_SECRET, '{"alg":"HS512"}', pp({ exp: NOW + 1800 }), 'SHA-512'), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_wrong_audience', token: await forge(ACTOR_SECRET, ph, pp({ aud: 'euchre-session', exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_aud_array', token: await forge(ACTOR_SECRET, ph, pp({ aud: ['x', 'euchre-actors'], exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_no_expiry', token: await forge(ACTOR_SECRET, ph, pp({})), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_exp_string', token: await forge(ACTOR_SECRET, ph, pp({ exp: '1800001800' })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_exp_float', token: await forge(ACTOR_SECRET, ph, pp({ exp: NOW + 1800.5, iat: NOW + 0.25 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_nbf_future', token: await forge(ACTOR_SECRET, ph, pp({ nbf: NOW + 66, exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_nbf_within_skew', token: await forge(ACTOR_SECRET, ph, pp({ nbf: NOW + 65, exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_iat_future', token: await forge(ACTOR_SECRET, ph, pp({ iat: NOW + 66, exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_lifetime_too_long', token: await forge(ACTOR_SECRET, ph, pp({ iat: NOW, exp: NOW + 3601 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_lifetime_max_ok', token: await forge(ACTOR_SECRET, ph, pp({ iat: NOW, exp: NOW + 3600 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_no_iat_long_ok', token: await forge(ACTOR_SECRET, ph, pp({ exp: NOW + 86_400 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_missing', token: await forge(ACTOR_SECRET, ph, pp({ sub: undefined, exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_empty', token: await forge(ACTOR_SECRET, ph, pp({ sub: '', exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_number', token: await forge(ACTOR_SECRET, ph, pp({ sub: 7, exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_128_ok', token: await forge(ACTOR_SECRET, ph, pp({ sub: 's'.repeat(128), exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_129', token: await forge(ACTOR_SECRET, ph, pp({ sub: 's'.repeat(129), exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_64_astral_ok', token: await forge(ACTOR_SECRET, ph, pp({ sub: '\u{1F0A1}'.repeat(64), exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sub_65_astral', token: await forge(ACTOR_SECRET, ph, pp({ sub: '\u{1F0A1}'.repeat(65), exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_header_null', token: await forge(ACTOR_SECRET, 'null', pp({ exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_header_array', token: await forge(ACTOR_SECRET, '[]', pp({ exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_payload_array', token: await forge(ACTOR_SECRET, ph, '[1]'), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_payload_string', token: await forge(ACTOR_SECRET, ph, '"x"'), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_not_json', token: `${pHead}.${b64url('{nope')}.${pSig}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_bad_base64', token: `${pHead}.%%%.${pSig}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_base64_len_mod4_1', token: `${pHead}.${pPayload}A.${pSig}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_padded_payload_segment', token: `${pHead}.${pPayload}${'='.repeat((4 - (pPayload.length % 4)) % 4)}.${pSig}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_tampered_payload', token: `${pHead}.${b64url(pp({ sub: 'b', iat: NOW, exp: NOW + 1800 }))}.${pSig}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_bad_signature_chars', token: `${pHead}.${pPayload}.*${pSig.slice(1)}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_sig_noncanonical_ok', token: `${pHead}.${pPayload}.${pSig.slice(0, -1)}${pNoncanonical}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_wrong_key', token: await forge('wrong-wrong-wrong-wrong', ph, pp({ exp: NOW + 1800 })), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_invalid_utf8_lossy', token: await forge(ACTOR_SECRET, ph, invalidUtf8), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_bom_payload', token: await forge(ACTOR_SECRET, ph, bomPayload), now_ms: NOW_MS, issuer: null },
	{ name: 'profile_two_parts', token: `${pHead}.${pPayload}`, now_ms: NOW_MS, issuer: null },
	{ name: 'profile_four_parts', token: `${profileToken}.x`, now_ms: NOW_MS, issuer: null }
];

const profileCases: ProfileCase[] = [];
for (const c of profileInputs) {
	if (c.issuer) process.env.EUCHRE_JWT_ISSUER = c.issuer;
	else delete process.env.EUCHRE_JWT_ISSUER;
	const r = await outcome(() => verifyProfileToken(c.token, c.now_ms));
	delete process.env.EUCHRE_JWT_ISSUER;
	if (r.ok) {
		const v = r.value as { userId: string; issuedAt: number; expiresAt: number };
		profileCases.push({ ...c, ok: true, user_id: v.userId, issued_at_ms: v.issuedAt, expires_at_ms: v.expiresAt });
	} else {
		profileCases.push({ ...c, ok: false, error: r.error });
	}
}

// ------------------------------------------------------------------ identity

const emailInputs = [
	'Ann@Example.com',
	'  Ann@Example.COM \n',
	' ﻿a@b.co ',
	'a@b',
	'@b.co',
	'a@.co',
	'a@b.',
	'a@b..',
	'a@@b.co',
	'a b@c.co',
	'a@b.c d',
	'',
	'   ',
	'ÅNN@EXAMPLE.COM',
	'İ@b.co',
	'ΣΑ@b.co',
	'ΑΣ@b.co',
	'xΣ@b.co',
	'straße@b.co',
	'\u{1F0A1}@b.co',
	`${'a'.repeat(248)}@b.co`,
	`${'a'.repeat(249)}@b.co`,
	`${'a'.repeat(250)}@b.co`,
	'a​@b.co',
	'a@b.co\u0000'
];
const emails = [];
for (const raw of emailInputs) {
	const normalized: string | null = normalizeEmail(raw);
	emails.push({ raw, normalized, user_id: normalized === null ? null : await deriveUserId(normalized) });
}

setClockSecs(NOW);
const idEmail = 'ann@example.com';
const idUser: string = await deriveUserId(idEmail);
const identityToken = await new SignJWT({ kind: 'email-nick', sub: idUser, em: idEmail })
	.setProtectedHeader({ alg: 'HS256' })
	.setSubject(idUser)
	.setIssuer('euchre-identity')
	.setAudience('euchre-identity')
	.setIssuedAt()
	.setExpirationTime('400d')
	.sign(enc.encode(IDENTITY_SECRET));

// Legacy plain-email cookies: TS `cookies.set` stored `encodeURIComponent(email)`, and
// `readIdentity` read `cookies.get` (cookie@2 decode) through normalizeEmail.
const legacyInputs = [
	'ann%40example.com',
	'Ann%40Example.COM',
	'ann@example.com',
	'ann%zz@example.com',
	'ann%40example.com%E0',
	'%C3%A9@b.co',
	'a%2540b.co',
	'%20ann%40example.com%20'
];
const legacyCookies = [];
for (const raw of legacyInputs) {
	const decoded: string | undefined = parseCookie(`euchre_email=${raw}`).euchre_email;
	const email: string | null = normalizeEmail(decoded);
	legacyCookies.push({ raw, decoded, email, user_id: email === null ? null : await deriveUserId(email) });
}

// ------------------------------------------------------------------ internal

const internalCases = [];
for (const [name, token] of [
	['internal_exact', INTERNAL_TOKEN],
	['internal_wrong', INTERNAL_TOKEN.slice(0, -1) + 'X'],
	['internal_prefix', INTERNAL_TOKEN.slice(0, 10)],
	['internal_longer', INTERNAL_TOKEN + 'x'],
	['internal_empty', ''],
	['internal_absent', null]
] as const) {
	const r = await outcome(() => assertInternalToken(token ?? undefined));
	internalCases.push({ name, token, ok: r.ok, ...(r.ok ? {} : { error: r.error }) });
}
const matchToken = '2c5ea4c0-4067-4f1a-9c4c-6b1a3d1f8e2a';
const matchCases = [
	{ expected: matchToken, got: matchToken },
	{ expected: matchToken, got: matchToken.toUpperCase() },
	{ expected: matchToken, got: matchToken.slice(0, 35) },
	{ expected: matchToken, got: null },
	{ expected: '', got: '' },
	{ expected: 'été', got: 'été' },
	{ expected: 'é', got: 'e' }
].map((c) => ({ ...c, ok: tokenOk(c.expected, c.got ?? undefined) as boolean }));

// ------------------------------------------------------------------ write

setClockMs(NOW * 1000);
const meta = {
	generated_by: 'crates/euchre-server-core/scripts/gen-auth-fixtures.ts',
	actor_secret: ACTOR_SECRET,
	internal_token: INTERNAL_TOKEN,
	identity_secret: IDENTITY_SECRET,
	now_secs: NOW
};
const write = (file: string, body: unknown): Promise<number> =>
	Bun.write(`${FIXTURES}/${file}`, `${JSON.stringify({ ...meta, ...(body as object) }, null, '\t')}\n`);

await write('guest.json', { minted: guestMinted, cases: guestCases });
await write('profile.json', {
	minted: [{ user_id: profileUser, iat: NOW, token: profileToken }],
	cases: profileCases
});
await write('identity.json', {
	emails,
	legacy_cookies: legacyCookies,
	minted: [{ email: idEmail, user_id: idUser, iat: NOW, token: identityToken }]
});
await write('internal.json', { cases: internalCases, match_cases: matchCases });
console.log(
	`wrote ${guestCases.length} guest, ${profileCases.length} profile, ${emails.length} email, ${internalCases.length + matchCases.length} internal cases to ${FIXTURES}`
);
