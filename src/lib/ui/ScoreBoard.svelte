<script lang="ts">
	import type { PublicGameView } from '#lib/protocol/index.ts';
	let { view }: { view: PublicGameView } = $props();
	const glyph = { S: '♠', H: '♥', D: '♦', C: '♣' };
</script>

<section class="score" aria-label="Match score" aria-live="polite">
	<a class="brand" href="/" aria-label="Euchre home">euchre<span aria-hidden="true">♣</span></a>
	<div class="scoreline">
		<div><span>US</span><strong>{view.score[0]}</strong></div>
		<span class="divider">:</span>
		<div><strong>{view.score[1]}</strong><span>THEM</span></div>
	</div>
	<div class="meta">
		<span>HAND <b>{view.handNo + 1}</b></span><span class="meta-divider"></span><span
			>{view.trump ? 'TRUMP' : 'FIRST TO'}
			<b class:red={view.trump === 'H' || view.trump === 'D'}
				>{view.trump ? glyph[view.trump] : '10'}</b
			></span
		>
	</div>
</section>

<style>
	.score {
		display: flex;
		align-items: center;
		gap: 42px;
		color: #233e32;
		min-width: 0;
		flex: 1;
		font-family: var(--font-sans);
	}
	.brand {
		font: 600 34px/1 var(--font-serif);
		letter-spacing: -0.075em;
		text-decoration: none;
		white-space: nowrap;
	}
	.brand span {
		font-size: 20px;
		margin-left: 5px;
	}
	.scoreline {
		display: flex;
		align-items: center;
		gap: 17px;
		padding-left: 35px;
		border-left: 1px solid #d9ded2;
	}
	.scoreline div {
		display: flex;
		gap: 13px;
		align-items: center;
	}
	.scoreline span {
		font-size: 12px;
		font-weight: 650;
		letter-spacing: 0.1em;
		color: #778271;
	}
	.scoreline strong {
		font-size: 32px;
		font-weight: 500;
		font-variant-numeric: tabular-nums;
		line-height: 1;
	}
	.scoreline .divider {
		font-size: 20px;
		color: #a6b19c;
	}
	.meta {
		margin-left: auto;
		display: flex;
		align-items: center;
		gap: 20px;
		font-size: 12px;
		letter-spacing: 0.12em;
		color: #778271;
	}
	.meta > span {
		display: flex;
		align-items: center;
		gap: 10px;
	}
	.meta b {
		font-size: 15px;
		color: #233e32;
		font-weight: 550;
	}
	.meta b.red {
		color: #b64e3c;
	}
	.meta .meta-divider {
		width: 1px;
		height: 20px;
		background: #d9ded2;
	}
	@media (max-width: 700px) {
		.score {
			gap: 20px;
			align-items: flex-start;
		}
		.brand {
			font-size: 27px;
		}
		.brand span {
			font-size: 16px;
		}
		.scoreline {
			padding-left: 18px;
			gap: 10px;
			height: 30px;
		}
		.scoreline div {
			gap: 7px;
		}
		.scoreline strong {
			font-size: 24px;
		}
		.scoreline span {
			font-size: 10px;
		}
		.meta {
			position: absolute;
			bottom: 12px;
			left: 20px;
			right: 20px;
			justify-content: center;
			gap: 18px;
		}
		.meta b {
			font-size: 13px;
		}
		.meta .meta-divider {
			height: 12px;
		}
	}
	@media (max-width: 380px) {
		.score {
			gap: 11px;
		}
		.scoreline {
			padding-left: 11px;
			gap: 7px;
		}
		.scoreline div {
			gap: 5px;
		}
		.scoreline div > span {
			position: absolute;
			width: 1px;
			height: 1px;
			clip: rect(0, 0, 0, 0);
			overflow: hidden;
		}
		.brand {
			font-size: 24px;
		}
	}
</style>
