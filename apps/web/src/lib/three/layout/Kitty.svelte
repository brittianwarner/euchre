<!--
  Kitty.svelte — the buried kitty, plus the up-card during and after bidding.

  Also table space, not anchor-local — like TrickPile.svelte, this sits at a
  fixed spot on the felt rather than inside any one seat.

  `PublicGameView` never reveals which three cards are buried (V12/V13); this
  component only ever consumes `kittyCount`, never a `CardId`, for the buried
  pile — the same "count only" discipline `OpponentHand.svelte` uses for a
  foreign hand. `upCard` is the one kitty card that is always publicly
  identified (V14), so it is the only one this component ever draws face up.

  `stage` is an explicit, caller-computed summary of where bidding stands,
  rather than something this component infers from raw view fields — the
  inference (`!view.trump ? 'upcard' : view.upCardTurnedDown ? 'turnedDown' :
  'buried'`) is a one-line derivation the composing page is better placed to
  own, and keeping it out of this component avoids baking protocol-shape
  knowledge into the render layer.

  - `'upcard'`     — bidding round 1, undecided: the up-card sits face up on
                     top of the three permanently-buried cards.
  - `'turnedDown'` — the dealer turned it down: it flips face down, is dimmed,
                     and is nudged aside — "the card stays publicly
                     identified" (the brief), just no longer the star.
  - `'buried'`     — bidding is over (ordered up, or round 2 settled): the
                     up-card is no longer distinguished from the pile; only
                     `kittyCount` (3 or 4) is shown, face down.

  ## The up-card's reveal

  `Card.svelte` renders face and back as two permanently-mounted, coplanar
  meshes gated by a static `faceUp` boolean (see its own doc comment on why —
  a `{#key}`-remounted material is a shipped regression in this project); it
  has no notion of an in-progress flip, and giving it one is out of this
  task's scope. So the "260ms slerp, geometry swaps at the 90deg crossing"
  reveal `docs/04-FRONTEND-UX.md` §9.2 describes for a true rotating flip is
  not reachable from here. What stands in for it: the up-card grows in from a
  sliver to full size (`height` is an ordinary animatable prop, unlike a flip)
  the moment `stage` first reads `'upcard'` for a given `upCard` — a
  deliberate, honest substitution, not an attempt to fake the real thing.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import Card from '#lib/three/cards/Card.svelte';
	import { CARD_ASPECT, stackPositions } from './layout';
	import { TEMPO, flightMs } from './tempo';
	import type { KittyStage } from './layout';
	import type { CardId } from '#lib/euchre/index.ts';

	interface Props {
		stage: KittyStage;
		/** `view.upCard`. Always non-null once dealt (V14); only drawn distinctly for `'upcard'` / `'turnedDown'`. */
		upCard: CardId | null;
		/** `view.kittyCount` — the buried pile's size (3, or 4 once a discard/turn-down has happened). Never a card id. */
		kittyCount: number;
		fourColor?: boolean;
		cardHeight?: number;
		/** World offset for the whole kitty, so the composing scene can place it on the felt. */
		position?: readonly [number, number, number];
		reducedMotion?: boolean;
	}

	let {
		stage,
		upCard,
		kittyCount,
		fourColor = false,
		cardHeight = 0.1,
		position = [0, 0, 0],
		reducedMotion = false
	}: Props = $props();

	/** See the module doc's "up-card's reveal" note: a grow-in stands in for the flip `Card.svelte` cannot do. */
	let revealedFor: CardId | null = null;
	let mounted = false;
	// `0` here is a placeholder immediately overwritten by the first `#effect`
	// run below (which always fires once at mount, snapping to `cardHeight`
	// with `duration: 0`) — never read before that runs, so capturing only
	// the prop's *initial* value here is correct, not a staleness bug.
	const revealHeight = new Tween(0, { easing: cubicOut });

	$effect(() => {
		const isUpcard = stage === 'upcard';
		const card = upCard;
		if (!mounted) {
			// Mount or hard resync: show the correct size immediately, even if
			// bidding is already mid-round-1 — a reconnect must snap, never replay
			// the reveal (docs/04-FRONTEND-UX.md §9.3's `hardResync` rule).
			mounted = true;
			revealedFor = isUpcard ? card : null;
			void revealHeight.set(cardHeight, { duration: 0 });
			return;
		}
		if (isUpcard && card !== null && card !== revealedFor) {
			revealedFor = card;
			void revealHeight.set(cardHeight * 0.15, { duration: 0 });
			void revealHeight.set(cardHeight, {
				duration: flightMs(TEMPO.upCardRevealMs, reducedMotion)
			});
		} else if (!isUpcard) {
			revealedFor = null;
			void revealHeight.set(cardHeight, { duration: 0 });
		}
	});

	/**
	 * Before bidding resolves, `kittyCount` is still `3` (the field only counts
	 * the up-card once its fate is sealed — see `PublicGameView.kittyCount`'s
	 * doc) even though a fourth, publicly-identified card sits on top. So the
	 * *truly hidden* pile is `3` for `'upcard'` / `'turnedDown'`, and the real
	 * `kittyCount` only once bidding is fully settled.
	 */
	const buriedCount = $derived(stage === 'buried' ? kittyCount : 3);

	const buriedPoses = $derived(stackPositions(buriedCount, { reclineDeg: 0 }));
	const cardWidth = $derived(cardHeight * CARD_ASPECT);

	// A modest lift above the buried stack's own tiny thickness, so the up-card
	// never z-fights the pile beneath it.
	const upCardLift = $derived(buriedCount * 0.0006 + 0.001);

	function withOrigin(local: readonly [number, number, number]): [number, number, number] {
		return [position[0] + local[0], position[1] + local[1], position[2] + local[2]];
	}
</script>

<T.Group>
	{#each buriedPoses as pose, i (`buried-${i}`)}
		<Card
			faceUp={false}
			position={withOrigin(pose.position)}
			rotation={pose.rotation}
			height={cardHeight}
		/>
	{/each}

	{#if stage === 'upcard'}
		<Card
			id={upCard}
			faceUp
			position={withOrigin([0, upCardLift, 0])}
			rotation={[-Math.PI / 2, 0, 0]}
			height={revealHeight.current}
			{fourColor}
		/>
	{:else if stage === 'turnedDown'}
		<!-- Turned face down, dimmed, and nudged aside: still there, no longer live. -->
		<Card
			id={upCard}
			faceUp={false}
			dimmed
			position={withOrigin([cardWidth * 0.5, upCardLift, 0])}
			rotation={[-Math.PI / 2, 0, Math.PI]}
			height={cardHeight}
		/>
	{/if}
</T.Group>
