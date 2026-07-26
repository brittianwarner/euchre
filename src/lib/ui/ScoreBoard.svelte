<!--
  ScoreBoard — team scores, trump, and whose turn it is.
  One job: show match status so the player always knows the state of play.
-->
<script lang="ts">
	import type { PublicGameView } from '$lib/protocol';

	let { view }: { view: PublicGameView } = $props();

	const SEAT_NAME = ['You', 'Rita (W)', 'Ned (N)', 'Eva (E)'] as const;

	const turnLabel = $derived(
		view.turnSeat === null
			? view.phase.replaceAll('_', ' ')
			: `${SEAT_NAME[view.turnSeat]} to act`
	);
</script>

<section class="score" aria-live="polite">
	<p class="brand">Euchre</p>
	<p class="scores">
		<span>Us {view.score[0]}</span>
		<span class="sep">–</span>
		<span>Them {view.score[1]}</span>
	</p>
	<p class="meta">
		Hand {view.handNo + 1}
		{#if view.trump}
			· Trump {view.trump}
		{/if}
		{#if view.upCard && !view.upCardTurnedDown}
			· Up {view.upCard}
		{/if}
	</p>
	<p class="turn">{turnLabel}</p>
	{#if view.status === 'complete'}
		<p class="result">
			{view.winnerTeam === 0 ? 'You win.' : 'They win.'}
		</p>
	{/if}
</section>

<style>
	.score {
		display: grid;
		gap: 0.35rem;
		padding: 1rem 1.25rem 0;
	}
	.brand {
		margin: 0;
		font-family: 'Iowan Old Style', 'Palatino Linotype', Palatino, serif;
		font-size: clamp(2rem, 5vw, 2.75rem);
		letter-spacing: -0.02em;
		color: #f2e8d5;
	}
	.scores {
		margin: 0;
		font-size: 1.35rem;
		font-weight: 600;
		color: #f7f1e4;
	}
	.sep {
		opacity: 0.5;
		margin: 0 0.35rem;
	}
	.meta,
	.turn,
	.result {
		margin: 0;
		color: #c9b89a;
		font-size: 0.95rem;
	}
	.result {
		color: #e8c27a;
		font-weight: 600;
	}
</style>
