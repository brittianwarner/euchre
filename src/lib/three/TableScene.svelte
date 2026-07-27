<!--
  TableScene.svelte — the actual furniture + cards, mounted as the sole child
  of `<Canvas>` inside `EuchreTable3D.svelte`.

  Why this is a separate file rather than inlined in `EuchreTable3D.svelte`:
  `interactivity()` (the plugin that makes `onclick`/`onpointerenter` work on
  `<T.Mesh>` — which `Card.svelte` already relies on) calls `injectPlugin`,
  which reads Threlte's context. That context is created by `<Canvas>` and is
  only visible to components instantiated *as its children* — i.e. components
  whose own `<script>` runs while `<Canvas>` is rendering its children. A
  component that itself *authors* the `<Canvas>` tag runs its `<script>`
  earlier than that (before its own markup, including `<Canvas>`, even
  exists), so `interactivity()` cannot be called there. This file exists
  solely to be that child; every other seam (`useThrelte()` in
  `SeatAnchors`/`CameraRig`/`Lights`) already relies on the same rule.

  Structurally still a function of `view`: every mesh you would see with all
  motion stripped out (cards, counts, nameplates, trump, whose turn) is
  exactly what `view` says, on every frame, unconditionally. What this file
  additionally does now is *decorate* the transition from one `view` to the
  next — a trump-called chime, a hand-score flourish, a game-won ring — by
  diffing consecutive `view`s it is handed, the same technique `Hand.svelte`,
  `OpponentHand.svelte` and `TrickPile.svelte` use for the deal/play/sweep
  beats (see each of their doc comments).

  ## Why view-diffing instead of the `Step[]` channel `docs/04-FRONTEND-UX.md`
  ## §9 describes

  The engine already computes a real `Step[]` per `sync` (see `Step` in
  `$lib/euchre/types.ts` and `SyncEvent.steps` in `$lib/protocol`) — this file
  simply never receives it. `EuchreTable3D.svelte` (the Canvas host,
  `$lib/three/EuchreTable3D.svelte`) and the page that composes it
  (`src/routes/play/[gameId]/+page.svelte`, via `TableStore.applySync` in
  `$lib/game/table.svelte.ts`) are outside this task's owned file set
  (`$lib/three/layout/**`, this file, `$lib/ui/sound/**`) and today thread
  only `PublicGameView` down, not `steps`. Reconstructing "what just happened"
  from two consecutive `view`s is strictly less information (it cannot see a
  packet's dealing order, a discard's timing, or a bid's text — none of which
  this file or its children need), but it is a *sound* substitute precisely
  because a `PublicGameView` is authoritative and total: every transition this
  file animates is a transition that actually happened to real server state,
  never a guess. If `steps` is threaded down here in a later change, the
  per-field diffs below can be deleted in favour of matching on `Step.t`
  directly — nothing downstream (the sound calls, the celebration ring) needs
  to change shape.

  ## Animation never gates truth

  Nothing in this file (or `Hand`/`OpponentHand`/`TrickPile`) delays applying
  `view` — every effect below only *reads* `view` to decide what decoration to
  layer on top of the render that already, unconditionally, reflects it. A
  `view` that lands mid-celebration simply gets its celebration cut short (the
  `setTimeout` below clears `celebration`; the next `$effect` run starts a new
  one if the new `view` calls for it) — there is no branch anywhere that waits
  for an animation to finish before the next `view` takes effect.
-->
<script lang="ts">
	import { HTML, interactivity } from '@threlte/extras';
	import { T } from '@threlte/core';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import { Table, Lights, CameraRig, SeatAnchors } from '$lib/three/scene';
	import { Hand, OpponentHand, TrickPile, Kitty, type KittyStage } from '$lib/three/layout';
	import { reducedMotion } from '$lib/three/layout/reducedMotion.svelte';
	import { TEMPO, flightMs } from '$lib/three/layout/tempo';
	import { warmCardTextures } from '$lib/three/cards/cardTexture';
	import {
		SoundToggle,
		playDeal,
		playEuchre,
		playError,
		playGameWon,
		playHandWon,
		playMarch,
		playTrumpCalled,
		vibrate
	} from '$lib/ui/sound';
	import type { CardId, HandResult, PublicGameView, Seat, Suit } from '$lib/euchre';

	interface Props {
		view: PublicGameView;
		fourColor?: boolean;
		disabled?: boolean;
		onplay?: (cardId: CardId) => void;
		onillegal?: (cardId: CardId) => void;
	}

	let { view, fourColor = false, disabled = false, onplay, onillegal }: Props = $props();

	// Enables pointer events (`onclick`, `onpointerenter`, `onpointerleave`) on
	// every `<T.Mesh>` beneath this component — `Card.svelte` is authored
	// against exactly this plugin. Called once; the returned context is not
	// needed here because every consumer (`Card.svelte`) reads it implicitly
	// through the props Threlte's plugin system already wires up.
	interactivity();

	const SEAT_NAME: Readonly<Record<Seat, string>> = { 0: 'You', 1: 'West', 2: 'North', 3: 'East' };
	const SUIT_GLYPH: Readonly<Record<Suit, string>> = { S: '♠', H: '♥', D: '♦', C: '♣' };

	// Bidding-round summary the doc comment on `Kitty.svelte` asks the caller to
	// derive — kept here, not inside the component, for exactly the reason it
	// gives: this is one line of protocol-shape knowledge, not render logic.
	const kittyStage = $derived<KittyStage>(
		!view.trump ? 'upcard' : view.upCardTurnedDown ? 'turnedDown' : 'buried'
	);

	// The current trick sits at the felt's true centre; the kitty is nudged
	// toward North so the two zones never overlap (trick radius defaults to
	// 0.055 m; 0.15 m of clearance is generous).
	const KITTY_POSITION: readonly [number, number, number] = [0, 0, -0.15];

	// Build all 25 card textures up front so no card's appearance depends on the
	// frame it first renders in. See `warmCardTextures` for why this is not
	// premature optimisation but a correctness fix.
	$effect(() => {
		warmCardTextures(fourColor);
	});

	/** Plays the sound first, then forwards to the caller's own handler unchanged — no dead clicks, and now no silent ones either. */
	function handleIllegal(cardId: CardId): void {
		playError();
		vibrate(70); // "error" — one long (doc §11)
		onillegal?.(cardId);
	}

	/* -------------------------------------------------------------------------- */
	/* Table-wide beats: trump called, hand scored, game won                      */
	/*                                                                            */
	/* Each block below is independent view-diffing, same shape and same "skip   */
	/* the first evaluation" mount/resync guard as `Hand.svelte`'s deal effect —  */
	/* see that file's doc comment for why the guard is enough on its own to     */
	/* make a reconnect snap instead of replaying every beat since the game       */
	/* began.                                                                     */
	/* -------------------------------------------------------------------------- */

	let sawFirstTrump = false;
	let prevTrump: Suit | null = null;
	$effect(() => {
		const trump = view.trump;
		if (!sawFirstTrump) {
			sawFirstTrump = true;
			prevTrump = trump;
			return;
		}
		if (trump !== null && prevTrump === null) playTrumpCalled(trump);
		prevTrump = trump;
	});

	let sawFirstHandNo = false;
	let prevHandNoForDeal: number | undefined;
	$effect(() => {
		const handNo = view.handNo;
		if (!sawFirstHandNo) {
			sawFirstHandNo = true;
			prevHandNoForDeal = handNo;
			return;
		}
		if (handNo !== prevHandNoForDeal) {
			// "One per packet, not per card" (doc §11) — the exact packet count
			// isn't available here (see the module doc's "why view-diffing" note),
			// so this fires a couple of riffles spaced across the deal's real
			// ~1s duration rather than one for all 20 cards or one per card.
			playDeal();
			setTimeout(() => playDeal(), flightMs(500, reducedMotion.enabled));
		}
		prevHandNoForDeal = handNo;
	});

	interface Celebration {
		readonly key: number;
		readonly color: string;
		readonly progress: Tween<number>;
	}
	let celebration = $state.raw<Celebration | null>(null);
	let celebrationTimer: ReturnType<typeof setTimeout> | undefined;
	let celebrationKey = 0;

	function celebrate(color: string, ms: number): void {
		clearTimeout(celebrationTimer);
		const progress = new Tween(0, { easing: cubicOut });
		celebrationKey += 1;
		celebration = { key: celebrationKey, color, progress };
		void progress.set(1, { duration: flightMs(ms, reducedMotion.enabled) });
		celebrationTimer = setTimeout(() => {
			celebration = null;
		}, ms + 80);
	}

	const HAND_RESULT_COLOR: Readonly<Record<HandResult, string>> = {
		point: '#e8c27a',
		march: '#e8c27a',
		lone_point: '#e8c27a',
		lone_march: '#e8c27a',
		euchre: '#c0554a',
		throw_in: '#8a8a8a'
	};

	let sawFirstResult = false;
	let prevResult: HandResult | null = null;
	let prevHandNoForScore: number | undefined;
	$effect(() => {
		const result = view.result;
		const handNo = view.handNo;
		if (!sawFirstResult) {
			sawFirstResult = true;
			prevResult = result;
			prevHandNoForScore = handNo;
			return;
		}
		// Guard on `handNo` too: a `result` that is non-null across a resync
		// (reconnecting mid `hand_score`) must not replay the celebration —
		// only a genuine null -> non-null transition within the same hand does.
		if (result !== null && prevResult === null && handNo === prevHandNoForScore) {
			const big = result === 'euchre' || view.aloneSeat !== null;
			celebrate(HAND_RESULT_COLOR[result], big ? TEMPO.celebrateEuchreLonerMs : TEMPO.celebratePointMarchMs);
			if (result === 'euchre') playEuchre();
			else if (result === 'march' || result === 'lone_march') playMarch();
			else if (result !== 'throw_in') playHandWon();
			if (result !== 'throw_in') vibrate([16, 60, 16]);
		}
		prevResult = result;
		prevHandNoForScore = handNo;
	});

	let sawFirstWinnerTeam = false;
	let prevWinnerTeam: 0 | 1 | null = null;
	$effect(() => {
		const winnerTeam = view.winnerTeam;
		if (!sawFirstWinnerTeam) {
			sawFirstWinnerTeam = true;
			prevWinnerTeam = winnerTeam;
			return;
		}
		if (winnerTeam !== null && prevWinnerTeam === null) {
			celebrate('#e8c27a', TEMPO.gameWonMs);
			playGameWon();
			vibrate([20, 60, 20, 60, 40]);
		}
		prevWinnerTeam = winnerTeam;
	});
</script>

<CameraRig />
<Lights />
<Table />

<SeatAnchors>
	{#snippet seat0(t)}
		{#if t.seat === view.you}
			<Hand
				cards={view.hand}
				legal={view.legal}
				{fourColor}
				{disabled}
				{onplay}
				onillegal={handleIllegal}
				handNo={view.handNo}
				dealerSeat={view.dealerSeat}
				seat={t.seat}
				reducedMotion={reducedMotion.enabled}
			/>
		{:else}
			<OpponentHand
				count={view.handCounts[t.seat]}
				handNo={view.handNo}
				dealerSeat={view.dealerSeat}
				seat={t.seat}
				reducedMotion={reducedMotion.enabled}
			/>
		{/if}
		<HTML position={[0, 0.05, -0.19]} center pointerEvents="none" transform={false}>
			<div
				class="nameplate"
				class:active={t.seat === view.turnSeat}
				class:sitting={t.seat === view.sittingSeat}
			>
				<span class="name">{SEAT_NAME[t.seat]}</span>
				{#if t.seat === view.dealerSeat}<span class="tag">Dealer</span>{/if}
				{#if t.seat === view.makerSeat && view.trump}
					<span class="tag trump">{SUIT_GLYPH[view.trump]} called</span>
				{/if}
				{#if t.seat === view.sittingSeat}<span class="tag">Sitting out</span>{/if}
			</div>
		</HTML>
	{/snippet}

	{#snippet seat1(t)}
		<OpponentHand
			count={view.handCounts[t.seat]}
			handNo={view.handNo}
			dealerSeat={view.dealerSeat}
			seat={t.seat}
			reducedMotion={reducedMotion.enabled}
		/>
		<HTML position={[0, 0.05, -0.12]} center pointerEvents="none" transform={false}>
			<div
				class="nameplate"
				class:active={t.seat === view.turnSeat}
				class:sitting={t.seat === view.sittingSeat}
			>
				<span class="name">{SEAT_NAME[t.seat]}</span>
				{#if t.seat === view.dealerSeat}<span class="tag">Dealer</span>{/if}
				{#if t.seat === view.makerSeat && view.trump}
					<span class="tag trump">{SUIT_GLYPH[view.trump]} called</span>
				{/if}
				{#if t.seat === view.sittingSeat}<span class="tag">Sitting out</span>{/if}
			</div>
		</HTML>
	{/snippet}

	{#snippet seat2(t)}
		<OpponentHand
			count={view.handCounts[t.seat]}
			handNo={view.handNo}
			dealerSeat={view.dealerSeat}
			seat={t.seat}
			reducedMotion={reducedMotion.enabled}
		/>
		<HTML position={[0, 0.05, -0.12]} center pointerEvents="none" transform={false}>
			<div
				class="nameplate"
				class:active={t.seat === view.turnSeat}
				class:sitting={t.seat === view.sittingSeat}
			>
				<span class="name">{SEAT_NAME[t.seat]}</span>
				{#if t.seat === view.dealerSeat}<span class="tag">Dealer</span>{/if}
				{#if t.seat === view.makerSeat && view.trump}
					<span class="tag trump">{SUIT_GLYPH[view.trump]} called</span>
				{/if}
				{#if t.seat === view.sittingSeat}<span class="tag">Sitting out</span>{/if}
			</div>
		</HTML>
	{/snippet}

	{#snippet seat3(t)}
		<OpponentHand
			count={view.handCounts[t.seat]}
			handNo={view.handNo}
			dealerSeat={view.dealerSeat}
			seat={t.seat}
			reducedMotion={reducedMotion.enabled}
		/>
		<HTML position={[0, 0.05, -0.12]} center pointerEvents="none" transform={false}>
			<div
				class="nameplate"
				class:active={t.seat === view.turnSeat}
				class:sitting={t.seat === view.sittingSeat}
			>
				<span class="name">{SEAT_NAME[t.seat]}</span>
				{#if t.seat === view.dealerSeat}<span class="tag">Dealer</span>{/if}
				{#if t.seat === view.makerSeat && view.trump}
					<span class="tag trump">{SUIT_GLYPH[view.trump]} called</span>
				{/if}
				{#if t.seat === view.sittingSeat}<span class="tag">Sitting out</span>{/if}
			</div>
		</HTML>
	{/snippet}
</SeatAnchors>

<!--
	The one, obvious mute switch (docs/04-FRONTEND-UX.md §11) — anchored to a
	stable point on the felt (just past North's nameplate), the same
	`transform={false}`/`center` pattern the nameplates above already use,
	rather than a viewport-`fixed` overlay.

	Why not `fixed`: `<HTML>`'s wrapper node always carries a JS-driven
	`style.transform` placing it at this 3D point's *projected screen
	position* — `fullscreen` only changes that wrapper's size, not this fact
	(see `@threlte/extras`'s `HTML.svelte`: the outer `<svelte:element>` always
	gets `transform: translate3d(...)`). A CSS `transform` on an ancestor
	becomes the containing block for any `position: fixed` descendant, so a
	toggle nested in a `fullscreen` `<HTML>` is fixed *to that 3D anchor's
	projection*, not to the viewport — confirmed by screenshot: it rendered
	pinned near the felt's centre instead of the screen's.

	Why not a real viewport corner either: `src/routes/play/[gameId]/+page.svelte`
	claims bottom-left/bottom-right/top-right already, and `$lib/ui/GlobalNav.svelte`
	claims top-right too (wide enough at 390px to reach past screen-centre —
	seen by screenshot, not assumed). Anchoring to the felt instead of the
	viewport sidesteps every one of those, at both breakpoints, the same way
	CameraRig already guarantees the felt (and therefore North's nameplate) is
	always framed — see `docs/04-FRONTEND-UX.md` §14.
-->
<HTML position={[0, 0.08, -0.42]} center transform={false}>
	<div class="sound-toggle-anchor"><SoundToggle /></div>
</HTML>

<Kitty
	stage={kittyStage}
	upCard={view.upCard}
	kittyCount={view.kittyCount}
	{fourColor}
	position={KITTY_POSITION}
	reducedMotion={reducedMotion.enabled}
/>

<TrickPile
	plays={view.trick.plays}
	winnerSeat={view.trick.winnerSeat}
	{fourColor}
	trickIndex={view.trick.index}
	handNo={view.handNo}
	reducedMotion={reducedMotion.enabled}
/>

<!--
	The hand-score / game-won beat: a brief expanding, fading ring at the
	felt's centre, colour-themed by outcome (see `HAND_RESULT_COLOR` and the
	game-won call above). Purely decorative — `celebration` is `$state.raw`,
	never read by anything that decides game logic, and `{#if celebration}`
	unmounting it the moment the timer clears is exactly "brief" (the brief's
	own word) rather than a lingering overlay.
-->
{#if celebration}
	{@const ring = celebration}
	{@const grow = 0.05 + ring.progress.current * 0.55}
	{@const fade = Math.max(0, 1 - ring.progress.current)}
	<T.Mesh position={[0, 0.012, 0]} rotation={[-Math.PI / 2, 0, 0]}>
		<T.RingGeometry args={[Math.max(0.001, grow * 0.72), grow, 48]} />
		<T.MeshBasicMaterial color={ring.color} transparent opacity={fade * 0.65} depthWrite={false} />
	</T.Mesh>
{/if}


<style>
	.nameplate {
		display: flex;
		align-items: center;
		gap: 0.35rem;
		padding: 0.2rem 0.55rem;
		border-radius: 999px;
		background: rgba(15, 20, 14, 0.72);
		border: 1px solid rgba(232, 194, 122, 0.25);
		color: #f2e8d5;
		font-family: 'Iowan Old Style', 'Palatino Linotype', Palatino, Georgia, serif;
		font-size: 0.7rem;
		white-space: nowrap;
		box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
	}
	.nameplate.active {
		border-color: #e8c27a;
		box-shadow: 0 0 0 1px #e8c27a inset;
	}
	.nameplate.sitting {
		opacity: 0.55;
	}
	.name {
		font-weight: 600;
	}
	.tag {
		font-size: 0.62rem;
		color: #c9b89a;
		border-left: 1px solid rgba(201, 184, 154, 0.35);
		padding-left: 0.35rem;
	}
	.tag.trump {
		color: #e8c27a;
	}

	.sound-toggle-anchor {
		pointer-events: auto;
	}
</style>
