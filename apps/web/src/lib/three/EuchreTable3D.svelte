<!--
  EuchreTable3D.svelte — the 3D table. Public entry point for the play route.

  Renders PURELY from the `view` prop: no local game state, no fetch, no
  socket. The caller (`play/[gameId]/+page.svelte`) owns the connection and
  passes down whichever `PublicGameView` it wants shown this frame (server
  truth, or a locally-`applyOptimistic`'d one while a move is in flight) —
  this component cannot tell the difference and does not need to.

  Structural non-leakage, inherited from what it composes: `view.hand` is the
  viewer's own cards; every other seat is drawn from `view.handCounts` only
  (`OpponentHand` has no prop that could carry a foreign `CardId` — see its
  own doc comment). This component adds no new card-shaped data path.

  `<Canvas>` mounts its children client-only (Threlte, not this file, owns
  that guarantee) — this is exactly why the composing route sets
  `export const ssr = false` in its own `+page.ts` rather than this file
  needing a `browser` check.

  The actual furniture/cards/nameplates live in `TableScene.svelte`, which
  must run as a child of `<Canvas>` for `interactivity()` to have a Threlte
  context to attach to — see that file's doc comment for why this is two
  files rather than one.
-->
<script lang="ts">
	import { Canvas } from '@threlte/core';
	import { NeutralToneMapping, PCFShadowMap, SRGBColorSpace } from 'three';
	import TableScene from './TableScene.svelte';
	import type { PublicGameView } from '#lib/euchre/index.ts';

	interface Props {
		view: PublicGameView;
		/** Four-colour deck (accessibility default; see docs/04-FRONTEND-UX.md §13). */
		fourColor?: boolean;
	}

	// Classic two-colour is the default deck. The four-colour deck is an
	// accessibility option a player opts into, not something to impose on
	// everyone — a lifelong euchre player opening this should see the deck they
	// have played with their whole life.
	let { view, fourColor = false }: Props = $props();

	// Clamped device pixel ratio: sharp on retina without paying for a 3x
	// buffer on a phone that reports one. `window` is safe unconditionally —
	// the composing route is `ssr = false`, so this component only ever
	// mounts in the browser.
	const dpr = Math.min(2, typeof window !== 'undefined' ? window.devicePixelRatio : 1);
</script>

<!--
	`aria-hidden`: this canvas is the *visual* layer only. Every fact a
	screen-reader user needs (score, trump, whose turn, playable cards) is
	surfaced by the DOM siblings the composing page renders on top —
	`CardRack` supplies native buttons for every card. Exposing this as a second, separately-announced `role="img"`
	summary would either duplicate that or drift from it; hiding it avoids
	both.
-->
<div class="euchre-stage" aria-hidden="true">
	<Canvas
		{dpr}
		shadows={PCFShadowMap}
		toneMapping={NeutralToneMapping}
		colorSpace={SRGBColorSpace}
		renderMode="on-demand"
	>
		<TableScene {view} {fourColor} />
	</Canvas>
</div>

<style>
	.euchre-stage {
		position: absolute;
		inset: 88px 0 140px;
		background: transparent;
		touch-action: none;
	}
	@media (max-width: 600px) {
		.euchre-stage {
			inset: 90px 0 150px;
		}
	}
	@media (min-width: 701px) and (max-height: 780px) {
		.euchre-stage {
			bottom: 190px;
		}
	}
</style>
