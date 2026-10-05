import { afterEach, describe, expect, it, vi } from 'vitest';
import { sendBounded } from './send-bounded';

afterEach(() => vi.useRealTimers());

describe('bounded sibling actor delivery', () => {
	it('releases the table when actor startup never completes, and cancels the request', async () => {
		vi.useFakeTimers();
		let signal: AbortSignal | undefined;
		const handle = {
			send: vi.fn((_name, _body, options) => {
				signal = options.signal;
				return new Promise(() => {});
			})
		};
		const delivery = sendBounded(handle, 'decide', {}, undefined);
		const check = expect(delivery).rejects.toThrow('interrupted');
		await vi.advanceTimersByTimeAsync(1000);
		await check;
		expect(signal?.aborted).toBe(true);
		expect(vi.getTimerCount()).toBe(0);
	});
	it('stops delivery when the table shuts down without waiting for its deadline', async () => {
		vi.useFakeTimers();
		const lifecycle = new AbortController();
		const handle = { send: vi.fn(() => new Promise(() => {})) };
		const delivery = sendBounded(handle, 'handEnd', {}, lifecycle.signal);
		const check = expect(delivery).rejects.toThrow('interrupted');
		lifecycle.abort();
		await check;
		expect(vi.getTimerCount()).toBe(0);
	});
	it('does not send anything for a stopped table', async () => {
		const lifecycle = new AbortController();
		lifecycle.abort();
		const handle = { send: vi.fn(async () => {}) };
		await expect(sendBounded(handle, 'decide', {}, lifecycle.signal)).rejects.toThrow();
		expect(handle.send).not.toHaveBeenCalled();
	});
});
