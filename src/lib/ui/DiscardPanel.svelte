<!--
  DiscardPanel — dealer chooses which of six cards to bury after ordering up.

  Self-contained panel (own background/border), same reasoning as
  `BidPanel.svelte`'s doc comment — this docks bottom-centre (same as that
  panel) and needs to read over the table's wood rim, not a dark page
  background.
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

	const discards = $derived(view.legal.filter((m) => m.id.startsWith('discard:')));
</script>

{#if view.phase === 'dealer_discard' && discards.length > 0 && view.turnSeat === view.you}
	<section class="discard" aria-label="Discard">
		<p class="hint">Discard one card face down</p>
		<div class="row">
			{#each discards as move (move.id)}
				<button type="button" {disabled} onclick={() => onPlay(move.id)}>
					{move.label}
				</button>
			{/each}
		</div>
	</section>
{/if}

<style>
	.discard {
		margin: 0;
		padding: 0.75rem 0.85rem;
		border: 1px solid rgba(232, 194, 122, 0.25);
		border-radius: 0.65rem;
		background: rgba(15, 20, 14, 0.85);
		box-shadow: 0 2px 10px rgba(0, 0, 0, 0.4);
	}
	.hint {
		margin: 0 0 0.5rem;
		color: #c9b89a;
		text-align: center;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		gap: 0.5rem;
	}
	button {
		min-width: 3.2rem;
		min-height: 44px;
		padding: 0.7rem 0.65rem;
		border: 1px solid #6a5638;
		border-radius: 0.4rem;
		background: #3a2c1a;
		color: #f2e8d5;
		font: inherit;
		font-weight: 600;
		cursor: pointer;
	}
	button:disabled {
		opacity: 0.5;
		cursor: default;
	}
</style>
