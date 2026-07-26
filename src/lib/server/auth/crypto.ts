/**
 * Small crypto primitives shared by the auth subsystem.
 *
 * Everything here is Web Crypto (`globalThis.crypto.subtle`), which exists in
 * every runtime this code touches: node 24 on Vercel, bun locally, and inside
 * the Rivet actor (same process — docs/06 §2). No node `crypto` import, so
 * nothing here pins the module to a node build.
 */

const encoder = new TextEncoder();

/** Base64url-encode bytes, no padding. */
export function base64UrlEncode(bytes: Uint8Array): string {
	let binary = '';
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/**
 * Base64url-decode. Returns `null` on anything that is not well-formed rather
 * than throwing, because every caller is parsing attacker-supplied input.
 */
export function base64UrlDecode(value: string): Uint8Array | null {
	if (!/^[A-Za-z0-9_-]*$/.test(value)) return null;
	const padded = value.replace(/-/g, '+').replace(/_/g, '/');
	const withPadding = padded + '='.repeat((4 - (padded.length % 4)) % 4);
	try {
		const binary = atob(withPadding);
		const bytes = new Uint8Array(binary.length);
		for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
		return bytes;
	} catch {
		return null;
	}
}

/** Import a UTF-8 secret as an HMAC-SHA-256 key. */
async function importHmacKey(secret: string, usages: KeyUsage[]): Promise<CryptoKey> {
	return crypto.subtle.importKey(
		'raw',
		encoder.encode(secret),
		{ name: 'HMAC', hash: 'SHA-256' },
		false,
		usages
	);
}

/** HMAC-SHA-256 over `message`, base64url-encoded. */
export async function hmacSign(secret: string, message: string): Promise<string> {
	const key = await importHmacKey(secret, ['sign']);
	const signature = await crypto.subtle.sign('HMAC', key, encoder.encode(message));
	return base64UrlEncode(new Uint8Array(signature));
}

/**
 * Verify an HMAC produced by {@link hmacSign}.
 *
 * Uses `crypto.subtle.verify` rather than comparing strings, so the comparison
 * does not short-circuit on the first differing byte. Returns `false` — never
 * throws — for a malformed signature.
 */
export async function hmacVerify(
	secret: string,
	message: string,
	signature: string
): Promise<boolean> {
	const bytes = base64UrlDecode(signature);
	if (!bytes) return false;
	const key = await importHmacKey(secret, ['verify']);
	try {
		return await crypto.subtle.verify(
			'HMAC',
			key,
			bytes as unknown as BufferSource,
			encoder.encode(message)
		);
	} catch {
		return false;
	}
}

/**
 * Constant-time string comparison.
 *
 * Compares over the longer of the two lengths so that an early length mismatch
 * does not itself leak. Used for the `authStore` internal-connection token,
 * where a naive `===` would leak the token a byte at a time.
 */
export function timingSafeEqual(a: string, b: string): boolean {
	const length = Math.max(a.length, b.length);
	let diff = a.length ^ b.length;
	for (let i = 0; i < length; i += 1) {
		diff |= (a.charCodeAt(i) || 0) ^ (b.charCodeAt(i) || 0);
	}
	return diff === 0;
}

/** SHA-256 of `value`, base64url-encoded. */
export async function sha256(value: string): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', encoder.encode(value));
	return base64UrlEncode(new Uint8Array(digest));
}
