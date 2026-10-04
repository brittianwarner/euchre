import { jwtVerify, SignJWT } from 'jose';

const ISSUER = 'euchre-guest';
const encoder = new TextEncoder();
function key(): Uint8Array {
	const secret =
		process.env.EUCHRE_ACTOR_JWT_SECRET ||
		process.env.BETTER_AUTH_SECRET ||
		process.env.AUTH_SECRET;
	if (secret && secret.length >= 32) return encoder.encode(secret);
	if (process.env.NODE_ENV !== 'production' && !process.env.RIVET_ENDPOINT) {
		return encoder.encode('euchre-local-only-guest-signing-key');
	}
	throw new Error('Set EUCHRE_ACTOR_JWT_SECRET to at least 32 characters');
}

export async function signGuest(
	userId: string,
	audience: 'euchre-session' | 'euchre-actors'
): Promise<string> {
	return new SignJWT({ kind: 'guest' })
		.setProtectedHeader({ alg: 'HS256' })
		.setSubject(userId)
		.setIssuer(ISSUER)
		.setAudience(audience)
		.setIssuedAt()
		.setExpirationTime(audience === 'euchre-session' ? '400d' : '2h')
		.sign(key());
}

export async function verifyGuest(
	token: string,
	audience: 'euchre-session' | 'euchre-actors'
): Promise<string> {
	const { payload } = await jwtVerify(token, key(), {
		issuer: ISSUER,
		audience,
		algorithms: ['HS256']
	});
	if (payload.kind !== 'guest' || !payload.sub?.startsWith('guest-'))
		throw new Error('Invalid guest identity');
	return payload.sub;
}
