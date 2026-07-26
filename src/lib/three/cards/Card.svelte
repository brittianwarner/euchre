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

	/**
	 * What the front mesh's material shows: the face when face-up and known,
	 * the back otherwise (face-down, or face-up but the id/texture isn't
	 * available yet — matching the pre-existing fallback semantics).
	 */
	const frontMap = $derived(faceUp && face !== null ? face : back);

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
			One persistent `MeshStandardMaterial`, never destroyed and recreated.

			This used to be a `{#key}` block that rebuilt the material whenever the
			texture identity changed, on the theory that three.js compiles the
			shader for the material it is given and won't notice a later `.map`
			assignment. That is not what actually caused the blank card: three.js's
			own program-cache key already accounts for whether `map` is set, so it
			recompiles on the next render regardless. What the rebuild *did* cause is
			worse — a genuine race. Destroying the old `<T.MeshStandardMaterial>`
			and mounting a new one means Threlte has to run a fresh
			attach-to-parent-mesh effect before three.js has anything to render for
			that mesh; on-demand rendering (`renderMode="on-demand"`) can and does
			paint a real frame in the gap where the mesh's `.material` is stale or
			absent, and if that happens to be the last frame rendered before the
			frame loop goes idle again, the card is stuck showing whatever
			three.js's default (a plain white `MeshBasicMaterial`) looks like —
			forever, since nothing re-invalidates on its own afterward. This was
			verified directly: instrumented logging showed the bound mesh's
			`material.map` still `undefined` several real rendered frames after the
			id resolved to its final value, on the up-card specifically (id arrives
			late, after the initial `null` paint — hand cards are dealt with their
			ids already known, which is why they never showed it).

			The fix is to never destroy the material at all. `map` (and every other
			prop below) is simply reactive: `frontMap`/`back` change, Threlte's
			ordinary prop-diffing (`useProps`) sets `.map` on the *same* material
			instance and calls `invalidate()`, and there is no attach step left to
			race because the material was attached once, at mount, and never torn
			down. `alphaTest` drops to `0` when there is no map at all (the
			both-unavailable edge case, effectively SSR/first-paint-before-any-canvas
			-only) so the plain fallback `color` shows solid instead of being
			discarded — the same "never invisible" guarantee the old `{:else}`
			branch gave, without needing a third material variant to do it.
		-->
		<T.MeshStandardMaterial
			map={frontMap}
			transparent
			alphaTest={frontMap !== null ? 0.5 : 0}
			side={DoubleSide}
			color={frontMap !== null ? tint : '#1d5b3a'}
			roughness={0.62}
			metalness={0}
		/>
	</T.Mesh>

	<!--
		The reverse. Offset by a hair so the two quads never z-fight, and rotated a
		half turn so the art is the right way up when the card is flipped. Only
		meaningful once the card is genuinely showing its face (not the
		face-down-fallback case, where the front mesh is already displaying the
		back and a second back-only mesh behind it would be redundant) — but it
		mounts once and stays mounted for as long as that stays true, same
		persistent-material reasoning as the front mesh above.
	-->
	{#if faceUp && face !== null}
		<T.Mesh position={[0, 0, -0.002]} rotation={[0, Math.PI, 0]} castShadow>
			<T.PlaneGeometry args={[width, height]} />
			<T.MeshStandardMaterial
				map={back}
				transparent
				alphaTest={back !== null ? 0.5 : 0}
				color={back !== null ? tint : '#1d5b3a'}
				roughness={0.62}
				metalness={0}
			/>
		</T.Mesh>
	{/if}
</T.Group>
