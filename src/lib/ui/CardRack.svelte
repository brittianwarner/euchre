<!-- Native card targets never move or overlap; only their non-interactive artwork lifts. -->
<script lang="ts">
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

	const suitSymbol = { S: '♠', H: '♥', D: '♦', C: '♣' } as const;

	const cardMoves = $derived.by(() => {
		const moves = new Map<CardId, LegalMoveId>();
		for (const entry of view.legal) {
			if (entry.move.t === 'play' || entry.move.t === 'discard') {
				moves.set(entry.move.card, entry.id);
			}
		}
		return moves;
	});

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

	function playCard(card: CardId): void {
		const move = cardMoves.get(card);
		if (!disabled && move) onPlay(move);
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

{#if view.hand.length > 0}
	<section class="card-rack" aria-label="Your cards" style:--card-count={view.hand.length}>
		<ul>
			{#each view.hand as card (card)}
				{@const legal = cardMoves.has(card) && !disabled}
				{@const isSelected = selected === card}
				<li>
					<button
						type="button"
						data-card-id={card}
						class:selected={isSelected}
						class:unavailable={!legal}
						class:must-follow={cardMoves.size > 0 && !cardMoves.has(card)}
						aria-pressed={isSelected}
						aria-disabled={!legal}
						aria-label={`Select ${cardName(card)}. ${legal ? 'Press to select; double-click to play.' : unavailableReason(card)}`}
						onclick={() => selectCard(card)}
						ondblclick={() => playCard(card)}
						onkeydown={moveFocus}
					>
						<span class="artwork" aria-hidden="true">
							<img
								src={`/art/cards/${card}.png`}
								alt=""
								width="635"
								height="889"
								draggable="false"
								decoding="async"
							/>
							{#if isSelected}<span class="selected-mark">✓</span>{/if}
						</span>
						<span
							class="card-label"
							class:red={card.endsWith('H') || card.endsWith('D')}
							aria-hidden="true"
						>
							{card[0] === 'T' ? '10' : card[0]}
							{suitSymbol[card[1] as keyof typeof suitSymbol]}
						</span>
					</button>
				</li>
			{/each}
		</ul>
	</section>
{/if}

<style>
	.card-rack {
		--card-width: clamp(112px, 15dvh, 148px);
		width: min(100%, calc(var(--card-count) * var(--card-width) + (var(--card-count) - 1) * 10px));
		margin-inline: auto;
		font-family: var(--font-sans);
	}
	ul {
		display: grid;
		grid-template-columns: repeat(var(--card-count), minmax(0, 1fr));
		gap: 10px;
		list-style: none;
		margin: 0;
		padding: 12px 0 0;
	}
	li {
		min-width: 0;
	}
	button {
		display: block;
		position: relative;
		width: 100%;
		min-width: 44px;
		min-height: 44px;
		aspect-ratio: 5 / 7;
		padding: 0;
		margin: 0;
		background: transparent;
		border: 0;
		border-radius: 7px;
		cursor: pointer;
		touch-action: manipulation;
		-webkit-tap-highlight-color: transparent;
	}
	.artwork {
		display: block;
		position: absolute;
		inset: 0;
		border: 2px solid #eee9da;
		border-radius: 7px;
		background: #faf6eb;
		box-shadow:
			0 3px 0 #c9c5b7,
			0 7px 15px #001b2440;
		pointer-events: none;
		transition:
			transform 150ms ease,
			border-color 150ms ease,
			box-shadow 150ms ease;
	}
	img {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: contain;
		border-radius: 4px;
		pointer-events: none;
		user-select: none;
	}
	button.unavailable {
		cursor: help;
	}
	button.must-follow .artwork {
		filter: saturate(0.8) brightness(0.9);
	}
	button.selected .artwork {
		transform: translateY(-10px);
		border-color: #d4ed9b;
		box-shadow:
			0 0 0 2px #d4ed9b,
			0 4px 0 #749052,
			0 10px 20px #001b2450;
		filter: none;
	}
	button:focus-visible {
		outline: 3px solid #d4ed9b;
		outline-offset: 5px;
	}
	.selected-mark {
		position: absolute;
		top: -9px;
		right: -7px;
		width: 21px;
		height: 21px;
		display: grid;
		place-items: center;
		background: #d4ed9b;
		color: #233e32;
		border: 2px solid #1c3929;
		border-radius: 50%;
		font: 700 12px/1 var(--font-sans);
	}
	.card-label {
		display: none;
	}
	@media (hover: hover) {
		button:not(.unavailable):not(.selected):hover .artwork {
			transform: translateY(-5px);
			border-color: #d4ed9b;
		}
	}
	@media (max-width: 700px) {
		ul {
			gap: 4px;
		}
		.artwork {
			border-width: 1px;
			border-radius: 5px;
		}
		button {
			border-radius: 5px;
			aspect-ratio: auto;
			padding-bottom: 28px;
		}
		.artwork {
			position: relative;
			aspect-ratio: 5 / 7;
		}
		.card-label {
			display: block;
			position: absolute;
			bottom: 0;
			width: 100%;
			color: #faf6eb;
			font: 600 18px/24px var(--font-sans);
			pointer-events: none;
		}
		.card-label.red {
			color: #ffb0a5;
		}
		img {
			border-radius: 4px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.artwork {
			transition: none;
		}
	}
</style>
