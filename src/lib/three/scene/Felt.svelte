<!--
  Felt.svelte — the playing surface: one flat, round mesh.

  This is deliberately the cheapest possible "felt": a single `CircleGeometry`
  with a procedural, module-cached texture (see `feltTexture.ts`) and a plain
  `MeshStandardMaterial` — no normal map, no environment map, no second
  material for an edge. `Table.svelte` adds the rail on top of this; nothing
  here draws more than once.

  High roughness (cloth, not lacquer) and zero metalness keep the key light
  from throwing a hot specular blob onto the felt — see Lights.svelte for the
  light this is tuned against.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import { DoubleSide } from 'three';
	import { feltTexture } from './feltTexture';

	interface Props {
		/** Felt radius, world metres. */
		radius?: number;
		/** Radial segment count — 64 reads as a smooth circle at any camera distance we ship. */
		segments?: number;
		/** World Y of the felt surface. */
		y?: number;
	}

	let { radius = 0.45, segments = 64, y = 0 }: Props = $props();

	const map = $derived(feltTexture());
</script>

<T.Mesh receiveShadow position={[0, y, 0]} rotation={[-Math.PI / 2, 0, 0]}>
	<T.CircleGeometry args={[radius, segments]} />
	<T.MeshStandardMaterial {map} color="#ffffff" roughness={0.95} metalness={0} side={DoubleSide} />
</T.Mesh>
