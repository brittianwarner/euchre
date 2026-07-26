<!--
  Hand.svelte — the human's own cards, fanned at the near edge.

  Renders inside seat 0's `SeatAnchor`, so every position/rotation below is in
  that anchor's local space (see `layout.ts`'s module doc) — this component has
  no idea where on the felt it physically sits, and does not need to.

  LEGIBILITY IS THE POINT: this is the one fan on the table someone is actually
  reading, so it reclines toward the camera (`reclineDeg`) instead of lying
  flat, and its cards render noticeably larger than `OpponentHand.svelte`'s.

  This component decides nothing about legality. `legal` (straight from
  `PublicGameView.legal`) is read only to compute which `CardId`s are currently
  offered — "server truth" per the hard rules — and a card outside that set
  renders `dimmed` and, if tapped, fires `onillegal` instead of `onplay`.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import Card from '$lib/three/cards/Card.svelte';
	import { CARD_ASPECT, fanPositions } from './layout';
	import type { CardId, LegalMove } from '$lib/euchre';

	interface Props {
		/** `view.hand` — your own cards, and only ever your own (V8). Caller-ordered; this component does not sort. */
		cards: readonly CardId[];
		/** `view.legal` — used only to derive which of `cards` are currently playable/discardable. */
		legal: readonly LegalMove[];
		fourColor?: boolean;
		/** World-unit card height; width follows the poker aspect ratio. */
		cardHeight?: number;
		/** `0..1`, the fraction of a card's width its neighbour covers. */
		overlap?: number;
		/** Degrees each end card tilts from the fan's centre. */
		maxTiltDeg?: number;
		/** Fraction of `cardHeight` the fan's centre bulges up. */
		archLift?: number;
		/** Degrees the whole fan reclines off flat-on-the-table toward the camera, for legibility. */
		reclineDeg?: number;
		/** True while a move is in flight — cards stop responding to hover/tap, but legality shading is untouched. */
		disabled?: boolean;
		/** Fires when a legal card is tapped. */
		onplay?: (cardId: CardId) => void;
		/** Fires when an illegal card is tapped, so the caller can show the engine's `whyIllegal` copy — no dead clicks. */
		onillegal?: (cardId: CardId) => void;
		onhover?: (cardId: CardId | null) => void;
	}

	let {
		cards,
		legal,
		fourColor = false,
		cardHeight = 0.115,
		overlap = 0.55,
		maxTiltDeg = 10,
		archLift = 0.04,
		reclineDeg = 42,
		disabled = false,
		onplay,
		onillegal,
		onhover
	}: Props = $props();

	/** `LegalMove.move` carries a `card` only for `play` and `discard` — the two shapes this hand ever offers. */
	const legalIds = $derived(
		new Set(
			legal.flatMap((m) => (m.move.t === 'play' || m.move.t === 'discard' ? [m.move.card] : []))
		)
	);

	const poses = $derived(
		fanPositions(cards.length, {
			cardWidth: cardHeight * CARD_ASPECT,
			cardHeight,
			overlap,
			maxTiltDeg,
			archLift,
			reclineDeg
		})
	);

	let hoveredId = $state<CardId | null>(null);

	function handleSelect(cardId: CardId): void {
		if (disabled) return;
		if (legalIds.has(cardId)) onplay?.(cardId);
		else onillegal?.(cardId);
	}

	function handleHover(cardId: CardId | null): void {
		hoveredId = cardId;
		onhover?.(cardId);
	}
</script>

<T.Group>
	{#each cards as cardId, i (cardId)}
		{@const pose = poses[i]}
		<Card
			id={cardId}
			faceUp
			position={pose.position}
			rotation={pose.rotation}
			height={cardHeight}
			{fourColor}
			dimmed={!legalIds.has(cardId)}
			highlighted={!disabled && hoveredId === cardId}
			interactive={!disabled}
			onselect={handleSelect}
			onhover={handleHover}
		/>
	{/each}
</T.Group>
