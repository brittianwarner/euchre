<!--
  CameraRig.svelte — the one `<T.PerspectiveCamera>` for the table.

  Framing is entirely `$derived` from the live canvas aspect ratio — no
  `$effect`, per the app-wide rule (docs/04-FRONTEND-UX.md §1). Three
  breakpoints, matching §14:

    portrait (< 0.8)         fov 44, dist 1.05  — pulled back, wide, tilted down more
    narrow landscape (< 1.4) fov 38, dist 0.85
    wide landscape            fov 34, dist 0.72

  One deliberate deviation from the §14 code sample: instead of an
  `oncreate={(ref) => ref.lookAt(...)}` one-shot, the camera's pitch is
  computed as a closed-form `$derived` (`rotation.x` via `atan2`) from its
  position and look-at target. `oncreate` only runs once at object
  creation — it does not rerun when `position` changes reactively, so a
  static `lookAt` call would leave the camera pointed at a stale target the
  next time the aspect ratio changes (a resize, or a phone rotation) without
  a full remount. Because this rig only ever pitches (every seat sits on the
  Z axis symmetry line, so there is never a yaw or roll component), the
  closed form is exact and — unlike an imperative call — recomputes for
  free every time `fov`/`dist` change, satisfying "reframe on a responsive
  breakpoint, using `$derived`, not an effect" for *every* frame it matters,
  not only the first.

  The look-at target itself also shifts a little with `portrait`: pulled
  back and wider, the rig tilts down slightly more so the trick zone at
  table centre stays inside the frame alongside the human hand — the
  "tilted" half of the brief, not just "pulled back / narrower fov".

  Tone mapping and colour space: set explicitly here via `useThrelte()`'s
  renderer, in addition to (and redundant with) whatever the hosting
  `<Canvas>` passes as `toneMapping`/`colorSpace` props. This component is
  mounted deep enough in the tree that it cannot itself own the `<Canvas>`
  element, but the renderer instance is a singleton for the app's lifetime,
  so setting it once here — the same one-time, non-reactive pattern the
  reference app uses for its environment map setup — guarantees the values
  are correct even if the integration layer's `<Canvas>` omits them.
  `NeutralToneMapping` is mandatory, not a preference: Threlte's default
  `AgXToneMapping` desaturates the felt green and pushes card reds toward
  brick (docs/04-FRONTEND-UX.md §6.3).
-->
<script lang="ts">
	import { T, useThrelte } from '@threlte/core';
	import { onMount } from 'svelte';
	import { NeutralToneMapping, SRGBColorSpace } from 'three';
	import { isNarrowLandscape, isPortrait } from './breakpoints';

	interface Props {
		near?: number;
		far?: number;
		/** Field of view, degrees, at each breakpoint. */
		fovPortrait?: number;
		fovNarrow?: number;
		fovWide?: number;
		/** Camera distance factor at each breakpoint (world metres, scaled internally). */
		distPortrait?: number;
		distNarrow?: number;
		distWide?: number;
	}

	let {
		near = 0.05,
		far = 6,
		fovPortrait = 50,
		fovNarrow = 46,
		fovWide = 42,
		distPortrait = 1.25,
		distNarrow = 1.1,
		distWide = 1.0
	}: Props = $props();

	const { size, renderer } = useThrelte();

	const aspect = $derived($size.width / Math.max(1, $size.height));
	const portrait = $derived(isPortrait(aspect));
	const narrow = $derived(isNarrowLandscape(aspect));

	const fov = $derived(portrait ? fovPortrait : narrow ? fovNarrow : fovWide);
	const dist = $derived(portrait ? distPortrait : narrow ? distNarrow : distWide);

	// Framing is arithmetic, not taste. The seat-0 hand sits at z = +SEAT_RADIUS
	// (0.3 m). With the previous rig — posY 0.78d, posZ 0.82d, fov 34, target
	// z -0.06 — the hand fell 21.9 deg off the camera axis against a 17 deg
	// half-FOV, i.e. just below the bottom edge: the player could not see their
	// own cards at all. These ratios put the hand ~13 deg off axis and North's
	// fan ~9 deg, so both near and far edges sit comfortably inside frame.
	const posY = $derived(dist * 0.62);
	const posZ = $derived(dist * 0.78);

	// Pulled back and wide in portrait, so tilt a little further down to keep
	// the trick zone in frame alongside the hand.
	const targetY = $derived(portrait ? -0.02 : 0);
	// Aim at (or just past) the table centre rather than beyond it: pushing the
	// target away from the player is what tipped the near edge out of frame.
	const targetZ = $derived(portrait ? 0 : 0.02);

	// Closed-form pitch: three.js's default camera forward is local -Z; after
	// rotating `θ` about X, that forward direction becomes (0, sinθ, -cosθ).
	// Solving for θ against the (y, z) look direction gives atan2(dy, -dz).
	const dy = $derived(targetY - posY);
	const dz = $derived(targetZ - posZ);
	const pitch = $derived(Math.atan2(dy, -dz));

	onMount(() => {
		renderer.toneMapping = NeutralToneMapping;
		renderer.outputColorSpace = SRGBColorSpace;
	});
</script>

<T.PerspectiveCamera
	makeDefault
	{fov}
	{near}
	{far}
	position={[0, posY, posZ]}
	rotation={[pitch, 0, 0]}
/>
