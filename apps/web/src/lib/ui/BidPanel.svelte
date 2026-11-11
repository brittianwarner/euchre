<!-- Bidding controls use only server-provided legal moves. The solo toggle selects the corresponding legal variant without inventing a move. -->
<script lang="ts">
	import { SUIT_NAME } from '#lib/euchre/index.ts';
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
					onclick={() => onPlay(move.id)}
					>{move.move.t === 'pass'
						? 'Pass'
						: move.move.t === 'orderUp'
							? `Make ${view.upCard ? SUIT_NAME[view.upCard[1] as keyof typeof SUIT_NAME].toLowerCase() : 'it'} trump`
							: move.move.t === 'call'
								? SUIT_NAME[move.move.suit]
								: move.move.t === 'cut'
									? move.move.cut
										? 'Cut the deck'
										: 'Deal the cards'
									: move.label}</button
				>{/each}
		</div>
	</section>
{/if}

<style>
	.bids {
		width: 100%;
		max-width: 850px;
		margin: auto;
	}
	.bid-heading {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 20px;
		margin-bottom: 14px;
	}

	.bid-heading label {
		display: flex;
		gap: 10px;
		align-items: center;
		font-size: 17px;
		min-height: 48px;
		cursor: pointer;
	}
	.bid-heading input {
		width: 22px;
		height: 22px;
		border-radius: 4px;
		accent-color: #285641;
	}
	.row {
		display: flex;
		justify-content: center;
		gap: 12px;
		flex-wrap: wrap;
	}
	button {
		min-height: 56px;
		flex: 1;
		min-width: 130px;
		padding: 12px 22px;
		border: 1px solid #295641;
		border-radius: 8px;
		background: #285641;
		color: #fffdf4;
		font-size: 19px;
		font-weight: 600;
		cursor: pointer;
	}
	button:hover:not(:disabled) {
		background: #1b4531;
	}
	button.secondary {
		background: #fffdf6;
		border-color: #85957b;
		color: #334d3e;
	}
	button.secondary:hover:not(:disabled) {
		background: #e9eddf;
	}
	button:disabled {
		opacity: 0.55;
		cursor: default;
	}
	@media (max-width: 600px) {
		.bid-heading {
			flex-wrap: wrap;
			gap: 4px 16px;
		}
		button {
			font-size: 17px;
			padding: 12px;
		}
	}

	.bids {
		display: flex;
		align-items: center;
		gap: 16px;
	}
	.bid-heading {
		order: 2;
		margin: 0;
		flex-shrink: 0;
	}
	.bid-heading label {
		white-space: nowrap;
	}
	.row {
		flex: 1;
		flex-wrap: nowrap;
		gap: 10px;
	}
	button {
		min-width: 0;
		padding: 10px 16px;
	}
	@media (max-width: 700px) {
		.bids {
			gap: 8px;
			flex-wrap: wrap;
		}
		.row {
			width: 100%;
			flex: auto;
			gap: 8px;
		}
		button {
			min-height: 46px;
			font-size: 15px;
			padding: 8px;
		}
		.bid-heading {
			width: 100%;
			justify-content: flex-end;
		}
		.bid-heading label {
			min-height: 28px;
			font-size: 14px;
		}
		.bid-heading input {
			width: 20px;
			height: 20px;
		}
	}
</style>
