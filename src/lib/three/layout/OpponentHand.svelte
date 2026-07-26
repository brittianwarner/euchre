<!--
  OpponentHand.svelte — a face-down fan for seats 1-3.

  Renders inside that seat's `SeatAnchor`, sharing the exact same anchor-local
  fan math as `Hand.svelte` (see `layout.ts`'s module doc: the fan is identical
  for all four seats because the anchor itself carries the seat's identity).
  It sits flatter than the human's fan (`reclineDeg` defaults lower) since
  there is nothing here to read — just a shape that reads as "a hand of cards".

  Structural non-leakage: this component's only card-shaped input is `count`.
  There is no prop through which a `CardId` could arrive, so an opponent's hand
  cannot leak here even if something upstream handed one to this file by
  mistake — the type signature makes it impossible, not merely undesirable.

  A seat sitting out under a loner has `count === 0` (V8) and renders nothing.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import Card from '$lib/three/cards/Card.svelte';
	import { CARD_ASPECT, fanPositions } from './layout';

	interface Props {
		/** `view.handCounts[seat]`. Never a card id — see the file doc. */
		count: number;
		cardHeight?: number;
		overlap?: number;
		maxTiltDeg?: number;
		archLift?: number;
		/** Flatter than the human's fan by default — there is nothing to read here. */
		reclineDeg?: number;
		/**
		 * Portrait compression (`docs/04-FRONTEND-UX.md` §14): cap how many card
		 * meshes render without changing what `count` means. The true count still
		 * belongs on a DOM number elsewhere — capping the mesh count here never
		 * loses information, only draw calls.
		 */
		maxVisible?: number;
	}

	let {
		count,
		cardHeight = 0.09,
		overlap = 0.6,
		maxTiltDeg = 8,
		archLift = 0.03,
		reclineDeg = 18,
		maxVisible
	}: Props = $props();

	const visibleCount = $derived(Math.max(0, Math.min(count, maxVisible ?? count)));

	const poses = $derived(
		fanPositions(visibleCount, {
			cardWidth: cardHeight * CARD_ASPECT,
			cardHeight,
			overlap,
			maxTiltDeg,
			archLift,
			reclineDeg
		})
	);
</script>

<T.Group>
	<!--
		These slots are anonymous and interchangeable by construction (a face-down
		back carries no identity), so a synthetic per-slot key is the correct,
		deliberate exception to "never key an each block by index" — there is no
		card identity here for a real key to track.
	-->
	{#each poses as pose, i (`slot-${i}`)}
		<Card faceUp={false} position={pose.position} rotation={pose.rotation} height={cardHeight} />
	{/each}
</T.Group>
