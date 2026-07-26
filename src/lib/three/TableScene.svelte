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

  Purely a function of `view`: no local game state, no animation state
  machine. Server truth in, meshes out (hard rule 4 — "animation never owns
  truth" — is satisfied trivially here because nothing here decides anything;
  it only ever mirrors the last `view` it was given).
-->
<script lang="ts">
	import { HTML, interactivity } from '@threlte/extras';
	import { Table, Lights, CameraRig, SeatAnchors } from '$lib/three/scene';
	import { Hand, OpponentHand, TrickPile, Kitty, type KittyStage } from '$lib/three/layout';
	import { warmCardTextures } from '$lib/three/cards/cardTexture';
	import type { CardId, PublicGameView, Seat, Suit } from '$lib/euchre';

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
</script>

<CameraRig />
<Lights />
<Table />

<SeatAnchors>
	{#snippet seat0(t)}
		{#if t.seat === view.you}
			<Hand cards={view.hand} legal={view.legal} {fourColor} {disabled} {onplay} {onillegal} />
		{:else}
			<OpponentHand count={view.handCounts[t.seat]} />
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
		<OpponentHand count={view.handCounts[t.seat]} />
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
		<OpponentHand count={view.handCounts[t.seat]} />
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
		<OpponentHand count={view.handCounts[t.seat]} />
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

<Kitty
	stage={kittyStage}
	upCard={view.upCard}
	kittyCount={view.kittyCount}
	{fourColor}
	position={KITTY_POSITION}
/>

<TrickPile plays={view.trick.plays} winnerSeat={view.trick.winnerSeat} {fourColor} />

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
</style>
