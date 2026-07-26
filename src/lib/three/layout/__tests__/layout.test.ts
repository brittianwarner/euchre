/**
 * `layout.test.ts` — geometry invariants for `layout.ts`.
 *
 * These are pure numeric checks; nothing here touches Threlte, three.js or a
 * renderer. The one exception is the "load-bearing case" suite, which cross-
 * checks the `[-π/2, 0, twist]` composition claim in the module doc against
 * `three`'s own `Euler`/`Matrix4` — that identity is what every other test in
 * this file assumes.
 */

import { Euler, Matrix4, Vector3 } from 'three';
import { describe, expect, it } from 'vitest';
import {
	DEFAULT_FAN_OPTIONS,
	MIN_STRIP_H,
	MIN_STRIP_W,
	SEAT_DIRECTION,
	SEAT_FACING_RAD,
	fanPositions,
	sizeFan,
	stackPositions,
	trickCardPose
} from '../layout';
import type { Seat } from '$lib/euchre';

const SEATS: readonly Seat[] = [0, 1, 2, 3];

/* -------------------------------------------------------------------------- */
/* The load-bearing case: verify the rotation composition against `three`     */
/* -------------------------------------------------------------------------- */

/** The face normal and "up" (top-edge) direction of a `[-π/2, 0, z]`-posed card. */
function normalAndUp(zRad: number): { normal: Vector3; up: Vector3 } {
	const rot = new Matrix4().extractRotation(
		new Matrix4().makeRotationFromEuler(new Euler(-Math.PI / 2, 0, zRad, 'XYZ'))
	);
	return {
		normal: new Vector3(0, 0, 1).applyMatrix4(rot),
		up: new Vector3(0, 1, 0).applyMatrix4(rot)
	};
}

describe('rotation composition (three.js ground truth)', () => {
	it('keeps a flat card face-up regardless of the z twist', () => {
		for (const deg of [0, 15, 45, 90, 180, 271]) {
			const { normal } = normalAndUp((deg * Math.PI) / 180);
			expect(normal.x).toBeCloseTo(0, 6);
			expect(normal.y).toBeCloseTo(1, 6);
			expect(normal.z).toBeCloseTo(0, 6);
		}
	});

	it('SEAT_FACING_RAD makes a flat card´s top edge point at its own seat direction', () => {
		for (const seat of SEATS) {
			const { up } = normalAndUp(SEAT_FACING_RAD[seat]);
			const [dx, dz] = SEAT_DIRECTION[seat];
			expect(up.x).toBeCloseTo(dx, 6);
			expect(up.y).toBeCloseTo(0, 6);
			expect(up.z).toBeCloseTo(dz, 6);
		}
	});

	it('a nonzero rotation.y (unlike rotation.z) tilts the face off flat — why layout.ts never sets it', () => {
		const rot = new Matrix4().extractRotation(
			new Matrix4().makeRotationFromEuler(new Euler(-Math.PI / 2, Math.PI / 2, 0, 'XYZ'))
		);
		const normal = new Vector3(0, 0, 1).applyMatrix4(rot);
		expect(normal.y).not.toBeCloseTo(1, 3); // no longer flat
	});
});

/* -------------------------------------------------------------------------- */
/* sizeFan — the three cases the spec pins                                    */
/* -------------------------------------------------------------------------- */

describe('sizeFan', () => {
	it('390px, n=5: overlap holds at 0.30', () => {
		const { cardWpx, cardHpx, overlap } = sizeFan(350, 5, 0.3);
		expect(overlap).toBeCloseTo(0.3, 5);
		expect(cardWpx).toBeCloseTo(92.1, 0);
		expect(cardHpx).toBeCloseTo(129, 0);
		expect(cardWpx * (1 - overlap)).toBeGreaterThanOrEqual(MIN_STRIP_W - 1e-6);
	});

	it('390px, n=6 (dealer discard): overlap still holds at 0.30', () => {
		const { cardWpx, overlap } = sizeFan(350, 6, 0.3);
		expect(overlap).toBeCloseTo(0.3, 5);
		expect(cardWpx).toBeCloseTo(78, 0);
	});

	it('320px, n=6: overlap relaxes toward 0 to clear the strip-width floor', () => {
		// docs/04-FRONTEND-UX.md §14 illustrates this exact case (available=300,
		// n=6) landing at overlap 0 / 50×70. Working the search by hand against
		// the algorithm given alongside that prose (which this function copies
		// verbatim) instead lands one step earlier, at overlap 0.2: the
		// strip-width floor (48px) is first cleared there
		// (strip = 300·0.8/(5·0.8+1) = 48 exactly), not at 0. The prose example
		// and its own code disagree; this test pins the code's actual, verified
		// output rather than the illustration.
		const { cardWpx, cardHpx, overlap } = sizeFan(300, 6, 0.3);
		expect(overlap).toBeCloseTo(0.2, 5);
		expect(cardWpx).toBeCloseTo(60, 0);
		expect(cardHpx).toBeCloseTo(84, 0);
		expect(cardWpx * (1 - overlap)).toBeCloseTo(MIN_STRIP_W, 0);
	});

	it('never returns a strip narrower than the affordance floor once one is reachable', () => {
		for (let px = 280; px <= 430; px += 10) {
			for (let n = 1; n <= 6; n++) {
				const { cardWpx, overlap } = sizeFan(px, n, 0.3);
				const strip = cardWpx * (1 - overlap);
				// Either the floor is met, or overlap has already been relaxed to 0
				// (the best any layout can do) and the floor may still legitimately
				// be missed at very small viewports.
				expect(strip >= MIN_STRIP_W - 1e-6 || overlap === 0).toBe(true);
			}
		}
	});

	it('degrades harmlessly for n<=0', () => {
		expect(sizeFan(300, 0)).toEqual({ cardWpx: 0, cardHpx: 0, overlap: 0.3 });
	});
});

/* -------------------------------------------------------------------------- */
/* fanPositions — monotonicity, symmetry, overlap, 5 and 6 card cases          */
/* -------------------------------------------------------------------------- */

describe('fanPositions', () => {
	it('returns nothing for an empty hand', () => {
		expect(fanPositions(0)).toEqual([]);
	});

	it('centres a single card with no tilt', () => {
		const [pose] = fanPositions(1);
		expect(pose.position[0]).toBeCloseTo(0, 9);
		expect(pose.rotation[2]).toBeCloseTo(0, 9);
	});

	for (const n of [2, 3, 4, 5, 6]) {
		it(`n=${n}: x is strictly monotonically increasing (left to right, no reordering)`, () => {
			const poses = fanPositions(n);
			for (let i = 1; i < poses.length; i++) {
				expect(poses[i].position[0]).toBeGreaterThan(poses[i - 1].position[0]);
			}
		});

		it(`n=${n}: the fan is centred at x=0`, () => {
			const poses = fanPositions(n);
			const xs = poses.map((p) => p.position[0]);
			const mid = (xs[0] + xs[xs.length - 1]) / 2;
			expect(mid).toBeCloseTo(0, 9);
		});

		it(`n=${n}: consecutive centres are exactly cardWidth*(1-overlap) apart — no extra overlap`, () => {
			const opts = { cardWidth: 0.07, overlap: 0.4 };
			const poses = fanPositions(n, opts);
			const expectedStep = opts.cardWidth * (1 - opts.overlap);
			for (let i = 1; i < poses.length; i++) {
				expect(poses[i].position[0] - poses[i - 1].position[0]).toBeCloseTo(expectedStep, 9);
			}
		});

		it(`n=${n}: the end cards sit at exactly ∓maxTiltDeg and the twist is monotonic`, () => {
			const maxTiltDeg = 10;
			const poses = fanPositions(n, { maxTiltDeg });
			const toDeg = (r: number) => (r * 180) / Math.PI;
			expect(toDeg(poses[0].rotation[2])).toBeCloseTo(-maxTiltDeg, 6);
			expect(toDeg(poses[poses.length - 1].rotation[2])).toBeCloseTo(maxTiltDeg, 6);
			for (let i = 1; i < poses.length; i++) {
				expect(poses[i].rotation[2]).toBeGreaterThan(poses[i - 1].rotation[2]);
			}
		});

		it(`n=${n}: the arc bulges up toward the centre and is symmetric about it`, () => {
			// stackStep: 0 isolates the arc bulge from the (separately tested) depth stack.
			const poses = fanPositions(n, { archLift: 0.05, cardHeight: 0.1, stackStep: 0 });
			const ys = poses.map((p) => p.position[1]);
			expect(ys[0]).toBeCloseTo(0, 9); // ends sit at the rest height
			expect(ys[ys.length - 1]).toBeCloseTo(0, 9);
			if (n > 2) {
				const mid = ys[Math.floor((n - 1) / 2)];
				expect(mid).toBeGreaterThan(0);
			}
			// symmetric: card i and its mirror across the centre have equal lift
			for (let i = 0; i < n; i++) {
				expect(ys[i]).toBeCloseTo(ys[n - 1 - i], 9);
			}
		});

		it(`n=${n}: depth stacks monotonically with index, for unambiguous overlap order`, () => {
			// archLift: 0 isolates the depth stack from the (separately tested) arc bulge.
			const poses = fanPositions(n, { stackStep: 0.001, reclineDeg: 30, archLift: 0 });
			// project each pose's offset onto the shared face normal (only y/z carry it)
			const depth = (p: (typeof poses)[number]) => p.position[1] + p.position[2];
			for (let i = 1; i < poses.length; i++) {
				expect(depth(poses[i])).toBeGreaterThan(depth(poses[i - 1]));
			}
		});

		it(`n=${n}: reclining the fan keeps every card's face pointed the same direction (no per-card tilt drift)`, () => {
			const poses = fanPositions(n, { reclineDeg: 40 });
			for (const p of poses) {
				expect(p.rotation[0]).toBeCloseTo(poses[0].rotation[0], 9);
			}
		});
	}

	it('DEFAULT_FAN_OPTIONS keeps cardWidth consistent with the poker aspect ratio', () => {
		expect(DEFAULT_FAN_OPTIONS.cardWidth).toBeCloseTo(
			DEFAULT_FAN_OPTIONS.cardHeight * (2.5 / 3.5),
			9
		);
	});
});

/* -------------------------------------------------------------------------- */
/* stackPositions — the kitty's buried pile                                   */
/* -------------------------------------------------------------------------- */

describe('stackPositions', () => {
	it('returns nothing for an empty pile', () => {
		expect(stackPositions(0)).toEqual([]);
	});

	it('stacks height monotonically with index and stays lying flat', () => {
		const poses = stackPositions(4, { stackStep: 0.001 });
		for (let i = 1; i < poses.length; i++) {
			expect(poses[i].position[1]).toBeGreaterThan(poses[i - 1].position[1]);
			expect(poses[i].rotation[0]).toBeCloseTo(poses[0].rotation[0], 9);
		}
	});

	it('disables jitter cleanly when scatter is 0', () => {
		const poses = stackPositions(3, { scatter: 0 });
		for (const p of poses) expect(p.position[0]).toBeCloseTo(0, 9);
	});
});

/* -------------------------------------------------------------------------- */
/* trickCardPose — four seats, distinct and evenly spaced                     */
/* -------------------------------------------------------------------------- */

describe('trickCardPose — four seat orientations', () => {
	it('places each seat at a distinct point equidistant from the trick centre', () => {
		const radius = 0.06;
		const poses = SEATS.map((seat) => trickCardPose(seat, 0, { radius }));
		for (const pose of poses) {
			const [x, , z] = pose.position;
			expect(Math.hypot(x, z)).toBeCloseTo(radius, 9);
		}
		// all four x/z pairs are distinct
		const seen = new Set(poses.map((p) => `${p.position[0].toFixed(6)},${p.position[2].toFixed(6)}`));
		expect(seen.size).toBe(4);
	});

	it('opposite seats (partners) sit on opposite sides of the centre', () => {
		const a = trickCardPose(0, 0);
		const b = trickCardPose(2, 0);
		expect(a.position[0] + b.position[0]).toBeCloseTo(0, 9);
		expect(a.position[2] + b.position[2]).toBeCloseTo(0, 9);

		const c = trickCardPose(1, 0);
		const d = trickCardPose(3, 0);
		expect(c.position[0] + d.position[0]).toBeCloseTo(0, 9);
		expect(c.position[2] + d.position[2]).toBeCloseTo(0, 9);
	});

	it('every seat renders flat, face-up (rotation.x = -π/2, rotation.y = 0)', () => {
		for (const seat of SEATS) {
			const pose = trickCardPose(seat, 0);
			expect(pose.rotation[0]).toBeCloseTo(-Math.PI / 2, 9);
			expect(pose.rotation[1]).toBe(0);
		}
	});

	it('four seats produce four distinct facing angles', () => {
		const angles = new Set(SEATS.map((s) => SEAT_FACING_RAD[s]));
		expect(angles.size).toBe(4);
	});

	it('play order only staggers height, never the seat direction', () => {
		const early = trickCardPose(1, 0);
		const late = trickCardPose(1, 3);
		expect(early.position[0]).toBeCloseTo(late.position[0], 9);
		expect(early.position[2]).toBeCloseTo(late.position[2], 9);
		expect(late.position[1]).toBeGreaterThan(early.position[1]);
		expect(early.rotation).toEqual(late.rotation);
	});
});
