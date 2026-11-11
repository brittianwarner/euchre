/**
 * Verify Rust-minted tokens with the real TS verifiers (the reverse of gen-auth-fixtures.ts).
 *
 *     bun crates/euchre-server-core/scripts/verify-rust-tokens.ts       # from the repo root
 *
 * Reads `crates/euchre-server-core/tests/fixtures/auth/rust-minted.json` (written by
 * `EUCHRE_WRITE_RUST_TOKENS=1 cargo test -p euchre-server-core --test foundation_auth`) and
 * checks each token with `verifyGuest`, `verifyProfileToken` and jose `jwtVerify`.
 * Exits non-zero on the first mismatch.
 */

import { FIXTURES, IDENTITY_SECRET, WEB, setClockSecs } from './auth-fixture-env.ts';

const { verifyGuest } = await import(`${WEB}/lib/actors/auth/guest.ts`);
const { verifyProfileToken } = await import(`${WEB}/lib/actors/player-profile/auth.ts`);
const { deriveUserId } = await import(`${WEB}/routes/games/_lib/session.server.ts`);
const { jwtVerify } = await import(`${WEB}/../node_modules/jose/dist/webapi/index.js`);

const minted = await Bun.file(`${FIXTURES}/rust-minted.json`).json();
let checked = 0;
const fail = (what: string): never => {
	console.error(`FAIL ${what}`);
	process.exit(1);
};

for (const g of minted.guest) {
	setClockSecs(g.iat + 1);
	const sub = await verifyGuest(g.token, g.aud);
	if (sub !== g.user_id) fail(`guest ${g.aud}: ${sub} != ${g.user_id}`);
	const other = g.aud === 'euchre-session' ? 'euchre-actors' : 'euchre-session';
	const crossed = await verifyGuest(g.token, other).then(
		() => true,
		() => false
	);
	if (crossed) fail(`guest ${g.aud} token accepted as ${other}`);
	checked++;
}

for (const p of minted.profile) {
	if (p.issuer) process.env.EUCHRE_JWT_ISSUER = p.issuer;
	else delete process.env.EUCHRE_JWT_ISSUER;
	const nowMs = p.iat * 1000 + 1000;
	const v = await verifyProfileToken(p.token, nowMs);
	if (v.userId !== p.user_id) fail(`profile: ${v.userId} != ${p.user_id}`);
	if (v.issuedAt !== p.iat * 1000 || v.expiresAt !== (p.iat + 1800) * 1000)
		fail(`profile times: ${v.issuedAt} ${v.expiresAt}`);
	delete process.env.EUCHRE_JWT_ISSUER;
	checked++;
}

for (const id of minted.identity) {
	setClockSecs(id.iat + 1);
	const { payload, protectedHeader } = await jwtVerify(id.token, new TextEncoder().encode(IDENTITY_SECRET), {
		issuer: 'euchre-identity',
		audience: 'euchre-identity',
		algorithms: ['HS256']
	});
	if (protectedHeader.alg !== 'HS256' || payload.kind !== 'email-nick') fail('identity header/kind');
	if (payload.em !== id.email || payload.sub !== id.user_id) fail('identity claims');
	if ((await deriveUserId(id.email)) !== id.user_id) fail('identity sub is not deriveUserId(em)');
	if (payload.exp !== id.iat + 400 * 86_400) fail(`identity exp ${payload.exp}`);
	checked++;
}

console.log(`ok: TS verified ${checked} Rust-minted tokens`);
