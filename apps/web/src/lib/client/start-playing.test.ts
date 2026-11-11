import { describe, expect, it, vi } from 'vitest';
import { ApiError } from './api.ts';
import { startPlaying } from './start-playing.ts';

const GAME_ID = '0195f0c2-7a3e-7d10-9b1a-2f6c1d8e4a90';

describe('startPlaying', () => {
	it('creates a table with the default house rules, then opens it', async () => {
		const fetchFn = vi.fn(async () => Response.json({ gameId: GAME_ID }));
		const navigate = vi.fn(async () => {});

		await expect(startPlaying({ fetch: fetchFn as typeof fetch, navigate })).resolves.toBe(GAME_ID);

		expect(fetchFn).toHaveBeenCalledTimes(1);
		const [path, init] = fetchFn.mock.calls[0] as unknown as [string, RequestInit];
		expect(path).toBe('/api/tables');
		expect(init.method).toBe('POST');
		expect(init.credentials).toBe('same-origin');
		expect(JSON.parse(String(init.body))).toEqual({ stick: false, natural: false });
		expect(navigate).toHaveBeenCalledExactlyOnceWith(`/play/${GAME_ID}`);
	});

	it('does not navigate when the table could not be created', async () => {
		const fetchFn = vi.fn(async () =>
			Response.json(
				{ message: 'Too many requests', error: { code: 'rate_limited', message: 'Slow down' } },
				{ status: 429 }
			)
		);
		const navigate = vi.fn(async () => {});

		const result = startPlaying({ fetch: fetchFn as typeof fetch, navigate });
		await expect(result).rejects.toBeInstanceOf(ApiError);
		await expect(result).rejects.toMatchObject({ status: 429 });
		expect(navigate).not.toHaveBeenCalled();
	});
});
