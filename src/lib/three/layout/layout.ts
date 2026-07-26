/**
 * `layout.ts` — pure geometry for the euchre table's card arrangements.
 *
 * Everything here is a function of numbers in, numbers out: no Threlte import,
 * no DOM, no `three` import, no randomness, no clock. `Hand.svelte`,
 * `OpponentHand.svelte`, `TrickPile.svelte` and `Kitty.svelte` are the only
 * consumers; they hand these results straight to `Card.svelte`'s `position` /
 * `rotation` props. That split is what makes the geometry unit-testable without
 * a renderer and keeps the components themselves tiny and declarative.
 *
 * ## Coordinate conventions
 *
 * Three families of pose live here, and each picks the convention that makes it
 * simplest — they are never mixed:
 *
 * 1. **Fan-local space** ({@link fanPositions}, {@link stackPositions}). Used by
 *    `Hand.svelte` and `OpponentHand.svelte`, which render *inside* a seat's
 *    `SeatAnchor` — an `Object3D` the scene-graph owner has already placed at
 *    the seat's position and rotated so that its local `+Z` axis points across
 *    the table, away from the seat, and its local `+X` axis points to the
 *    seat's own right. Because the anchor already carries the seat's identity,
 *    the fan math below is **identical for all four seats** — that is the
 *    point of anchor-local space (`docs/04-FRONTEND-UX.md` §5): re-framing the
 *    camera for portrait never touches per-card math, and neither does which
 *    chair a hand belongs to.
 *
 * 2. **Table space** ({@link trickCardPose}, `SEAT_DIRECTION`,
 *    `SEAT_FACING_RAD`). Used by `TrickPile.svelte`, which is *not* nested
 *    inside any one seat's anchor — it sits at the felt's centre and needs to
 *    know, in the shared table frame, which direction each of the four seats
 *    is in. World `+Z` is toward the camera (south, seat `0`, the human);
 *    world `+X` is screen-right (east, seat `3`); world `Y` is up. This matches
 *    `Rig.svelte`'s camera, which sits on `+Z` looking toward `-Z`.
 *
 * 3. **Card orientation.** A card lying flat and face-up on the felt has its
 *    face normal pointing straight up (`+Y`). `Card.svelte`'s `<T.Group>`
 *    renders with three.js's default Euler order (`'XYZ'`), and under that
 *    order a rotation of `[-π/2, 0, twist]` is the one combination that (a)
 *    lies the card flat, face up, and (b) lets `twist` spin the card freely in
 *    the table plane *without ever tilting the face normal* — because in
 *    `'XYZ'` order the `z` component is applied to the mesh's own normal
 *    *before* the `x` tilt, and rotating a vector around its own axis is a
 *    no-op. Introducing a nonzero `rotation.y` does **not** have this property
 *    (it tangles with the `x` tilt and lifts the card onto its edge), which is
 *    why nothing below ever sets it. This was verified numerically against
 *    `three`'s own `Euler`/`Matrix4` (not just derived on paper) before being
 *    relied on here; see the module's test file for the load-bearing case.
 *
 *    A `twist` of `0` always means "face-up, oriented the way a document lying
 *    on a desk in front of the camera would be" — legible from the standard
 *    elevated camera in `Rig.svelte` regardless of which seat's anchor a card
 *    sits in. `reclineDeg` (fan options) tilts the *whole* fan back toward the
 *    camera from that flat rest pose, for legibility, without disturbing the
 *    per-card twist math.
 */

import { CARD_ASPECT } from '$lib/three/cards/faces';
import type { Seat } from '$lib/euchre';

export { CARD_ASPECT };

/** A pose in whichever local space its producing function documents. Matches `Card.svelte`'s own prop shapes 1:1. */
export interface CardPose {
	readonly position: readonly [number, number, number];
	readonly rotation: readonly [number, number, number];
}

const DEG = Math.PI / 180;

/* ========================================================================== */
/* 1. CSS-pixel affordance sizing — the legibility floor                      */
/* ========================================================================== */

/** WCAG 2.2 AAA-adjacent floor for the exposed strip of a fanned card, in CSS px. */
export const MIN_STRIP_W = 48;
export const MIN_STRIP_H = 64;

export interface FanSizing {
	readonly cardWpx: number;
	readonly cardHpx: number;
	readonly overlap: number;
}

/**
 * The largest card width whose fan spans `availablePx`, with `overlap` relaxed
 * toward `0` until the exposed (non-overlapped) strip clears
 * {@link MIN_STRIP_W} × {@link MIN_STRIP_H}.
 *
 * This is the affordance floor from `docs/04-FRONTEND-UX.md` §14: "someone with
 * reading glasses on a phone must never squint or mis-tap." It is deliberately
 * a search rather than a closed form, because the floor is a *pair* of
 * constraints (strip width **and** strip height) and relaxing overlap trades
 * one against the other non-linearly once the aspect ratio is involved.
 *
 * `n <= 0` returns a degenerate, harmless zero-size result — callers never
 * render a fan with no cards, but a defensive caller should not have to guard
 * against a crash if they briefly do.
 */
export function sizeFan(availablePx: number, n: number, preferredOverlap = 0.3): FanSizing {
	if (n <= 0) return { cardWpx: 0, cardHpx: 0, overlap: preferredOverlap };
	for (let overlap = preferredOverlap; overlap >= 0; overlap -= 0.05) {
		const k = 1 - overlap;
		const cardWpx = availablePx / ((n - 1) * k + 1);
		const strip = cardWpx * k;
		if (strip >= MIN_STRIP_W && cardWpx / CARD_ASPECT >= MIN_STRIP_H) {
			return { cardWpx, cardHpx: cardWpx / CARD_ASPECT, overlap };
		}
	}
	const cardWpx = availablePx / n; // overlap 0: the widest possible strip
	return { cardWpx, cardHpx: cardWpx / CARD_ASPECT, overlap: 0 };
}

/**
 * CSS pixels per world metre at `depth` metres from the camera, for a vertical
 * field of view `fovDeg`. Lets a caller convert a `sizeFan` result (CSS px) into
 * the `cardHeight` metres that `fanPositions` and `Card.svelte` want, using the
 * live camera's `fov` and distance rather than a hard-coded scale.
 */
export function pxPerMetreAt(viewportHeightPx: number, depth: number, fovDeg: number): number {
	const fovRad = fovDeg * DEG;
	return viewportHeightPx / (2 * depth * Math.tan(fovRad / 2));
}

/* ========================================================================== */
/* 2. The fan arc — anchor-local space, seat-agnostic                          */
/* ========================================================================== */

export interface FanOptions {
	/** World-unit card width. Callers normally pass `cardHeight * CARD_ASPECT`. */
	readonly cardWidth: number;
	/** World-unit card height. */
	readonly cardHeight: number;
	/** `0..1`: the fraction of `cardWidth` the next card covers. `0` = edge to edge. */
	readonly overlap: number;
	/** The two end cards sit at `∓maxTiltDeg` around `restAngleDeg`; the total sweep is `2×maxTiltDeg`. */
	readonly maxTiltDeg: number;
	/** Fraction of `cardHeight` the arc's centre rises above its ends. `0` = a flat row. */
	readonly archLift: number;
	/** World-unit gap, along the fan's shared face normal, between neighbouring cards' depth — resolves overlap order unambiguously with no z-fighting. */
	readonly stackStep: number;
	/** `0` = lying flat on the felt (face normal `+Y`). `90` = standing straight up, facing across the table. Tilts legibility toward the camera without disturbing the per-card twist. */
	readonly reclineDeg: number;
	/** The twist (see the module doc) at the fan's centre, before the per-card `∓maxTiltDeg` spread is added. `0` is the standard "legible from the camera" rest pose. */
	readonly restAngleDeg: number;
}

export const DEFAULT_FAN_OPTIONS: FanOptions = {
	cardWidth: 0.09 * CARD_ASPECT,
	cardHeight: 0.09,
	overlap: 0.55,
	maxTiltDeg: 10,
	archLift: 0.04,
	stackStep: 0.0004,
	reclineDeg: 0,
	restAngleDeg: 0
};

/**
 * The arc for a fanned hand of `n` cards, in anchor-local space, centred at the
 * anchor's origin. Handles `n` from `0` (empty) through `6` (the dealer's brief
 * six-card hand during the discard) and beyond — nothing below assumes a fixed
 * count.
 *
 * `x` is monotonically increasing in index (card `0` is leftmost); the arc's
 * vertical bulge and the fan's rotational sweep are both symmetric about the
 * centre card (or the midpoint, for an even `n`), so the two end cards always
 * carry `∓maxTiltDeg` regardless of `n`.
 */
export function fanPositions(n: number, options: Partial<FanOptions> = {}): CardPose[] {
	if (n <= 0) return [];
	const opts: FanOptions = { ...DEFAULT_FAN_OPTIONS, ...options };

	const rotX = -Math.PI / 2 + opts.reclineDeg * DEG;
	const ny = -Math.sin(rotX);
	const nz = Math.cos(rotX);

	const lateralStep = opts.cardWidth * (1 - opts.overlap);
	const span = lateralStep * (n - 1);
	const restAngleRad = opts.restAngleDeg * DEG;
	const maxTiltRad = opts.maxTiltDeg * DEG;

	const poses: CardPose[] = [];
	for (let i = 0; i < n; i++) {
		const t = n === 1 ? 0 : (i / (n - 1)) * 2 - 1; // -1..1 across the fan
		const x = i * lateralStep - span / 2;
		const arch = opts.archLift * opts.cardHeight * (1 - t * t);
		const twist = restAngleRad + t * maxTiltRad;
		const stack = i * opts.stackStep;

		poses.push({
			position: [x, arch + stack * ny, stack * nz],
			rotation: [rotX, 0, twist]
		});
	}
	return poses;
}

/* ========================================================================== */
/* 3. A flat stack — the kitty's buried pile                                  */
/* ========================================================================== */

export interface StackOptions {
	readonly stackStep: number;
	/** World-unit lateral jitter so a pile of face-down cards doesn't read as one slab. `0` disables it. */
	readonly scatter: number;
	readonly reclineDeg: number;
}

export const DEFAULT_STACK_OPTIONS: StackOptions = {
	stackStep: 0.0006,
	scatter: 0.002,
	reclineDeg: 0
};

/**
 * `n` face-down cards stacked flat with a small deterministic per-card jitter,
 * centred at the origin. Used for the kitty's three permanently-buried cards
 * (never the up-card, which is publicly identified and rendered separately by
 * `Kitty.svelte`).
 */
export function stackPositions(n: number, options: Partial<StackOptions> = {}): CardPose[] {
	if (n <= 0) return [];
	const opts: StackOptions = { ...DEFAULT_STACK_OPTIONS, ...options };
	const rotX = -Math.PI / 2 + opts.reclineDeg * DEG;
	const ny = -Math.sin(rotX);
	const nz = Math.cos(rotX);

	const poses: CardPose[] = [];
	for (let i = 0; i < n; i++) {
		const jitter = opts.scatter * (((i % 3) - 1) * 0.6);
		const stack = i * opts.stackStep;
		poses.push({ position: [jitter, stack * ny, stack * nz], rotation: [rotX, 0, 0] });
	}
	return poses;
}

/* ========================================================================== */
/* 4. Table space — the four seats, for the trick pile                        */
/* ========================================================================== */

/**
 * Unit direction, in the table's `(x, z)` plane, from the felt's centre toward
 * each seat. Seat `0` (south, the human) is nearest the camera on `+Z`; seats
 * run clockwise from there, matching `Seat`'s own doc comment in
 * `$lib/euchre`.
 */
export const SEAT_DIRECTION: Readonly<Record<Seat, readonly [number, number]>> = {
	0: [0, 1],
	1: [-1, 0],
	2: [0, -1],
	3: [1, 0]
};

/**
 * The `rotation.z` twist (see the module doc) that orients a flat, face-up card
 * so its top edge points back toward the given seat — as if it had just slid
 * out of that player's hand into the trick. Solved directly from
 * {@link SEAT_DIRECTION} against the verified `up = (-sin z, 0, -cos z)`
 * identity for a `[-π/2, 0, z]` pose (see the module doc and the test file).
 */
export const SEAT_FACING_RAD: Readonly<Record<Seat, number>> = {
	0: Math.PI,
	1: Math.PI / 2,
	2: 0,
	3: -Math.PI / 2
};

export interface TrickOptions {
	/** Metres from the trick's centre to each seat's card slot. */
	readonly radius: number;
	/** World-unit height gap between successive plays, in play order, so the read pause never z-fights. */
	readonly stackStep: number;
}

export const DEFAULT_TRICK_OPTIONS: TrickOptions = { radius: 0.055, stackStep: 0.0006 };

/**
 * Where one played card lands in the trick zone: offset from the centre toward
 * `seat`, lying flat, oriented back toward that seat. `order` is this play's
 * 0-based index within the current trick (0–3, or 0–2 under a loner) and only
 * affects the tiny anti-z-fight height stagger — it never changes which
 * direction the card faces, so a card's pose depends on nothing but *who*
 * played it.
 */
export function trickCardPose(seat: Seat, order: number, options: Partial<TrickOptions> = {}): CardPose {
	const opts: TrickOptions = { ...DEFAULT_TRICK_OPTIONS, ...options };
	const [dx, dz] = SEAT_DIRECTION[seat];
	return {
		position: [dx * opts.radius, order * opts.stackStep, dz * opts.radius],
		rotation: [-Math.PI / 2, 0, SEAT_FACING_RAD[seat]]
	};
}
