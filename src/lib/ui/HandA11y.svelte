<!--
  HandA11y — focusable button list for the human hand / legal plays.
  M2 ships this un-hidden as the playable card table (no 3D yet).
-->
<script lang="ts">
	import type { LegalMoveId, PublicGameView } from '$lib/protocol';

	let {
		view,
		disabled = false,
		onPlay
	}: {
		view: PublicGameView;
		disabled?: boolean;
		onPlay: (moveId: LegalMoveId) => void;
	} = $props();

	/** Card play moves only — bidding/discard live in sibling panels. */
	const playMoves = $derived(view.legal.filter((m) => m.id.startsWith('play:')));

	const playableIds = $derived(new Set(playMoves.map((m) => m.id)));

	function cardMoveId(card: string): LegalMoveId {
		return `play:${card}` as LegalMoveId;
	}
</script>

<section class="hand" aria-label="Your hand">
	{#if view.phase === 'trick_play' && view.turnSeat === view.you}
		<p class="hint">Play a card</p>
	{:else if view.hand.length > 0}
		<p class="hint">Your cards</p>
	{/if}

	<ul>
		{#each view.hand as card (card)}
			{@const id = cardMoveId(card)}
			{@const legal = playableIds.has(id)}
			<li>
				<button
					type="button"
					class:legal
					disabled={disabled || !legal || view.phase !== 'trick_play'}
					aria-label={legal
						? `${card}, playable`
						: `${card}, not playable`}
					onclick={() => onPlay(id)}
				>
					{card}
				</button>
			</li>
		{/each}
	</ul>
</section>

<style>
	.hand {
		padding: 1rem 1.25rem 1.5rem;
	}
	.hint {
		margin: 0 0 0.6rem;
		color: #c9b89a;
		font-size: 0.9rem;
	}
	ul {
		list-style: none;
		display: flex;
		flex-wrap: wrap;
		gap: 0.55rem;
		margin: 0;
		padding: 0;
	}
	button {
		min-width: 3.4rem;
		padding: 0.85rem 0.7rem;
		border: 1px solid #5c4a32;
		border-radius: 0.45rem;
		background: #1f1810;
		color: #e8dcc6;
		font: inherit;
		font-weight: 600;
		cursor: pointer;
		transition:
			transform 140ms ease,
			background 140ms ease,
			border-color 140ms ease;
	}
	button.legal:not(:disabled) {
		background: #2d4a2a;
		border-color: #6f9a5c;
		color: #f4f7e8;
	}
	button.legal:not(:disabled):hover {
		transform: translateY(-3px);
	}
	button:disabled {
		opacity: 0.55;
		cursor: default;
	}
</style>
