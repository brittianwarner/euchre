<script lang="ts">
	import { onDestroy, onMount, untrack, type Component } from 'svelte';
	import { createGameRivet } from '#lib/client/rivet.ts';
	import { warmCardArt } from '#lib/ui/card-art.ts';
	import { TableStore, type TableActorHandle } from '#lib/game/table.svelte.ts';
	import { applyOptimistic, whyIllegal } from '#lib/euchre/index.ts';
	import HandReview from '#lib/ui/HandReview.svelte';
	import GameFeedback from '#lib/ui/GameFeedback.svelte';
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

	let FeltLayer = $state<Component | null>(null);
	onMount(() => {
		warmCardArt();
		let active = true;
		void import('#lib/three/TableFelt3D.svelte')
			.then((module) => {
				if (active) FeltLayer = module.default;
			})
			.catch(() => {
				/* CSS felt remains usable without WebGL. */
			});
		return () => {
			active = false;
		};
	});

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
		getParams,
		getActorId
	} = createGameRivet(untrack(() => data.gameId));
	const store = new TableStore();

	let actorId = $state<string>();
	let connectionError = $state<string>();
	onMount(() => {
		let mounted = true;
		void getActorId()
			.then((id) => {
				if (mounted) actorId = id;
			})
			.catch((error: unknown) => {
				if (mounted)
					connectionError = error instanceof Error ? error.message : 'Unable to open this table.';
			});
		return () => {
			mounted = false;
		};
	});

	const table = useActor(() => ({
		name: 'euchreTable' as const,
		actorId,
		enabled: Boolean(actorId),
		noCreate: true,
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
	let pageRoot: HTMLElement | undefined;
	function attachElement(node: HTMLElement) {
		pageRoot = node;
		return () => {
			pageRoot = undefined;
		};
	}
	let settingsOpen = $state(false);
	let settingsPinned = $state(false);
	let menuPosition = $state.raw<{ x: number; y: number } | null>(null);
	let menuNode: HTMLElement | undefined;
	function attachMenu(node: HTMLElement) {
		menuNode = node;
		return () => {
			menuNode = undefined;
		};
	}
	let dragStart: { x: number; y: number; left: number; top: number } | null = null;
	function pinSettings() {
		const rect = menuNode?.getBoundingClientRect();
		settingsPinned = !settingsPinned;
		menuPosition =
			settingsPinned && rect ? { x: Math.max(8, rect.left), y: Math.max(8, rect.top) } : null;
	}
	function startSettingsDrag(event: PointerEvent) {
		if (!settingsPinned || (event.target as HTMLElement).closest('button')) return;
		const rect = menuNode?.getBoundingClientRect();
		if (!rect) return;
		dragStart = { x: event.clientX, y: event.clientY, left: rect.left, top: rect.top };
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function positionSettings(x: number, y: number) {
		menuPosition = {
			x: Math.max(8, Math.min(window.innerWidth - (menuNode?.offsetWidth ?? 350) - 8, x)),
			y: Math.max(8, Math.min(window.innerHeight - (menuNode?.offsetHeight ?? 80) - 8, y))
		};
	}
	function dragSettings(event: PointerEvent) {
		if (dragStart)
			positionSettings(
				dragStart.left + event.clientX - dragStart.x,
				dragStart.top + event.clientY - dragStart.y
			);
	}

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
		if (view.phase === 'trick_resolve' && view.reviewTricks)
			return 'Press Continue to finish reviewing this trick before playing.';
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

<svelte:window
	onresize={() => {
		if (settingsPinned && menuPosition) positionSettings(menuPosition.x, menuPosition.y);
	}}
/>
<svelte:head><title>Euchre — your table</title></svelte:head>
<main class="table-page" {@attach attachElement}>
	<Walkthrough view={displayView} bind:open={tourOpen} />
	<GameAnnouncer steps={lastSteps} view={displayView} />
	{#if !displayView}
		<div class="loading" role="status">
			<span aria-hidden="true">♣</span>
			<h1>{table.isConnected ? 'Your hand is on its way.' : 'A seat at the table.'}</h1>
			<p>{table.isConnected ? 'Dealing your cards…' : 'Connecting to your game…'}</p>
			<button type="button" onclick={() => window.location.reload()}>Reconnect to this game</button
			><a href="/games">Your saved games</a>{#if table.lastError || connectionError}<p class="err">
					{connectionError ??
						'We couldn’t open this table. Try reconnecting, or open Your saved games.'}
				</p>{/if}
		</div>
	{:else}
		<GameFeedback view={displayView} steps={lastSteps} />
		<header class="game-header">
			<ScoreBoard view={displayView} />
			<div class="table-tools">
				<RulesPanel view={displayView} onReplayTour={() => (tourOpen = true)} />
				<div class="table-menu">
					<button
						type="button"
						class="settings-trigger"
						aria-expanded={settingsOpen}
						onclick={() => (settingsOpen = !settingsOpen)}
						>Settings <span aria-hidden="true">⌄</span></button
					>
					{#if settingsOpen}<aside
							class="menu-panel"
							class:pinned={settingsPinned}
							{@attach attachMenu}
							style:left={menuPosition ? `${menuPosition.x}px` : undefined}
							style:top={menuPosition ? `${menuPosition.y}px` : undefined}
							aria-label="Table settings"
						>
							<div
								class="settings-handle"
								role="toolbar"
								tabindex="0"
								aria-label="Move settings"
								onkeydown={(event) => {
									if (
										settingsPinned &&
										menuPosition &&
										['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)
									) {
										event.preventDefault();
										positionSettings(
											menuPosition.x +
												(event.key === 'ArrowLeft' ? -16 : event.key === 'ArrowRight' ? 16 : 0),
											menuPosition.y +
												(event.key === 'ArrowUp' ? -16 : event.key === 'ArrowDown' ? 16 : 0)
										);
									}
								}}
								onpointerdown={startSettingsDrag}
								onpointermove={dragSettings}
								onpointerup={() => (dragStart = null)}
								onpointercancel={() => (dragStart = null)}
							>
								<strong>Table settings</strong><button type="button" onclick={pinSettings}
									>{settingsPinned ? 'Unpin' : 'Pin'}</button
								><button
									type="button"
									aria-label="Close settings"
									onclick={() => (settingsOpen = false)}>×</button
								>
							</div>
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
							<HandReview view={displayView} load={(handNo) => table.getHandReview(handNo)} />
							<Hint view={displayView} {lastDecision} />
							<a href="/new">Start a new game</a><a href="/">Back to home</a>
							<a href="/games">Your saved games</a>
							<details class="conversation">
								<summary>Table talk</summary>
								<div>
									{#each store.chat as line (line.msgId)}<p>
											<strong>{SEAT_NAME[line.seat] ?? 'Table'}</strong>
											{line.text}
										</p>{:else}<p>The conversation starts with the first call.</p>{/each}
								</div>
							</details>
						</aside>{/if}
				</div>
			</div>
		</header>
		<div class="table-layout">
			{#if FeltLayer}<FeltLayer />{/if}
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
								: displayView.phase === 'trick_resolve' && displayView.reviewTricks
									? 'Review the trick.'
									: displayView.turnSeat === displayView.you &&
										  displayView.phase !== 'trick_resolve'
										? 'Your turn.'
										: 'Make yourself at home.'}
						</h1>
					</div>
					{#if displayView.status === 'active' && (displayView.phase === 'trick_play' || displayView.phase === 'trick_resolve' || displayView.phase === 'dealer_discard')}
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
					{:else if displayView.status !== 'active'}<a class="confirm-card" href="/new"
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
	.settings-trigger {
		font-family: inherit;
		background: transparent;
		color: inherit;
	}
	.settings-handle {
		display: flex;
		align-items: center;
		gap: 8px;
		padding-bottom: 12px;
	}
	.settings-handle strong {
		margin-right: auto;
	}
	.settings-handle button {
		min-height: 44px;
		padding: 8px;
		border: 1px solid #bdc9ae;
		border-radius: 6px;
		background: #fffdf6;
		color: #233e32;
		cursor: pointer;
	}
	.menu-panel.pinned {
		position: fixed;
		right: auto;
		z-index: 20;
		max-height: calc(100dvh - 100px);
		overflow-y: auto;
	}
	.pinned .settings-handle {
		cursor: move;
		touch-action: none;
	}
	.loading button {
		min-height: 48px;
		padding: 12px 20px;
		border-radius: 8px;
		border: 1px solid #bdc9ae;
		background: #285641;
		color: white;
		font: 600 18px var(--font-sans);
		cursor: pointer;
	}

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
	.settings-trigger {
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
	.settings-trigger::-webkit-details-marker {
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

	/* One full-width table. Help and settings scroll independently. */
	.table-page {
		height: 100dvh;
		min-height: 0;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.game-header {
		flex-shrink: 0;
		min-height: 64px;
		padding: 8px 24px;
		background: #f4f3e9;
		color: #233e32;
	}
	.table-layout {
		position: relative;
		isolation: isolate;
		flex: 1;
		min-height: 0;
		width: 100%;
		max-width: none;
		margin: 0;
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		grid-template-rows: minmax(0, 1fr) clamp(280px, 46dvh, 420px);
		gap: 8px;
		padding: 8px 24px max(8px, env(safe-area-inset-bottom));
		background: #17452f;
		color: #fff8e8;
	}
	.table-layout > :global(.felt),
	.hand-area {
		position: relative;
		z-index: 1;
		min-height: 0;
	}
	.hand-area {
		width: 100%;
		padding: 8px 24px 0;
		display: grid;
		grid-template-rows: auto auto minmax(0, 1fr) auto;
		border-top: 1px solid #e2dbbd26;
	}
	.hand-heading {
		grid-row: 1;
		flex-direction: row;
		align-items: center;
		gap: 12px;
	}
	.hand-heading > div {
		display: flex;
		align-items: baseline;
		gap: 14px;
	}
	.hand-heading h1 {
		font-size: 25px;
		margin: 0;
	}
	.hand-heading .eyebrow {
		color: #e9e9d5;
		font-size: 16px;
	}
	.instruction {
		grid-row: 2;
		font-size: 17px;
		line-height: 1.3;
		margin: 4px 0;
		min-height: 0;
	}
	.hand-area :global(.card-rack) {
		grid-row: 3;
		max-width: 980px;
	}
	.hand-area :global(.card-label) {
		color: #f7f1df;
	}
	.hand-area :global(.card-label small) {
		color: #ddd5bd;
	}
	.hand-area :global(button.selected .artwork) {
		outline-color: #f5d98e;
	}
	.hand-area :global(button:focus-visible) {
		outline-color: #f5d98e;
	}
	.confirm-card {
		min-width: 0;
		min-height: 48px;
		font-size: 17px;
		padding: 8px 18px;
		background: #f5e8c3;
		color: #233e32;
		border-color: #ead8aa;
	}
	.confirm-card:hover:not(:disabled) {
		background: #fff0c7;
	}
	.confirm-card:disabled {
		background: #e8eadb;
		color: #6a7466;
		border-color: #d5ddc7;
	}
	.bid-controls {
		grid-row: 4;
	}
	.bid-controls:has(:global(.bids)) {
		margin: 4px 0 0;
	}
	.bid-controls :global(button:not(.secondary)) {
		background: #f5e8c3;
		color: #233e32;
		border-color: #ead8aa;
	}
	.hand-area :global(.bid-heading) {
		margin: 0;
	}
	.table-footer {
		display: none;
	}
	.table-layout > .notice,
	.table-layout > .err,
	.hand-area > .err {
		position: absolute;
		z-index: 6;
		left: 24px;
		right: 24px;
		top: 8px;
		color: #233e32;
	}
	.hand-area > .err {
		top: auto;
		bottom: 12px;
	}
	@media (min-width: 701px) and (min-height: 501px) {
		.hand-area {
			max-width: 1100px;
			margin-inline: auto;
			padding-inline: 16px;
		}
		.hand-area :global(.card-rack) {
			width: 100%;
			max-width: 980px;
		}
		.hand-heading .eyebrow {
			display: none;
		}
		.hand-heading h1 {
			font-size: 28px;
		}
		.confirm-card {
			min-width: 230px;
		}
	}

	@media (max-width: 700px) {
		.game-header {
			padding: 6px 12px;
			gap: 6px;
		}
		.table-tools {
			width: 100%;
			justify-content: space-between;
			gap: 8px;
		}
		.table-tools :global(.trigger),
		.settings-trigger {
			min-height: 44px;
			padding: 7px 12px;
			font-size: 15px;
		}
		.table-layout {
			padding: 6px 10px max(8px, env(safe-area-inset-bottom));
			grid-template-rows: minmax(0, 0.92fr) minmax(0, 1.08fr);
			gap: 6px;
		}
		.hand-area {
			padding: 6px 4px 0;
		}
		.hand-heading h1 {
			display: none;
		}
		.hand-heading .eyebrow {
			font-size: 18px;
			font-weight: 650;
			white-space: nowrap;
		}
		.confirm-card {
			width: auto;
			min-height: 48px;
			padding: 8px 12px;
			font-size: 15px;
			gap: 8px;
		}
		.instruction {
			font-size: 15px;
			margin: 5px 0;
		}
		.hand-area.bidding .hand-heading {
			display: none;
		}
		.menu-panel.pinned {
			width: min(280px, calc(100vw - 24px));
			max-height: 50dvh;
			padding: 14px;
		}
	}
	@media (max-width: 700px) and (max-height: 750px) and (orientation: portrait) {
		.table-layout {
			grid-template-rows: minmax(0, 1fr) 230px;
		}
		.table-layout:has(.hand-area.bidding) {
			grid-template-rows: minmax(0, 1fr) 250px;
		}
	}
	@media (max-width: 360px) and (max-height: 650px) and (orientation: portrait) {
		.table-layout {
			grid-template-rows: minmax(0, 1fr) 190px;
		}
		.table-layout:has(.hand-area.bidding) {
			grid-template-rows: minmax(0, 1fr) 220px;
		}
	}
	@media (orientation: landscape) and (max-height: 500px) {
		.game-header {
			padding: 6px 12px;
			gap: 8px;
			min-height: 60px;
		}
		.table-tools {
			width: auto;
			margin-left: auto;
		}
		.table-layout,
		.table-layout:has(.hand-area.bidding) {
			grid-template-rows: minmax(0, 1fr) 125px;
			padding: 4px 10px;
		}
		.hand-area {
			grid-template-columns: 180px minmax(0, 1fr);
			grid-template-rows: auto minmax(0, 1fr);
			gap: 4px 14px;
			padding: 4px;
		}
		.hand-heading {
			grid-column: 1;
			grid-row: 1;
		}
		.hand-heading .eyebrow,
		.hand-heading h1 {
			display: none;
		}
		.confirm-card {
			width: 100%;
			min-height: 44px;
			font-size: 14px;
			padding: 6px;
		}
		.instruction {
			grid-column: 1;
			grid-row: 2;
			font-size: 14px;
		}
		.hand-area :global(.card-rack) {
			grid-column: 2;
			grid-row: 1 / 3;
		}
		.hand-area :global(.card-rack ul) {
			grid-template-columns: repeat(var(--card-count), minmax(0, 1fr));
			gap: 8px;
		}
		.hand-area :global(.card-label) {
			display: none;
		}
		.hand-area :global(.artwork) {
			width: min(100cqw, calc((100cqh - 8px) * 5 / 7));
		}
		.hand-area.bidding .bid-controls {
			grid-column: 1;
			grid-row: 1;
		}
		.hand-area.bidding .instruction {
			grid-row: 2;
		}
	}
</style>
