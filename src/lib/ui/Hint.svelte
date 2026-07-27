<!--
  Hint — contextual coach mode.

  OFF by default, toggled here, never anywhere else — the toggle and the
  persisted preference both live in this one component so there is exactly
  one place that decides whether a nudge is shown. When it's on, and it's
  your decision, it names the same top move `rankMoves()` would hand an AI
  seat, in the AI seat's own words (`RankedMove.why` — see
  `$lib/ai/heuristic.ts`, which names "the coach hints in the browser" as one
  of its four sanctioned consumers). Nothing here re-derives euchre judgement;
  it only chooses which of the engine's own words to show.

  Per docs/04-FRONTEND-UX.md §15.2, a hint that keeps being followed
  correctly, unassisted, three times running for the same *kind* of decision
  retires itself for that kind without the player hunting for a setting — see
  `noteDecision`. It comes back the moment a different choice is made, so a
  rough patch never leaves someone stranded without help they clearly still
  want.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { rankMoves } from '$lib/ai/heuristic';
	import type { LegalMoveId, PublicGameView } from '$lib/protocol';

	const ENABLED_KEY = 'euchre:hints:v1:enabled';
	const STREAK_KEY = 'euchre:hints:v1:streaks';
	const RETIRE_AFTER = 3;

	interface Props {
		view: PublicGameView | null;
		/**
		 * The exact `(view, moveId)` the page just submitted for the human seat —
		 * set once per real decision, at the moment of commit. Passing the pre-move
		 * `view` alongside it (rather than relying on this component to notice the
		 * table's turn change) is what makes "was the hint's suggestion followed"
		 * an exact comparison instead of a guess about timing.
		 */
		lastDecision?: { view: PublicGameView; moveId: LegalMoveId } | null;
	}

	let { view, lastDecision = null }: Props = $props();

	type SituationClass = 'cut' | 'bid1' | 'bid2' | 'discard' | 'lead' | 'follow';

	function classify(v: PublicGameView): SituationClass | null {
		switch (v.phase) {
			case 'cutting':
				return 'cut';
			case 'bid_round_1':
				return 'bid1';
			case 'bid_round_2':
				return 'bid2';
			case 'dealer_discard':
				return 'discard';
			case 'trick_play':
				return v.trick.ledSuit === null ? 'lead' : 'follow';
			default:
				return null;
		}
	}

	function readEnabled(): boolean {
		try {
			return localStorage.getItem(ENABLED_KEY) === '1';
		} catch {
			return false;
		}
	}

	function readStreaks(): Partial<Record<SituationClass, number>> {
		try {
			const raw = localStorage.getItem(STREAK_KEY);
			return raw ? (JSON.parse(raw) as Partial<Record<SituationClass, number>>) : {};
		} catch {
			return {};
		}
	}

	let enabled = $state(readEnabled());
	let streaks = $state<Partial<Record<SituationClass, number>>>(readStreaks());

	function setEnabled(next: boolean): void {
		enabled = next;
		try {
			localStorage.setItem(ENABLED_KEY, next ? '1' : '0');
		} catch {
			/* No persistence available — the toggle just won't survive a reload. */
		}
	}

	function bump(cls: SituationClass, correct: boolean): void {
		// Read the current streak without subscribing to it: this runs inside
		// the `lastDecision` $effect below, and if that effect tracked `streaks`
		// as a read, the `streaks = next` write two lines down would re-trigger
		// the very same effect — reading and writing the same state — for an
		// infinite loop (Svelte's own diagnosis: effect_update_depth_exceeded).
		// `lastDecision` hasn't changed on that rerun, so it would bump forever.
		const current = untrack(() => streaks);
		const next = { ...current, [cls]: correct ? (current[cls] ?? 0) + 1 : 0 };
		streaks = next;
		try {
			localStorage.setItem(STREAK_KEY, JSON.stringify(next));
		} catch {
			/* Same as above: best-effort only. */
		}
	}

	/**
	 * The only place this component writes anything: turns the page's report of
	 * "here's what actually got submitted" into the retire/reinstate streak.
	 * A pure `$derived` can't do this — it's a deliberate write to persisted
	 * state in response to an event, not a computation from current props.
	 */
	$effect(() => {
		const decision = lastDecision;
		if (decision === null) return;
		const cls = classify(decision.view);
		if (cls === null) return;
		const ranked = rankMoves(decision.view);
		const top = ranked[0];
		if (top === undefined) return;
		bump(cls, top.id === decision.moveId);
	});

	const situation = $derived(view !== null ? classify(view) : null);

	const yourDecision = $derived(
		view !== null && view.status === 'active' && view.turnSeat === view.you && view.legal.length > 1
	);

	const suggestion = $derived.by((): { label: string; why: string } | null => {
		if (!yourDecision || view === null || situation === null) return null;
		const top = rankMoves(view)[0];
		if (top === undefined) return null;
		const label = view.legal.find((m) => m.id === top.id)?.label ?? top.id;
		return { label, why: top.why };
	});

	const retired = $derived(situation !== null && (streaks[situation] ?? 0) >= RETIRE_AFTER);

	/** Some heuristic `why` strings ("run 'em") just restate the move's own label ("Run 'em") — skip the redundant echo. */
	const whyIsRedundant = $derived(
		suggestion !== null && suggestion.why.trim().toLowerCase() === suggestion.label.trim().toLowerCase()
	);

	/**
	 * A short, one-time plain-language gloss for the euchre-table shorthand the
	 * heuristic's `why` strings use ("teach jargon once" — the assignment's own
	 * words). Tracked per browser session only: it need not survive a reload,
	 * it only needs to not repeat itself inside one sitting.
	 */
	const JARGON: ReadonlyArray<readonly [RegExp, string]> = [
		[/\bruff\b/i, 'trump a card in a suit that wasn’t led'],
		[/\blay off\b/i, 'play a low card since you can’t win this one'],
		[/\bthird hand high\b/i, 'you’re third to play, so play your best card'],
		[/\bsecond hand\b/i, 'you’re the second to play this trick'],
		[/\bbower\b/i, 'one of the two top trump jacks']
	];
	let explainedTerms = $state.raw<ReadonlySet<string>>(new Set());

	function gloss(why: string): string {
		if (why.length === 0) return '';
		const sentence = why.charAt(0).toUpperCase() + why.slice(1);
		for (const [pattern, meaning] of JARGON) {
			if (!pattern.test(why)) continue;
			const key = pattern.source;
			if (explainedTerms.has(key)) return `${sentence}.`;
			explainedTerms = new Set(explainedTerms).add(key);
			return `${sentence} — meaning ${meaning}.`;
		}
		return `${sentence}.`;
	}
</script>

<div class="hint">
	<label class="toggle">
		<input type="checkbox" checked={enabled} onchange={(e) => setEnabled(e.currentTarget.checked)} />
		Hints
	</label>

	{#if enabled && suggestion !== null && !retired}
		<p class="suggestion" role="status">
			Try: <strong>{suggestion.label}</strong>{#if suggestion.why && !whyIsRedundant} — {gloss(suggestion.why)}{/if}
		</p>
	{/if}
</div>

<style>
	.hint {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
	}
	.toggle {
		display: inline-flex;
		align-items: center;
		gap: 0.4rem;
		min-height: 44px;
		width: fit-content;
		padding: 0.2rem 0.5rem;
		border-radius: 0.4rem;
		color: #c9b89a;
		font-size: 0.85rem;
		cursor: pointer;
	}
	.toggle input {
		width: 1.15rem;
		height: 1.15rem;
		accent-color: #e8c27a;
	}
	.suggestion {
		margin: 0;
		max-width: 22rem;
		padding: 0.5rem 0.7rem;
		border-radius: 0.5rem;
		border: 1px solid rgba(232, 194, 122, 0.4);
		background: rgba(232, 194, 122, 0.12);
		color: #f4ecd8;
		font-size: 0.85rem;
		line-height: 1.35;
	}
	.suggestion strong {
		color: #f7dfa0;
	}
</style>
