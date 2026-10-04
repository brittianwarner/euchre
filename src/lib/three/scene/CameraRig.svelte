<script lang="ts">
	import { T, useThrelte } from '@threlte/core';
	const { size } = useThrelte();
	// A restrained overhead perspective: the whole table fits inside its own
	// viewport, leaving the HUD and decision dock independent of 3D projection.
	const aspect = $derived($size.width / Math.max(1, $size.height));
	const span = $derived(Math.max(0.9, 1.08 / Math.max(0.1, aspect)));
</script>

<T.OrthographicCamera
	makeDefault
	zoom={Math.max(1, $size.height) / span}
	near={0.01}
	far={8}
	position={[0, 1.4, 0.82]}
	rotation={[-Math.atan2(1.4, 0.8), 0, 0]}
/>
