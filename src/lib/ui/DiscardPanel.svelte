<!--
  DiscardPanel — dealer chooses which of six cards to bury after ordering up.
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
		padding: 0 1.25rem 0.75rem;
	}
	.hint {
		margin: 0 0 0.5rem;
		color: #c9b89a;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	button {
		min-width: 3.2rem;
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
