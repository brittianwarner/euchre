<script lang="ts">
	import { T } from '@threlte/core';
	import { useGltf } from '@threlte/extras';
	import Felt from './Felt.svelte';
	let { radius = 0.45, y = 0 }: { radius?: number; y?: number } = $props();
	const furniture = useGltf('/art/table.glb');
</script>

<Felt {radius} {y} />
{#if $furniture}
	<T.Group position={[0, y, 0]} scale={radius / 0.45}>
		<T is={$furniture.scene} />
	</T.Group>
{:else}
	<T.Mesh position={[0, y - 0.01, 0]} rotation={[-Math.PI / 2, 0, 0]}>
		<T.TorusGeometry args={[radius + 0.011, 0.025, 12, 96]} />
		<T.MeshStandardMaterial color="#102820" roughness={0.8} />
	</T.Mesh>
{/if}
