<script lang="ts">
	import { useGltf } from '@threlte/extras';
	import { T, useThrelte } from '@threlte/core';
	import { onMount } from 'svelte';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import { Table, Lights, CameraRig, SeatAnchors } from '#lib/three/scene/index.ts';
	import { OpponentHand, TrickPile, Kitty, type KittyStage } from '#lib/three/layout/index.ts';
	import { reducedMotion } from '#lib/three/layout/reducedMotion.svelte.ts';
	import { TEMPO, flightMs } from '#lib/three/layout/tempo.ts';
	import { warmCardTextures, loadTraditionalDeck } from '#lib/three/cards/cardTexture.ts';
	import {
		playDeal,
		playEuchre,
		playGameWon,
		playHandWon,
		playMarch,
		playTrumpCalled,
		vibrate
	} from '#lib/ui/sound/index.ts';
	import type { HandResult, PublicGameView, Seat, Suit } from '#lib/euchre/index.ts';

	const { invalidate } = useThrelte();
	onMount(() => {
		void loadTraditionalDeck().then(invalidate);
	});

	interface Props {
		view: PublicGameView;
		fourColor?: boolean;
	}

	let { view, fourColor = false }: Props = $props();

	const dealerToken = useGltf('/art/dealer.glb');
	const dealerPosition = $derived(
		(
			[
				[0.22, 0.003, 0.21],
				[-0.27, 0.003, -0.12],
				[-0.17, 0.003, -0.26],
				[0.27, 0.003, 0.12]
			] as [number, number, number][]
		)[view.dealerSeat]
	);

	// Bidding-round summary the doc comment on `Kitty.svelte` asks the caller to
	// derive — kept here, not inside the component, for exactly the reason it
	// gives: this is one line of protocol-shape knowledge, not render logic.
	const kittyStage = $derived<KittyStage>(
		!view.trump ? 'upcard' : view.upCardTurnedDown ? 'turnedDown' : 'buried'
	);

	// The current trick sits at the felt's true centre; the kitty is nudged
	// toward North so the two zones never overlap (trick radius defaults to
	// 0.055 m; 0.15 m of clearance is generous).
	const KITTY_POSITION: readonly [number, number, number] = [-0.13, 0.003, -0.12];

	// Build all 25 card textures up front so no card's appearance depends on the
	// frame it first renders in. See `warmCardTextures` for why this is not
	// premature optimisation but a correctness fix.
	$effect(() => {
		warmCardTextures(fourColor);
	});

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
			celebrate(
				HAND_RESULT_COLOR[result],
				big ? TEMPO.celebrateEuchreLonerMs : TEMPO.celebratePointMarchMs
			);
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
{#if $dealerToken}
	<T.Group position={dealerPosition}><T is={$dealerToken.scene} /></T.Group>
{/if}

<SeatAnchors>
	{#snippet seat1(t)}
		<OpponentHand
			count={view.handCounts[t.seat]}
			handNo={view.handNo}
			dealerSeat={view.dealerSeat}
			seat={t.seat}
			reducedMotion={reducedMotion.enabled}
		/>
	{/snippet}

	{#snippet seat2(t)}
		<OpponentHand
			count={view.handCounts[t.seat]}
			handNo={view.handNo}
			dealerSeat={view.dealerSeat}
			seat={t.seat}
			reducedMotion={reducedMotion.enabled}
		/>
	{/snippet}

	{#snippet seat3(t)}
		<OpponentHand
			count={view.handCounts[t.seat]}
			handNo={view.handNo}
			dealerSeat={view.dealerSeat}
			seat={t.seat}
			reducedMotion={reducedMotion.enabled}
		/>
	{/snippet}
</SeatAnchors>

{#if view.phase === 'bid_round_1' || view.phase === 'bid_round_2' || view.phase === 'cutting'}
	<Kitty
		stage={kittyStage}
		upCard={view.upCard}
		kittyCount={view.kittyCount}
		{fourColor}
		position={KITTY_POSITION}
		reducedMotion={reducedMotion.enabled}
	/>
{/if}
<T.Group position={[0, 0.003, 0]}>
	<TrickPile
		plays={view.trick.plays}
		winnerSeat={view.trick.winnerSeat}
		{fourColor}
		trickIndex={view.trick.index}
		handNo={view.handNo}
		reducedMotion={reducedMotion.enabled}
	/>
</T.Group>

{#if celebration}
	{@const ring = celebration}
	{@const grow = 0.05 + ring.progress.current * 0.55}
	{@const fade = Math.max(0, 1 - ring.progress.current)}
	<T.Mesh position={[0, 0.012, 0]} rotation={[-Math.PI / 2, 0, 0]}>
		<T.RingGeometry args={[Math.max(0.001, grow * 0.72), grow, 48]} />
		<T.MeshBasicMaterial color={ring.color} transparent opacity={fade * 0.65} depthWrite={false} />
	</T.Mesh>
{/if}
