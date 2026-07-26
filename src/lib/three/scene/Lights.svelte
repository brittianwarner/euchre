<!--
  Lights.svelte — key + fill + a single baked contact shadow.

  Shadow-map budget: **zero**. Every light below has `castShadow={false}`.
  A real-time directional shadow map re-rendered every frame is, per
  docs/04-FRONTEND-UX.md §5, "the most expensive thing a card table can
  do" — this scene ships none. The only shadow anywhere is `ContactShadows`'
  single baked blob, and it is *not* on Threlte's per-frame render loop:
  `frames={0}` means its internal `useTask` never fires on its own (the
  running-condition is `count < frames`, i.e. `0 < 0`, always false). It
  renders exactly once per call to the exported `refreshShadows()` — one
  depth pass over the scene plus two small Gaussian blur passes, all onto a
  512×512 offscreen target — and stays static between calls. Steady-state
  per-frame cost: **0 additional draw calls**. Cost per `refreshShadows()`
  call: 3 small render passes at 512², which is why the doc calls it
  "refresh() only when a card lands" rather than every frame.

  Composition, warm-table read:
    - one `AmbientLight`             — the soft, low, warm fill
    - one `DirectionalLight` (key)   — the "lamp over the table" source
    - one `DirectionalLight` (rim)   — low, cool counter-light so the far
                                        side of the felt doesn't go flat
    - `ContactShadows`                — the only shadow, baked on demand

  Call the exported `refreshShadows()` whenever a card (or anything else)
  settles onto the table — the director/table-store layer owns that timing,
  since this component has no notion of game events. An initial bake runs
  once on mount so the table isn't shadowless before the first such call,
  via `onMount` (a one-time, non-reactive setup — not the `$effect` this
  app reserves for exactly two other files).
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import { ContactShadows } from '@threlte/extras';
	import { onMount } from 'svelte';

	interface Props {
		fillIntensity?: number;
		fillColor?: string;
		keyIntensity?: number;
		keyColor?: string;
		keyPosition?: [number, number, number];
		rimIntensity?: number;
		rimColor?: string;
		rimPosition?: [number, number, number];
		/** Contact-shadow tuning — defaults match docs/04-FRONTEND-UX.md §5. */
		shadowOpacity?: number;
		shadowBlur?: number;
		shadowFar?: number;
		shadowScale?: number | [number, number];
		shadowResolution?: number;
		shadowColor?: string;
		/** World Y the shadow plane sits at — the felt surface. */
		shadowY?: number;
	}

	let {
		fillIntensity = 0.55,
		fillColor = '#e8dcc8',
		keyIntensity = 2.1,
		keyColor = '#fff2df',
		keyPosition = [1.2, 2.4, 1.0],
		rimIntensity = 0.32,
		rimColor = '#8fb0d8',
		rimPosition = [-1.4, 1.6, -1.2],
		shadowOpacity = 0.42,
		shadowBlur = 2.4,
		shadowFar = 0.12,
		shadowScale = 1.25,
		shadowResolution = 512,
		shadowColor = '#000000',
		shadowY = 0.0005
	}: Props = $props();

	// `ContactShadows` exports a plain `refresh()` function from its instance
	// (not a ref to the underlying Object3D) — captured via `bind:this` like
	// any other component-instance export.
	let shadows: { refresh: () => void } | undefined = $state();

	/** Bake the contact-shadow blob once, immediately. Cheap; see budget above. */
	export function refreshShadows(): void {
		shadows?.refresh();
	}

	onMount(() => {
		refreshShadows();
	});
</script>

<T.AmbientLight intensity={fillIntensity} color={fillColor} />

<T.DirectionalLight
	intensity={keyIntensity}
	color={keyColor}
	position={keyPosition}
	castShadow={false}
/>

<T.DirectionalLight
	intensity={rimIntensity}
	color={rimColor}
	position={rimPosition}
	castShadow={false}
/>

<ContactShadows
	bind:this={shadows}
	frames={0}
	scale={shadowScale}
	blur={shadowBlur}
	far={shadowFar}
	opacity={shadowOpacity}
	resolution={shadowResolution}
	color={shadowColor}
	position={[0, shadowY, 0]}
/>
