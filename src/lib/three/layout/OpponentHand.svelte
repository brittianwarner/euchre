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

  ## Portrait: West/East edge clipping (this component's own fix, no caller change)

  West and East sit at `x = ∓SEAT_RADIUS` (`seatLayout.ts`) with only a narrow
  horizontal FOV to work with in portrait — `CameraRig` pulls back and widens
  *vertically* (taller `fov`) for portrait, but the horizontal FOV is that
  vertical FOV squeezed by the (now narrow) aspect ratio, so it shrinks, not
  grows. `SeatAnchors` already shrinks the seat ring for portrait (0.30 m ->
  0.26 m), but that alone still leaves West/East mostly outside the frustum —
  confirmed by screenshot at 390×844: only a sliver of the fan's inner edge
  crossed into frame.

  Two portrait-only adjustments below, both scoped to seats 1/3 (never seat 2,
  North, which isn't clipped and whose own local `+Z` points a different way
  — see `seatLayout.ts`'s table): pull the whole fan toward the table's
  centre along the anchor's local `+Z` axis (the doc-guaranteed "away from
  the seat, across the table" direction — for West/East specifically that
  is toward centre, never toward the human or North), and default
  `maxVisible` to the 3-card portrait compression `docs/04-FRONTEND-UX.md`
  §14 already calls for (a narrower fan needs less lateral room). Both are
  no-ops in landscape and for a caller that passes its own `maxVisible`.
  Camera and seat-ring radius are untouched — both are tuned/verified
  elsewhere, so this fix stays local to the one component that owns "where
  an opponent's cards render".
-->
<script lang="ts">
	import { T, useThrelte } from '@threlte/core';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import Card from '$lib/three/cards/Card.svelte';
	import { CARD_ASPECT, fanPositions } from './layout';
	import { dealOrigin, lerpPose } from './cardMotion';
	import { TEMPO, flightMs } from './tempo';
	import { isPortrait } from '$lib/three/scene/breakpoints';
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

	const { size } = useThrelte();
	const aspect = $derived($size.width / Math.max(1, $size.height));
	const portrait = $derived(isPortrait(aspect));

	/** West/East only — see the module doc's "Portrait: West/East edge clipping" section. North isn't clipped and its local `+Z` points toward the human, not the centre. */
	const isSideSeat = $derived(seat === 1 || seat === 3);

	/**
	 * §14's "3-card visual stack" is the default the moment nobody overrides
	 * it — a caller's own `maxVisible` (if `TableScene.svelte` ever passes
	 * one) still wins outright.
	 */
	const effectiveMaxVisible = $derived(maxVisible ?? (portrait ? 3 : count));
	const visibleCount = $derived(Math.max(0, Math.min(count, effectiveMaxVisible)));

	/**
	 * World-unit pull toward the table centre, along this anchor's local
	 * `+Z` — the fixed amount needed to bring West/East back inside the
	 * portrait camera's (much narrower) horizontal FOV. Verified empirically
	 * against a 390×844 screenshot (`docs/04-FRONTEND-UX.md` §14's own
	 * "verified against a screenshot, not guessed" standard) rather than
	 * derived from the camera's closed form: `CameraRig`'s pitch couples fov,
	 * `dist` and the look-at target together in a way that isn't worth
	 * re-deriving here for a single scalar this component alone consumes.
	 */
	const PORTRAIT_SIDE_INSET = 0.1;
	const sideInset = $derived(portrait && isSideSeat ? PORTRAIT_SIDE_INSET : 0);

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

<T.Group position={[0, 0, sideInset]}>
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
