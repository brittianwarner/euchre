<!--
  Table.svelte — the felt plus its rail: two meshes, two draw calls, the
  whole "kitchen table at night" read.

  docs/04-FRONTEND-UX.md §6.4 budgets "Felt · rail · dealer button · contact
  shadows" at 4 draws combined; the dealer button and contact shadows are
  drawn elsewhere (button by the card layer, shadows by `Lights.svelte`), so
  this component's job is to stay at exactly 2: one `CircleGeometry` felt
  (`Felt.svelte`) and one `TorusGeometry` rail, both cheap enough that this
  is a rounding error against the card atlas' texture and draw-call budget.

  The rail sits a hair proud of the felt radius so the seam between the two
  materials is hidden by the tube's own curvature rather than needing a
  third trim mesh.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import Felt from './Felt.svelte';

	interface Props {
		/** Felt radius, world metres — also the value `SeatAnchors` should be told about if it differs from the default. */
		radius?: number;
		/** World Y of the felt surface; the rail centres slightly below it to read as an edge, not a hoop floating over the cloth. */
		y?: number;
		/** Rail tube thickness, world metres. */
		railTube?: number;
		/** Leather/wood rail colour. */
		railColor?: string;
	}

	let { radius = 0.45, y = 0, railTube = 0.028, railColor = '#3c2a1e' }: Props = $props();

	// Slight overlap with the felt edge hides the seam in the tube's curvature.
	const railCenterRadius = $derived(radius + railTube * 0.6);
</script>

<Felt {radius} {y} />

<T.Mesh position={[0, y - railTube * 0.35, 0]} rotation={[-Math.PI / 2, 0, 0]}>
	<T.TorusGeometry args={[railCenterRadius, railTube, 16, 96]} />
	<T.MeshStandardMaterial color={railColor} roughness={0.55} metalness={0.06} />
</T.Mesh>
