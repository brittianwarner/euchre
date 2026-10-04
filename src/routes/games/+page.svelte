<!--
  /games — the scrapbook. Not a database table: a warm, chronological record
  of every match this browser's player has sat down for, with the lifetime
  numbers up top so the whole history reads at a glance.
-->
<script lang="ts">
	import type { MatchOutcome } from '#lib/protocol/index.ts';
	import IdentityGate from '#lib/ui/IdentityGate.svelte';
	import PlayingAs from '#lib/ui/PlayingAs.svelte';

	let { data, form } = $props();

	const dateFormatter = new Intl.DateTimeFormat(undefined, {
		month: 'short',
		day: 'numeric',
		year: 'numeric',
		hour: 'numeric',
		minute: '2-digit'
	});

	function formatDate(ts: number): string {
		return dateFormatter.format(new Date(ts));
	}

	const OUTCOME_LABEL: Record<MatchOutcome, string> = {
		won: 'Won',
		lost: 'Lost',
		abandoned: 'Left unfinished'
	};

	function winRatePct(rate: number): string {
		return `${Math.round(rate * 100)}%`;
	}
</script>

<svelte:head>
	<title>Your games · Euchre</title>
</svelte:head>

<main class="page">
	<div class="content">
		{#if data.identity === null}
			<IdentityGate
				heading="Whose scrapbook is this?"
				lede="Tell us an email and we'll start keeping score — every match you play from here on will show up on this page."
				errorMessage={form?.error ?? null}
			/>
		{:else}
			<header class="page-head">
				<p class="brand">Euchre</p>
				<h1>Your games</h1>
				<PlayingAs email={data.identity.email} />
			</header>

			{#if data.loadError}
				<p class="banner error">Couldn't open your scrapbook just now. Try refreshing the page.</p>
			{:else if data.stats && data.stats.matches > 0}
				<section class="stats" aria-label="Lifetime stats">
					<div class="stat">
						<span class="value">{data.stats.matches}</span>
						<span class="label">Matches played</span>
					</div>
					<div class="stat">
						<span class="value">{winRatePct(data.stats.winRate)}</span>
						<span class="label">Win rate</span>
					</div>
					<div class="stat">
						<span class="value">{data.stats.won}–{data.stats.lost}</span>
						<span class="label"
							>Record{data.stats.abandoned > 0
								? ` (+${data.stats.abandoned} left unfinished)`
								: ''}</span
						>
					</div>
					<div class="stat">
						<span class="value">{data.stats.euchresFor}–{data.stats.euchresAgainst}</span>
						<span class="label">Euchres sent–taken</span>
					</div>
					<div class="stat">
						<span class="value">{data.stats.lonersMade}/{data.stats.lonersAttempted}</span>
						<span class="label">Loners made</span>
					</div>
					<div class="stat">
						<span class="value">{data.stats.handsPlayed}</span>
						<span class="label">Hands played</span>
					</div>
				</section>

				{#if data.page && data.page.rows.length > 0}
					<ul class="games">
						{#each data.page.rows as row (row.matchId)}
							<li>
								<a class="game-card" href={`/games/${row.matchId}`}>
									<div class="card-top">
										<span class="date">{formatDate(row.playedAt)}</span>
										<span class="outcome outcome-{row.outcome}">{OUTCOME_LABEL[row.outcome]}</span>
									</div>
									<div class="card-mid">
										<span class="score">You {row.score[0]} – {row.score[1]} them</span>
										<span class="hands"
											>{row.handsPlayed} hand{row.handsPlayed === 1 ? '' : 's'}</span
										>
									</div>
									{#if row.opponents.length > 0}
										<p class="table">At the table: {row.opponents.join(' · ')}</p>
									{/if}
								</a>
							</li>
						{/each}
					</ul>

					<nav class="pager" aria-label="Older and newer games">
						{#if data.newerHref}
							<a href={data.newerHref}>← Newer games</a>
						{:else}
							<span></span>
						{/if}
						{#if data.olderHref}
							<a href={data.olderHref}>Older games →</a>
						{/if}
					</nav>
				{:else}
					<p class="banner">No more games on this page.</p>
				{/if}
			{:else}
				<section class="empty">
					<h2>Nothing here yet</h2>
					<p>
						Play your first hand and it'll land right here — the date, the final score, whether you
						sent them home early, all of it. Think of this as the scrapbook on top of the fridge.
					</p>
					<a class="cta" href="/play">Deal a hand</a>
				</section>
			{/if}
		{/if}
	</div>
</main>

<style>
	.page {
		width: 100%;
		min-height: 100dvh;
		color: #f2e8d5;
		font-family: 'Source Serif 4', 'Iowan Old Style', Georgia, serif;
		background: #0b0906;
	}
	.content {
		max-width: 42rem;
		margin: 0 auto;
		padding: 2rem 1.25rem 4rem;
	}
	.page-head {
		margin-bottom: 1.25rem;
	}
	.brand {
		margin: 0;
		font-size: 1rem;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: #93876f;
	}
	h1 {
		margin: 0.15rem 0 0.9rem;
		font-size: clamp(1.8rem, 5vw, 2.4rem);
	}
	.banner {
		margin: 0 0 1.5rem;
		padding: 0.85rem 1rem;
		border-radius: 0.6rem;
		background: rgba(15, 20, 14, 0.6);
		border: 1px solid rgba(232, 194, 122, 0.22);
		color: #c9b89a;
	}
	.banner.error {
		background: rgba(74, 32, 24, 0.5);
		border-color: rgba(224, 140, 110, 0.4);
		color: #f7e2d8;
	}
	.stats {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 0.6rem;
		margin: 0 0 2rem;
	}
	.stat {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		padding: 0.75rem 0.6rem;
		border-radius: 0.6rem;
		background: rgba(15, 20, 14, 0.6);
		border: 1px solid rgba(232, 194, 122, 0.18);
		text-align: center;
	}
	.stat .value {
		font-size: 1.35rem;
		font-weight: 600;
		color: #e8c27a;
	}
	.stat .label {
		font-size: 0.72rem;
		color: #93876f;
		line-height: 1.3;
	}
	.games {
		list-style: none;
		margin: 0 0 1.5rem;
		padding: 0;
		display: grid;
		gap: 0.75rem;
	}
	.game-card {
		display: block;
		padding: 0.9rem 1rem;
		border-radius: 0.7rem;
		background: rgba(15, 20, 14, 0.55);
		border: 1px solid rgba(232, 194, 122, 0.18);
		color: inherit;
		text-decoration: none;
		transition:
			border-color 160ms ease,
			background 160ms ease;
	}
	.game-card:hover,
	.game-card:focus-visible {
		border-color: rgba(232, 194, 122, 0.5);
		background: rgba(20, 26, 16, 0.75);
	}
	.card-top {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		gap: 0.5rem;
		margin-bottom: 0.3rem;
	}
	.date {
		color: #93876f;
		font-size: 0.85rem;
	}
	.outcome {
		font-size: 0.78rem;
		font-weight: 700;
		letter-spacing: 0.02em;
		text-transform: uppercase;
		padding: 0.15rem 0.55rem;
		border-radius: 999px;
	}
	.outcome-won {
		background: rgba(61, 107, 56, 0.35);
		color: #b7dcaa;
	}
	.outcome-lost {
		background: rgba(107, 56, 56, 0.35);
		color: #e0b0a6;
	}
	.outcome-abandoned {
		background: rgba(90, 80, 60, 0.35);
		color: #cbbf9e;
	}
	.card-mid {
		display: flex;
		justify-content: space-between;
		gap: 0.5rem;
		font-size: 1.05rem;
		font-weight: 600;
	}
	.hands {
		font-size: 0.85rem;
		font-weight: 400;
		color: #93876f;
		align-self: center;
	}
	.table {
		margin: 0.35rem 0 0;
		font-size: 0.82rem;
		color: #93876f;
	}
	.pager {
		display: flex;
		justify-content: space-between;
		gap: 1rem;
	}
	.pager a {
		min-height: 44px;
		display: inline-flex;
		align-items: center;
		padding: 0 0.9rem;
		border-radius: 0.5rem;
		border: 1px solid #6a5638;
		background: #241b10;
		color: #f2e8d5;
		text-decoration: none;
		font-size: 0.9rem;
	}
	.pager a:hover {
		border-color: #e8c27a;
	}
	.empty {
		max-width: 30rem;
		padding: 2rem 0;
	}
	.empty h2 {
		margin: 0 0 0.6rem;
		font-size: 1.4rem;
	}
	.empty p {
		margin: 0 0 1.25rem;
		color: #c9b89a;
		line-height: 1.55;
	}
	.cta {
		display: inline-flex;
		min-height: 44px;
		align-items: center;
		padding: 0 1.2rem;
		border-radius: 0.5rem;
		background: #3d6b38;
		color: #f4f7e8;
		text-decoration: none;
		font-weight: 600;
	}
	.cta:hover {
		background: #4a7f44;
	}
	@media (max-width: 480px) {
		.stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
</style>
