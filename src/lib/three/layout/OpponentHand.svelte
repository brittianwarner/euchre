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
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import Card from '$lib/three/cards/Card.svelte';
	import { CARD_ASPECT, fanPositions } from './layout';
	import { dealOrigin, lerpPose } from './cardMotion';
	import { TEMPO, flightMs } from './tempo';
	import type { Seat } from '$lib/euchre';

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
		/** `view.handNo` — see `Hand.svelte`'s doc comment on `handNo` for why this is the deal-in trigger. */
		handNo?: number;
		/** `view.dealerSeat`, with `seat` — phases this seat's deal-in stagger against the other three. */
		dealerSeat?: Seat;
		/** This seat (`1`/`2`/`3` — `TableScene.svelte` passes a literal per anchor). */
		seat?: Seat;
		reducedMotion?: boolean;
	}

	let {
		count,
		cardHeight = 0.09,
		overlap = 0.6,
		maxTiltDeg = 8,
		archLift = 0.03,
		reclineDeg = 18,
		maxVisible,
		handNo,
		dealerSeat = 0,
		seat = 1,
		reducedMotion = false
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

	/**
	 * Deal-in choreography — same approximation and same "no `steps` channel
	 * here" reasoning as `Hand.svelte`'s identical block; kept independent
	 * (not shared) because the two components key their per-card tweens
	 * differently (`CardId` there, positional slot here — an opponent's fan
	 * carries no card identity at all, matching this file's own "structural
	 * non-leakage" doc above).
	 */
	let dealTweens = $state.raw(new Map<number, Tween<number>>());
	let sawFirstHandNo = false;

	$effect(() => {
		const signal = handNo;
		if (!sawFirstHandNo) {
			sawFirstHandNo = true;
			return;
		}
		if (signal === undefined) return;
		const n = visibleCount; // `count`/`maxVisible` at the moment of the deal — reading this here (not `untrack`) is fine, since unlike `Hand.svelte`'s per-`CardId` map a slot count changing mid-hand (a loner's sitting partner going to 0) should simply stop rendering that slot, tweened or not.
		const order = ((seat - dealerSeat + 4) % 4) as number;
		const flight = flightMs(TEMPO.dealFlightMs, reducedMotion);
		const stagger = reducedMotion ? 0 : TEMPO.dealStaggerMs;
		const fresh = new Map<number, Tween<number>>();
		for (let i = 0; i < n; i++) {
			const t = new Tween(0, { easing: cubicOut });
			fresh.set(i, t);
			void t.set(1, { duration: flight, delay: (order + i * 4) * stagger });
		}
		dealTweens = fresh;
	});

	function renderPose(i: number) {
		const target = poses[i];
		const tween = dealTweens.get(i);
		if (!tween || tween.current >= 1) return target;
		return lerpPose(dealOrigin(target), target, tween.current, reducedMotion ? 0 : cardHeight * 0.5);
	}
</script>

<T.Group>
	<!--
		These slots are anonymous and interchangeable by construction (a face-down
		back carries no identity), so a synthetic per-slot key is the correct,
		deliberate exception to "never key an each block by index" — there is no
		card identity here for a real key to track.
	-->
	{#each poses as _, i (`slot-${i}`)}
		{@const pose = renderPose(i)}
		<Card faceUp={false} position={pose.position} rotation={pose.rotation} height={cardHeight} />
	{/each}
</T.Group>
