<!--
  Card.svelte — one playing card in the 3D scene.

  A card is two coplanar quads back to back: the face, and the back rotated a
  half turn. Rounded corners come from the texture's own alpha rather than from
  geometry, which keeps the mesh at four triangles and means the silhouette is
  exactly the drawn card with no seam to line up.

  `faceUp={false}` renders the shared back texture and, critically, does NOT
  build or reference the face texture at all — so an opponent's card cannot leak
  through the scene graph even if something upstream handed us an id it should
  not have.

  Position/rotation are driven entirely by props: this component owns no layout
  and no game logic. Animation is the caller's business, because the server is
  authoritative and animation must never own truth.
-->
<script lang="ts">
	import { T } from '@threlte/core';
	import { DoubleSide, type Mesh } from 'three';
	import { backTexture, faceTexture } from './cardTexture';
	import { CARD_ASPECT } from './faces';
	import type { CardFaceId } from './faces';

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

	// Highlight lifts the card toward the viewer rather than scaling it, so a
	// fanned hand keeps its spacing and nothing jumps under the pointer.
	const lift = $derived(highlighted ? height * 0.12 : 0);
	const tint = $derived(dimmed ? '#8a8a8a' : '#ffffff');

	let mesh = $state<Mesh | undefined>(undefined);

	function handleClick(e: { stopPropagation?: () => void }): void {
		if (!interactive || id === null) return;
		e.stopPropagation?.();
		onselect?.(id);
	}
</script>

<T.Group
	position={[position[0], position[1] + lift, position[2]]}
	rotation={[rotation[0], rotation[1], rotation[2]]}
>
	<!-- Face -->
	<T.Mesh
		bind:ref={mesh}
		castShadow
		receiveShadow
		onclick={handleClick}
		onpointerenter={() => interactive && onhover?.(id)}
		onpointerleave={() => interactive && onhover?.(null)}
	>
		<T.PlaneGeometry args={[width, height]} />
		<!--
			`{#key}` rebuilds the material whenever the texture identity changes.

			This is load-bearing, not defensive. A card's face texture is only known
			after the first server `sync`, so on the first paint `face` is null and the
			material is created with `map: null`. three.js compiles the shader for the
			material it was given; assigning `.map` afterwards does NOT recompile it
			unless `needsUpdate` is set, so the card renders as a blank white quad
			forever. Card BACKS never showed this because `backTexture()` resolves
			synchronously on first render, which is exactly why the bug looked like
			"faces are broken" rather than "late-arriving textures are broken".
		-->
		{#key faceUp && face !== null ? face : back}
			{#if faceUp && face !== null}
				<T.MeshStandardMaterial
					map={face}
					transparent
					alphaTest={0.5}
					side={DoubleSide}
					color={tint}
					roughness={0.62}
					metalness={0}
				/>
			{:else if back !== null}
				<T.MeshStandardMaterial
					map={back}
					transparent
					alphaTest={0.5}
					side={DoubleSide}
					color={tint}
					roughness={0.62}
					metalness={0}
				/>
			{:else}
				<!--
					Both textures unavailable. A mesh with NO material is not invisible —
					three.js substitutes a default white MeshBasicMaterial, which is
					exactly the blank white card this used to show. Rendering felt-green
					here means a texture failure degrades to something that disappears
					into the table instead of shouting at the player.
				-->
				<T.MeshStandardMaterial color="#1d5b3a" side={DoubleSide} roughness={0.9} />
			{/if}
		{/key}
	</T.Mesh>

	<!--
		The reverse. Offset by a hair so the two quads never z-fight, and rotated a
		half turn so the art is the right way up when the card is flipped.
	-->
	{#if faceUp && face !== null && back !== null}
		<T.Mesh position={[0, 0, -0.002]} rotation={[0, Math.PI, 0]} castShadow>
			<T.PlaneGeometry args={[width, height]} />
			{#key back}
				<T.MeshStandardMaterial
					map={back}
					transparent
					alphaTest={0.5}
					color={tint}
					roughness={0.62}
					metalness={0}
				/>
			{/key}
		</T.Mesh>
	{/if}
</T.Group>
