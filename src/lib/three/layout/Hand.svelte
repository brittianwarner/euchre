<!--
  Hand.svelte — the human's own cards, fanned at the near edge.

  Renders inside seat 0's `SeatAnchor`, so every position/rotation below is in
  that anchor's local space (see `layout.ts`'s module doc) — this component has
  no idea where on the felt it physically sits, and does not need to.

  LEGIBILITY IS THE POINT: this is the one fan on the table someone is actually
  reading, so it reclines toward the camera (`reclineDeg`) instead of lying
  flat, and its cards render noticeably larger than `OpponentHand.svelte`'s —
  large enough that a player reads their own five cards at a glance, without
  leaning toward the screen.

  `cardHeight`/`overlap` default to a size read from the live canvas aspect
  (via `useThrelte()`, the same signal `CameraRig` frames from), not one fixed
  constant, because "big" and "stays inside the frame" pull against each other
  differently per breakpoint:

    - Wide landscape (a laptop — the primary target) has a wide horizontal FOV
      at the hand's depth relative to its ~13 deg vertical offset from the
      camera axis, so cards can render large with lots of margin to spare.
    - A portrait phone gets its horizontal FOV *from* a fixed vertical FOV
      through a narrow aspect ratio, so the same generous size runs the fan
      past the left/right frame edges — verified by rendering it and looking,
      not by guessing. Portrait's size is the largest that still clears the
      dealer's 6-card discard fan with margin at that breakpoint's actual
      camera geometry (see `CameraRig.svelte`).

  An explicit `cardHeight`/`overlap` prop still wins over the responsive
  default — this is a caller override, not a replacement for one.

  This component decides nothing about legality. `legal` (straight from
  `PublicGameView.legal`) is read only to compute which `CardId`s are currently
  offered — "server truth" per the hard rules — and a card outside that set
  renders `dimmed` and, if tapped, fires `onillegal` instead of `onplay`.
-->
<script lang="ts">
	import { T, useThrelte } from '@threlte/core';
	import Card from '$lib/three/cards/Card.svelte';
	import { CARD_ASPECT, fanPositions } from './layout';
	import { isNarrowLandscape, isPortrait } from '$lib/three/scene/breakpoints';
	import type { CardId, LegalMove } from '$lib/euchre';

	interface Props {
		/** `view.hand` — your own cards, and only ever your own (V8). Caller-ordered; this component does not sort. */
		cards: readonly CardId[];
		/** `view.legal` — used only to derive which of `cards` are currently playable/discardable. */
		legal: readonly LegalMove[];
		fourColor?: boolean;
		/** World-unit card height; width follows the poker aspect ratio. Defaults to a breakpoint-responsive size — see module doc. */
		cardHeight?: number;
		/** `0..1`, the fraction of a card's width its neighbour covers. Defaults to a breakpoint-responsive size — see module doc. */
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
		cardHeight: cardHeightProp,
		overlap: overlapProp,
		maxTiltDeg = 10,
		archLift = 0.04,
		/**
		 * Recline the fan toward the viewer.
		 *
		 * Flat on the felt (0) the cards are edge-on from the camera's ~39 deg
		 * pitch and unreadable — you cannot play a hand you cannot see. The
		 * hand-to-camera vector is (0, 0.79, 0.61), so ~38 deg points the cards
		 * exactly at the lens; 34 keeps them legible while still reading as cards
		 * resting on a table rather than billboards floating above it.
		 */
		reclineDeg = 34,
		disabled = false,
		onplay,
		onillegal,
		onhover
	}: Props = $props();

	// Same live-aspect signal `CameraRig` frames from — see that file for the
	// breakpoint thresholds these two share.
	const { size } = useThrelte();
	const aspect = $derived($size.width / Math.max(1, $size.height));
	const portrait = $derived(isPortrait(aspect));
	const narrow = $derived(isNarrowLandscape(aspect));

	/**
	 * Sized (and margin-checked against the camera's actual FOV/distance at
	 * each breakpoint, worst case the dealer's 6-card discard fan) so the fan
	 * never touches the frustum's edges — see module doc.
	 */
	// Sized against SEAT_RADIUS (0.3 m). At 0.25 the cards were taller than the
	// distance from table centre to the seat and spilled past the bottom of the
	// frame. These read large and clear while staying fully inside the camera's
	// 21 deg half-FOV, including the lower edge once the fan is reclined.
	const baseCardHeight = $derived(portrait ? 0.15 : narrow ? 0.165 : 0.185);
	const baseOverlap = $derived(portrait ? 0.62 : 0.6);

	const cardHeight = $derived(cardHeightProp ?? baseCardHeight);
	const overlap = $derived(overlapProp ?? baseOverlap);

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

<!--
	Pulled well in from the seat anchor toward the table centre.

	Note the sign: each seat group is rotated so its local +Z faces the middle of
	the table, so moving "in" is POSITIVE z. Negative pushes the fan out over the
	rail and off the bottom of the screen.

	The anchor marks where a player SITS; their cards rest on the felt in front of
	them, not on the rail. Two things follow. The fan stops hugging the bottom of
	the frame, where it was being clipped, and it moves into the large empty region
	of felt the camera was otherwise wasting. It also vacates the bottom strip of
	the screen entirely, so the action panel has somewhere to live that is not on
	top of the cards the player is trying to read.
-->
<T.Group position={[0, 0, 0.13]}>
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
