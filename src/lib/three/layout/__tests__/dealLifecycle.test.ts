/** Exercise each component's actual deal effect with compiled Svelte reactivity. */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { compileModule } from 'svelte/compiler';
import { transpileModule, ModuleKind, ScriptTarget } from 'typescript';
import { describe, expect, it } from 'vitest';

interface Inputs {
	handNo?: number;
	cards: string[];
	count: number;
	maxVisible: number;
	dealerSeat: number;
	seat: number;
	reducedMotion: boolean;
	animationTick: number;
}
interface Harness {
	patch: (changes: Partial<Inputs>) => void;
	created: () => number;
	keys: () => (string | number)[];
	dispose: () => void;
}

async function loadHarness(component: string): Promise<(initial: Inputs) => Harness> {
	const file = resolve('src/lib/three/layout', `${component}.svelte`);
	const source = readFileSync(file, 'utf8');
	const start = source.indexOf('let dealTweens =');
	const end = source.indexOf('\n\tfunction renderPose', start);
	if (start < 0 || end < 0) throw new Error(`Deal effect not found in ${file}`);
	const core = pathToFileURL(createRequire(import.meta.url).resolve('svelte/internal/client')).href;
	const moduleSource = `
		import { untrack, flush } from ${JSON.stringify(core)};
		export function create(initial) {
			let input = $state(initial);
			const handNo = $derived(input.handNo);
			const cards = $derived(input.cards);
			const visibleCount = $derived(Math.min(input.count, input.maxVisible));
			const seat = $derived(input.seat);
			const dealerSeat = $derived(input.dealerSeat);
			const reducedMotion = $derived(input.reducedMotion);
			let created = 0;
			class Tween<T> {
				current: number;
				constructor(value: number, _opts: unknown) { this.current = value; created++; }
				set(value: number, _opts: unknown) { input.animationTick; this.current = value; return Promise.resolve(); }
			}
			type CardId = string;
			const cubicOut = x => x;
			const TEMPO = { dealFlightMs: 60, dealStaggerMs: 50 };
			const flightMs = value => value;
			let keys = () => [];
			const dispose = $effect.root(() => {
				${source.slice(start, end)}
				keys = () => [...dealTweens.keys()];
			});
			flush();
			return { created: () => created, keys, dispose, patch(changes) { Object.assign(input, changes); flush(); } };
		}
	`;
	const js = transpileModule(moduleSource, {
		compilerOptions: { target: ScriptTarget.ES2022, module: ModuleKind.ESNext }
	}).outputText;
	const compiled = compileModule(js, {
		filename: `${component}-deal.svelte.js`,
		generate: 'client',
		dev: false
	}).js.code.replaceAll('svelte/internal/client', core);
	const module = await import(
		/* @vite-ignore */ `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
	);
	return module.create;
}

const initial = (): Inputs => ({
	handNo: 4,
	cards: ['9S', 'TS', 'JS', 'QS', 'KS'],
	count: 5,
	maxVisible: 5,
	dealerSeat: 0,
	seat: 1,
	reducedMotion: false,
	animationTick: 0
});

for (const component of ['Hand', 'OpponentHand']) {
	describe(`${component} deal lifecycle`, () => {
		it('snaps on mount and reconnect; same-hand updates never deal', async () => {
			const create = await loadHarness(component);
			const hand = create(initial());
			try {
				hand.patch({ count: 4, cards: ['9S', 'TS', 'JS', 'QS'] });
				hand.patch({ maxVisible: 3, dealerSeat: 2, reducedMotion: true });
				hand.patch({ reducedMotion: false, animationTick: 1 });
				expect(hand.created()).toBe(0);
				expect(hand.keys()).toEqual([]);
			} finally {
				hand.dispose();
			}
			const reconnect = create(initial());
			try {
				expect(reconnect.created()).toBe(0);
			} finally {
				reconnect.dispose();
			}
		});

		it('deals once per new hand, never when a card leaves or layout/tweens change', async () => {
			const hand = (await loadHarness(component))(initial());
			try {
				hand.patch({ handNo: 5 });
				expect(hand.created()).toBe(5);
				hand.patch({ count: 4, cards: ['9S', 'TS', 'JS', 'QS'] });
				hand.patch({ maxVisible: 3, dealerSeat: 2, seat: 3 });
				hand.patch({ reducedMotion: true });
				hand.patch({ reducedMotion: false, animationTick: 1 });
				hand.patch({ handNo: 5 });
				expect(hand.created()).toBe(5);
				hand.patch({ ...initial(), handNo: 6 });
				expect(hand.created()).toBe(10);
			} finally {
				hand.dispose();
			}
		});

		it('snaps a reduced-motion deal without replaying when motion is reenabled', async () => {
			const hand = (await loadHarness(component))(initial());
			try {
				hand.patch({ handNo: 5, reducedMotion: true });
				expect(hand.created()).toBe(0);
				hand.patch({ reducedMotion: false });
				expect(hand.created()).toBe(0);
			} finally {
				hand.dispose();
			}
		});
	});
}
