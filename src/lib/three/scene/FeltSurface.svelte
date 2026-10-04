<script lang="ts">
	import { T, useThrelte } from '@threlte/core';
	import { feltTexture } from './feltTexture';
	const map = feltTexture(false);
	const { size } = useThrelte();
	const aspect = $derived($size.width / Math.max(1, $size.height));
</script>

<T.OrthographicCamera
	makeDefault
	position={[0, 0, 3]}
	zoom={$size.height / 2}
	near={0.1}
	far={10}
/>
<T.AmbientLight intensity={1.8} />
<T.PointLight position={[-0.8, 1.2, 2]} intensity={5} color="#fff1c9" />
<T.Mesh position={[0, 0, 0]}>
	<T.PlaneGeometry args={[2 * aspect, 2]} />
	<T.MeshStandardMaterial {map} color="#a4c49e" roughness={0.96} metalness={0} />
</T.Mesh>
