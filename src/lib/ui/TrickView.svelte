<!--
  TrickView — cards currently on the table for this trick.

  A compact corner badge, not a full-width block: the 3D table itself already
  shows the cards lying on the felt, so this is a small always-there summary
  (tricks won so far, and this trick's plays as text for anyone who can't
  make out the 3D scene) that must never grow large enough to sit over the
  felt it is reporting on — see `+page.svelte`'s `.hud-tr` corner.
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
		margin: 0;
		padding: 0.6rem 0.8rem;
		border: 1px solid rgba(232, 194, 122, 0.2);
		border-radius: 0.65rem;
		background:
			radial-gradient(ellipse at 30% 20%, rgba(70, 110, 60, 0.4), transparent 55%),
			radial-gradient(ellipse at 80% 80%, rgba(40, 60, 35, 0.6), transparent 50%),
			rgba(15, 20, 14, 0.85);
		font-size: 0.82rem;
	}
	.empty {
		margin: 0;
		color: #9aab8a;
	}
	ul {
		list-style: none;
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem;
		margin: 0;
		padding: 0;
	}
	li {
		display: grid;
		gap: 0.1rem;
		min-width: 2.4rem;
		padding: 0.25rem 0.35rem;
		border-radius: 0.35rem;
		background: rgba(0, 0, 0, 0.3);
		color: #f2e8d5;
	}
	.seat {
		font-size: 0.62rem;
		color: #b8c4a4;
	}
	.card {
		font-weight: 700;
		font-size: 0.88rem;
	}
	.tricks {
		margin: 0.45rem 0 0;
		color: #c9b89a;
		font-size: 0.78rem;
	}
</style>
