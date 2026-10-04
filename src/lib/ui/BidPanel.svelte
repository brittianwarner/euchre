<!-- Bidding controls use only server-provided legal moves. The solo toggle selects the corresponding legal variant without inventing a move. -->
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

	const bidPhases = new Set(['cutting', 'bid_round_1', 'bid_round_2']);

	const moves = $derived(
		bidPhases.has(view.phase)
			? view.legal.filter((m) => !m.id.startsWith('play:') && !m.id.startsWith('discard:'))
			: []
	);

	const aloneMoves = $derived(moves.filter((m) => m.id.includes('+alone')));
	const plainMoves = $derived(moves.filter((m) => !m.id.includes('+alone')));
	let soloChoice = $state<{ decision: string; enabled: boolean } | null>(null);
	const decision = $derived(`${view.handNo}:${view.phase}`);
	const goAlone = $derived(soloChoice?.decision === decision && soloChoice.enabled);
	const visibleMoves = $derived(
		plainMoves.map((move) =>
			goAlone ? (aloneMoves.find((alone) => alone.id === `${move.id}+alone`) ?? move) : move
		)
	);
</script>

{#if moves.length > 0 && view.turnSeat === view.you}
	<section class="bids" aria-label="Bidding">
		<div class="bid-heading">
			<p>
				{view.phase === 'cutting'
					? 'A fresh deck. Your call.'
					: view.phase === 'bid_round_1'
						? 'Make it trump, or pass?'
						: 'Choose your trump suit.'}
			</p>
			{#if aloneMoves.length > 0}<label
					><input
						type="checkbox"
						checked={goAlone}
						{disabled}
						onchange={(e) => (soloChoice = { decision, enabled: e.currentTarget.checked })}
					/> Go alone</label
				>{/if}
		</div>
		<div class="row">
			{#each visibleMoves as move (move.id)}<button
					type="button"
					class:secondary={move.id === 'pass' || move.id === 'cut:no'}
					{disabled}
					onclick={() => onPlay(move.id)}>{move.label}</button
				>{/each}
		</div>
	</section>
{/if}

<style>
	.bids {
		width: 100%;
		max-width: 570px;
		margin: auto;
		font-family: var(--font-sans);
	}
	.bid-heading {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 20px;
		margin-bottom: 12px;
	}
	.bid-heading p {
		margin: 0;
		font-size: 13px;
		color: #52624c;
	}
	.bid-heading label {
		display: flex;
		gap: 7px;
		align-items: center;
		font-size: 11px;
		color: #6a7763;
		white-space: nowrap;
		min-height: 24px;
		cursor: pointer;
	}
	.bid-heading input {
		width: 14px;
		height: 14px;
		border-radius: 3px;
		accent-color: #274d3a;
	}
	.row {
		display: flex;
		justify-content: center;
		gap: 9px;
	}
	button {
		min-height: 44px;
		min-width: 100px;
		padding: 10px 22px;
		border: 1px solid #cbdfa3;
		border-radius: 7px;
		background: #d4ed9b;
		color: #203c2d;
		font: 600 13px var(--font-sans);
		cursor: pointer;
		transition:
			background 0.15s,
			transform 0.15s;
	}
	button:hover:not(:disabled) {
		background: #c5e285;
		transform: translateY(-1px);
	}
	button.secondary {
		background: #fffdf6;
		border-color: #d5dbcc;
		color: #62705b;
	}
	button.secondary:hover:not(:disabled) {
		background: #e9eddf;
	}
	button:disabled {
		opacity: 0.5;
		cursor: default;
	}
	@media (max-width: 700px) {
		.bid-heading {
			gap: 14px;
			margin-bottom: 9px;
		}
		.bid-heading p {
			font-size: 12px;
		}
		.row {
			gap: 6px;
		}
		button {
			min-width: 0;
			flex: 1;
			padding: 10px 9px;
			font-size: 11px;
		}
		.bids {
			max-width: 460px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		button {
			transition: none;
		}
	}
</style>
