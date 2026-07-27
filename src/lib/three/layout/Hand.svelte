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
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import { untrack } from 'svelte';
	import Card from '$lib/three/cards/Card.svelte';
	import { CARD_ASPECT, fanPositions } from './layout';
	import { dealOrigin, lerpPose } from './cardMotion';
	import { TEMPO, flightMs } from './tempo';
	import { isNarrowLandscape, isPortrait } from '$lib/three/scene/breakpoints';
	import { playLift } from '$lib/ui/sound';
	import type { CardId, LegalMove, Seat } from '$lib/euchre';

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
		/**
		 * `view.handNo` — a change is this seat's only signal that a fresh hand
		 * was just dealt (there is no `steps` channel reaching this component;
		 * see `TableScene.svelte`'s doc comment on why view-diffing stands in for
		 * one). The very first value seen is never animated (a mount/reconnect
		 * always snaps — see the `#dealt` effect below), so passing this is safe
		 * even when the caller cannot yet distinguish "fresh load" from "fresh
		 * hand" itself.
		 */
		handNo?: number;
		/** `view.dealerSeat` — with `seat`, only used to phase this seat's deal-in stagger against the other three (see `#dealt`). */
		dealerSeat?: Seat;
		/** This hand's own seat. Always `0` (the viewer), kept as a prop rather than hard-coded so the stagger math reads the same as `OpponentHand.svelte`'s. */
		seat?: Seat;
		/** `prefers-reduced-motion` (or the in-app override) — collapses the deal flight, per `docs/04-FRONTEND-UX.md` §9.2. */
		reducedMotion?: boolean;
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
		 * Flat on the felt (0) the cards are edge-on and unreadable — you cannot
		 * play a hand you cannot see.
		 *
		 * NOTE THE SIGN. Seat 0's group is rotated 180 deg about Y (which is why
		 * moving the fan "toward the table centre" is +Z in seat-local space), so a
		 * POSITIVE recline tips the cards AWAY from the viewer. At +45 they went
		 * exactly edge-on and vanished to a hairline.
		 *
		 * Geometry: a plane's normal is +Z; rotating theta about X sends it to
		 * (0, -sin theta, cos theta), and rotX = -90 + recline. The fan sits at
		 * z = 0.17 (seat radius 0.3 pulled in 0.13) with the camera at
		 * (0, 0.62, 0.78), so the hand-to-camera direction is (0, 0.712, 0.700).
		 * recline = -45 gives a seat-local normal of (0, 0.707, -0.707), which the
		 * 180 deg seat rotation maps to (0, 0.707, 0.707) in world space — square
		 * to the camera. If you move the camera, recompute; do not eyeball it.
		 */
		reclineDeg = -45,
		disabled = false,
		onplay,
		onillegal,
		onhover,
		handNo,
		dealerSeat = 0,
		seat = 0,
		reducedMotion = false
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
	// Sized against HAND_FORWARD. Moving the fan toward the player also moves it
	// toward the camera, which magnifies it — at 0.205 the cards filled half the
	// frame and clipped at the bottom again. These keep them large and legible
	// while sitting fully inside the viewport near the near edge.
	const baseCardHeight = $derived(portrait ? 0.13 : narrow ? 0.145 : 0.155);
	const baseOverlap = $derived(portrait ? 0.62 : 0.6);

	const cardHeight = $derived(cardHeightProp ?? baseCardHeight);
	const overlap = $derived(overlapProp ?? baseOverlap);

	/** `LegalMove.move` carries a `card` only for `play` and `discard` — the two shapes this hand ever offers. */
	const legalIds = $derived(
		new Set(
			legal.flatMap((m) => (m.move.t === 'play' || m.move.t === 'discard' ? [m.move.card] : []))
		)
	);

	/**
	 * Vertical clearance so the reclined fan never sinks into the felt.
	 * Roughly cardHeight/2 * sin(45 deg) plus a margin, for the largest size.
	 */
	const HAND_LIFT = 0.085;

	/**
	 * How far the fan sits from the seat anchor, along the seat's local +Z (which
	 * points at the table centre, because seat 0's group is rotated 180 deg).
	 *
	 * Negative moves the fan back toward the player — i.e. DOWN the screen, toward
	 * the bottom edge. Kept small: at -0.13 the fan overshoots the rail and leaves
	 * the frame entirely, and at +0.13 it sprawls over the trick zone in the middle
	 * of the table.
	 */
	const HAND_FORWARD = -0.04;

	/**
	 * The hand as dealt, in fan order — the card's *slot*, not its current index.
	 *
	 * Without this the fan was laid out with `fanPositions(cards.length)`, so the
	 * instant any card left the hand every remaining card slid to a new position
	 * to re-centre the fan. Four seats playing meant the whole hand rearranged
	 * several times a trick, which reads as the hand being re-dealt — it is what
	 * "the cards keep getting redealt after each player plays" actually was.
	 *
	 * A real hand does not re-fan itself after every trick: your cards stay put and
	 * the gap stays. Slots are captured at deal time and cards keep them for the
	 * whole hand.
	 */
	let slotOrder = $state.raw<readonly CardId[]>([]);

	/** Slots, extended for any card that arrives mid-hand (the dealer's pickup). */
	const slots = $derived.by(() => {
		const known = new Set(slotOrder);
		const extra = cards.filter((c) => !known.has(c));
		return extra.length === 0 ? slotOrder : [...slotOrder, ...extra];
	});

	/** Where a card sits in the fan. Falls back to render order before the first deal. */
	function slotOf(cardId: CardId, fallback: number): number {
		const i = slots.indexOf(cardId);
		return i >= 0 ? i : fallback;
	}

	const poses = $derived(
		fanPositions(Math.max(slots.length, cards.length), {
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
		const wasHovering = hoveredId !== null;
		hoveredId = cardId;
		onhover?.(cardId);
		// Only the null -> id edge, not id -> id (crossing from one overlapping
		// card straight onto its neighbour, which fires the same enter/leave
		// pair) and not id -> null (leaving plays no sound in this design).
		if (cardId !== null && !wasHovering) playLift();
	}

	/*
	 * Deal-in choreography (docs/04-FRONTEND-UX.md §9.2: 60ms flight/card, 50ms
	 * stagger, in "the engine's 3-2/2-3 packet order").
	 *
	 * There is no `steps` channel reaching this component (`TableScene.svelte`'s
	 * doc comment explains why — no other file in this task's scope threads
	 * `Step[]` down from the actor's `sync` event) so `handNo` changing is the
	 * only signal a fresh deal just happened. This is not a second source of
	 * truth: the *positions* rendered below always come from `poses` (a pure
	 * function of the current `cards`/`legal` props, exactly as before this
	 * animation existed) — `dealProgress` only ever blends *toward* that
	 * already-correct target, and a card whose entry never gets a tween (the
	 * component mounting mid-hand, or `reducedMotion`) just renders at its
	 * final pose immediately, which is what `lerpPose` at `t = 1` already is.
	 *
	 * The engine's true packet order (which seat/how many cards per round) is
	 * intentionally not reimplemented here — `$lib/euchre` is the one rulebook,
	 * and packet order is a dealing-rule detail, not a rendering one. This
	 * approximates it with a round-robin ("everyone gets a card, four times")
	 * phased by `seat - dealerSeat`, which lands on the same 20-card/1010ms
	 * total the tempo table gives and, being round-robin rather than
	 * seat-by-seat, actually reads *more* like a real deal (one card at a time
	 * around the table) than a literal 3-2 replay would from a static seat
	 * offset alone.
	 */
	let dealTweens = $state.raw(new Map<CardId, Tween<number>>());
	let sawFirstHandNo = false;

	$effect(() => {
		const signal = handNo;
		if (!sawFirstHandNo) {
			// First evaluation is always a mount or a hard resync — snap, per the
			// same rule `docs/04-FRONTEND-UX.md` §9.3 gives `hardResync`: "teleport
			// every card; no animation."
			sawFirstHandNo = true;
			return;
		}
		if (signal === undefined) return;
		// EVERYTHING below runs untracked.
		//
		// `new Tween()` owns internal `$state` that ticks every animation frame.
		// Constructing tweens (and calling `.set()`) directly in this effect made
		// the effect depend on that per-frame state, so each frame re-ran the
		// effect, which built fresh tweens, which ticked again — a self-sustaining
		// loop that restarts the deal-in continuously. On screen that reads exactly
		// as "the game reshuffles and re-deals every hand", which is how it was
		// reported. Same failure class as the `Hint.svelte` self-trigger: an effect
		// must not take a reactive dependency on something it creates or writes.
		untrack(() => {
			const currentCards = cards;
			slotOrder = [...currentCards]; // fix the fan's slots for this hand
			const order = ((seat - dealerSeat + 4) % 4) as number;
			const flight = flightMs(TEMPO.dealFlightMs, reducedMotion);
			const stagger = reducedMotion ? 0 : TEMPO.dealStaggerMs;
			const fresh = new Map<CardId, Tween<number>>();
			currentCards.forEach((id, i) => {
				const t = new Tween(0, { easing: cubicOut });
				fresh.set(id, t);
				void t.set(1, { duration: flight, delay: (order + i * 4) * stagger });
			});
			dealTweens = fresh;
		});
	});

	/** `poses[i]` unless that card is still (or freshly) mid-deal-flight, in which case an eased blend from `dealOrigin`. */
	function renderPose(cardId: CardId, i: number) {
		const target = poses[i];
		const tween = dealTweens.get(cardId);
		if (!tween || tween.current >= 1) return target;
		return lerpPose(dealOrigin(target), target, tween.current, reducedMotion ? 0 : cardHeight * 0.5);
	}
</script>

<!--
	Lifted clear of the felt, and only slightly in from the seat anchor.

	The LIFT is not decoration. Reclined toward the viewer, the lower half of each
	card dips below y=0 and the table surface clips it — the bottoms of the cards
	simply vanish. A card of height h reclined by `recline` needs at least
	h/2 * sin(recline) of clearance; HAND_LIFT below carries that plus a margin.

	The inward offset is deliberately SMALL (0.05, not 0.13). Further in and the
	fan sprawls across the middle of the table and covers the trick — the player's
	own hand should sit in front of them, not over the cards everyone is playing.

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
<T.Group position={[0, HAND_LIFT, HAND_FORWARD]}>
	{#each cards as cardId, i (cardId)}
		{@const pose = renderPose(cardId, slotOf(cardId, i))}
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
