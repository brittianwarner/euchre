<!--
  Live euchre table — the 3D table (visual layer) plus the DOM M2 client
  (accessible layer), composed together.

  Connects to euchreTable via @rivetkit/svelte exactly as before: useActor +
  withActorParams + store.bind + store.applySync, unchanged. The only
  addition on top of the M2 wiring is a local, purely-cosmetic optimistic
  preview (`displayView`) so a tap feels instant — the server `sync` is still
  the only thing that ever actually changes the game.
-->
<script lang="ts">
	import { onDestroy, untrack } from 'svelte';
	import { createGameRivet } from '#lib/client/rivet.ts';
	import { TableStore, type TableActorHandle } from '#lib/game/table.svelte.ts';
	import { applyOptimistic, whyIllegal } from '#lib/euchre/index.ts';
	import EuchreTable3D from '#lib/three/EuchreTable3D.svelte';
	import BidPanel from '#lib/ui/BidPanel.svelte';
	import CardRack from '#lib/ui/CardRack.svelte';
	import Hint from '#lib/ui/Hint.svelte';
	import RulesPanel from '#lib/ui/RulesPanel.svelte';
	import GameAnnouncer from '#lib/ui/onboarding/GameAnnouncer.svelte';
	import Walkthrough from '#lib/ui/onboarding/Walkthrough.svelte';
	import ScoreBoard from '#lib/ui/ScoreBoard.svelte';
	import SoundToggle from '#lib/ui/sound/SoundToggle.svelte';
	import TrickView from '#lib/ui/TrickView.svelte';
	import TableStatusBar from '#lib/ui/TableStatusBar.svelte';

	/** Compass names for the spoken log. Seat 0 is the player. */
	const SEAT_NAME = ['You', 'West', 'North', 'East'] as const;
	import type {
		CardId,
		LegalMoveId,
		PublicGameView,
		Step,
		SyncEvent
	} from '#lib/protocol/index.ts';

	let { data } = $props();

	// The parent keys this page by game ID, so tokens never cross between tables.
	const {
		rivet: { useActor },
		getParams
	} = createGameRivet(untrack(() => data.gameId));
	const store = new TableStore();

	const table = useActor(() => ({
		name: 'euchreTable' as const,
		noCreate: true,
		actorId: data.actorId,
		key: ['table', data.gameId],
		getParams
	}));

	// onEvent must be registered during component init — not inside $effect.
	// The table speaks: engine-authored calls plus screened banter. Registered
	// here beside `sync` because onEvent must run during component init.
	table.onEvent('chat', (payload: { msgId: string; seat: number; kind: string; text: string }) => {
		store.applyChat(payload);
	});

	/**
	 * Every step since this connection's last sync, for the screen-reader
	 * narration (`GameAnnouncer.svelte`) — `TableStore` only keeps `view`, so
	 * this is captured here rather than added to the store.
	 */
	let lastSteps = $state.raw<readonly Step[]>([]);

	table.onEvent('sync', (payload: SyncEvent) => {
		store.applySync(payload);
		lastSteps = payload.steps;
	});

	store.bind(table as unknown as TableActorHandle);

	$effect(() => {
		if (table.isConnected) {
			void store.resync();
		}
	});

	/* ---------------------------------------------------------------------- *
	 * Optimistic local preview.
	 *
	 * `applyOptimistic` is exact only on `OPTIMISTIC_FIELDS` and always clears
	 * `legal` (see $lib/euchre/optimistic.ts — reproducing the server's legal
	 * set client-side would be a second rulebook). It is shown ONLY until the
	 * version it was computed against is superseded by a real `sync`: stamping
	 * the pre-move `v` and comparing against the live store view on every read
	 * means a stale or wrong optimistic view can never survive past the next
	 * server truth, with no explicit "clear" needed on the happy path.
	 * ---------------------------------------------------------------------- */
	let optimistic = $state.raw<{ readonly forV: number; readonly view: PublicGameView } | null>(
		null
	);

	/**
	 * The last view we ever had, never revoked.
	 *
	 * This is a latch on purpose. The 3D table is mounted behind `{#if displayView}`,
	 * so a SINGLE frame where this returns `null` tears `EuchreTable3D` down and
	 * builds it again — and a fresh mount replays the whole deal-in choreography.
	 * On screen that is the hand appearing to be re-dealt, and because it is driven
	 * by a transient rather than by state, it recurs unpredictably as play goes on.
	 *
	 * Once a hand exists there is no legitimate reason to show *nothing*: a dropped
	 * or in-flight sync should leave the last frame on screen until the next one
	 * lands, exactly like every other realtime renderer.
	 */
	let lastGoodView = $state.raw<PublicGameView | null>(null);

	const displayView = $derived.by((): PublicGameView | null => {
		const live = store.view;
		if (!live) return lastGoodView; // hold the last frame, never blank the table
		return optimistic && optimistic.forV === live.v ? optimistic.view : live;
	});

	$effect(() => {
		const v = displayView;
		if (v) untrack(() => (lastGoodView = v));
	});

	/**
	 * The exact `(view, moveId)` pair of the human's last real decision, for
	 * `Hint.svelte`'s auto-retiring coach mode: it compares the move actually
	 * made against the suggestion it would have shown for that same `view`.
	 * Captured here — at the moment of commit, from the pre-move view already
	 * in scope — rather than inferred later from a view diff, so there is no
	 * ambiguity from the optimistic/animation layers about which move this was.
	 */
	let lastDecision = $state<{ view: PublicGameView; moveId: LegalMoveId } | null>(null);

	/** Reopens the first-run tour from `RulesPanel`'s "take the tour again" link. */
	let tourOpen = $state(false);

	/** Submits any legal move id, previewing its effect locally first. Used by the bid panel, card rack, and confirm button — so the whole table gets the same instant feedback from one place. */
	async function submitMove(moveId: LegalMoveId): Promise<void> {
		const view = displayView;
		if (!view || store.submitting || !table.isConnected) return;
		const entry = view.legal.find((m) => m.id === moveId);
		if (!entry) return; // Every caller here already filtered against this same `legal` set.
		lastDecision = { view, moveId };
		optimistic = { forV: view.v, view: applyOptimistic(view, entry.move) };
		const ok = await store.play(moveId);
		if (!ok) optimistic = null; // Rejected — store already resynced; show its truth, not our guess.
	}

	/**
	 * Plain-language reason a tapped card wasn't offered — reuses
	 * `whyIllegal`, the engine's own "learner-facing copy" (its doc comment
	 * names it as the `IllegalWhy` panel's source), so this is never a second
	 * legality decision: the server already decided by leaving the id out of
	 * `view.legal`, and everything here does is choose which true sentence to
	 * show about that fact.
	 */
	function illegalReason(view: PublicGameView, card: CardId): string {
		if (view.status !== 'active') return 'This game is finished.';
		if (view.turnSeat !== view.you) return "It's not your turn yet.";
		if (view.phase === 'dealer_discard') {
			return "It's not your turn to discard.";
		}
		if (view.phase !== 'trick_play') return 'Finish bidding before you play a card.';
		return (
			whyIllegal(view.hand, card, view.trick.ledSuit, view.trump) ??
			"That card can't be played right now."
		);
	}

	let cardSelection = $state<{ handNo: number; card: CardId } | null>(null);
	const selectedCard = $derived(
		cardSelection &&
			displayView &&
			cardSelection.handNo === displayView.handNo &&
			displayView.hand.includes(cardSelection.card)
			? cardSelection.card
			: null
	);
	const selectedMove = $derived(
		displayView?.legal.find(
			(entry) =>
				(entry.move.t === 'play' || entry.move.t === 'discard') && entry.move.card === selectedCard
		)
	);
	function selectCard(card: CardId): void {
		if (displayView) cardSelection = { handNo: displayView.handNo, card };
	}
	function cardLabel(card: CardId): string {
		return `${card.slice(0, -1).replace('T', '10')}${({ S: '♠', H: '♥', D: '♦', C: '♣' } as Record<string, string>)[card.slice(-1)]}`;
	}

	let illegalMessage = $state<string | null>(null);
	let illegalTimer: ReturnType<typeof setTimeout> | undefined;

	function handleCardIllegal(card: CardId): void {
		const view = displayView;
		if (!view) return;
		illegalMessage = illegalReason(view, card);
		clearTimeout(illegalTimer);
		illegalTimer = setTimeout(() => {
			illegalMessage = null;
		}, 3200);
	}
	onDestroy(() => clearTimeout(illegalTimer));
</script>

<svelte:head>
	<title>Euchre — play</title>
</svelte:head>

<main class="table-page">
	<Walkthrough view={displayView} bind:open={tourOpen} />
	<GameAnnouncer steps={lastSteps} view={displayView} />
	{#if !displayView}
		<div class="loading" role="status">
			<span aria-hidden="true">♣</span>
			<h1>{table.isConnected ? 'Your hand is on its way.' : 'A seat at the table.'}</h1>
			<p>{table.isConnected ? 'Dealing your cards…' : 'Connecting to your game…'}</p>
			<a href="/">Back to the club</a>{#if table.lastError}<p class="err">
					{String(table.lastError)}
				</p>{/if}
		</div>
	{:else}
		<EuchreTable3D view={displayView} />
		<header class="game-header">
			<ScoreBoard view={displayView} />
			<div class="table-tools">
				<SoundToggle />
				<RulesPanel view={displayView} onReplayTour={() => (tourOpen = true)} />
				<details class="table-menu">
					<summary aria-label="Table options">⋯</summary>
					<div class="menu-panel">
						<p class="menu-title">AT YOUR TABLE</p>
						<Hint view={displayView} {lastDecision} /><a href="/play"
							>Deal a new game <span aria-hidden="true">↗</span></a
						><a href="/">Back to the club</a>
						<details class="conversation">
							<summary>Table talk <span>{store.chat.length}</span></summary>
							<div class="conversation-lines">
								{#each store.chat as line (line.msgId)}<p>
										<strong>{SEAT_NAME[line.seat] ?? 'Table'}</strong>
										{line.text}
									</p>{:else}<p>The conversation starts with the first call.</p>{/each}
							</div>
						</details>
					</div>
				</details>
			</div>
		</header>
		{#if !table.isConnected}<p class="reconnecting" role="status">
				Reconnecting · your table is saved
			</p>{/if}
		<div class="seat-marker partner" class:current={displayView.turnSeat === 2}>
			<span class="seat-dot"></span>Partner <small>NORTH</small>
		</div>
		<div class="seat-marker west" class:current={displayView.turnSeat === 1}>
			<span class="seat-dot"></span>West
		</div>
		<div class="seat-marker east" class:current={displayView.turnSeat === 3}>
			<span class="seat-dot"></span>East
		</div>
		{#if displayView.hand.length > 0}
			<div class="hand-rack">
				<CardRack
					view={displayView}
					disabled={store.submitting || !table.isConnected}
					selected={selectedCard}
					onSelect={selectCard}
					onPlay={submitMove}
					onIllegal={handleCardIllegal}
				/>
			</div>
		{/if}

		<footer class="action-dock">
			<div class="dock-inner">
				<div class="dock-status">
					<TableStatusBar view={displayView} {illegalMessage} /><TrickView view={displayView} />
				</div>
				{#if store.error}<p class="err" role="alert">{store.error}</p>{/if}
				<BidPanel
					view={displayView}
					disabled={store.submitting || !table.isConnected}
					onPlay={submitMove}
				/>
				{#if displayView.status === 'complete'}<div class="quiet-action">
						<p>
							{displayView.winnerTeam === 0
								? 'A hand well played. A match well won.'
								: 'There’s always another good hand.'}
						</p>
						<a href="/play">Play again <span aria-hidden="true">↗</span></a>
					</div>
				{:else if store.submitting}
					<div class="waiting">
						<p>Making your move…</p>
						<span>One moment at the table.</span>
					</div>
				{:else if displayView.turnSeat !== displayView.you || displayView.phase === 'hand_score' || displayView.phase === 'trick_resolve'}<div
						class="waiting"
					>
						<p>
							{displayView.phase === 'hand_score'
								? 'Counting the hand…'
								: displayView.phase === 'trick_resolve'
									? 'The next trick is coming.'
									: `${displayView.turnSeat === null ? 'The table' : SEAT_NAME[displayView.turnSeat]} is making a move.`}
						</p>
						<span>Settle in. Your turn is coming.</span>
					</div>
				{:else if displayView.phase === 'trick_play' || displayView.phase === 'dealer_discard'}
					<div class="card-action">
						<p>
							{selectedCard
								? `${cardLabel(selectedCard)} selected`
								: displayView.phase === 'dealer_discard'
									? 'Select a card to discard.'
									: 'Select a card from your hand.'}
						</p>
						<button
							type="button"
							class="confirm-card"
							disabled={!selectedMove || store.submitting || !table.isConnected}
							onclick={() => selectedMove && submitMove(selectedMove.id)}
						>
							{displayView.phase === 'dealer_discard' ? 'Discard' : 'Play'}{selectedCard
								? ` ${cardLabel(selectedCard)}`
								: ' card'} <span aria-hidden="true">↗</span>
						</button>
					</div>
				{/if}
			</div>
		</footer>
	{/if}
</main>

<style>
	.table-page {
		position: relative;
		min-height: 100dvh;
		overflow: hidden;
		background: radial-gradient(ellipse at 50% 46%, #264336 0%, #172f25 58%, #10241d 100%);
		color: #233e32;
		font-family: var(--font-sans);
	}
	.game-header {
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
		z-index: 5;
		height: 88px;
		display: flex;
		align-items: center;
		gap: 34px;
		padding: 0 36px;
		background: #f4f3e9;
		border-bottom: 1px solid #dce1d4;
	}
	.table-tools {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-shrink: 0;
	}
	.table-tools :global(.sound-toggle),
	.table-tools :global(.trigger),
	.table-menu > summary {
		display: flex;
		align-items: center;
		justify-content: center;
		min-width: 36px;
		min-height: 36px;
		width: 36px;
		height: 36px;
		background: transparent;
		border: 1px solid #dce1d4;
		border-radius: 50%;
		box-shadow: none;
		color: #53694a;
		font: 500 16px var(--font-sans);
		cursor: pointer;
	}
	.table-tools :global(.sound-toggle:hover),
	.table-tools :global(.trigger:hover),
	.table-menu > summary:hover {
		background: #e5e9dc;
	}
	.table-tools :global(svg) {
		width: 16px;
		height: 16px;
	}
	summary {
		list-style: none;
	}
	summary::-webkit-details-marker {
		display: none;
	}
	.table-menu {
		position: relative;
	}
	.table-menu > summary {
		font-size: 24px;
		padding-bottom: 8px;
	}
	.menu-panel {
		position: absolute;
		right: 0;
		top: 48px;
		width: 270px;
		padding: 20px;
		border: 1px solid #d8dfcb;
		border-radius: 12px;
		background: #fbfaf3;
		box-shadow: 0 18px 50px #09271920;
		max-height: calc(100dvh - 120px);
		overflow: auto;
	}
	.menu-title {
		font-size: 12px;
		letter-spacing: 0.13em;
		color: #7a866d;
		margin: 0 0 10px;
	}
	.menu-panel > a {
		display: flex;
		justify-content: space-between;
		font-size: 12px;
		padding: 12px 0;
		border-top: 1px solid #e3e7da;
		text-decoration: none;
		color: #324a29;
	}
	.menu-panel :global(.toggle) {
		color: #52624c;
		font-size: 12px;
		padding: 0;
	}
	.menu-panel :global(.suggestion) {
		color: #52624c;
		background: #edf0e3;
		border-color: #d5ddc5;
		font-size: 12px;
		margin-bottom: 12px;
	}
	.menu-panel :global(.suggestion strong) {
		color: #2c4d26;
	}
	.conversation > summary {
		display: flex;
		justify-content: space-between;
		padding-top: 12px;
		border-top: 1px solid #e3e7da;
		font-size: 12px;
		cursor: pointer;
	}
	.conversation > summary span {
		color: #8b947e;
	}
	.conversation-lines {
		max-height: 220px;
		overflow: auto;
	}
	.conversation-lines p {
		font-size: 13px;
		line-height: 1.6;
		color: #78816f;
	}
	.conversation-lines strong {
		color: #3f5635;
		font-weight: 550;
	}
	.seat-marker {
		position: absolute;
		z-index: 2;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 7px;
		font: 550 14px var(--font-sans);
		color: #d5e0d1;
		pointer-events: none;
		letter-spacing: 0.01em;
	}
	.seat-marker small {
		font-size: 13px;
		letter-spacing: 0.11em;
		color: #8da28b;
		margin-left: 4px;
	}
	.seat-dot {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: #8da28b;
	}
	.seat-marker.current {
		color: #f1f5e7;
	}
	.current .seat-dot {
		background: #d4ed9b;
		box-shadow: 0 0 0 3px #d4ed9b15;
	}
	.partner {
		top: 107px;
		left: 50%;
		transform: translateX(-50%);
	}
	.west {
		top: calc(50% - 26px);
		left: max(24px, calc(50% - (100dvh - 228px) * 0.56 - 36px));
	}
	.east {
		top: calc(50% - 26px);
		right: max(24px, calc(50% - (100dvh - 228px) * 0.56 - 36px));
	}
	.action-dock {
		position: absolute;
		bottom: 0;
		left: 0;
		right: 0;
		min-height: 140px;
		z-index: 3;
		background: #f4f3e9;
		border-top: 1px solid #dce1d4;
		padding: 12px 32px 15px;
	}
	.dock-inner {
		width: min(100%, 680px);
		margin: auto;
	}
	.dock-status {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 13px;
	}
	.waiting {
		text-align: center;
		padding-top: 4px;
	}
	.waiting p,
	.quiet-action p {
		margin: 0;
		color: #384f30;
		font: 500 19px var(--font-serif);
	}
	.waiting > span {
		display: block;
		font-size: 13px;
		color: #829075;
		margin-top: 7px;
	}
	.quiet-action {
		display: flex;
		justify-content: center;
		gap: 25px;
		align-items: center;
	}
	.quiet-action a {
		display: flex;
		align-items: center;
		gap: 24px;
		text-decoration: none;
		font-size: 12px;
		font-weight: 600;
		background: #d4ed9b;
		border: 1px solid #ccdfa9;
		border-radius: 7px;
		padding: 13px 18px;
	}
	.err {
		text-align: center;
		color: #a24d37;
		font-size: 12px;
		margin: 0 0 8px;
	}
	.reconnecting {
		position: absolute;
		top: 98px;
		left: 50%;
		transform: translateX(-50%);
		z-index: 6;
		background: #f5ecd7;
		border: 1px solid #ddc798;
		border-radius: 8px;
		padding: 10px 16px;
		font-size: 13px;
		white-space: nowrap;
	}
	.loading {
		min-height: 100dvh;
		display: flex;
		align-items: center;
		justify-content: center;
		flex-direction: column;
		gap: 16px;
		background: #f4f3e9;
	}
	.loading > span {
		font-size: 46px;
	}
	.loading h1 {
		font: 500 32px var(--font-serif);
		letter-spacing: -0.04em;
		margin: 0;
	}
	.loading p {
		font-size: 13px;
		color: #76816b;
		margin: 0;
	}
	.loading a {
		font-size: 12px;
		color: #46653b;
		text-underline-offset: 4px;
	}
	@media (max-width: 700px) {
		.game-header {
			height: 90px;
			padding: 17px 16px;
			align-items: flex-start;
			gap: 10px;
		}
		.table-tools {
			margin-left: auto;
			gap: 4px;
		}
		.table-tools :global(.sound-toggle),
		.table-tools :global(.trigger),
		.table-menu > summary {
			width: 30px;
			height: 30px;
			min-width: 30px;
			min-height: 30px;
		}
		.table-tools :global(svg) {
			width: 14px;
			height: 14px;
		}
		.partner {
			top: calc(50% - 26px - 46vw);
			font-size: 12px;
		}
		.partner small {
			display: none;
		}
		.west {
			left: 13px;
			font-size: 12px;
		}
		.east {
			right: 13px;
			font-size: 12px;
		}
		.action-dock {
			min-height: 150px;
			padding: 13px 18px 20px;
		}
		.dock-status {
			margin-bottom: 15px;
		}
		.quiet-action {
			gap: 12px;
		}
		.quiet-action p {
			font-size: 15px;
		}
		.waiting p {
			font-size: 18px;
		}
		.waiting > span {
			font-size: 12px;
		}
	}
	@media (min-width: 701px) and (max-height: 780px) {
		.west,
		.east {
			top: calc(50% - 51px);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.table-page :global(*) {
			animation-duration: 0.001ms !important;
			transition-duration: 0.001ms !important;
		}
	}
	.hand-rack {
		position: absolute;
		z-index: 2;
		left: 50%;
		bottom: 160px;
		transform: translateX(-50%);
		width: min(948px, calc(100% - 20px));
	}
	.card-action {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 24px;
		padding-top: 9px;
	}
	.card-action p {
		font-size: 15px;
		margin: 0;
		color: #52624c;
	}
	.confirm-card {
		min-height: 48px;
		min-width: 142px;
		padding: 12px 20px;
		background: #d4ed9b;
		border: 1px solid #c2d993;
		border-radius: 8px;
		font: 600 15px var(--font-sans);
		color: #203e2c;
		cursor: pointer;
	}
	.confirm-card span {
		margin-left: 12px;
	}
	.confirm-card:disabled {
		opacity: 0.45;
		cursor: default;
	}
	.confirm-card:focus-visible {
		outline: 3px solid #456833;
		outline-offset: 3px;
	}
	@media (max-width: 700px) {
		.hand-rack {
			bottom: 172px;
		}
		.card-action {
			gap: 12px;
		}
		.card-action p {
			font-size: 13px;
		}
		.confirm-card {
			min-width: 126px;
			font-size: 14px;
			padding: 10px 14px;
		}
	}
</style>
