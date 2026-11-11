/**
 * `seatLayout.ts` — where the four chairs are, and which way they face.
 *
 * Pure data and pure functions, deliberately not a `.svelte` file: the card
 * layer (owned by another agent) needs these numbers whether or not it ever
 * mounts `SeatAnchors.svelte` as a parent, so the maths lives somewhere it
 * can import without pulling in Threlte.
 *
 * Seat 0 (South) is always the human, nearest the camera. Seats 1–3 go
 * clockwise as seen from above (bird's-eye, +Y up): 1 = West (camera's
 * left), 2 = North (across the table), 3 = East (camera's right). Every
 * consumer — `SeatAnchors`, the hand-fan layout, HUD seat badges — reads a
 * seat's position from here, so "clockwise from South" is defined exactly
 * once for the whole app.
 *
 * Local-space convention for anything parented under a seat's `<T.Group>`:
 * the group's local **+Z axis points from the seat toward the table
 * centre**. A hand fan authored once — cards spread along local X, tilted
 * toward local +Z — is therefore correct at all four seats with zero
 * per-seat maths; only the group's own transform differs. This is exactly
 * the property docs/04-FRONTEND-UX.md §5 asks for: "portrait re-framing
 * changes only the camera and the seat radius, never per-card maths."
 */

import type { Seat } from '#lib/euchre/index.ts';

export type SeatLabel = 'south' | 'west' | 'north' | 'east';

export const SEAT_LABELS: Readonly<Record<Seat, SeatLabel>> = Object.freeze({
	0: 'south',
	1: 'west',
	2: 'north',
	3: 'east'
});

/** All four seats, South first, then clockwise — the canonical iteration order. */
export const SEAT_ORDER: readonly Seat[] = Object.freeze([0, 1, 2, 3]);

/** Distance from the felt centre to a seat anchor, world metres — landscape framing. */
export const SEAT_RADIUS_LANDSCAPE = 0.3;

/** Same, portrait framing (docs/04-FRONTEND-UX.md §14: "seat radius shrinks from 0.30 m to 0.26 m"). */
export const SEAT_RADIUS_PORTRAIT = 0.26;

/**
 * Unit direction `(x, z)` from the table centre to each seat, and the
 * y-axis rotation (radians) that turns a seat's local +Z axis to face back
 * toward the centre. Derived by hand once, not by a general formula — a
 * sign error in a formula is a much quieter bug than a wrong literal review
 * catches at a glance:
 *
 * ```
 *   seat  label   (x, z)     rotY     local +Z points toward (world)
 *   0     South   (0,  1)    π        (0, 0,-1)  — north, i.e. centre
 *   1     West    (-1, 0)    π/2      (1, 0, 0)  — east,  i.e. centre
 *   2     North   (0, -1)    0        (0, 0, 1)  — south, i.e. centre
 *   3     East    (1,  0)   -π/2      (-1,0, 0)  — west,  i.e. centre
 * ```
 *
 * (Three.js `rotateY(θ)` maps local `(0,0,1)` to world `(sinθ, 0, cosθ)`;
 * each row above is that formula evaluated at the listed `rotY`.)
 */
const SEAT_DIRECTION: Readonly<Record<Seat, readonly [x: number, z: number]>> = Object.freeze({
	0: [0, 1],
	1: [-1, 0],
	2: [0, -1],
	3: [1, 0]
});

const SEAT_ROTATION_Y: Readonly<Record<Seat, number>> = Object.freeze({
	0: Math.PI,
	1: Math.PI / 2,
	2: 0,
	3: -Math.PI / 2
});

export interface SeatTransform {
	readonly seat: Seat;
	readonly label: SeatLabel;
	/** World position of the seat anchor, metres. */
	readonly position: readonly [x: number, y: number, z: number];
	/** Euler XYZ, radians — only Y is ever non-zero. */
	readonly rotation: readonly [x: number, y: number, z: number];
}

export interface SeatLayoutOptions {
	/** Distance from centre to anchor, world metres. Default {@link SEAT_RADIUS_LANDSCAPE}. */
	radius?: number;
	/** Height of the anchor pivot above the felt surface, world metres. Default 0. */
	y?: number;
}

/** The transform for one seat. */
export function seatTransform(seat: Seat, opts: SeatLayoutOptions = {}): SeatTransform {
	const radius = opts.radius ?? SEAT_RADIUS_LANDSCAPE;
	const y = opts.y ?? 0;
	const [dx, dz] = SEAT_DIRECTION[seat];
	return {
		seat,
		label: SEAT_LABELS[seat],
		position: [dx * radius, y, dz * radius],
		rotation: [0, SEAT_ROTATION_Y[seat], 0]
	};
}

/** All four seat transforms, in {@link SEAT_ORDER}. */
export function seatLayout(opts: SeatLayoutOptions = {}): readonly SeatTransform[] {
	return SEAT_ORDER.map((seat) => seatTransform(seat, opts));
}

/** The seat radius appropriate for the given canvas aspect ratio (width / height). */
export function seatRadiusForAspect(aspect: number, portraitThreshold = 0.8): number {
	return aspect < portraitThreshold ? SEAT_RADIUS_PORTRAIT : SEAT_RADIUS_LANDSCAPE;
}
