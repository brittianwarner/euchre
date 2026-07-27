/**
 * `reducedMotion.svelte.ts` — one reactive read of `prefers-reduced-motion`.
 *
 * A class holding `$state`, not a store (project convention). A single
 * instance is shared (module-level singleton) because it mirrors one piece of
 * ambient browser state — there is nothing per-consumer about it, and every
 * animating component in this layer (`Hand`, `OpponentHand`, `TrickPile`,
 * `TableScene`) needs to agree on the same value at the same instant.
 *
 * SSR-safe: `matchMedia` only exists in the browser, so the initial value is
 * `false` (motion allowed) until the constructor's browser branch runs; every
 * component here uses `<Canvas>` client-only anyway (see `EuchreTable3D.svelte`),
 * so this is never read during SSR in practice.
 */
export class ReducedMotion {
	enabled = $state(false);

	constructor() {
		if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
		const query = window.matchMedia('(prefers-reduced-motion: reduce)');
		this.enabled = query.matches;
		query.addEventListener('change', (e) => {
			this.enabled = e.matches;
		});
	}
}

/** Shared instance — see the class doc for why this is a singleton, not a factory. */
export const reducedMotion = new ReducedMotion();
