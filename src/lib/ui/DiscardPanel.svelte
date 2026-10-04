<!-- Compact, keyboard-accessible discard choices in the action dock. Card names remain available to assistive technology. -->
<script lang="ts">
	import type { LegalMoveId, PublicGameView } from '#lib/protocol/index.ts';

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
		<p>Choose one card to tuck away.</p>
		<div class="row">
			{#each discards as move (move.id)}<button
					type="button"
					{disabled}
					aria-label={move.label}
					onclick={() => onPlay(move.id)}
					>{#if move.move.t === 'discard'}{move.move.card.slice(0, -1).replace('T', '10')}<span
							class:red={['H', 'D'].includes(move.move.card.slice(-1))}
							>{{ S: '♠', H: '♥', D: '♦', C: '♣' }[
								move.move.card.slice(-1) as 'S' | 'H' | 'D' | 'C'
							]}</span
						>{/if}</button
				>{/each}
		</div>
	</section>
{/if}

<style>
	.discard {
		font-family: var(--font-sans);
		text-align: center;
	}
	.discard p {
		font-size: 13px;
		color: #52624c;
		margin: 0 0 12px;
	}
	.row {
		display: flex;
		justify-content: center;
		gap: 8px;
	}
	button {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 4px;
		min-width: 46px;
		min-height: 44px;
		padding: 8px;
		border: 1px solid #d5dbcc;
		border-radius: 7px;
		background: #fffdf6;
		color: #203c2d;
		font: 600 16px var(--font-serif);
		cursor: pointer;
	}
	button:hover:not(:disabled) {
		background: #e6eecf;
		border-color: #94a96d;
	}
	.red {
		color: #b64e3c;
	}
	button:disabled {
		opacity: 0.5;
		cursor: default;
	}
</style>
