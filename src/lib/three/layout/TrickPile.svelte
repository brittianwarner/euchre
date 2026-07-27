<!--
  TrickPile.svelte — the cards currently on the table, one per seat that has
  played this trick.

  Unlike Hand/OpponentHand, this does not live inside a seat's `SeatAnchor` —
  it sits at the felt's centre, so its cards use table space directly
  (`trickCardPose` in `layout.ts`): each play is offset toward, and rotated to
  face, the seat that played it. A loner trick shows as few as 0-3 plays; there
  is nothing seat-count-specific to handle, since this simply maps over
  whatever `plays` it is given.

  The winning card is made "readable after the trick resolves" (the brief)
  using `Card.svelte`'s own existing `highlighted` prop — it lifts and
  brightens exactly like a legal card does in `Hand.svelte`. No new mechanism,
  no ownership of the read-pause timing: `winnerSeat` is `null` until the
  server has decided, and this component just reflects whatever it is told.

  ## Animation never owns truth (docs/04-FRONTEND-UX.md §9.1)

  The base render below (`{#each plays as play (play.seat)}`) is an unconditional,
  pure function of the current `plays`/`winnerSeat` props — exactly as before
  this file grew an animation layer. Everything added is a *supplementary*
  decorative pass on top of it:

    - A newly-appended play blends in from `playOrigin` over `playTweens`
      instead of popping in place. If that tween is ever missing or already
      done, `renderPose` falls straight through to the same `trickCardPose`
      target the un-animated version used — there is no code path where the
      animation is the only thing standing between a card and its correct
      slot.
    - A trick that resolves and clears spawns a transient `sweeping` overlay —
      cards this component no longer owns (they are already gone from `plays`)
      finishing their visual trip to the winner's pile. If a `sync` lands
      before that overlay's `setTimeout` fires — a resync, a fast AI turn, a
      throttled tab catching up — `plays` simply reflects the new truth
      immediately and the stale overlay is cleared the moment the *next*
      transition's guard clause runs (see `#detectTransition`'s `sameHand` /
      `trickIndex` checks below); nothing here can delay or gate what `plays`
      shows.

  Every heuristic below is view-diffing, not a `steps` channel — see
  `TableScene.svelte`'s doc comment for why (no file in this task's scope
  threads `Step[]` down from the actor's `sync` event). The diff is
  deliberately conservative: anything that doesn't match one of the two
  recognised shapes (§ "one card appended" / § "trick cleared with a winner")
  animates nothing and just renders the new `plays` as-is, which was already
  correct before this file had any notion of motion.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import Card from '$lib/three/cards/Card.svelte';
	import { trickCardPose, type CardPose, type TrickOptions } from './layout';
	import { lerpPose, playOrigin } from './cardMotion';
	import { TEMPO, flightMs } from './tempo';
	import { playPlace, playTrickTake, vibrate } from '$lib/ui/sound';
	import type { CardId, Seat } from '$lib/euchre';

	interface Props {
		/** The current trick's plays in order, e.g. `view.trick.plays`. 0-4 entries (0-3 under a loner). */
		plays: readonly { seat: Seat; card: CardId }[];
		/** `view.trick.winnerSeat` — `null` until the trick resolves. */
		winnerSeat?: Seat | null;
		fourColor?: boolean;
		cardHeight?: number;
		/** Metres from the trick's centre to each seat's card slot. */
		radius?: number;
		/** World offset for the whole trick zone, so the composing scene can place it on the felt. */
		position?: readonly [number, number, number];
		/** `view.trick.index` — with `handNo`, lets the diff below tell "next trick in this hand" apart from a resync. */
		trickIndex?: number;
		/** `view.handNo` — a change here means "not the same trick sequence any more", so any in-flight overlay is dropped rather than animated. */
		handNo?: number;
		/** `prefers-reduced-motion` (or the in-app override). */
		reducedMotion?: boolean;
	}

	let {
		plays,
		winnerSeat = null,
		fourColor = false,
		cardHeight = 0.1,
		radius = 0.055,
		position = [0, 0, 0],
		trickIndex,
		handNo,
		reducedMotion = false
	}: Props = $props();

	const trickOptions = $derived<Partial<TrickOptions>>({ radius });

	function withOrigin(local: readonly [number, number, number]): [number, number, number] {
		return [position[0] + local[0], position[1] + local[1], position[2] + local[2]];
	}

	function withOriginPose(local: CardPose): CardPose {
		return { position: withOrigin(local.position), rotation: local.rotation };
	}

	/* -------------------------------------------------------------------------- */
	/* Entrance flight — one newly-played card at a time                          */
	/* -------------------------------------------------------------------------- */

	/** Keyed by seat: within one trick a seat plays exactly once, and the map is reset (by reuse — see `flightIn`) across tricks. */
	let playTweens = $state.raw(new Map<Seat, Tween<number>>());

	function flightIn(seat: Seat): void {
		const existing = playTweens.get(seat);
		if (existing) {
			void existing.set(0, { duration: 0 }); // reused across tricks — snap back before re-animating
			void existing.set(1, { duration: flightMs(TEMPO.cardPlayFlightMs, reducedMotion) });
			return;
		}
		const t = new Tween(0, { easing: cubicOut });
		const fresh = new Map(playTweens);
		fresh.set(seat, t);
		playTweens = fresh;
		void t.set(1, { duration: flightMs(TEMPO.cardPlayFlightMs, reducedMotion) });
	}

	/** `trickCardPose(...)` unless `seat`'s card is still mid-flight, in which case an eased blend from `playOrigin`. */
	function renderPose(seat: Seat, order: number): CardPose {
		const target = trickCardPose(seat, order, trickOptions);
		const tween = playTweens.get(seat);
		if (!tween || tween.current >= 1) return withOriginPose(target);
		const arc = reducedMotion ? 0 : TEMPO.cardPlayArcM;
		return withOriginPose(lerpPose(playOrigin(target), target, tween.current, arc));
	}

	/* -------------------------------------------------------------------------- */
	/* Sweep — the resolved trick sliding to the winner's pile                    */
	/* -------------------------------------------------------------------------- */

	interface SweepCard {
		readonly key: string;
		readonly card: CardId;
		readonly from: CardPose;
		readonly to: CardPose;
		readonly progress: Tween<number>;
	}

	let sweeping = $state.raw<readonly SweepCard[]>([]);
	let sweepTimer: ReturnType<typeof setTimeout> | undefined;

	/** Further out than the trick's own radius — "toward the winner's actual seat", not just their trick slot. */
	const SWEEP_RADIUS_PAD = 0.16;

	function triggerSweep(outgoing: readonly { seat: Seat; card: CardId }[], winner: Seat): void {
		clearTimeout(sweepTimer);
		const duration = flightMs(TEMPO.trickSweepMs, reducedMotion);
		const stagger = reducedMotion ? 0 : TEMPO.trickSweepStaggerMs;
		const arc = reducedMotion ? 0 : TEMPO.cardPlayArcM;

		const cards: SweepCard[] = outgoing.map((play, i) => {
			const from = trickCardPose(play.seat, i, trickOptions);
			const to = trickCardPose(winner, i, { radius: radius + SWEEP_RADIUS_PAD });
			const progress = new Tween(0, { easing: cubicOut });
			void progress.set(1, { duration, delay: i * stagger });
			return { key: `${play.seat}-${play.card}`, card: play.card, from, to, progress };
		});
		sweeping = cards;

		const totalMs = duration + Math.max(0, cards.length - 1) * stagger + 60;
		sweepTimer = setTimeout(() => {
			sweeping = [];
		}, totalMs);

		playTrickTake();
		vibrate([12, 40, 12]); // "success" — two short (doc §11)
	}

	function sweepPose(sc: SweepCard): CardPose {
		const arc = reducedMotion ? 0 : TEMPO.cardPlayArcM;
		return withOriginPose(lerpPose(sc.from, sc.to, sc.progress.current, arc));
	}

	/* -------------------------------------------------------------------------- */
	/* The diff itself                                                            */
	/* -------------------------------------------------------------------------- */

	let mounted = false;
	let prevPlays: readonly { seat: Seat; card: CardId }[] = [];
	let prevWinnerSeat: Seat | null = null;
	let prevTrickIndex: number | undefined;
	let prevHandNo: number | undefined;

	function samePrefix(a: readonly { seat: Seat; card: CardId }[], b: readonly { seat: Seat; card: CardId }[]): boolean {
		return a.every((p, i) => b[i] !== undefined && b[i].seat === p.seat && b[i].card === p.card);
	}

	$effect(() => {
		const currentPlays = plays;
		const currentWinner = winnerSeat;
		const currentIndex = trickIndex;
		const currentHandNo = handNo;

		if (!mounted) {
			// Mount or hard resync: never animate the first thing this component sees.
			mounted = true;
			prevPlays = currentPlays;
			prevWinnerSeat = currentWinner;
			prevTrickIndex = currentIndex;
			prevHandNo = currentHandNo;
			return;
		}

		const sameHand = prevHandNo === undefined || currentHandNo === undefined || prevHandNo === currentHandNo;

		if (
			sameHand &&
			prevTrickIndex === currentIndex &&
			currentPlays.length === prevPlays.length + 1 &&
			samePrefix(prevPlays, currentPlays)
		) {
			// One card appended, same trick: a play just happened.
			const appended = currentPlays[currentPlays.length - 1];
			flightIn(appended.seat);
			playPlace(appended.card);
			vibrate(14); // "medium" on commit (doc §11)
		} else if (
			sameHand &&
			currentPlays.length === 0 &&
			prevPlays.length > 0 &&
			prevWinnerSeat !== null &&
			currentIndex !== undefined &&
			prevTrickIndex !== undefined &&
			currentIndex === prevTrickIndex + 1
		) {
			// The trick this component was showing just resolved and cleared: sweep it.
			triggerSweep(prevPlays, prevWinnerSeat);
		}
		// Anything else (a rejected-optimistic revert, a resync, a throw-in) is left
		// to render `plays` as-is — see the module doc's "animation never owns truth" note.

		prevPlays = currentPlays;
		prevWinnerSeat = currentWinner;
		prevTrickIndex = currentIndex;
		prevHandNo = currentHandNo;
	});
</script>

<T.Group>
	<!-- Keyed on `seat`, not index: within one trick a seat plays exactly one card, so `seat` is real identity. -->
	{#each plays as play, i (play.seat)}
		{@const pose = renderPose(play.seat, i)}
		<Card
			id={play.card}
			faceUp
			position={pose.position}
			rotation={pose.rotation}
			height={cardHeight}
			{fourColor}
			highlighted={winnerSeat === play.seat}
		/>
	{/each}

	<!-- The just-resolved trick, still finishing its slide to the winner's pile. Not `plays` any more — see the module doc. Still face up: these cards were already public the moment they were played. -->
	{#each sweeping as sc (sc.key)}
		{@const pose = sweepPose(sc)}
		<Card id={sc.card} faceUp position={pose.position} rotation={pose.rotation} height={cardHeight} {fourColor} />
	{/each}
</T.Group>
