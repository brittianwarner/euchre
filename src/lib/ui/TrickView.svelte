<!--
  TrickView — cards currently on the table for this trick.
-->
<script lang="ts">
	import type { PublicGameView } from '$lib/protocol';

	let { view }: { view: PublicGameView } = $props();

	const SEAT_NAME = ['You', 'West', 'North', 'East'] as const;
</script>

<section class="trick" aria-label="Current trick">
	{#if view.trick.plays.length === 0}
		<p class="empty">Table is clear</p>
	{:else}
		<ul>
			{#each view.trick.plays as play (play.seat + play.card)}
				<li>
					<span class="seat">{SEAT_NAME[play.seat]}</span>
					<span class="card">{play.card}</span>
				</li>
			{/each}
		</ul>
	{/if}
	<p class="tricks">Tricks · Us {view.tricksWon[0]} · Them {view.tricksWon[1]}</p>
</section>

<style>
	.trick {
		margin: 0.75rem 1.25rem;
		padding: 1rem;
		min-height: 7rem;
		border: 1px solid rgba(232, 194, 122, 0.2);
		border-radius: 0.75rem;
		background:
			radial-gradient(ellipse at 30% 20%, rgba(70, 110, 60, 0.35), transparent 55%),
			radial-gradient(ellipse at 80% 80%, rgba(40, 60, 35, 0.5), transparent 50%),
			#1a2614;
	}
	.empty {
		margin: 0;
		color: #9aab8a;
	}
	ul {
		list-style: none;
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
		margin: 0;
		padding: 0;
	}
	li {
		display: grid;
		gap: 0.2rem;
		min-width: 3.5rem;
		padding: 0.45rem 0.55rem;
		border-radius: 0.4rem;
		background: rgba(0, 0, 0, 0.28);
		color: #f2e8d5;
	}
	.seat {
		font-size: 0.75rem;
		color: #b8c4a4;
	}
	.card {
		font-weight: 700;
		font-size: 1.1rem;
	}
	.tricks {
		margin: 0.75rem 0 0;
		color: #c9b89a;
		font-size: 0.9rem;
	}
</style>
