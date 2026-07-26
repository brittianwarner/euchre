/**
 * `scene/index.ts` — barrel for the table furniture layer.
 *
 * Owned here: the felt, the rail, the four seat anchors, the lights and
 * contact shadow, and the camera rig. Nothing in this directory renders a
 * card, a hand, the kitty, or the trick zone — that is the card layer,
 * composed on top of `SeatAnchors`' snippets.
 */

export { default as Table } from './Table.svelte';
export { default as Felt } from './Felt.svelte';
export { default as SeatAnchors } from './SeatAnchors.svelte';
export { default as Lights } from './Lights.svelte';
export { default as CameraRig } from './CameraRig.svelte';

export {
	SEAT_LABELS,
	SEAT_ORDER,
	SEAT_RADIUS_LANDSCAPE,
	SEAT_RADIUS_PORTRAIT,
	seatLayout,
	seatTransform,
	seatRadiusForAspect,
	type SeatLabel,
	type SeatTransform,
	type SeatLayoutOptions
} from './seatLayout';

export {
	PORTRAIT_ASPECT_THRESHOLD,
	NARROW_ASPECT_THRESHOLD,
	isPortrait,
	isNarrowLandscape
} from './breakpoints';

export { feltTexture, disposeFeltTexture } from './feltTexture';
