<!-- Calm turn and validation feedback inside the action dock. Transient feedback never covers the player's cards. -->
<script lang="ts">
	import type { PublicGameView } from '#lib/protocol/index.ts';

	let {
		view,
		illegalMessage = null
	}: {
		view: PublicGameView;
		illegalMessage?: string | null;
	} = $props();

	const yourTurn = $derived(
		view.status === 'active' && view.turnSeat === view.you && view.legal.length > 0
	);
</script>

<div class="status-rail">
	<p class:active={yourTurn} role="status">
		<span class="dot" aria-hidden="true"></span>{view.status === 'complete'
			? view.winnerTeam === 0
				? 'You won the match'
				: 'Match complete'
			: yourTurn
				? 'Your turn'
				: 'At the table'}
	</p>
	{#if illegalMessage}<p class="illegal-toast" role="alert">{illegalMessage}</p>{/if}
</div>

<style>
	.status-rail {
		font: 600 12px var(--font-sans);
		letter-spacing: 0.09em;
		text-transform: uppercase;
	}
	.status-rail p {
		display: flex;
		align-items: center;
		gap: 7px;
		margin: 0;
		color: #7d8875;
	}
	.status-rail p.active {
		color: #325437;
	}
	.dot {
		width: 5px;
		height: 5px;
		background: #a8b09e;
		border-radius: 50%;
	}
	.active .dot {
		background: #587d3c;
	}
	.status-rail .illegal-toast {
		position: absolute;
		left: 50%;
		bottom: calc(100% + 12px);
		transform: translateX(-50%);
		padding: 12px 18px;
		max-width: 90vw;
		width: max-content;
		border: 1px solid #e0bbac;
		border-radius: 8px;
		background: #fff1e9;
		color: #904d37;
		font-size: 12px;
		font-weight: 500;
		text-transform: none;
		letter-spacing: 0;
		box-shadow: 0 6px 24px #0001;
	}
</style>
