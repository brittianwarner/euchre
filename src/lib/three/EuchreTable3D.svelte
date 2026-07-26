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
	import { NeutralToneMapping, SRGBColorSpace } from 'three';
	import TableScene from './TableScene.svelte';
	import type { CardId, PublicGameView } from '$lib/euchre';

	interface Props {
		view: PublicGameView;
		/** Four-colour deck (accessibility default; see docs/04-FRONTEND-UX.md §13). */
		fourColor?: boolean;
		/** True while a move is in flight — cards stop responding, legality shading does not. */
		disabled?: boolean;
		/** Fires when a legal card in the human's own hand is tapped. */
		onplay?: (cardId: CardId) => void;
		/** Fires when an illegal card is tapped, so the caller can show the engine's `whyIllegal` copy. */
		onillegal?: (cardId: CardId) => void;
	}

	let { view, fourColor = true, disabled = false, onplay, onillegal }: Props = $props();

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
	`HandA11y` is "a complete, playable card table on its own" per its own
	doc comment. Exposing this as a second, separately-announced `role="img"`
	summary would either duplicate that or drift from it; hiding it avoids
	both.
-->
<div class="euchre-stage" aria-hidden="true">
	<Canvas
		{dpr}
		shadows={false}
		toneMapping={NeutralToneMapping}
		colorSpace={SRGBColorSpace}
		renderMode="on-demand"
	>
		<TableScene {view} {fourColor} {disabled} {onplay} {onillegal} />
	</Canvas>
</div>

<style>
	.euchre-stage {
		position: absolute;
		inset: 0;
		background: radial-gradient(ellipse at 50% 35%, #1c1712 0%, #0b0906 70%, #060504 100%);
		touch-action: none;
	}
</style>
