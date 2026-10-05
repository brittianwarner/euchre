import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { createRequire } from 'node:module';
import { describe, expect, it } from 'vitest';

const require = createRequire(import.meta.url);
const runtimeDir = dirname(require.resolve('rivetkit'));
const onChange = require(require.resolve('@rivetkit/on-change', { paths: [runtimeDir] })).default;

/** Exercise the private helper in the actual installed runtime, including both shipped bundles. */
function runtimeProxy(bundle: string) {
	const source = readFileSync(join(runtimeDir, bundle), 'utf8');
	const start = source.indexOf('function createWriteThroughProxy(');
	const end = source.indexOf('// src/registry/native.ts', start);
	if (start < 0 || end < 0) throw new Error('RivetKit proxy helper moved; update this regression');
	return new Function(
		'onChange',
		'_onchange2',
		source.slice(start, end) + '\nreturn createWriteThroughProxy;'
	)(onChange, { default: onChange }) as <T>(
		value: T,
		commit: (next: T) => void,
		validate?: (value: unknown) => void
	) => T;
}

describe.each(['mod.js', 'mod.cjs'])('RivetKit nested state in %s', (bundle) => {
	it('does not retain child proxies across repeated table-style reducer commits', () => {
		const watch = runtimeProxy(bundle);
		let raw = { game: { hand: { cards: ['JH', 'AH', '9H'] }, v: 0 }, journal: [{ card: '9S' }] };
		for (let turn = 0; turn < 64; turn++) {
			const state = watch(raw, (next) => {
				raw = next;
			});
			state.game = { ...state.game, v: state.game.v + 1 };
			state.journal = [...state.journal, { card: '9C' }].slice(-40);
			expect(onChange.target(raw.game.hand)).toBe(raw.game.hand);
			expect(onChange.target(raw.journal[0])).toBe(raw.journal[0]);
		}
		expect(raw.game.v).toBe(64);
	});
	it('keeps rejected writes unchanged instead of unwrapping their input', () => {
		const watch = runtimeProxy(bundle);
		const donor = watch({ hand: { cards: ['JH'] } }, () => {});
		const incoming = { hand: donor.hand };
		const retained = incoming.hand;
		const state = watch(
			{ game: { hand: { cards: ['9H'] } } },
			() => {},
			() => {
				throw new Error('rejected');
			}
		);
		expect(() => {
			state.game = incoming;
		}).toThrow('rejected');
		expect(incoming.hand).toBe(retained);
		expect(state.game.hand.cards).toEqual(['9H']);
	});
});
