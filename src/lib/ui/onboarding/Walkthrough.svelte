<!--
  Walkthrough — the first-run table tour.

  Appears exactly once per browser, the moment a real hand is on the felt —
  computed from the live `view` (see `ready` below), not merely from a `view`
  existing at all: a `view` can arrive as early as the `cutting` phase, before
  a single card is dealt, and this tour must never cover that screen. It
  never blocks play: every
  step has a "Skip" as well as "Next", Esc and a backdrop click all dismiss it,
  and dismissing it — by any route — marks it seen for good. A caller may also
  reopen it deliberately (`bind:open`) for a "show me around again" link
  (`RulesPanel.svelte` does this), which is why "seen" and "open" are tracked
  separately: reopening the tour must never re-arm the auto-show.

  Five short beats, in the order a newcomer needs them (docs/04-FRONTEND-UX.md
  §15): who's at the table, the deck, the bowers (the one rule everyone gets
  wrong), the two bidding rounds + going alone, and scoring. Nothing here is
  invented — every fact matches `$lib/euchre` (`docs/02-GAME-RULES-ENGINE.md`
  is the source of truth) so this can never drift from what the engine
  actually enforces.
-->
<script lang="ts">
	import type { PublicGameView } from '$lib/protocol';

	const SEEN_KEY = 'euchre:onboarding:v1:seen';

	/**
	 * Phases in which `view.hand` is legitimately still empty because no hand
	 * has been dealt yet (`deal` is the engine-only phase between `cutting`
	 * and the first bid — see `GamePhase`'s doc comment in `$lib/euchre/types.ts`).
	 * Every later phase can *also* show an empty `hand` — the sitting seat under
	 * a loner (V8) — which is exactly why "hand non-empty" alone isn't the full
	 * gate below; either signal proves a hand was actually dealt this game.
	 */
	const PRE_DEAL_PHASES = new Set<PublicGameView['phase']>(['lobby', 'cutting', 'deal']);

	interface Props {
		/**
		 * The live view, or `null` before one exists at all. A `view` can arrive
		 * as early as the `cutting` phase — before a single card is dealt — so
		 * `ready` below (a *real, dealt* hand on the felt) is computed from it
		 * rather than taken as a caller-supplied flag: a caller checking only
		 * "does a view exist" was exactly the bug that let this tour open over
		 * the cutting screen.
		 */
		view: PublicGameView | null;
		/** Bindable so a "replay the tour" affordance elsewhere can force it open. */
		open?: boolean;
	}

	let { view, open = $bindable(false) }: Props = $props();

	/** True once a real, dealt hand is showing — not merely once a `view` exists (`cutting`/`deal` don't count; see `PRE_DEAL_PHASES`). */
	const ready = $derived(
		view !== null && (view.hand.length > 0 || !PRE_DEAL_PHASES.has(view.phase))
	);

	function hasSeenTour(): boolean {
		try {
			return localStorage.getItem(SEEN_KEY) === '1';
		} catch {
			return false; // Storage disabled/unavailable: fail open to "not seen" once, not every load.
		}
	}

	function markSeen(): void {
		try {
			localStorage.setItem(SEEN_KEY, '1');
		} catch {
			/* No persistence available — the tour will just reappear next visit. Not worth surfacing. */
		}
	}

	// Plain (non-reactive) latch: the auto-open check must run exactly once,
	// the instant `ready` first becomes true, never again — including when
	// `open` later flips true/false from a manual reopen.
	let checkedAutoShow = false;

	interface Slide {
		readonly title: string;
		readonly body: string;
	}

	const SLIDES: readonly Slide[] = [
		{
			title: 'Welcome to the table',
			body:
				'Four players, two teams. You and North are partners, sitting across ' +
				'from each other; West and East play together. First team to 10 points wins.'
		},
		{
			title: 'The deck',
			body:
				'Euchre uses 24 cards: nine, ten, jack, queen, king and ace, in all four ' +
				"suits. That's it — nothing lower than a nine is in the deck."
		},
		{
			title: 'The bowers — the one rule to know',
			body:
				"The jack of the trump suit is the best card in the game — it's called " +
				'the right bower. But there is a second trump jack: the jack of the ' +
				'OTHER suit that shares its colour. If hearts are trump, the jack of ' +
				"diamonds turns into a heart for the rest of the hand — that's the left " +
				"bower, and it's the second-best card on the table."
		},
		{
			title: 'Bidding',
			body:
				'Each hand turns one card face up. Round one: you may "order it up" to ' +
				'make its suit trump, or pass. If everyone passes, that card is turned ' +
				'down and round two lets you name any OTHER suit, or pass again. Confident ' +
				'in your hand? Go alone — your partner sits out and a perfect hand is ' +
				'worth double.'
		},
		{
			title: 'Scoring',
			body:
				'Take 3 or 4 tricks: 1 point. Take all 5 — a march — 2 points. Go alone ' +
				'and take all 5: 4 points. Call trump and win fewer than 3 tricks and you’ve ' +
				'been euchred: the other team scores 2 instead.'
		},
		{
			title: "You're ready",
			body:
				'Everything else you’ll pick up as you go: a card you can’t play is ' +
				'dimmed, and tapping it explains why in plain words. Turn on Hints any ' +
				'time for a nudge on your turn, and the ? in the corner has the full ' +
				'rulebook whenever you want it.'
		}
	];

	let step = $state(0);
	const lastStep = SLIDES.length - 1;
	const slide = $derived(SLIDES[step]!);

	let dialogEl = $state<HTMLDialogElement | undefined>(undefined);

	// The only defensible use of $effect here: keeping an imperative <dialog>
	// (showModal/close have no declarative attribute equivalent — the plain
	// `open` attribute renders a non-modal, backdrop-less dialog) in sync with
	// the boolean prop that everything else in this component treats
	// declaratively.
	$effect(() => {
		const el = dialogEl;
		if (!el) return;
		if (open && !el.open) {
			step = 0;
			el.showModal();
		} else if (!open && el.open) {
			el.close();
		}
	});

	$effect(() => {
		if (!ready || checkedAutoShow) return;
		checkedAutoShow = true;
		if (!hasSeenTour()) open = true;
	});

	function dismiss(): void {
		markSeen();
		open = false;
	}

	function next(): void {
		if (step >= lastStep) {
			dismiss();
			return;
		}
		step += 1;
	}

	function back(): void {
		if (step > 0) step -= 1;
	}
</script>

<dialog
	bind:this={dialogEl}
	class="tour"
	aria-labelledby="tour-title"
	onclose={dismiss}
	onclick={(e) => {
		// A click on the ::backdrop lands on the <dialog> element itself, never
		// on `.sheet` inside it — that's the standard way to detect it.
		if (e.target === dialogEl) dismiss();
	}}
>
	<div class="sheet">
		<p class="eyebrow">
			Quick tour · {step + 1} of {SLIDES.length}
		</p>
		<h2 id="tour-title">{slide.title}</h2>
		<p class="body">{slide.body}</p>

		<div class="dots" aria-hidden="true">
			{#each SLIDES as s, i (s.title)}
				<span class="dot" class:active={i === step}></span>
			{/each}
		</div>

		<div class="row">
			<button type="button" class="ghost" onclick={dismiss}>Skip</button>
			<div class="spacer"></div>
			{#if step > 0}
				<button type="button" class="ghost" onclick={back}>Back</button>
			{/if}
			<button type="button" class="primary" onclick={next}>
				{step >= lastStep ? 'Deal me in' : 'Next'}
			</button>
		</div>
	</div>
</dialog>

<style>
	.tour {
		border: none;
		padding: 0;
		background: transparent;
		max-width: min(30rem, 92vw);
		width: 100%;
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
	}
	.tour::backdrop {
		background: rgba(6, 5, 3, 0.72);
	}
	.sheet {
		padding: 1.5rem 1.5rem 1.25rem;
		border-radius: 0.9rem;
		border: 1px solid rgba(232, 194, 122, 0.35);
		background: #17130d;
		box-shadow: 0 12px 40px rgba(0, 0, 0, 0.55);
	}
	.eyebrow {
		margin: 0 0 0.3rem;
		font-size: 0.78rem;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: #c9b89a;
	}
	h2 {
		margin: 0 0 0.6rem;
		font-size: 1.4rem;
		color: #f7f1e4;
	}
	.body {
		margin: 0 0 1.1rem;
		line-height: 1.5;
		color: #e8dcc6;
	}
	.dots {
		display: flex;
		gap: 0.35rem;
		margin-bottom: 1.1rem;
	}
	.dot {
		width: 0.45rem;
		height: 0.45rem;
		border-radius: 50%;
		background: rgba(232, 194, 122, 0.28);
	}
	.dot.active {
		background: #e8c27a;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	.spacer {
		flex: 1;
	}
	button {
		min-height: 44px;
		min-width: 44px;
		padding: 0.6rem 1.1rem;
		border-radius: 0.4rem;
		font: inherit;
		font-weight: 600;
		cursor: pointer;
	}
	.primary {
		border: 1px solid #e8c27a;
		background: #4a3a1c;
		color: #fff6e0;
	}
	.ghost {
		border: 1px solid rgba(232, 194, 122, 0.35);
		background: transparent;
		color: #c9b89a;
	}
	@media (prefers-reduced-motion: reduce) {
		.tour,
		.tour::backdrop {
			transition: none !important;
			animation: none !important;
		}
	}
</style>
