<!--
  BidPanel — cut, order-up, pass, and round-2 suit calls from view.legal.

  Rendered as a self-contained panel (own background/border) rather than
  bare text-on-felt: the composing page docks this bottom-centre
  (`+page.svelte`'s `.corner-bc`), which over the table's wood rim needs its
  own legible backing rather than assuming a dark page background behind it.
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
	.alone {
		margin-top: 0.45rem;
	}
	button {
		min-height: 44px;
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
