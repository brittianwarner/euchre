<!-- A compact trick tally for the action dock, with the current plays preserved as accessible text. -->
<script lang="ts">
	import type { PublicGameView } from '#lib/protocol/index.ts';

	let { view }: { view: PublicGameView } = $props();

	const SEAT_NAME = ['You', 'West', 'North', 'East'] as const;
</script>

<section class="trick" aria-label="Tricks won">
	<span>TRICKS</span><strong>{view.tricksWon[0]}</strong><i>—</i><strong>{view.tricksWon[1]}</strong
	><span class="sr-only"
		>Us {view.tricksWon[0]}, them {view.tricksWon[1]}. {view.trick.plays
			.map((play) => `${SEAT_NAME[play.seat]} played ${play.card}`)
			.join('. ')}</span
	>
</section>

<style>
	.trick {
		display: flex;
		align-items: center;
		gap: 10px;
		font-family: var(--font-sans);
		color: #5d6c55;
	}
	.trick > span:first-child {
		font-size: 12px;
		font-weight: 600;
		letter-spacing: 0.12em;
		margin-right: 3px;
	}
	.trick strong {
		font-size: 19px;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}
	.trick i {
		font-style: normal;
		color: #a2ae95;
		font-size: 14px;
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		white-space: nowrap;
	}
</style>
