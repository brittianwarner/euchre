<!-- Native card targets never move or overlap; only their non-interactive artwork lifts. -->
<script lang="ts">
	import { untrack } from 'svelte';
	import { reducedMotion } from '#lib/three/layout/reducedMotion.svelte.ts';
	import { playDeal } from './sound';
	import { reconcileHandSlots, type HandSlots } from './table-presentation';
	import { cardName, whyIllegal } from '#lib/euchre/index.ts';
	import type { CardId, LegalMoveId, PublicGameView } from '#lib/protocol/index.ts';

	let {
		view,
		disabled = false,
		selected = null,
		onSelect,
		onPlay,
		onIllegal
	}: {
		view: PublicGameView;
		disabled?: boolean;
		selected?: CardId | null;
		onSelect: (card: CardId) => void;
		onPlay: (move: LegalMoveId) => void;
		onIllegal: (card: CardId) => void;
	} = $props();

	let slots = $state.raw<HandSlots | null>(null);
	$effect(() => {
		const next = view;
		untrack(() => {
			slots = reconcileHandSlots(slots, next);
		});
	});

	const cardMoves = $derived.by(() => {
		const moves = new Map<CardId, LegalMoveId>();
		for (const entry of view.legal) {
			if (entry.move.t === 'play' || entry.move.t === 'discard') {
				moves.set(entry.move.card, entry.id);
			}
		}
		return moves;
	});

	// The initial snapshot is already dealt. Only newly dealt cards animate;
	// a sync or reconnect cannot replay cards we have already shown.
	const dealtKeys = untrack(() => view.hand.map((card) => `${view.handNo}:${card}`));
	function dealCard(node: HTMLElement) {
		const context = untrack(() => ({
			handNo: view.handNo,
			reduced: reducedMotion.enabled,
			phase: view.phase
		}));
		const key = `${context.handNo}:${node.dataset.card}`;
		if (dealtKeys.includes(key)) return;
		dealtKeys.push(key);
		if (dealtKeys.length > 30) dealtKeys.shift();
		if (context.reduced || context.phase === 'dealer_discard') return;
		const index = Number(node.dataset.index);
		const animation = node.animate(
			[
				{ transform: 'translateY(-24px) rotateY(18deg) scale(.92)', opacity: 0 },
				{ transform: 'none', opacity: 1 }
			],
			{ duration: 580, delay: index * 75, easing: 'cubic-bezier(.22,.7,.25,1)', fill: 'backwards' }
		);
		if (index === 0) playDeal();
		return () => animation.cancel();
	}

	function unavailableReason(card: CardId): string {
		if (disabled) return 'Please wait until the table is ready.';
		if (view.status !== 'active') return 'This match is finished.';
		if (view.turnSeat !== view.you) return 'Wait for your turn.';
		if (view.phase !== 'trick_play' && view.phase !== 'dealer_discard') {
			return 'Finish bidding before selecting a card.';
		}
		return (
			whyIllegal(view.hand, card, view.trick.ledSuit, view.trump) ??
			'This card is not available right now.'
		);
	}

	function selectCard(card: CardId): void {
		if (disabled) return;
		if (!cardMoves.has(card)) {
			onIllegal(card);
			return;
		}
		onSelect(card);
	}

	function moveFocus(event: KeyboardEvent): void {
		if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
		const current = event.currentTarget as HTMLButtonElement;
		const buttons = Array.from(
			current.closest('ul')?.querySelectorAll<HTMLButtonElement>('button[data-card-id]') ?? []
		);
		if (!buttons.length) return;
		event.preventDefault();
		const index = buttons.indexOf(current);
		const next =
			event.key === 'Home'
				? 0
				: event.key === 'End'
					? buttons.length - 1
					: (index + (event.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length;
		buttons[next]?.focus();
	}
</script>

{#if slots && slots.cards.length > 0}
	<section class="card-rack" aria-label="Your cards" style:--card-count={slots.cards.length}>
		<ul>
			{#each slots.cards as card, index (`${slots.handNo}:${card}`)}
				{@const held = view.hand.includes(card)}
				{@const legal = cardMoves.has(card) && !disabled}
				{@const isSelected = selected === card}
				<li>
					{#if held}
						<button
							type="button"
							data-card-id={card}
							class:selected={isSelected}
							class:unavailable={!legal}
							class:must-follow={cardMoves.size > 0 && !cardMoves.has(card)}
							aria-pressed={isSelected}
							aria-disabled={!legal}
							aria-label={`Select ${cardName(card)}. ${legal ? 'Press to select, then use Play. Or double-click to play.' : unavailableReason(card)}`}
							onclick={() => selectCard(card)}
							ondblclick={() => {
								const move = cardMoves.get(card);
								if (!disabled && move) onPlay(move);
							}}
							onkeydown={moveFocus}
						>
							<span
								class="artwork"
								aria-hidden="true"
								data-card={card}
								data-index={index}
								{@attach dealCard}
							>
								<img
									src={`/art/cards/${card}.png`}
									alt=""
									width="635"
									height="889"
									draggable="false"
									decoding="async"
								/>
							</span>
							<span class="card-label" aria-hidden="true"
								>{cardName(card)}<small
									>{isSelected
										? 'Selected ✓'
										: cardMoves.size > 0 && !legal
											? 'Must follow suit'
											: ' '}</small
								></span
							>
						</button>
					{:else}<div class="empty-slot" aria-hidden="true"></div>{/if}
				</li>
			{/each}
		</ul>
	</section>
{/if}

<style>
	.card-rack {
		width: min(100%, calc(var(--card-count) * 150px + (var(--card-count) - 1) * 18px));
		margin-inline: auto;
	}
	ul {
		display: grid;
		grid-template-columns: repeat(var(--card-count), minmax(0, 1fr));
		gap: 18px;
		list-style: none;
		padding: 12px 0 0;
		margin: 0;
	}
	li {
		min-width: 0;
	}
	button {
		display: block;
		width: 100%;
		position: relative;
		padding: 0;
		border: 0;
		background: transparent;
		border-radius: 8px;
		cursor: pointer;
		touch-action: manipulation;
		color: #233e32;
	}
	.artwork {
		display: block;
		aspect-ratio: 5/7;
		border-radius: 7px;
		background: white;
		box-shadow:
			0 1px 0 #fff,
			0 3px 0 #c4c7b9,
			0 7px 13px #16382c24;
		pointer-events: none;
		transition:
			transform 180ms ease,
			box-shadow 180ms ease;
	}
	img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: contain;
		border-radius: 7px;
		user-select: none;
	}
	.card-label {
		display: block;
		font-size: 16px;
		font-weight: 600;
		padding-top: 12px;
		line-height: 1.35;
		min-height: 56px;
		pointer-events: none;
	}
	.card-label small {
		display: block;
		font-size: 14px;
		line-height: 1.5;
		min-height: 21px;
		font-weight: 400;
	}
	button.selected .artwork {
		transform: perspective(1000px) translateY(-8px) rotateX(2deg);
		outline: 3px solid #295744;
		outline-offset: 3px;
		box-shadow:
			0 4px 0 #bac2b1,
			0 15px 20px #15301d30;
	}
	button.selected small {
		font-weight: 700;
	}
	button.unavailable {
		cursor: help;
	}
	button.must-follow small {
		color: #735548;
	}
	button:focus-visible {
		outline: 3px solid #295744;
		outline-offset: 6px;
	}
	.empty-slot {
		aspect-ratio: 5/7;
		border: 1px dashed #b9c3b3;
		border-radius: 7px;
		color: #53664e;
		display: grid;
		place-items: center;
		font-size: 16px;
	}
	@media (hover: hover) {
		button:not(.unavailable):not(.selected):hover .artwork {
			transform: translateY(-4px);
		}
	}
	@media (min-width: 701px) and (max-height: 850px) {
		.card-rack {
			width: min(100%, calc(var(--card-count) * 126px + (var(--card-count) - 1) * 18px));
		}
		.card-label {
			padding-top: 8px;
			min-height: 44px;
			font-size: 15px;
		}
		.card-label small {
			min-height: 18px;
			line-height: 1.25;
		}
	}
	@media (max-width: 600px) {
		.card-rack {
			max-width: 400px;
		}
		ul {
			grid-template-columns: repeat(3, minmax(0, 1fr));
			gap: 14px 12px;
		}
		.card-label {
			font-size: 14px;
			min-height: 48px;
			padding-top: 8px;
		}
		.card-label small {
			font-size: 13px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.artwork {
			transition: none;
		}
		button.selected .artwork {
			transform: none;
		}
	}
	/* Each fixed slot sizes its artwork to the space left by the game controls. */
	.card-rack {
		height: 100%;
		min-height: 0;
		width: min(100%, 850px);
	}
	ul {
		height: 100%;
		min-height: 0;
		padding: 10px 0 0;
		grid-auto-rows: minmax(0, 1fr);
	}
	li {
		container-type: size;
	}
	button {
		height: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
	}
	.artwork {
		width: min(100cqw, calc((100cqh - 46px) * 5 / 7));
		height: auto;
		flex: none;
	}
	.card-label {
		min-height: 0;
		padding-top: 8px;
		font-size: 15px;
		line-height: 1.25;
	}
	.card-label small {
		line-height: 1.2;
		min-height: 17px;
		font-size: 13px;
	}
	.empty-slot {
		height: calc(100% - 46px);
		width: auto;
		margin: auto;
	}
	@media (max-width: 700px) {
		.card-rack {
			max-width: 420px;
		}
		ul {
			grid-template-columns: repeat(3, minmax(0, 1fr));
			gap: 6px 14px;
			padding-top: 8px;
		}
		.artwork {
			width: min(100cqw, calc((100cqh - 34px) * 5 / 7));
		}
		.card-label {
			font-size: 13px;
			line-height: 1.2;
			padding-top: 6px;
		}
		.card-label small {
			display: none;
		}
		.empty-slot {
			height: calc(100% - 34px);
			font-size: 14px;
		}
	}
	.empty-slot {
		visibility: hidden;
		width: 100%;
		height: 100%;
		aspect-ratio: auto;
		border: 0;
	}
	@media (max-width: 700px) and (max-height: 750px) {
		ul {
			grid-template-columns: repeat(var(--card-count), minmax(0, 1fr));
			gap: 7px;
		}
		.card-label {
			font-size: 12px;
		}
	}
	button {
		justify-content: flex-start;
	}
</style>
