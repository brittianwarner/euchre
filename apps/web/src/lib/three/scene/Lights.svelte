<script lang="ts">
	import { T } from '@threlte/core';
</script>

<T.HemisphereLight skyColor="#fffaf0" groundColor="#586e60" intensity={2.0} />
<T.DirectionalLight
	intensity={2.4}
	color="#fff3df"
	position={[-0.6, 1.8, 0.9]}
	castShadow
	shadow.mapSize={[1024, 1024]}
	shadow.bias={-0.00015}
	shadow.camera.left={-0.65}
	shadow.camera.right={0.65}
	shadow.camera.top={0.65}
	shadow.camera.bottom={-0.65}
	shadow.camera.near={0.1}
	shadow.camera.far={4}
/>
<T.DirectionalLight intensity={0.65} color="#d8e8f5" position={[2, 1, -1]} />
