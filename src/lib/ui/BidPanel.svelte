<!--
  BidPanel — cut, order-up, pass, and round-2 suit calls from view.legal.
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

	const bidPhases = new Set(['cutting', 'bid_round_1', 'bid_round_2']);

	const moves = $derived(
		bidPhases.has(view.phase)
			? view.legal.filter((m) => !m.id.startsWith('play:') && !m.id.startsWith('discard:'))
			: []
	);

	const aloneMoves = $derived(moves.filter((m) => m.id.includes('+alone')));
	const plainMoves = $derived(moves.filter((m) => !m.id.includes('+alone')));
</script>

{#if moves.length > 0 && view.turnSeat === view.you}
	<section class="bids" aria-label="Bidding">
		<p class="hint">
			{#if view.phase === 'cutting'}
				Cut the deck?
			{:else if view.phase === 'bid_round_1'}
				Order it up?
			{:else}
				Call a suit
			{/if}
		</p>
		<div class="row">
			{#each plainMoves as move (move.id)}
				<button type="button" {disabled} onclick={() => onPlay(move.id)}>
					{move.label}
				</button>
			{/each}
		</div>
		{#if aloneMoves.length > 0}
			<div class="row alone">
				{#each aloneMoves as move (move.id)}
					<button type="button" class="alone-btn" {disabled} onclick={() => onPlay(move.id)}>
						{move.label}
					</button>
				{/each}
			</div>
		{/if}
	</section>
{/if}

<style>
	.bids {
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
	.alone {
		margin-top: 0.45rem;
	}
	button {
		padding: 0.65rem 0.95rem;
		border: 1px solid #6a5638;
		border-radius: 0.4rem;
		background: #3a2c1a;
		color: #f2e8d5;
		font: inherit;
		cursor: pointer;
	}
	.alone-btn {
		background: #4a2818;
		border-color: #a0653a;
	}
	button:disabled {
		opacity: 0.5;
		cursor: default;
	}
</style>
