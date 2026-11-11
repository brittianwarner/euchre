<script lang="ts">
	let { score }: { score: number } = $props();
	const six = [
		[10, 13],
		[24, 13],
		[10, 24],
		[24, 24],
		[10, 35],
		[24, 35]
	];
	const four = [
		[10, 13],
		[24, 13],
		[10, 35],
		[24, 35]
	];
</script>

<svg viewBox="0 0 78 50" role="img" aria-label={`${score} points, counted with a six and a four`}>
	{#each [6, 4] as rank, index (rank)}<g transform={`translate(${index * 40},0)`}>
			<rect x="0.5" y="0.5" width="35" height="48" rx="4" fill="#fffdf6" stroke="#99a58a" />
			<text x="4" y="10" font-size="9" fill={index === 0 ? '#aa342e' : '#243e32'}>{rank}</text>
			{#each index === 0 ? six : four as point, i (i)}<text
					x={point[0]}
					y={point[1] + 4}
					text-anchor="middle"
					font-size="13"
					fill={index === 0 ? '#aa342e' : '#243e32'}
					opacity={i < (index === 0 ? Math.min(score, 6) : Math.max(0, score - 6)) ? 1 : 0.08}
					>{index === 0 ? '♥' : '♠'}</text
				>{/each}
		</g>{/each}
</svg>

<style>
	svg {
		width: 72px;
		height: 46px;
		flex-shrink: 0;
	}
	@media (max-width: 700px) {
		svg {
			width: 42px;
			height: 28px;
		}
	}
</style>
