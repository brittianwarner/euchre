import { describe, expect, test } from 'bun:test';
import { createPublicOriginResolver } from './public-origin';

describe('public domain routing behind Railway TLS termination', () => {
	const originFor = createPublicOriginResolver([
		'https://euchre.sh',
		'https://www.euchre.sh',
		'https://euchre-production-9809.up.railway.app'
	]);
	test.each(['euchre.sh', 'www.euchre.sh', 'euchre-production-9809.up.railway.app'])(
		'preserves %s for session checks and cookie-bound WebSockets',
		(host) => {
			const origin = originFor(new URL(`http://${host}/api/table-session/game`));
			expect(origin).toBe(`https://${host}`);
		}
	);
	test.each(['attacker.example', 'euchre.sh.attacker.example', 'www.euchre.sh:444'])(
		'rejects unconfigured host %s',
		(host) => expect(originFor(new URL(`http://${host}/`))).toBeUndefined()
	);
	test.each(['https://euchre.sh/path', 'https://user:password@euchre.sh', 'ftp://euchre.sh'])(
		'rejects invalid configured origin %s',
		(origin) => expect(() => createPublicOriginResolver([origin])).toThrow()
	);
	test('rejects ambiguous schemes for the same host', () => {
		expect(() => createPublicOriginResolver(['http://euchre.sh', 'https://euchre.sh'])).toThrow();
	});
});
