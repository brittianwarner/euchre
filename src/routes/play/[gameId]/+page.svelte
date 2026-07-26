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
	import { withActorParams } from '@rivetkit/svelte';
	import { rivetContext } from '$lib/client/rivet';
	import { TableStore, type TableActorHandle } from '$lib/game/table.svelte';
	import { applyOptimistic, whyIllegal } from '$lib/euchre';
	import EuchreTable3D from '$lib/three/EuchreTable3D.svelte';
	import BidPanel from '$lib/ui/BidPanel.svelte';
	import DiscardPanel from '$lib/ui/DiscardPanel.svelte';
	import HandA11y from '$lib/ui/HandA11y.svelte';
	import ScoreBoard from '$lib/ui/ScoreBoard.svelte';
	import TrickView from '$lib/ui/TrickView.svelte';
	import TableStatusBar from '$lib/ui/TableStatusBar.svelte';
	import type { CardId, LegalMoveId, PublicGameView, SyncEvent } from '$lib/protocol';

	let { data } = $props();

	const { useActor } = rivetContext.get();
	const store = new TableStore();

	const table = useActor(
		withActorParams(
			() => ({
				name: 'euchreTable' as const,
				key: ['table', data.gameId]
			}),
			() => ({ token: data.token })
		)
	);

	// onEvent must be registered during component init — not inside $effect.
	table.onEvent('sync', (payload: SyncEvent) => {
		store.applySync(payload);
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
	let optimistic = $state<{ readonly forV: number; readonly view: PublicGameView } | null>(null);

	const displayView = $derived.by((): PublicGameView | null => {
		if (!store.view) return null;
		if (optimistic && optimistic.forV === store.view.v) return optimistic.view;
		return store.view;
	});

	/** Submits any legal move id, previewing its effect locally first. Used by every input surface — 3D card taps, the DOM bid/discard panels, and the keyboard/screen-reader hand — so the whole table gets the same instant feedback from one place. */
	async function submitMove(moveId: LegalMoveId): Promise<void> {
		const view = displayView;
		if (!view) return;
		const entry = view.legal.find((m) => m.id === moveId);
		if (!entry) return; // Every caller here already filtered against this same `legal` set.
		optimistic = { forV: view.v, view: applyOptimistic(view, entry.move) };
		const ok = await store.play(moveId);
		if (!ok) optimistic = null; // Rejected — store already resynced; show its truth, not our guess.
	}

	function findCardMoveId(view: PublicGameView, card: CardId): LegalMoveId | null {
		for (const m of view.legal) {
			if ((m.move.t === 'play' || m.move.t === 'discard') && m.move.card === card) return m.id;
		}
		return null;
	}

	async function handleCardPlay(card: CardId): Promise<void> {
		const view = displayView;
		if (!view) return;
		const moveId = findCardMoveId(view, card);
		if (!moveId) return; // Hand.svelte only calls onplay for ids it found in this same `view.legal`.
		await submitMove(moveId);
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
</script>

<svelte:head>
	<title>Euchre — play</title>
</svelte:head>

<main class="table-page">
	{#if !store.view}
		<p class="loading">
			{table.isConnected ? 'Dealing…' : 'Connecting to the table…'}
		</p>
		{#if table.lastError}
			<p class="err">{String(table.lastError)}</p>
		{/if}
	{:else if displayView}
		<EuchreTable3D
			view={displayView}
			disabled={store.submitting}
			onplay={handleCardPlay}
			onillegal={handleCardIllegal}
		/>

		<TableStatusBar view={displayView} {illegalMessage} />

		<div class="hud">
			<ScoreBoard view={displayView} />
			<TrickView view={displayView} />
			{#if store.error}
				<p class="err" role="alert">{store.error}</p>
			{/if}
			<BidPanel view={displayView} disabled={store.submitting} onPlay={submitMove} />
			<DiscardPanel view={displayView} disabled={store.submitting} onPlay={submitMove} />
			<!--
				Visually hidden but focusable: the 3D table is the visual layer,
				this is the permanent keyboard/screen-reader path
				(docs/04-FRONTEND-UX.md §13). `:focus-within` restores it to the
				flow so a keyboard user always sees where focus landed.
			-->
			<div class="a11y-hand">
				<HandA11y view={displayView} disabled={store.submitting} onPlay={submitMove} />
			</div>
			<p class="footer">
				<a href="/play">Deal a new game</a>
				· game {data.gameId.slice(0, 8)}
			</p>
		</div>
	{/if}
</main>

<style>
	.table-page {
		position: relative;
		min-height: 100dvh;
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
		background: #0b0906;
	}
	.loading,
	.err,
	.footer {
		padding: 1.25rem;
		margin: 0;
	}
	.err {
		color: #e8a090;
	}
	.footer {
		color: #8a7a62;
		font-size: 0.85rem;
	}
	.footer a {
		color: #d4b57a;
	}

	.hud {
		position: relative;
		z-index: 1;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		min-height: 100dvh;
		pointer-events: none;
	}
	.hud > :global(*) {
		pointer-events: auto;
	}

	.a11y-hand {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
		border: 0;
	}
	.a11y-hand:focus-within {
		position: static;
		width: auto;
		height: auto;
		margin: 0;
		overflow: visible;
		clip: auto;
		white-space: normal;
	}

	@media (prefers-reduced-motion: reduce) {
		.table-page :global(*) {
			animation-duration: 0.001ms !important;
			transition-duration: 0.001ms !important;
		}
	}
</style>
