<!--
  TableStatusBar — the "never confused" chip: a persistent, calm "YOUR TURN"
  indicator, plus a transient plain-language explanation when a tap on the 3D
  table hit an illegal card ("no dead clicks" — see docs/04-FRONTEND-UX.md §8).

  Both signals are DOM, on top of the canvas, so they read at any zoom level
  and are never the only channel (color is never load-bearing here — the
  turn chip carries a dot *and* the word "turn"; the illegal notice is text).
-->
<script lang="ts">
	import type { PublicGameView } from '$lib/protocol';

	let {
		view,
		illegalMessage = null
	}: {
		view: PublicGameView;
		illegalMessage?: string | null;
	} = $props();

	const yourTurn = $derived(view.status === 'active' && view.turnSeat === view.you);
</script>

<div class="status-rail">
	{#if yourTurn}
		<p class="turn-chip" role="status">
			<span class="dot" aria-hidden="true"></span>
			Your turn
		</p>
	{/if}
	{#if illegalMessage}
		<p class="illegal-toast" role="alert">{illegalMessage}</p>
	{/if}
</div>

<style>
	.status-rail {
		position: absolute;
		bottom: calc(1rem + env(safe-area-inset-bottom));
		left: 50%;
		transform: translateX(-50%);
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.5rem;
		pointer-events: none;
		z-index: 2;
	}
	.turn-chip {
		margin: 0;
		display: flex;
		align-items: center;
		gap: 0.4rem;
		padding: 0.4rem 0.85rem;
		border-radius: 999px;
		background: rgba(20, 26, 16, 0.85);
		border: 1px solid rgba(232, 194, 122, 0.55);
		color: #f4ecd8;
		font-weight: 600;
		font-size: 0.85rem;
		letter-spacing: 0.02em;
		box-shadow: 0 2px 10px rgba(0, 0, 0, 0.4);
	}
	.dot {
		width: 0.5rem;
		height: 0.5rem;
		border-radius: 50%;
		background: #e8c27a;
		animation: pulse 1.8s ease-in-out infinite;
	}
	@media (prefers-reduced-motion: reduce) {
		.dot {
			animation: none;
		}
	}
	@keyframes pulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.35;
		}
	}
	.illegal-toast {
		margin: 0;
		max-width: min(90vw, 26rem);
		padding: 0.55rem 0.9rem;
		border-radius: 0.5rem;
		background: rgba(74, 32, 24, 0.92);
		border: 1px solid rgba(224, 140, 110, 0.55);
		color: #f7e2d8;
		font-size: 0.85rem;
		text-align: center;
	}
</style>
