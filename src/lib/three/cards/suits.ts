/**
 * `suits.ts` — the four suit pips, drawn as vector paths.
 *
 * Font glyphs (`♠♥♦♣`) are tempting and wrong: they vary by platform, hint badly
 * at small sizes, and give no control over the proportions that make a pip read
 * as a *playing card* pip rather than dingbat clip-art. These are hand-built
 * Bézier outlines drawn into a unit box, so a pip is identical on every device
 * and stays crisp at any texture resolution.
 *
 * Each function draws into the box `(-0.5, -0.5) … (0.5, 0.5)` with the canvas
 * origin already translated and scaled by the caller, so a pip can be stamped at
 * any size or rotation without re-deriving coordinates.
 */

export type SuitId = 'S' | 'H' | 'D' | 'C';

/** Traditional two-colour deck. */
export const CLASSIC_COLORS: Readonly<Record<SuitId, string>> = {
	S: '#141414',
	C: '#141414',
	H: '#c0121a',
	D: '#c0121a'
};

/**
 * Four-colour deck, for colour-blind players and for anyone who has ever mis-read
 * a heart for a diamond at a glance.
 *
 * Values chosen for luminance separation as well as hue, so the suits remain
 * distinguishable in greyscale and under deuteranopia: black, a deep red, a
 * strong blue, and a dark green.
 */
export const FOUR_COLOR_COLORS: Readonly<Record<SuitId, string>> = {
	S: '#141414',
	H: '#c0121a',
	D: '#1660c4',
	C: '#127a3e'
};

/* ========================================================================== */
/* Pip outlines                                                                */
/* ========================================================================== */

/** ♥ — two lobes meeting in a cleft, tapering to a point. */
function heartPath(ctx: CanvasRenderingContext2D): void {
	ctx.beginPath();
	ctx.moveTo(0, 0.42);
	ctx.bezierCurveTo(-0.16, 0.08, -0.5, -0.02, -0.5, -0.26);
	ctx.bezierCurveTo(-0.5, -0.46, -0.3, -0.5, -0.18, -0.44);
	ctx.bezierCurveTo(-0.08, -0.39, -0.02, -0.3, 0, -0.24);
	ctx.bezierCurveTo(0.02, -0.3, 0.08, -0.39, 0.18, -0.44);
	ctx.bezierCurveTo(0.3, -0.5, 0.5, -0.46, 0.5, -0.26);
	ctx.bezierCurveTo(0.5, -0.02, 0.16, 0.08, 0, 0.42);
	ctx.closePath();
	ctx.fill();
}

/** ♦ — a slightly waisted rhombus; straight edges read as cheap. */
function diamondPath(ctx: CanvasRenderingContext2D): void {
	ctx.beginPath();
	ctx.moveTo(0, -0.5);
	ctx.bezierCurveTo(0.12, -0.2, 0.3, -0.06, 0.36, 0);
	ctx.bezierCurveTo(0.3, 0.06, 0.12, 0.2, 0, 0.5);
	ctx.bezierCurveTo(-0.12, 0.2, -0.3, 0.06, -0.36, 0);
	ctx.bezierCurveTo(-0.3, -0.06, -0.12, -0.2, 0, -0.5);
	ctx.closePath();
	ctx.fill();
}

/** ♠ — an inverted heart on a flared stem. */
function spadePath(ctx: CanvasRenderingContext2D): void {
	ctx.beginPath();
	ctx.moveTo(0, -0.46);
	ctx.bezierCurveTo(0.14, -0.16, 0.5, -0.04, 0.5, 0.18);
	ctx.bezierCurveTo(0.5, 0.36, 0.32, 0.42, 0.2, 0.36);
	ctx.bezierCurveTo(0.12, 0.32, 0.06, 0.26, 0.04, 0.2);
	// Stem: two concave flanks flaring to a foot.
	ctx.bezierCurveTo(0.05, 0.32, 0.1, 0.42, 0.2, 0.5);
	ctx.lineTo(-0.2, 0.5);
	ctx.bezierCurveTo(-0.1, 0.42, -0.05, 0.32, -0.04, 0.2);
	ctx.bezierCurveTo(-0.06, 0.26, -0.12, 0.32, -0.2, 0.36);
	ctx.bezierCurveTo(-0.32, 0.42, -0.5, 0.36, -0.5, 0.18);
	ctx.bezierCurveTo(-0.5, -0.04, -0.14, -0.16, 0, -0.46);
	ctx.closePath();
	ctx.fill();
}

/** ♣ — three lobes on a flared stem. */
function clubPath(ctx: CanvasRenderingContext2D): void {
	const r = 0.2;
	ctx.beginPath();
	ctx.arc(0, -0.24, r, 0, Math.PI * 2);
	ctx.fill();
	ctx.beginPath();
	ctx.arc(-0.24, 0.1, r, 0, Math.PI * 2);
	ctx.fill();
	ctx.beginPath();
	ctx.arc(0.24, 0.1, r, 0, Math.PI * 2);
	ctx.fill();
	// Stem, drawn last so it fuses the lobes into one silhouette.
	ctx.beginPath();
	ctx.moveTo(0.04, 0.02);
	ctx.bezierCurveTo(0.06, 0.24, 0.12, 0.4, 0.22, 0.5);
	ctx.lineTo(-0.22, 0.5);
	ctx.bezierCurveTo(-0.12, 0.4, -0.06, 0.24, -0.04, 0.02);
	ctx.closePath();
	ctx.fill();
}

const DRAW: Readonly<Record<SuitId, (ctx: CanvasRenderingContext2D) => void>> = {
	H: heartPath,
	D: diamondPath,
	S: spadePath,
	C: clubPath
};

/**
 * Stamp one pip centred at `(x, y)` with height `size`, optionally rotated a half
 * turn — which is what gives a traditional card its point symmetry.
 */
export function drawPip(
	ctx: CanvasRenderingContext2D,
	suit: SuitId,
	x: number,
	y: number,
	size: number,
	color: string,
	upsideDown = false
): void {
	ctx.save();
	ctx.translate(x, y);
	ctx.scale(size, size);
	if (upsideDown) ctx.rotate(Math.PI);
	ctx.fillStyle = color;
	DRAW[suit](ctx);
	ctx.restore();
}
