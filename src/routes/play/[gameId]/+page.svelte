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
	import { untrack } from 'svelte';
	import { withActorParams } from '@rivetkit/svelte';
	import { rivetContext } from '$lib/client/rivet';
	import { TableStore, type TableActorHandle } from '$lib/game/table.svelte';
	import { applyOptimistic, whyIllegal } from '$lib/euchre';
	import EuchreTable3D from '$lib/three/EuchreTable3D.svelte';
	import BidPanel from '$lib/ui/BidPanel.svelte';
	import DiscardPanel from '$lib/ui/DiscardPanel.svelte';
	import HandA11y from '$lib/ui/HandA11y.svelte';
	import Hint from '$lib/ui/Hint.svelte';
	import RulesPanel from '$lib/ui/RulesPanel.svelte';
	import GameAnnouncer from '$lib/ui/onboarding/GameAnnouncer.svelte';
	import Walkthrough from '$lib/ui/onboarding/Walkthrough.svelte';
	import ScoreBoard from '$lib/ui/ScoreBoard.svelte';
	import TrickView from '$lib/ui/TrickView.svelte';
	import TableStatusBar from '$lib/ui/TableStatusBar.svelte';

	/** Compass names for the spoken log. Seat 0 is the player. */
	const SEAT_NAME = ['You', 'West', 'North', 'East'] as const;
	import type { CardId, LegalMoveId, PublicGameView, Step, SyncEvent } from '$lib/protocol';

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
	let optimistic = $state<{ readonly forV: number; readonly view: PublicGameView } | null>(null);

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

	/** Submits any legal move id, previewing its effect locally first. Used by every input surface — 3D card taps, the DOM bid/discard panels, and the keyboard/screen-reader hand — so the whole table gets the same instant feedback from one place. */
	async function submitMove(moveId: LegalMoveId): Promise<void> {
		const view = displayView;
		if (!view) return;
		const entry = view.legal.find((m) => m.id === moveId);
		if (!entry) return; // Every caller here already filtered against this same `legal` set.
		lastDecision = { view, moveId };
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
	<!--
		Onboarding-layer overlays: neither is part of the felt's corner grid.
		`Walkthrough` is a modal `<dialog>` (its own ::backdrop covers the
		viewport regardless of where it sits in the DOM) that only auto-opens
		once `displayView` shows a real, dealt hand — it derives that itself
		from the `view` passed straight through here, so it never appears over
		the "Connecting…" state *or* the cutting screen (a `view` exists,
		non-null, during `cutting` too — see `Walkthrough`'s own doc comment).
		`GameAnnouncer` is a visually hidden aria-live region; it renders
		nothing on screen.
	-->
	<Walkthrough view={displayView} bind:open={tourOpen} />
	<GameAnnouncer steps={lastSteps} view={displayView} />

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

		<!--
			All chrome lives at the screen edges, so nothing ever sits over a
			seat's cards or nameplate (the felt itself is the only thing allowed
			in the middle of the screen):

			  top-left      score, hand number, trump, whose turn (ScoreBoard +
			                TableStatusBar's turn chip/illegal toast)
			  top-right     the current trick + tricks won, kept small (TrickView)
			  bottom-centre the bid/discard action panel — the one place action
			                ever happens. Centred, not a corner: both seat 0's own
			                "You" nameplate (an HTML overlay from
			                `TableScene.svelte`) and North's sit dead-centre, but
			                well *above* this panel's vertical band (verified
			                against a screenshot at 390×844, the tightest
			                breakpoint) — a corner placement narrow enough to
			                dodge that nameplate sideways left too little width
			                for its three buttons to avoid wrapping tall enough
			                to reach the human's own hand instead.

			The low-priority "deal a new game" link lives at the bottom of the
			top-left column rather than its own bottom-left spot: the
			bottom-centre action panel above is wide enough at every breakpoint
			(up to 92vw) that a separate bottom-left corner would sit right
			under its edge — top-left is the one corner nothing else ever grows
			tall enough to reach.

			`.hud` itself is one full-bleed, non-interactive layer (per the
			project's existing pattern); each corner opts back into
			`pointer-events` only where it actually has controls.
		-->
		<div class="hud">
			<div class="corner corner-tl">
				<ScoreBoard view={displayView} />
				<TableStatusBar view={displayView} {illegalMessage} />
				<Hint view={displayView} {lastDecision} />
				<!--
					The persistent "?" (docs/04-FRONTEND-UX.md §15) lives in this column,
					not its own top-centre spot: `GlobalNav.svelte` is fixed top-RIGHT
					but, being width-to-content, reaches well past horizontal centre on
					a narrow phone — a separate centred corner collided with it there.
					top-left's column is the one place nothing else ever grows into.
					`.help-row` overrides the column's `align-items: stretch` so this
					stays a small round button instead of stretching into a pill.
				-->
				<div class="help-row">
					<RulesPanel view={displayView} onReplayTour={() => (tourOpen = true)} />
				</div>
				<p class="footer">
					<a href="/play">Deal a new game</a>
					· game {data.gameId.slice(0, 8)}
				</p>
			</div>

			<div class="corner corner-tr">
				<TrickView view={displayView} />
			</div>

			<!--
				What the table says out loud. Bottom-LEFT, opposite the action panel,
				so the two never collide and neither sits over the fan.
			-->
			{#if store.chat.length > 0}
				<div class="corner corner-bl" aria-live="polite" aria-label="Table talk">
					{#each store.chat as line (line.msgId)}
						<p class="said" class:banter={line.kind === 'banter'}>
							<span class="who">{SEAT_NAME[line.seat] ?? 'Table'}</span>
							{line.text}
						</p>
					{/each}
				</div>
			{/if}

			<div class="corner corner-bc">
				{#if store.error}
					<p class="err" role="alert">{store.error}</p>
				{/if}
				<BidPanel view={displayView} disabled={store.submitting} onPlay={submitMove} />
				<DiscardPanel view={displayView} disabled={store.submitting} onPlay={submitMove} />
			</div>

			<!--
				Visually hidden but focusable: the 3D table is the visual layer,
				this is the permanent keyboard/screen-reader path
				(docs/04-FRONTEND-UX.md §13). `:focus-within` restores it to the
				flow so a keyboard user always sees where focus landed.
			-->
			<div class="a11y-hand">
				<HandA11y view={displayView} disabled={store.submitting} onPlay={submitMove} />
			</div>
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
	.loading {
		padding: 1.25rem;
		margin: 0;
	}
	.err {
		margin: 0;
		color: #e8a090;
	}
	.footer {
		margin: 0.4rem 0 0;
		color: #8a7a62;
		font-size: 0.8rem;
	}
	.footer a {
		color: #d4b57a;
	}
	/*
	 * North's nameplate is an `<HTML center>` billboard anchored to a fixed 3D
	 * point, not a DOM sibling this column can push against: its own font/tags
	 * add roughly no height, but its projected screen Y is a near-constant
	 * *fraction* of the canvas height (CameraRig's portrait fov/dist are
	 * constants, so the vertical projection fraction of any fixed depth is
	 * independent of the canvas's actual width or height) — confirmed by
	 * measurement at 390 CSS px wide: its badge sits at ~28% of viewport
	 * height at every height from 667 to 1500px.
	 *
	 * `.footer`, by contrast, sits at a content-driven, essentially
	 * viewport-height-*independent* pixel offset from the top (this column's
	 * own text never changes size with a taller window). Those two facts
	 * combine badly for a portrait window that is merely narrow rather than
	 * phone-shaped — e.g. a desktop browser resized narrow at full monitor
	 * height, not just an actual handset: past roughly 960 CSS px of height
	 * North's ~28%-of-height badge catches up to and passes this link's fixed
	 * position, overlapping it (verified by a width/height sweep — no overlap
	 * below ~960px or above ~1150px, real overlap in between, at every
	 * portrait width tried from 320 to 760).
	 *
	 * The fix moves the chrome: give the link a floor that *also* grows with
	 * viewport height, at a steeper 32%-of-height rate than North's ~28–31%,
	 * so above the crossover this margin out-paces North's badge and the gap
	 * only widens from there — while `max()` keeps it a no-op (falls back to
	 * the plain 0.4rem above) at every normal phone height, where the vh term
	 * is negative. Scoped to portrait only (`isPortrait`'s own 0.8 threshold,
	 * `$lib/three/scene/breakpoints.ts`) since landscape's nameplate sits at a
	 * different, unrelated fraction.
	 */
	@media (max-aspect-ratio: 4/5) {
		.footer {
			margin-top: max(0.4rem, calc(32vh - 18.5rem));
		}
	}

	.hud {
		position: absolute;
		inset: 0;
		z-index: 1;
		pointer-events: none;
	}
	.hud > :global(*) {
		pointer-events: auto;
	}

	.corner {
		position: absolute;
	}
	.corner-tl {
		top: max(0.25rem, env(safe-area-inset-top));
		left: max(0.25rem, env(safe-area-inset-left));
		max-width: min(62vw, 24rem);
		display: flex;
		flex-direction: column;
		gap: 0.6rem;
	}
	.corner-tr {
		top: max(0.75rem, env(safe-area-inset-top));
		right: max(0.75rem, env(safe-area-inset-right));
		max-width: min(46vw, 15rem);
	}
	.help-row {
		align-self: flex-start;
	}
	/*
	 * The action panel lives bottom-RIGHT, not bottom-centre.
	 *
	 * Centred, it sat directly on top of the player's fan — you could not read the
	 * cards you were being asked to bid on. The hand occupies the centre column,
	 * so the chrome gets the corner.
	 */
	.corner-bl {
		bottom: max(0.75rem, env(safe-area-inset-bottom));
		left: max(0.75rem, env(safe-area-inset-left));
		max-width: min(22rem, 32vw);
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	.said {
		margin: 0;
		font-size: 0.95rem;
		line-height: 1.35;
		color: #e8e2d2;
		text-shadow: 0 1px 3px rgb(0 0 0 / 0.8);
	}

	/* Persona chatter reads quieter than a real call, so a trump call never gets
	   lost in banter. */
	.said.banter {
		opacity: 0.72;
		font-style: italic;
	}

	.said .who {
		font-weight: 700;
		color: #d8b464;
		margin-right: 0.3rem;
	}

	.corner-bc {
		bottom: max(0.75rem, env(safe-area-inset-bottom));
		right: max(0.75rem, env(safe-area-inset-right));
		/*
		 * Wide enough that "Pass" / "Order it up" / "Order it up, alone" wrap to
		 * two rows, not three — three rows in a narrower column pushed this
		 * panel's top edge up into the human's own hand at 390×844 (verified
		 * against a screenshot, not guessed). Centred keeps it clear of the
		 * felt's only other bottom-band occupant, seat 0's own nameplate, which
		 * sits well above this panel's band at every breakpoint.
		 */
		max-width: min(92vw, 26rem);
		width: min(92vw, 26rem);
		display: flex;
		flex-direction: column;
		/*
		 * `stretch`, not `center`: a shrink-to-fit flex item containing its own
		 * wrapping row (the bid buttons) sizes to that row's *min-content* —
		 * one button wide — which then wraps every button onto its own line
		 * and (again) pushes this panel's top edge up into the seat 0 nameplate
		 * band above it. Stretching the panel to the container's actual width
		 * lets "Pass"/"Order it up" share a row as intended.
		 */
		align-items: stretch;
		gap: 0.5rem;
	}
	.corner-bc .err {
		margin: 0;
		text-align: center;
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
