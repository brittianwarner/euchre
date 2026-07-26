/**
 * `faces.ts` — renders a traditional playing-card face to a canvas.
 *
 * Euchre uses a 24-card deck (9 T J Q K A × four suits), so this draws six ranks
 * rather than thirteen. Everything is vector: no image assets ship, nothing to
 * 404, and the same code produces a crisp face at any texture resolution.
 *
 * What makes a card read as *traditional* rather than as a coloured rectangle
 * with a letter on it:
 *
 * - **Corner indices in both corners, the bottom one rotated a half turn.** This
 *   is the single strongest cue, and it is why you can fan a real hand and still
 *   read it.
 * - **Canonical pip arrangements.** A nine is not "nine pips somewhere"; it is
 *   two columns of four with one pip in the middle. A ten is two columns of four
 *   with two pips on the centre line. Getting these wrong is uncanny even to
 *   someone who has never thought about it.
 * - **Point symmetry.** Pips below the midline are drawn upside down, so the card
 *   looks identical rotated 180°, exactly like the real thing.
 * - **A white face with a thin border and rounded corners**, not a flat fill.
 */

import { CLASSIC_COLORS, FOUR_COLOR_COLORS, drawPip, type SuitId } from './suits';

export type RankId = '9' | 'T' | 'J' | 'Q' | 'K' | 'A';
/** e.g. `"JS"`, `"TH"` — matches `CardId` from `$lib/euchre`. */
export type CardFaceId = `${RankId}${SuitId}`;

/** Poker proportions (2.5 × 3.5in). Every layout constant below is a fraction of these. */
export const CARD_ASPECT = 2.5 / 3.5;
export const FACE_W = 500;
export const FACE_H = Math.round(FACE_W / CARD_ASPECT); // 700

/** What the index corner prints. `T` is shown as `10`, as on a real card. */
const INDEX_LABEL: Readonly<Record<RankId, string>> = {
	'9': '9',
	T: '10',
	J: 'J',
	Q: 'Q',
	K: 'K',
	A: 'A'
};

/**
 * Canonical pip positions, in a normalised pip field where `x` runs 0→1 across
 * the three columns (left, centre, right) and `y` runs 0→1 top→bottom.
 *
 * Only the top half plus the centre line is listed; {@link mirrorLayout} derives
 * the bottom half by reflection, which is what guarantees point symmetry.
 */
type Pip = readonly [x: number, y: number];

const COL_L = 0;
const COL_C = 0.5;
const COL_R = 1;

/** Top-half and centre-line pips for each numeric rank. */
const HALF_LAYOUT: Readonly<Record<'9' | 'T' | 'A', readonly Pip[]>> = {
	// Nine: four down each side, one dead centre.
	'9': [
		[COL_L, 0],
		[COL_R, 0],
		[COL_L, 1 / 3],
		[COL_R, 1 / 3],
		[COL_C, 0.5]
	],
	// Ten: four down each side, two straddling the centre line.
	T: [
		[COL_L, 0],
		[COL_R, 0],
		[COL_L, 1 / 3],
		[COL_R, 1 / 3],
		[COL_C, 1 / 6]
	],
	// Ace: one large pip, centred.
	A: [[COL_C, 0.5]]
};

/**
 * Reflect the top-half layout through the centre to produce the full set.
 *
 * A pip exactly on the centre line (`y === 0.5`) is its own mirror and must not
 * be duplicated, or an ace grows a second pip.
 */
function mirrorLayout(half: readonly Pip[]): readonly (readonly [number, number, boolean])[] {
	const out: (readonly [number, number, boolean])[] = [];
	for (const [x, y] of half) out.push([x, y, false]);
	for (const [x, y] of half) {
		if (Math.abs(y - 0.5) < 1e-6) continue;
		out.push([x, 1 - y, true]); // upside down, as on a real card
	}
	return out;
}

/* ========================================================================== */
/* Court cards                                                                 */
/* ========================================================================== */

/**
 * A court card, drawn as the traditional mirrored half-figure.
 *
 * Real J/Q/K art is centuries of accumulated engraving that no one is going to
 * reproduce in a canvas call. What actually communicates "court card" is the
 * structure: a panelled frame, a figure divided on the diagonal, and the same
 * image rotated a half turn. This draws that structure honestly rather than
 * attempting portraiture and landing on clip-art.
 */
function drawCourt(
	ctx: CanvasRenderingContext2D,
	rank: 'J' | 'Q' | 'K',
	suit: SuitId,
	color: string,
	x: number,
	y: number,
	w: number,
	h: number
): void {
	const ink = '#23201c';
	const gold = '#b8912f';

	ctx.save();

	// Panel with a double keyline, the way a real court card is framed.
	roundRect(ctx, x, y, w, h, w * 0.05);
	ctx.fillStyle = '#fbf6ea';
	ctx.fill();
	ctx.lineWidth = Math.max(2, w * 0.02);
	ctx.strokeStyle = ink;
	ctx.stroke();
	roundRect(ctx, x + w * 0.035, y + h * 0.025, w * 0.93, h * 0.95, w * 0.035);
	ctx.lineWidth = Math.max(1, w * 0.008);
	ctx.strokeStyle = gold;
	ctx.stroke();

	roundRect(ctx, x, y, w, h, w * 0.05);
	ctx.clip();

	/** Half a court figure, occupying the top half of the panel. */
	const figure = (flip: boolean) => {
		ctx.save();
		ctx.translate(x + w / 2, y + h / 2);
		if (flip) ctx.rotate(Math.PI);
		ctx.translate(-w / 2, -h / 2);

		const cx = w / 2;

		// ---- Robe: a broad shoulder mass, patterned so it reads as fabric ----
		ctx.beginPath();
		ctx.moveTo(w * 0.12, h * 0.5);
		ctx.quadraticCurveTo(w * 0.16, h * 0.34, cx, h * 0.33);
		ctx.quadraticCurveTo(w * 0.84, h * 0.34, w * 0.88, h * 0.5);
		ctx.lineTo(w * 0.88, h * 0.52);
		ctx.lineTo(w * 0.12, h * 0.52);
		ctx.closePath();
		ctx.fillStyle = color;
		ctx.fill();
		ctx.lineWidth = Math.max(1, w * 0.008);
		ctx.strokeStyle = ink;
		ctx.stroke();

		// Robe trim
		ctx.beginPath();
		ctx.moveTo(w * 0.3, h * 0.52);
		ctx.lineTo(w * 0.3, h * 0.38);
		ctx.moveTo(w * 0.7, h * 0.52);
		ctx.lineTo(w * 0.7, h * 0.38);
		ctx.strokeStyle = gold;
		ctx.lineWidth = Math.max(1, w * 0.012);
		ctx.stroke();

		// ---- Ruff collar ----
		ctx.beginPath();
		ctx.ellipse(cx, h * 0.335, w * 0.15, h * 0.035, 0, 0, Math.PI * 2);
		ctx.fillStyle = '#fbf6ea';
		ctx.fill();
		ctx.lineWidth = Math.max(1, w * 0.008);
		ctx.strokeStyle = ink;
		ctx.stroke();

		// ---- Face ----
		ctx.beginPath();
		ctx.ellipse(cx, h * 0.245, w * 0.105, h * 0.075, 0, 0, Math.PI * 2);
		ctx.fillStyle = '#fbf6ea';
		ctx.fill();
		ctx.strokeStyle = ink;
		ctx.lineWidth = Math.max(1, w * 0.008);
		ctx.stroke();

		// Eyes, brow and mouth: small marks, but they are the difference between
		// a face and an oval.
		ctx.fillStyle = ink;
		const eye = Math.max(1.5, w * 0.014);
		ctx.beginPath();
		ctx.arc(cx - w * 0.042, h * 0.235, eye, 0, Math.PI * 2);
		ctx.arc(cx + w * 0.042, h * 0.235, eye, 0, Math.PI * 2);
		ctx.fill();
		ctx.lineWidth = Math.max(1, w * 0.007);
		ctx.strokeStyle = ink;
		ctx.beginPath();
		ctx.moveTo(cx - w * 0.065, h * 0.212);
		ctx.quadraticCurveTo(cx - w * 0.042, h * 0.203, cx - w * 0.018, h * 0.212);
		ctx.moveTo(cx + w * 0.018, h * 0.212);
		ctx.quadraticCurveTo(cx + w * 0.042, h * 0.203, cx + w * 0.065, h * 0.212);
		ctx.stroke();
		ctx.beginPath();
		ctx.moveTo(cx - w * 0.03, h * 0.283);
		ctx.quadraticCurveTo(cx, h * 0.295, cx + w * 0.03, h * 0.283);
		ctx.stroke();

		// King gets a beard; the queen and jack do not. It is the fastest read.
		if (rank === 'K') {
			ctx.beginPath();
			ctx.moveTo(cx - w * 0.1, h * 0.262);
			ctx.quadraticCurveTo(cx, h * 0.36, cx + w * 0.1, h * 0.262);
			ctx.fillStyle = '#d8d2c4';
			ctx.fill();
			ctx.strokeStyle = ink;
			ctx.stroke();
		}

		// ---- Headwear, by rank ----
		ctx.lineWidth = Math.max(1, w * 0.008);
		if (rank === 'K') {
			// Pointed crown with jewels.
			ctx.beginPath();
			ctx.moveTo(cx - w * 0.13, h * 0.185);
			ctx.lineTo(cx - w * 0.13, h * 0.145);
			ctx.lineTo(cx - w * 0.07, h * 0.175);
			ctx.lineTo(cx, h * 0.125);
			ctx.lineTo(cx + w * 0.07, h * 0.175);
			ctx.lineTo(cx + w * 0.13, h * 0.145);
			ctx.lineTo(cx + w * 0.13, h * 0.185);
			ctx.closePath();
			ctx.fillStyle = gold;
			ctx.fill();
			ctx.strokeStyle = ink;
			ctx.stroke();
			ctx.fillStyle = color;
			for (const jx of [-0.07, 0, 0.07]) {
				ctx.beginPath();
				ctx.arc(cx + w * jx, h * 0.152, Math.max(1.5, w * 0.014), 0, Math.PI * 2);
				ctx.fill();
			}
		} else if (rank === 'Q') {
			// Rounded tiara.
			ctx.beginPath();
			ctx.moveTo(cx - w * 0.12, h * 0.19);
			ctx.quadraticCurveTo(cx - w * 0.1, h * 0.135, cx - w * 0.045, h * 0.155);
			ctx.quadraticCurveTo(cx, h * 0.115, cx + w * 0.045, h * 0.155);
			ctx.quadraticCurveTo(cx + w * 0.1, h * 0.135, cx + w * 0.12, h * 0.19);
			ctx.closePath();
			ctx.fillStyle = gold;
			ctx.fill();
			ctx.strokeStyle = ink;
			ctx.stroke();
		} else {
			// Jack: a soft cap with a feather.
			ctx.beginPath();
			ctx.moveTo(cx - w * 0.125, h * 0.192);
			ctx.quadraticCurveTo(cx - w * 0.06, h * 0.128, cx + w * 0.1, h * 0.16);
			ctx.quadraticCurveTo(cx + w * 0.13, h * 0.172, cx + w * 0.125, h * 0.192);
			ctx.closePath();
			ctx.fillStyle = color;
			ctx.fill();
			ctx.strokeStyle = ink;
			ctx.stroke();
			ctx.beginPath();
			ctx.moveTo(cx + w * 0.09, h * 0.155);
			ctx.quadraticCurveTo(cx + w * 0.17, h * 0.1, cx + w * 0.13, h * 0.052);
			ctx.strokeStyle = gold;
			ctx.lineWidth = Math.max(1.5, w * 0.016);
			ctx.stroke();
		}

		ctx.restore();
	};

	figure(false);
	figure(true);

	// The dividing diagonal, drawn last so it sits over both figures — this is
	// what makes the half-turn symmetry legible rather than merely present.
	ctx.beginPath();
	ctx.moveTo(x, y + h);
	ctx.lineTo(x + w, y);
	ctx.lineWidth = Math.max(1, w * 0.01);
	ctx.strokeStyle = ink;
	ctx.stroke();

	// Suit pips in the free corners.
	const pip = w * 0.17;
	drawPip(ctx, suit, x + w * 0.15, y + h * 0.1, pip, color);
	drawPip(ctx, suit, x + w - w * 0.15, y + h - h * 0.1, pip, color, true);

	ctx.restore();
}

/* ========================================================================== */
/* Face                                                                        */
/* ========================================================================== */

function roundRect(
	ctx: CanvasRenderingContext2D,
	x: number,
	y: number,
	w: number,
	h: number,
	r: number
): void {
	ctx.beginPath();
	ctx.moveTo(x + r, y);
	ctx.arcTo(x + w, y, x + w, y + h, r);
	ctx.arcTo(x + w, y + h, x, y + h, r);
	ctx.arcTo(x, y + h, x, y, r);
	ctx.arcTo(x, y, x + w, y, r);
	ctx.closePath();
}

/** One corner index: rank above a small pip, optionally rotated a half turn. */
function drawIndex(
	ctx: CanvasRenderingContext2D,
	rank: RankId,
	suit: SuitId,
	color: string,
	cx: number,
	cy: number,
	scale: number,
	flip: boolean
): void {
	ctx.save();
	ctx.translate(cx, cy);
	if (flip) ctx.rotate(Math.PI);

	ctx.fillStyle = color;
	ctx.textAlign = 'center';
	ctx.textBaseline = 'alphabetic';
	// A condensed grotesque reads closest to real index type, and "10" must not
	// crowd the corner — so the two-glyph label gets squeezed horizontally.
	const label = INDEX_LABEL[rank];
	ctx.font = `700 ${Math.round(scale * 1.0)}px "Helvetica Neue", Helvetica, Arial, sans-serif`;
	ctx.save();
	if (label.length > 1) ctx.scale(0.78, 1);
	ctx.fillText(label, 0, 0);
	ctx.restore();

	drawPip(ctx, suit, 0, scale * 0.52, scale * 0.62, color);
	ctx.restore();
}

export interface FaceOptions {
	/** Four-colour deck for colour-blind players. Default `false`. */
	readonly fourColor?: boolean;
}

/**
 * Draw a complete card face. The canvas is assumed to be exactly
 * {@link FACE_W} × {@link FACE_H}.
 */
export function drawCardFace(
	ctx: CanvasRenderingContext2D,
	rank: RankId,
	suit: SuitId,
	opts: FaceOptions = {}
): void {
	const W = FACE_W;
	const H = FACE_H;
	const color = (opts.fourColor ? FOUR_COLOR_COLORS : CLASSIC_COLORS)[suit];

	// Stock: not pure white — real card stock is warm, and #fff glares under a
	// bright key light.
	ctx.clearRect(0, 0, W, H);
	roundRect(ctx, 0, 0, W, H, W * 0.075);
	ctx.fillStyle = '#fdfcf8';
	ctx.fill();

	// Hairline border, inset, as on a cut deck.
	roundRect(ctx, W * 0.035, H * 0.025, W * 0.93, H * 0.95, W * 0.05);
	ctx.lineWidth = Math.max(1.5, W * 0.006);
	ctx.strokeStyle = 'rgba(0,0,0,0.16)';
	ctx.stroke();

	// Corner indices, top-left and bottom-right (point symmetric).
	const idxScale = W * 0.125;
	drawIndex(ctx, rank, suit, color, W * 0.125, H * 0.1, idxScale, false);
	drawIndex(ctx, rank, suit, color, W - W * 0.125, H - H * 0.1, idxScale, true);

	if (rank === 'J' || rank === 'Q' || rank === 'K') {
		const pw = W * 0.6;
		const ph = H * 0.62;
		drawCourt(ctx, rank, suit, color, (W - pw) / 2, (H - ph) / 2, pw, ph);
		return;
	}

	// Numeric / ace pip field, inset clear of the indices.
	const fieldX = W * 0.26;
	const fieldW = W * 0.48;
	const fieldY = H * 0.14;
	const fieldH = H * 0.72;
	const pipSize = rank === 'A' ? W * 0.34 : W * 0.155;

	for (const [nx, ny, flip] of mirrorLayout(HALF_LAYOUT[rank])) {
		drawPip(ctx, suit, fieldX + nx * fieldW, fieldY + ny * fieldH, pipSize, color, flip);
	}
}

/**
 * The card back: a two-tone guilloché-ish lattice with a border.
 *
 * Deliberately busy and dark so a face-down card is unmistakable at a glance,
 * and so the fanned opponent hands read as a block of "cards" rather than as
 * blank rectangles.
 */
export function drawCardBack(ctx: CanvasRenderingContext2D, hue = '#7d1d1d'): void {
	const W = FACE_W;
	const H = FACE_H;

	ctx.clearRect(0, 0, W, H);
	roundRect(ctx, 0, 0, W, H, W * 0.075);
	ctx.fillStyle = '#fdfcf8';
	ctx.fill();

	roundRect(ctx, W * 0.04, H * 0.028, W * 0.92, H * 0.944, W * 0.05);
	ctx.fillStyle = hue;
	ctx.fill();

	// Lattice
	ctx.save();
	roundRect(ctx, W * 0.04, H * 0.028, W * 0.92, H * 0.944, W * 0.05);
	ctx.clip();
	ctx.strokeStyle = 'rgba(255,255,255,0.22)';
	ctx.lineWidth = Math.max(1, W * 0.005);
	const step = W * 0.075;
	for (let i = -H; i < W + H; i += step) {
		ctx.beginPath();
		ctx.moveTo(i, 0);
		ctx.lineTo(i + H, H);
		ctx.stroke();
		ctx.beginPath();
		ctx.moveTo(i + H, 0);
		ctx.lineTo(i, H);
		ctx.stroke();
	}
	ctx.restore();

	// Inner keyline
	roundRect(ctx, W * 0.1, H * 0.075, W * 0.8, H * 0.85, W * 0.035);
	ctx.strokeStyle = 'rgba(255,255,255,0.5)';
	ctx.lineWidth = Math.max(1.5, W * 0.007);
	ctx.stroke();
}
