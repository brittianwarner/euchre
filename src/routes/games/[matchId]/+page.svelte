<!--
  /games/[matchId] — one page of the scrapbook, opened. The header is the
  whole match; the list below is every hand that made it up.
-->
<script lang="ts">
	import type { HandResult, Seat, Suit } from '#lib/protocol/index.ts';

	let { data } = $props();
	const replay = $derived(data.replay);

	const dateFormatter = new Intl.DateTimeFormat(undefined, {
		weekday: 'long',
		month: 'short',
		day: 'numeric',
		year: 'numeric',
		hour: 'numeric',
		minute: '2-digit'
	});

	const SEAT_NAME = ['You', 'West', 'North', 'East'] as const satisfies readonly string[];

	function seatName(seat: Seat | null): string {
		return seat === null ? '—' : SEAT_NAME[seat];
	}

	const SUIT_NAME: Record<Suit, string> = { S: 'Spades', H: 'Hearts', D: 'Diamonds', C: 'Clubs' };
	const SUIT_SYMBOL: Record<Suit, string> = { S: '♠', H: '♥', D: '♦', C: '♣' };

	const RESULT_LABEL: Record<HandResult, string> = {
		point: 'Took the majority — 1 point',
		march: 'Swept all five tricks — 2 points',
		lone_point: 'Went alone, took the majority — 1 point',
		lone_march: 'Went alone, swept all five — 4 points',
		euchre: 'Euchred the makers — 2 points',
		throw_in: 'Thrown in — no points'
	};

	const OUTCOME_LABEL = { won: 'You won', lost: 'They won', abandoned: 'Left unfinished' } as const;
</script>

<svelte:head>
	<title>Game on {dateFormatter.format(new Date(replay.playedAt))} · Euchre</title>
</svelte:head>

<main class="page">
	<div class="content">
		<a class="back" href="/games">← Back to your games</a>

		<header class="head">
			<p class="date">{dateFormatter.format(new Date(replay.playedAt))}</p>
			<h1>{OUTCOME_LABEL[replay.outcome]}</h1>
			<p class="score">You {replay.score[0]} – {replay.score[1]} them</p>
			{#if replay.opponents.length > 0}
				<p class="table">At the table: {replay.opponents.join(' · ')}</p>
			{/if}
		</header>

		<section class="stats" aria-label="Match totals">
			<div class="stat">
				<span class="value">{replay.handsPlayed}</span>
				<span class="label">Hands played</span>
			</div>
			<div class="stat">
				<span class="value">{replay.stats.euchresFor}–{replay.stats.euchresAgainst}</span>
				<span class="label">Euchres sent–taken</span>
			</div>
			<div class="stat">
				<span class="value">{replay.stats.lonersMade}/{replay.stats.lonersAttempted}</span>
				<span class="label">Loners made</span>
			</div>
			<div class="stat">
				<span class="value">{replay.stats.marches}</span>
				<span class="label">Marches</span>
			</div>
			<div class="stat">
				<span class="value">{replay.stats.throwIns}</span>
				<span class="label">Thrown in</span>
			</div>
			<div class="stat">
				<span class="value">{replay.stats.tricks[0]}–{replay.stats.tricks[1]}</span>
				<span class="label">Tricks won</span>
			</div>
		</section>

		{#if replay.hands.length > 0}
			<ol class="hands">
				{#each replay.hands as hand (hand.handNo)}
					<li class="hand">
						<div class="hand-top">
							<span class="hand-no">Hand {hand.handNo + 1}</span>
							<span class="dealer">Dealer: {seatName(hand.dealerSeat)}</span>
						</div>
						<div class="hand-mid">
							{#if hand.trump}
								<span class="trump suit-{hand.trump}">
									{SUIT_SYMBOL[hand.trump]}
									{SUIT_NAME[hand.trump]} called by {seatName(hand.makerSeat)}
									{#if hand.aloneSeat !== null}· alone{/if}
								</span>
							{:else}
								<span class="trump muted">No trump — thrown in</span>
							{/if}
							<span class="tricks">{hand.tricksWon[0]}–{hand.tricksWon[1]} tricks</span>
						</div>
						<p class="result">
							{hand.result ? RESULT_LABEL[hand.result] : 'Unresolved'}
							{#if hand.delta}
								<span class="delta"
									>({hand.delta[0] > 0
										? `+${hand.delta[0]} us`
										: hand.delta[1] > 0
											? `+${hand.delta[1]} them`
											: 'no score'})</span
								>
							{/if}
						</p>
					</li>
				{/each}
			</ol>
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
		max-width: 40rem;
		margin: 0 auto;
		padding: 2rem 1.25rem 4rem;
	}
	.back {
		display: inline-block;
		margin-bottom: 1.5rem;
		color: #c9b89a;
		text-decoration: none;
		font-size: 0.9rem;
	}
	.back:hover {
		color: #e8c27a;
	}
	.head {
		margin-bottom: 1.5rem;
	}
	.date {
		margin: 0;
		color: #93876f;
		font-size: 0.85rem;
	}
	h1 {
		margin: 0.15rem 0 0.4rem;
		font-size: clamp(1.6rem, 5vw, 2.1rem);
	}
	.score {
		margin: 0 0 0.3rem;
		font-size: 1.15rem;
		font-weight: 600;
		color: #e8c27a;
	}
	.table {
		margin: 0;
		color: #93876f;
		font-size: 0.9rem;
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
		padding: 0.7rem 0.5rem;
		border-radius: 0.6rem;
		background: rgba(15, 20, 14, 0.6);
		border: 1px solid rgba(232, 194, 122, 0.18);
		text-align: center;
	}
	.stat .value {
		font-size: 1.2rem;
		font-weight: 600;
		color: #e8c27a;
	}
	.stat .label {
		font-size: 0.7rem;
		color: #93876f;
		line-height: 1.3;
	}
	.hands {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0.6rem;
	}
	.hand {
		padding: 0.75rem 0.9rem;
		border-radius: 0.6rem;
		background: rgba(15, 20, 14, 0.45);
		border: 1px solid rgba(232, 194, 122, 0.14);
	}
	.hand-top {
		display: flex;
		justify-content: space-between;
		font-size: 0.8rem;
		color: #93876f;
		margin-bottom: 0.3rem;
	}
	.hand-mid {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		gap: 0.5rem;
		font-weight: 600;
	}
	.trump.muted {
		color: #93876f;
		font-weight: 400;
	}
	.tricks {
		color: #c9b89a;
		font-size: 0.9rem;
		font-weight: 400;
	}
	.result {
		margin: 0.3rem 0 0;
		font-size: 0.9rem;
		color: #c9b89a;
	}
	.delta {
		color: #93876f;
	}
	@media (max-width: 480px) {
		.stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
</style>
