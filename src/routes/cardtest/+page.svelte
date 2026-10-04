<script lang="ts">
	import { Canvas, T } from '@threlte/core';
	import Card from '#lib/three/cards/Card.svelte';
	const hand = ['JS', 'AH', 'TD', 'QC', 'KS'] as const;
</script>

<div style="width:100vw;height:100vh;background:#123">
	<Canvas>
		<T.PerspectiveCamera makeDefault position={[0, 0, 5]} fov={50} />
		<T.AmbientLight intensity={1.2} />
		<T.DirectionalLight position={[3, 5, 4]} intensity={1.5} />
		{#each hand as id, i (id)}
			<Card {id} faceUp position={[(i - 2) * 1.15, 0.6, 0]} height={1.6} />
		{/each}
		<Card id="JS" faceUp={false} position={[-1.2, -1.3, 0]} height={1.6} />
		<Card id="AH" faceUp dimmed position={[0, -1.3, 0]} height={1.6} />
		<Card id="KS" faceUp highlighted position={[1.2, -1.3, 0]} height={1.6} />
	</Canvas>
</div>
