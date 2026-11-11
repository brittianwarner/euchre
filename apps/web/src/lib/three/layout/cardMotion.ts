/**
 * `cardMotion.ts` — pure interpolation helpers for the choreography layer.
 *
 * Numbers in, numbers out, exactly like `layout.ts` (no Threlte, no three.js,
 * no clock, no randomness) — that split is what keeps this file unit-testable
 * without a renderer and keeps the animating components themselves small.
 * Every caller drives the actual clock itself, normally with a `Tween<number>`
 * from `svelte/motion` holding a `0..1` progress value (see `Hand.svelte`,
 * `OpponentHand.svelte` and `TrickPile.svelte`): this module turns that single
 * number into a `CardPose`.
 *
 * `Card.svelte` is not owned by this layer and exposes no ref a `useTask` could
 * mutate imperatively — it only ever reads `position`/`rotation` as ordinary
 * props (see its own doc comment: "position/rotation are driven entirely by
 * props"). So a flight here is not the `Map<string, CardMotion>` + single
 * `useTask` shape `docs/04-FRONTEND-UX.md` §9.3 sketches for a component that
 * *does* own its meshes — it is one small `Tween<number>` per card that is
 * actually in flight (never more than a hand's worth, briefly), which is
 * exactly the pattern `Card.svelte` already uses for its own hover `lift`.
 */

import type { CardPose } from './layout';

/** Linear interpolation between two 3-tuples. */
export function lerp3(
	a: readonly [number, number, number],
	b: readonly [number, number, number],
	t: number
): [number, number, number] {
	return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

/** `height` at `t = 0.5`, `0` at both ends — the mid-flight arc every card play/deal/sweep uses. */
export function arcLift(t: number, height: number): number {
	return height * 4 * t * (1 - t);
}

/**
 * Interpolates position and rotation between two poses, with an added vertical
 * arc that peaks at the flight's midpoint. `t` is expected `0..1`; values
 * outside that range are not clamped (a caller driving `t` from an eased
 * `Tween` may briefly overshoot with certain easings, and clamping here would
 * fight that on purpose).
 */
export function lerpPose(from: CardPose, to: CardPose, t: number, arcHeight = 0): CardPose {
	const position = lerp3(from.position, to.position, t);
	position[1] += arcLift(t, arcHeight);
	return { position, rotation: lerp3(from.rotation, to.rotation, t) };
}

/**
 * Where a dealt card starts its flight from, given the fan pose it is headed
 * to: pulled in toward the table's centre and lifted slightly, so it reads as
 * "arriving from the deck" rather than materialising in place. Anchor-local
 * space only (see `layout.ts`'s module doc, family 1) — `+Z` already points
 * toward the table centre for every seat, which is what makes this formula
 * seat-agnostic.
 */
export function dealOrigin(to: CardPose): CardPose {
	return {
		position: [to.position[0] * 0.2, to.position[1] + 0.06, to.position[2] + 0.22],
		rotation: to.rotation
	};
}

/**
 * Where a played card starts its flight from, given the resting `trickCardPose`
 * it is headed to: further out along the same seat direction, i.e. back toward
 * where that seat's hand actually sits. Table space (see `layout.ts`'s module
 * doc, family 2) — `trickCardPose`'s own `position` already encodes the unit
 * direction toward `seat`, so scaling it up is exactly "further from centre,
 * toward that seat."
 */
export function playOrigin(to: CardPose, seatOutset = 0.16): CardPose {
	const [x, y, z] = to.position;
	const len = Math.hypot(x, z) || 1;
	const scale = (len + seatOutset) / len;
	return { position: [x * scale, y, z * scale], rotation: to.rotation };
}
