<script lang="ts">
	import ScoreCards from './ScoreCards.svelte';
	import type { PublicGameView } from '#lib/protocol/index.ts';
	let { view }: { view: PublicGameView } = $props();
	const team = $derived(view.you % 2);
</script>

<section class="score" aria-label="Match score">
	<a class="brand" href="/" aria-label="Euchre home">euchre<span aria-hidden="true">♣</span></a>
	<div class="scoreline">
		<div>
			<ScoreCards score={view.score[team]} /><span>Your team</span><strong
				>{view.score[team]}</strong
			>
		</div>
		<span class="divider">—</span>
		<div>
			<strong>{view.score[1 - team]}</strong><span>Other team</span><ScoreCards
				score={view.score[1 - team]}
			/>
		</div>
	</div>
	<span class="goal">First to 10</span>
</section>

<style>
	.score {
		display: flex;
		align-items: center;
		gap: 28px;
		color: #233e32;
		min-width: 0;
		flex: 1;
	}
	.brand {
		font: 600 38px/1 var(--font-serif);
		letter-spacing: -0.075em;
		text-decoration: none;
		white-space: nowrap;
	}
	.brand span {
		font-size: 23px;
		margin-left: 6px;
	}
	.scoreline {
		display: flex;
		align-items: center;
		gap: 18px;
		padding-left: 28px;
		border-left: 1px solid #d3d9c9;
	}
	.scoreline div {
		display: flex;
		align-items: center;
		gap: 12px;
	}
	.scoreline span {
		font-size: 17px;
		font-weight: 500;
	}
	.scoreline strong {
		font-size: 36px;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
		line-height: 1;
	}
	.divider {
		color: #78856d;
	}
	.goal {
		font-size: 15px;
		color: #596952;
		white-space: nowrap;
	}
	@media (max-width: 1050px) {
		.goal {
			display: none;
		}
		.score {
			gap: 18px;
		}
		.scoreline {
			gap: 12px;
			padding-left: 18px;
		}
	}
	@media (max-width: 700px) {
		.brand {
			display: none;
		}
		.scoreline {
			width: 100%;
			justify-content: space-between;
		}
		.score {
			width: 100%;
			flex-wrap: wrap;
		}
		.brand {
			font-size: 32px;
		}
		.scoreline {
			border: 0;
			margin-left: auto;
			padding: 0;
			gap: 10px;
		}
		.scoreline div {
			gap: 8px;
		}
		.scoreline span {
			font-size: 15px;
		}
		.scoreline strong {
			font-size: 30px;
		}
	}
	@media (max-width: 420px) {
		.brand {
			font-size: 25px;
		}
		.brand span {
			font-size: 16px;
			margin-left: 3px;
		}
		.score {
			gap: 8px;
		}
		.scoreline {
			gap: 7px;
		}
		.scoreline div {
			gap: 5px;
			flex-direction: row;
		}
		.scoreline span {
			font-size: 13px;
		}
		.scoreline strong {
			font-size: 25px;
		}
	}
	@media (max-width: 350px) {
		.brand {
			font-size: 20px;
		}
		.brand span {
			display: none;
		}
		.scoreline span {
			font-size: 12px;
		}
		.scoreline {
			gap: 5px;
		}
	}
	@container game (max-width: 1100px) {
		.goal {
			display: none;
		}
		.score {
			gap: 16px;
		}
		.brand {
			font-size: 30px;
		}
		.scoreline {
			gap: 12px;
			padding-left: 16px;
		}
		.scoreline div {
			gap: 8px;
		}
	}
	@container game (max-width: 850px) {
		.score {
			width: 100%;
			flex: auto;
		}
		.scoreline {
			margin-left: auto;
		}
	}
	@container game (max-width: 700px) {
		.brand {
			display: none;
		}
		.scoreline {
			width: 100%;
			border: 0;
			padding: 0;
			justify-content: space-between;
		}
	}
	@media (orientation: landscape) and (max-height: 500px) and (min-width: 600px) {
		.score {
			width: auto;
			flex: 1;
		}
		.scoreline {
			gap: 8px;
			justify-content: center;
		}
	}
</style>
