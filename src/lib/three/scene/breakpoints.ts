/**
 * `breakpoints.ts` — the one place "portrait" is defined for the 3D scene.
 *
 * `CameraRig` and `SeatAnchors` each read the live canvas aspect ratio and
 * must agree on when a phone has flipped into portrait: a camera that pulls
 * back for portrait while the seat ring stays at the landscape radius (or
 * vice versa) desyncs the framing from the hand it is meant to fit. Both
 * components derive `portrait` from this single threshold so they can never
 * drift apart. See docs/04-FRONTEND-UX.md §14.
 */

/** Below this aspect (width / height) the table is framed for a portrait phone. */
export const PORTRAIT_ASPECT_THRESHOLD = 0.8;

/** Below this aspect (and at/above the portrait threshold) the table is a landscape phone/small tablet. */
export const NARROW_ASPECT_THRESHOLD = 1.4;

export function isPortrait(aspect: number): boolean {
	return aspect < PORTRAIT_ASPECT_THRESHOLD;
}

export function isNarrowLandscape(aspect: number): boolean {
	return !isPortrait(aspect) && aspect < NARROW_ASPECT_THRESHOLD;
}
