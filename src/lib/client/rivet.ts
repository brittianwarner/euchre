/**
 * Browser Rivet client — one shared transport for the whole app.
 *
 * Uses app-local `createRivetContext` (not the removed package-global helpers).
 * The endpoint is this origin's `/api/rivet` mount: metadata discovery, then the
 * browser opens its WebSocket to Rivet Cloud (or the local engine).
 *
 * @see docs/06-REVISED-ARCHITECTURE.md §6
 */

import { browser } from '$app/environment';
import {
	createClient,
	createRivetContext,
	createSharedRivetKit
} from '@rivetkit/svelte';
import type { registry } from '$lib/actors/registry';

/** Typed context key for the euchre Rivet kit. */
export const rivetContext = createRivetContext<typeof registry>('EuchreRivet');

/** Lazy singleton client — created only in the browser. */
const getClient = (() => {
	let client: ReturnType<typeof createClient<typeof registry>> | null = null;
	return () => {
		if (!browser) {
			throw new Error('Rivet client is browser-only');
		}
		if (!client) {
			client = createClient<typeof registry>({
				endpoint: `${window.location.origin}/api/rivet`
			});
		}
		return client;
	};
})();

/**
 * Shared kit: `useActor` / `createReactiveActor` share one transport.
 * Action defaults live here — `ActorOptions` from framework-base does not
 * declare `actionDefaults`, so they are not set per-call.
 */
export const getRivet = createSharedRivetKit<typeof registry>(getClient, {
	actionDefaults: {
		timeout: 12_000,
		throwOnError: false,
		guardConnection: true
	}
});
