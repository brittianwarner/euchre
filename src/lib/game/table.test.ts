import { describe, expect, it, vi } from 'vitest';
import { TableStore, type TableActorHandle } from './table.svelte';
import { createGame, project } from '#lib/euchre/index.ts';
import type { PublicGameView } from '#lib/protocol/index.ts';

const view = (v: number, turnId: string): PublicGameView =>
	({
		...project(createGame({ gameId: 'store-test', seed: 'store-test' }), 0),
		v,
		turnId,
		legal: [{ id: 'play:9H', move: { t: 'play', card: '9H' }, label: 'Nine of hearts' }]
	}) as PublicGameView;
function fixture(submitMove: TableActorHandle['submitMove'], snapshot = view(2, 'next')) {
	const store = new TableStore();
	const handle = { isConnected: true, submitMove, snapshot: vi.fn(async () => snapshot) };
	store.bind(handle);
	store.applySync({ view: view(1, 'turn'), steps: [] } as never);
	return { store, handle };
}
describe('uncertain move acknowledgements', () => {
	it('recognizes a committed move after its acknowledgement is lost, without resubmitting', async () => {
		const submit = vi.fn(async () => undefined);
		const { store, handle } = fixture(submit);
		expect(await store.play('play:9H')).toBe(true);
		expect(submit).toHaveBeenCalledTimes(1);
		expect(handle.snapshot).toHaveBeenCalledTimes(1);
		expect(store.view?.turnId).toBe('next');
		expect(store.error).toBeNull();
	});
	it('offers a retry only when the same authoritative turn is still current', async () => {
		const { store } = fixture(async () => ({ ok: false, code: 'stale_turn' }), view(1, 'turn'));
		expect(await store.play('play:9H')).toBe(false);
		expect(store.error).toContain('Please try again');
		expect(store.submitting).toBe(false);
	});
	it('holds newer sync state when an older snapshot arrives', async () => {
		const { store } = fixture(async () => undefined, view(1, 'turn'));
		store.applySync({ view: view(3, 'newer'), steps: [] } as never);
		await store.resync();
		expect(store.view?.v).toBe(3);
		expect(store.view?.turnId).toBe('newer');
	});
});
