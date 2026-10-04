<script lang="ts">
	import { onDestroy, untrack } from 'svelte';
	import { createGameRivet } from '#lib/client/rivet.ts';
	import { TableStore, type TableActorHandle } from '#lib/game/table.svelte.ts';
	import { applyOptimistic, whyIllegal } from '#lib/euchre/index.ts';
	import TableStage, { type CardOrigin } from '#lib/ui/TableStage.svelte';
	import { cardName } from '#lib/euchre/index.ts';
	import { turnInstruction } from '#lib/ui/table-presentation.ts';
	import BidPanel from '#lib/ui/BidPanel.svelte';
	import CardRack from '#lib/ui/CardRack.svelte';
	import Hint from '#lib/ui/Hint.svelte';
	import RulesPanel from '#lib/ui/RulesPanel.svelte';
	import GameAnnouncer from '#lib/ui/onboarding/GameAnnouncer.svelte';
	import Walkthrough from '#lib/ui/onboarding/Walkthrough.svelte';
	import ScoreBoard from '#lib/ui/ScoreBoard.svelte';
	import SoundToggle from '#lib/ui/sound/SoundToggle.svelte';

	/** Compass names for the spoken log. Seat 0 is the player. */
	const SEAT_NAME = ['You', 'Left opponent', 'Your partner', 'Right opponent'] as const;
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
	let flightOrigin = $state.raw<CardOrigin | null>(null);
	let reviewBusy = $state(false);
	let reviewError = $state<string | null>(null);
	let pageRoot = $state<HTMLElement>();

	/** Submits any legal move id, previewing its effect locally first. Used by the bid panel, card rack, and confirm button — so the whole table gets the same instant feedback from one place. */
	async function submitMove(moveId: LegalMoveId): Promise<void> {
		const view = displayView;
		if (!view || store.submitting || !table.isConnected) return;
		const entry = view.legal.find((m) => m.id === moveId);
		if (!entry) return; // Every caller here already filtered against this same `legal` set.
		lastDecision = { view, moveId };
		if (entry.move.t === 'play') {
			const target = pageRoot?.querySelector<HTMLElement>(
				`[data-card-id="${entry.move.card}"] .artwork`
			);
			flightOrigin = target
				? { card: entry.move.card, rect: target.getBoundingClientRect(), at: performance.now() }
				: null;
		}
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
	async function reviewTrick(
		request: { action: 'set'; enabled: boolean } | { action: 'continue'; turnId: string }
	): Promise<boolean> {
		if (!table.isConnected || reviewBusy) return false;
		reviewBusy = true;
		reviewError = null;
		try {
			const ack = await table.reviewTrick(request);
			if (!ack?.queued) throw new Error('The table did not confirm. Please try again.');
			return true;
		} catch {
			reviewError = 'Could not update the table. Please try again.';
			return false;
		} finally {
			reviewBusy = false;
		}
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

<svelte:head><title>Euchre — your table</title></svelte:head>
<main class="table-page" bind:this={pageRoot}>
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
		<header class="game-header">
			<ScoreBoard view={displayView} />
			<div class="table-tools">
				<RulesPanel view={displayView} onReplayTour={() => (tourOpen = true)} />
				<details class="table-menu">
					<summary>Settings <span aria-hidden="true">⌄</span></summary>
					<div class="menu-panel">
						<div class="sound-setting"><span>Table sounds</span><SoundToggle /></div>
						<label class="review-setting"
							><input
								type="checkbox"
								checked={displayView.reviewTricks ?? false}
								disabled={reviewBusy || !table.isConnected}
								onchange={(event) =>
									void reviewTrick({ action: 'set', enabled: event.currentTarget.checked })}
							/><span
								>Wait for me after each trick<small>Review the cards, then press Continue.</small
								></span
							></label
						>
						<Hint view={displayView} {lastDecision} />
						<a href="/play">Start a new game</a><a href="/">Back to the club</a>
						<details class="conversation">
							<summary>Table talk</summary>
							<div>
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
		<div class="table-layout">
			{#if !table.isConnected}<p class="notice" role="status">
					Reconnecting… Your table is saved. Controls will return when you’re connected.
				</p>{/if}
			{#if reviewError}<p class="err" role="alert">{reviewError}</p>{/if}
			<TableStage
				view={displayView}
				origin={flightOrigin}
				connected={table.isConnected}
				onContinue={(turnId) =>
					displayView ? reviewTrick({ action: 'continue', turnId }) : Promise.resolve(false)}
			/>
			<section
				class="hand-area"
				class:bidding={displayView.phase.startsWith('bid') || displayView.phase === 'cutting'}
				aria-label="Your hand and controls"
			>
				<div class="hand-heading">
					<div>
						<span class="eyebrow">Your hand</span>
						<h1>
							{displayView.status !== 'active'
								? 'A good game, well played.'
								: displayView.turnSeat === displayView.you && displayView.phase !== 'trick_resolve'
									? 'Your turn.'
									: 'Make yourself at home.'}
						</h1>
					</div>
					{#if displayView.status === 'active' && (displayView.phase === 'trick_play' || displayView.phase === 'dealer_discard')}
						<button
							type="button"
							class="confirm-card"
							disabled={!selectedMove || store.submitting || !table.isConnected}
							onclick={() => selectedMove && submitMove(selectedMove.id)}
							>{store.submitting
								? 'Playing…'
								: displayView.phase === 'dealer_discard'
									? 'Discard'
									: 'Play'}
							{selectedCard ? cardName(selectedCard) : 'selected card'}
							<span aria-hidden="true">→</span></button
						>
					{:else if displayView.status !== 'active'}<a class="confirm-card" href="/play"
							>Play again <span aria-hidden="true">→</span></a
						>{/if}
				</div>
				<p class="instruction" role="status">{illegalMessage ?? turnInstruction(displayView)}</p>
				{#if store.error}<p class="err" role="alert">{store.error}</p>{/if}
				<CardRack
					view={displayView}
					disabled={store.submitting || !table.isConnected}
					selected={selectedCard}
					onSelect={selectCard}
					onPlay={submitMove}
					onIllegal={handleCardIllegal}
				/>
				<div class="bid-controls">
					<BidPanel
						view={displayView}
						disabled={store.submitting || !table.isConnected}
						onPlay={submitMove}
					/>
				</div>
			</section>
			<footer class="table-footer">
				<span>A little friendly competition.</span><span>Select a card. Then press Play.</span>
			</footer>
		</div>
	{/if}
</main>

<style>
	.table-page {
		min-height: 100dvh;
		background: #f4f3e9;
		color: #233e32;
		font-family: var(--font-sans);
	}
	.game-header {
		position: relative;
		z-index: 5;
		display: flex;
		align-items: center;
		gap: 24px;
		min-height: 92px;
		padding: 18px max(28px, calc((100vw - 1190px) / 2));
		border-bottom: 1px solid #dce1d4;
	}
	.table-tools {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-shrink: 0;
	}
	.table-menu {
		position: relative;
	}
	.table-menu > summary {
		display: flex;
		gap: 14px;
		align-items: center;
		min-height: 52px;
		padding: 10px 16px;
		border: 1px solid #cad3c1;
		border-radius: 8px;
		font-size: 17px;
		cursor: pointer;
		list-style: none;
	}
	.table-menu > summary::-webkit-details-marker {
		display: none;
	}
	.menu-panel {
		position: absolute;
		right: 0;
		top: calc(100% + 12px);
		width: min(350px, calc(100vw - 32px));
		padding: 20px;
		background: #fffdf6;
		border: 1px solid #cad3c1;
		border-radius: 12px;
		box-shadow: 0 18px 48px #203c2d30;
		font-size: 17px;
		max-height: 70dvh;
		overflow: auto;
	}
	.menu-panel > a {
		display: block;
		padding: 14px 0;
		border-top: 1px solid #dce1d4;
		text-decoration: none;
	}
	.sound-setting {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 14px;
	}
	.review-setting {
		display: flex;
		align-items: flex-start;
		gap: 12px;
		padding: 16px 0;
		border-block: 1px solid #dce1d4;
		margin-bottom: 12px;
		cursor: pointer;
	}
	.review-setting input {
		width: 24px;
		height: 24px;
		margin-top: 3px;
		flex-shrink: 0;
		accent-color: #285641;
	}
	.review-setting small {
		display: block;
		font-size: 14px;
		line-height: 1.5;
		margin-top: 5px;
		color: #586b51;
	}
	.conversation summary {
		cursor: pointer;
		padding: 12px 0;
	}
	.conversation p {
		margin: 12px 0;
		font-size: 15px;
		line-height: 1.5;
	}
	.table-layout {
		max-width: 1190px;
		margin: auto;
		padding: 24px 28px 0;
	}
	.hand-area {
		padding: 26px 28px 6px;
	}
	.hand-heading {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 24px;
	}
	.eyebrow {
		display: block;
		font-size: 15px;
		color: #55684e;
	}
	h1 {
		font: 500 clamp(26px, 3vw, 34px)/1.2 var(--font-serif);
		margin: 4px 0 0;
		letter-spacing: -0.02em;
	}
	.instruction {
		margin: 10px 0 14px;
		font-size: 18px;
		line-height: 1.5;
		min-height: 27px;
	}
	.confirm-card {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 18px;
		min-width: 240px;
		min-height: 56px;
		padding: 12px 24px;
		background: #285641;
		border: 1px solid #285641;
		border-radius: 8px;
		color: #fffdf6;
		font-size: 19px;
		font-weight: 600;
		text-decoration: none;
		cursor: pointer;
	}
	.confirm-card:disabled {
		color: #52624b;
		background: #dce3d3;
		border-color: #bdc9b2;
		cursor: default;
	}
	.confirm-card:hover:not(:disabled) {
		background: #1a4431;
	}
	.bid-controls:has(:global(.bids)) {
		margin: 20px auto 12px;
	}
	.table-footer {
		display: flex;
		justify-content: space-between;
		gap: 12px;
		padding: 18px 0;
		border-top: 1px solid #d4dccb;
		color: #53664c;
		font-size: 14px;
	}
	.notice,
	.err {
		padding: 14px 18px;
		border: 1px solid #d5b68c;
		background: #fff1d9;
		border-radius: 8px;
		font-size: 17px;
		line-height: 1.5;
	}
	.loading {
		min-height: 100dvh;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		text-align: center;
		gap: 20px;
		padding: 32px;
	}
	.loading > span {
		font-size: 54px;
	}
	.loading p {
		font-size: 19px;
	}
	.loading a {
		padding: 16px;
		font-size: 18px;
	}
	@media (min-width: 701px) and (max-height: 850px) {
		.game-header {
			min-height: 70px;
			padding-block: 10px;
		}
		.table-layout {
			padding-top: 12px;
		}
		.hand-area {
			padding-top: 14px;
		}
		.hand-heading h1 {
			font-size: 26px;
		}
		.hand-heading > div {
			display: flex;
			align-items: baseline;
			gap: 14px;
		}
		.instruction {
			margin: 6px 0 4px;
			font-size: 17px;
		}
		.table-footer {
			display: none;
		}
		.bid-controls:has(:global(.bids)) {
			margin: 4px auto;
		}
		.hand-area :global(.bid-heading) {
			margin-bottom: 4px;
		}
	}
	@media (max-width: 900px) {
		.game-header {
			flex-wrap: wrap;
			gap: 16px;
			padding: 18px 24px;
		}
		.table-tools {
			margin-left: auto;
		}
		.table-layout {
			padding: 20px 20px 0;
		}
		.hand-area {
			padding-inline: 10px;
		}
	}
	@media (max-width: 600px) {
		.game-header {
			padding: 14px 16px;
		}
		.table-tools {
			width: 100%;
			justify-content: space-between;
		}
		.table-layout {
			padding: 14px 12px 0;
		}
		.hand-area {
			padding: 20px 4px 10px;
		}
		.hand-heading {
			align-items: flex-start;
			flex-direction: column;
			gap: 12px;
		}
		.confirm-card {
			width: 100%;
			font-size: 17px;
			min-width: 0;
		}
		.instruction {
			font-size: 17px;
			margin-block: 12px;
		}
		.table-footer {
			flex-wrap: wrap;
			font-size: 14px;
		}
		.menu-panel {
			right: 0;
		}
	}
	/* The game uses the visible viewport. Menus and help scroll independently. */
	.table-page {
		height: 100dvh;
		min-height: 0;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.game-header {
		flex-shrink: 0;
		min-height: 72px;
		padding-block: 10px;
	}
	.table-layout {
		flex: 1;
		min-height: 0;
		width: 100%;
		display: grid;
		grid-template-rows: minmax(0, 1.06fr) minmax(0, 0.94fr);
		gap: 12px;
		padding: 14px 28px 10px;
	}
	.hand-area {
		min-height: 0;
		padding: 0 28px;
		display: grid;
		grid-template-rows: auto auto minmax(0, 1fr) auto;
	}
	.hand-heading h1 {
		font-size: 27px;
	}
	.hand-heading > div {
		display: flex;
		align-items: baseline;
		gap: 12px;
	}
	.instruction {
		margin: 4px 0 5px;
		font-size: 17px;
		line-height: 1.4;
		min-height: 0;
	}
	.confirm-card {
		min-height: 52px;
		padding: 10px 18px;
	}
	.table-footer {
		display: none;
	}
	.bid-controls:has(:global(.bids)) {
		margin: 4px 0 0;
	}
	.table-layout > .notice,
	.table-layout > .err {
		position: absolute;
		top: 80px;
		left: 24px;
		right: 24px;
		z-index: 6;
	}
	.hand-area > .err {
		position: absolute;
		bottom: 20px;
		left: 24px;
		right: 24px;
		z-index: 6;
	}
	@media (max-width: 700px) {
		.game-header {
			padding: 8px 14px;
			gap: 6px;
		}
		.table-tools {
			width: 100%;
			gap: 8px;
			justify-content: space-between;
		}
		.table-tools :global(.trigger),
		.table-menu > summary {
			min-height: 44px;
			padding: 7px 12px;
			font-size: 15px;
		}
		.table-layout {
			grid-template-rows: minmax(0, 0.92fr) minmax(0, 1.08fr);
			gap: 8px;
			padding: 8px 10px;
		}
		.hand-area {
			padding: 0 4px;
		}
		.hand-heading {
			flex-direction: row;
			align-items: center;
			gap: 10px;
		}
		.hand-heading h1 {
			display: none;
		}
		.hand-heading .eyebrow {
			color: #233e32;
			font-size: 18px;
			font-weight: 650;
			white-space: nowrap;
		}
		.confirm-card {
			width: auto;
			min-width: 0;
			min-height: 48px;
			font-size: 15px;
			gap: 8px;
			padding: 9px 12px;
		}
		.instruction {
			font-size: 15px;
			margin: 5px 0;
			line-height: 1.3;
		}
		.hand-area.bidding .hand-heading {
			display: none;
		}
	}
	@media (min-width: 701px) and (max-height: 500px) {
		.game-header {
			min-height: 60px;
		}
		.table-layout {
			grid-template-columns: 1fr 1fr;
			grid-template-rows: minmax(0, 1fr);
		}
		.hand-area {
			padding: 0 6px;
		}
		.hand-heading h1 {
			display: none;
		}
		.confirm-card {
			min-width: 0;
			font-size: 16px;
		}
	}
	@media (max-width: 700px) and (max-height: 750px) {
		.table-layout {
			grid-template-rows: minmax(0, 1.3fr) minmax(0, 0.7fr);
		}
		.hand-area.bidding .bid-controls {
			margin: 0;
		}
		.hand-area.bidding {
			grid-template-rows: auto auto minmax(0, 1fr) auto;
		}
	}
	@media (max-width: 700px) and (max-height: 750px) {
		.table-layout:has(.hand-area.bidding) {
			grid-template-rows: minmax(0, 1fr) minmax(0, 1fr);
		}
	}
</style>
