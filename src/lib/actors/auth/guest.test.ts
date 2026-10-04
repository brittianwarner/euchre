import { describe, expect, it, vi, afterEach } from 'vitest';
import { signGuest, verifyGuest } from './guest';

afterEach(() => vi.unstubAllEnvs());
describe('anonymous match ownership', () => {
	it('separates session cookies from actor tokens and rejects tampering', async () => {
		vi.stubEnv('EUCHRE_ACTOR_JWT_SECRET', 'test-key-long-enough-for-hmac-signing-123');
		const cookie = await signGuest('guest-player-one', 'euchre-session');
		expect(await verifyGuest(cookie, 'euchre-session')).toBe('guest-player-one');
		await expect(verifyGuest(cookie, 'euchre-actors')).rejects.toThrow();
		await expect(verifyGuest(cookie.slice(0, -8) + 'tampered', 'euchre-session')).rejects.toThrow();
		const actorToken = await signGuest('guest-player-two', 'euchre-actors');
		expect(await verifyGuest(actorToken, 'euchre-actors')).toBe('guest-player-two');
	});
	it('refuses production without a strong signing key', async () => {
		vi.stubEnv('NODE_ENV', 'production');
		vi.stubEnv('EUCHRE_ACTOR_JWT_SECRET', 'short');
		await expect(signGuest('guest-player', 'euchre-actors')).rejects.toThrow('32 characters');
	});
});
