/**
 * `cardMotion.test.ts` — numeric invariants for the flight-interpolation
 * helpers. Pure math, no renderer, matching `layout.test.ts`'s own style.
 */

import { describe, expect, it } from 'vitest';
import { arcLift, dealOrigin, lerp3, lerpPose, playOrigin } from '../cardMotion';
import type { CardPose } from '../layout';

describe('lerp3', () => {
	it('returns `a` at t=0 and `b` at t=1', () => {
		const a: [number, number, number] = [0, 1, 2];
		const b: [number, number, number] = [4, -1, 10];
		expect(lerp3(a, b, 0)).toEqual(a);
		expect(lerp3(a, b, 1)).toEqual(b);
	});

	it('is linear at the midpoint', () => {
		const a: [number, number, number] = [0, 0, 0];
		const b: [number, number, number] = [2, 4, -6];
		expect(lerp3(a, b, 0.5)).toEqual([1, 2, -3]);
	});
});

describe('arcLift', () => {
	it('is zero at both flight ends', () => {
		expect(arcLift(0, 0.05)).toBe(0);
		expect(arcLift(1, 0.05)).toBe(0);
	});

	it('peaks at exactly `height` at the midpoint', () => {
		expect(arcLift(0.5, 0.08)).toBeCloseTo(0.08, 10);
	});

	it('is a true parabola: symmetric about t=0.5', () => {
		expect(arcLift(0.25, 0.1)).toBeCloseTo(arcLift(0.75, 0.1), 10);
	});

	it('collapses to no lift when height is 0', () => {
		for (const t of [0, 0.3, 0.5, 0.9, 1]) expect(arcLift(t, 0)).toBe(0);
	});
});

describe('lerpPose', () => {
	const from: CardPose = { position: [0, 0, 0], rotation: [0, 0, 0] };
	const to: CardPose = { position: [1, 0, 1], rotation: [0, 0, Math.PI] };

	it('matches the endpoints at t=0 and t=1 with no arc', () => {
		expect(lerpPose(from, to, 0)).toEqual(from);
		expect(lerpPose(from, to, 1)).toEqual(to);
	});

	it('adds the arc lift on top of the interpolated y, not in place of it', () => {
		const withArc = lerpPose(from, to, 0.5, 0.06);
		const flat = lerpPose(from, to, 0.5, 0);
		expect(withArc.position[1]).toBeCloseTo(flat.position[1] + 0.06, 10);
		// x/z/rotation are untouched by the arc term.
		expect(withArc.position[0]).toBe(flat.position[0]);
		expect(withArc.position[2]).toBe(flat.position[2]);
		expect(withArc.rotation).toEqual(flat.rotation);
	});
});

describe('dealOrigin', () => {
	it('pulls the origin toward the anchor centre and lifts it, in anchor-local space', () => {
		const to: CardPose = { position: [0.2, 0.09, -0.05], rotation: [-Math.PI / 2, 0, 0.1] };
		const from = dealOrigin(to);
		expect(from.position[0]).toBeCloseTo(0.04, 10); // 0.2 * 0.2
		expect(from.position[1]).toBeGreaterThan(to.position[1]); // lifted
		expect(from.position[2]).toBeGreaterThan(to.position[2]); // pulled toward +Z (table centre)
		expect(from.rotation).toEqual(to.rotation); // orientation is set once, not animated
	});

	it('never divides by anything that could be zero (pure arithmetic, always defined)', () => {
		const to: CardPose = { position: [0, 0.1, 0], rotation: [0, 0, 0] };
		expect(() => dealOrigin(to)).not.toThrow();
	});
});

describe('playOrigin', () => {
	it('moves strictly further from the trick centre along the same direction', () => {
		const to: CardPose = { position: [0, 0, 0.055], rotation: [-Math.PI / 2, 0, 0] };
		const from = playOrigin(to, 0.16);
		expect(from.position[0]).toBeCloseTo(0, 10);
		expect(from.position[2]).toBeCloseTo(0.215, 10);
		expect(from.rotation).toEqual(to.rotation);
	});

	it('preserves the seat direction for every compass point', () => {
		const cases: CardPose[] = [
			{ position: [0, 0, 0.055], rotation: [0, 0, 0] }, // south
			{ position: [-0.055, 0, 0], rotation: [0, 0, 0] }, // west
			{ position: [0, 0, -0.055], rotation: [0, 0, 0] }, // north
			{ position: [0.055, 0, 0], rotation: [0, 0, 0] } // east
		];
		for (const to of cases) {
			const from = playOrigin(to);
			const dot = from.position[0] * to.position[0] + from.position[2] * to.position[2];
			expect(dot).toBeGreaterThan(0); // same direction, not flipped
			const fromLen = Math.hypot(from.position[0], from.position[2]);
			const toLen = Math.hypot(to.position[0], to.position[2]);
			expect(fromLen).toBeGreaterThan(toLen);
		}
	});

	it('does not divide by zero when `to` sits exactly at the trick centre', () => {
		const to: CardPose = { position: [0, 0, 0], rotation: [0, 0, 0] };
		expect(() => playOrigin(to)).not.toThrow();
		const from = playOrigin(to);
		expect(Number.isFinite(from.position[0])).toBe(true);
		expect(Number.isFinite(from.position[2])).toBe(true);
	});
});
