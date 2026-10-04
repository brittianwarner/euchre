<!-- Shared Blender card stock with persistent face/back materials. Hidden hands never resolve a face texture. -->
<script lang="ts">
	import { T } from '@threlte/core';
	import { useGltf } from '@threlte/extras';
	import { DoubleSide, type Mesh } from 'three';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import { backTexture, faceTexture } from './cardTexture';
	import { cardShadow } from './shadowTexture';
	import { CARD_ASPECT } from './faces';
	import type { CardFaceId } from './faces';

	const stock = useGltf('/art/card-stock.glb');
	const shadow = cardShadow();
	interface Props {
		/** Which card. Ignored — and never loaded — when `faceUp` is false. */
		id?: CardFaceId | null;
		faceUp?: boolean;
		position?: [number, number, number];
		/** Euler XYZ, radians. */
		rotation?: [number, number, number];
		/** Card height in world units; width follows the poker aspect ratio. */
		height?: number;
		/** Four-colour deck, for colour-blind players. */
		fourColor?: boolean;
		/** Dim the card — used for "you cannot legally play this". */
		dimmed?: boolean;
		/** Raise slightly and brighten, for hover / selected. */
		highlighted?: boolean;
		/** Pointer events. Only wired for cards the player may actually act on. */
		interactive?: boolean;
		onselect?: (id: CardFaceId) => void;
		onhover?: (id: CardFaceId | null) => void;
	}

	let {
		id = null,
		faceUp = true,
		position = [0, 0, 0],
		rotation = [0, 0, 0],
		height = 1,
		fourColor = false,
		dimmed = false,
		highlighted = false,
		interactive = false,
		onselect,
		onhover
	}: Props = $props();

	const width = $derived(height * CARD_ASPECT);

	// Only resolve the face texture when the card is actually face up.
	const face = $derived(faceUp && id !== null ? faceTexture(id, fourColor) : null);
	const back = $derived(backTexture());

	/**
	 * What the front mesh's material shows: the face when face-up and known,
	 * the back otherwise (face-down, or face-up but the id/texture isn't
	 * available yet — matching the pre-existing fallback semantics).
	 */
	const frontMap = $derived(faceUp && face !== null ? face : back);

	// Highlight lifts the card toward the viewer rather than scaling it, so a
	// fanned hand keeps its spacing and nothing jumps under the pointer.
	//
	// Tweened rather than snapped: an instant jump under the cursor reads as a
	// glitch, and with cards overlapping it makes the fan feel twitchy as the
	// pointer crosses boundaries. 140ms with an ease-out is long enough to read as
	// motion and short enough to feel immediate.
	const lift = new Tween(0, { duration: 140, easing: cubicOut });
	$effect(() => {
		lift.target = highlighted ? height * 0.16 : 0;
	});
	const tint = $derived(dimmed ? '#b9bdb6' : '#ffffff');

	let mesh = $state<Mesh | undefined>(undefined);

	function handleClick(e: { stopPropagation?: () => void }): void {
		if (!interactive || id === null) return;
		e.stopPropagation?.();
		onselect?.(id);
	}
</script>

<T.Group
	position={[position[0], position[1] + lift.current, position[2]]}
	rotation={[rotation[0], rotation[1], rotation[2]]}
>
	<T.Mesh position={[height * 0.016, -height * 0.02, -height * 0.012]}>
		<T.PlaneGeometry args={[width * 1.15, height * 1.12]} />
		<T.MeshBasicMaterial map={shadow} transparent opacity={0.22} depthWrite={false} />
	</T.Mesh>
	{#if $stock}
		<T.Mesh
			geometry={$stock.nodes.CardStock.geometry}
			scale={height}
			position={[0, 0, -height * 0.004]}
		>
			<T.MeshStandardMaterial color="#eee9da" roughness={0.72} />
		</T.Mesh>
	{/if}

	<T.Mesh
		bind:ref={mesh}
		position={[0, 0, height * 0.0001]}
		castShadow
		receiveShadow
		onclick={handleClick}
		onpointerenter={() => interactive && onhover?.(id)}
		onpointerleave={() => interactive && onhover?.(null)}
	>
		<T.PlaneGeometry args={[width, height]} />

		<T.MeshStandardMaterial
			map={frontMap}
			emissive="#ffffff"
			emissiveMap={frontMap}
			emissiveIntensity={dimmed ? 0.02 : 0.18}
			transparent
			alphaTest={frontMap !== null ? 0.5 : 0}
			side={DoubleSide}
			color={frontMap !== null ? tint : '#1d5b3a'}
			roughness={0.8}
			metalness={0}
		/>
	</T.Mesh>

	{#if faceUp && face !== null}
		<T.Mesh position={[0, 0, -height * 0.008]} rotation={[0, Math.PI, 0]} castShadow>
			<T.PlaneGeometry args={[width, height]} />
			<T.MeshStandardMaterial
				map={back}
				transparent
				alphaTest={back !== null ? 0.5 : 0}
				color={back !== null ? tint : '#1d5b3a'}
				roughness={0.8}
				metalness={0}
			/>
		</T.Mesh>
	{/if}
</T.Group>
