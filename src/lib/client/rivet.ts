/** One authenticated transport per mounted game, shared by its reactive actions. */
import { dev } from '$app/env';
import { createClient, createRivetKitWithClient } from '@rivetkit/svelte';
import type { registry } from '#lib/actors/registry.ts';

interface TableSession {
	token: string;
	gatewayToken?: string;
	expiresAt: number;
}

export function createGameRivet(gameId: string) {
	let session: TableSession | undefined;
	let pending: Promise<TableSession> | undefined;
	function credentials(forceRefresh = false): Promise<TableSession> {
		if (!forceRefresh && session && Date.now() < session.expiresAt - 10_000) {
			return Promise.resolve(session);
		}
		return (pending ??= fetch(`/api/table-session/${encodeURIComponent(gameId)}`, {
			method: 'POST',
			credentials: 'same-origin',
			headers: { 'Content-Type': 'application/json' }
		})
			.then(async (response) => {
				if (!response.ok)
					throw new Error(
						response.status === 403
							? 'This table belongs to another browser. Open Your games to find your table.'
							: response.status === 401
								? 'Please start a game in this browser first.'
								: 'Your table is temporarily unavailable. Please reconnect.'
					);
				session = (await response.json()) as TableSession;
				return session;
			})
			.finally(() => {
				pending = undefined;
			}));
	}
	const client = createClient<typeof registry>({
		endpoint: `${window.location.origin}/api/rivet`,
		devtools: false,
		gateway: { skipReadyWait: true },
		...(dev
			? {}
			: {
					getToken: async ({ forceRefresh }: { forceRefresh: boolean }) => {
						const value = await credentials(forceRefresh);
						if (!value.gatewayToken) throw new Error('Missing table gateway token');
						return value.gatewayToken;
					}
				})
	});
	return {
		rivet: createRivetKitWithClient<typeof registry>(client, {
			actionDefaults: { timeout: 12_000, throwOnError: false, guardConnection: true }
		}),
		getParams: async () => ({ token: (await credentials()).token })
	};
}
