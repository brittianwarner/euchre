/** One authenticated transport per mounted game, shared by its reactive actions. */
import { createClient, createRivetKitWithClient } from '@rivetkit/svelte';
import type { TableRegistry } from './table-registry.ts';

interface TableSession {
	actorId: string;
	token: string;
	gatewayToken: string;
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
	// The Rust server has one gateway, `/rivet/**`, and it requires the table's
	// gateway token in every mode, dev included (D3 Option A). D19 sets JSON.
	const client = createClient<TableRegistry>({
		endpoint: `${window.location.origin}/rivet`,
		encoding: 'json',
		devtools: false,
		gateway: { skipReadyWait: true },
		getToken: async ({ forceRefresh }: { forceRefresh: boolean }) => {
			const value = await credentials(forceRefresh);
			if (!value.gatewayToken) throw new Error('Missing table gateway token');
			return value.gatewayToken;
		}
	});
	return {
		rivet: createRivetKitWithClient<TableRegistry>(client, {
			actionDefaults: { timeout: 12_000, throwOnError: false, guardConnection: true }
		}),
		getActorId: async () => (await credentials()).actorId,
		getParams: async () => ({ token: (await credentials()).token })
	};
}
