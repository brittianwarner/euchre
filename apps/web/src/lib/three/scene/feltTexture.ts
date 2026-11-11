/**
 * `feltTexture.ts` — a cheap, procedural felt surface.
 *
 * No image asset ships and nothing is fetched: a small canvas (256²) is
 * drawn once — a soft radial vignette (darker rail-ward, so the table reads
 * as a dish rather than a flat sticker) plus a light fibre-noise speckle —
 * and cached at module scope like `cards/cardTexture.ts`. One 256×256 RGBA
 * texture is ~256 KiB resident (plus ~⅓ for mips), which is noise next to
 * the card atlas budget in docs/04-FRONTEND-UX.md §6.2. `CircleGeometry`'s
 * UVs always span the full `[0,1]` range regardless of radius, so the same
 * canvas — vignette baked to its edges — fits any table size unstretched;
 * no tiling or resampling needed.
 */

import {
	CanvasTexture,
	ClampToEdgeWrapping,
	LinearFilter,
	LinearMipmapLinearFilter,
	SRGBColorSpace
} from 'three';

const SIZE = 512;
let cached: CanvasTexture | null = null;
let plainCached: CanvasTexture | null = null;

/** Deterministic "noise" — a fixed felt grain, not a per-load random one, so screenshots are stable. */
function grain(x: number, y: number): number {
	const s = Math.sin(x * 12.9898 + y * 78.233) * 43758.5453;
	return s - Math.floor(s);
}

function paint(ctx: CanvasRenderingContext2D, marked: boolean): void {
	const base = '#245a49'; // warm felt green, not a cold billiard green
	const edge = '#10382c';

	const grad = ctx.createRadialGradient(
		SIZE / 2,
		SIZE / 2,
		SIZE * 0.15,
		SIZE / 2,
		SIZE / 2,
		SIZE * 0.72
	);
	grad.addColorStop(0, base);
	grad.addColorStop(1, edge);
	ctx.fillStyle = grad;
	ctx.fillRect(0, 0, SIZE, SIZE);

	// Fibre speckle: tiny low-contrast dots, cheap to draw once and free at runtime.
	const dots = 900;
	for (let i = 0; i < dots; i++) {
		const x = grain(i, 1.7) * SIZE;
		const y = grain(i, 91.3) * SIZE;
		const shade = grain(i, 5.2) > 0.5 ? 255 : 0;
		ctx.fillStyle = `rgba(${shade},${shade},${shade},0.05)`;
		ctx.fillRect(x, y, 1, 1);
	}
	if (!marked) return;
	// Baked tailoring and a quiet club mark add detail without extra geometry,
	// lights, shaders, or work in the animation loop.
	ctx.strokeStyle = 'rgba(218,230,176,0.22)';
	ctx.lineWidth = 1;
	ctx.beginPath();
	ctx.arc(SIZE / 2, SIZE / 2, SIZE * 0.455, 0, Math.PI * 2);
	ctx.stroke();
	ctx.setLineDash([2, 4]);
	ctx.beginPath();
	ctx.arc(SIZE / 2, SIZE / 2, SIZE * 0.443, 0, Math.PI * 2);
	ctx.stroke();
	ctx.setLineDash([]);
	ctx.fillStyle = 'rgba(223,234,197,0.16)';
	ctx.textAlign = 'center';
	ctx.textBaseline = 'middle';
	ctx.font = '36px Georgia';
	ctx.fillText('♣', SIZE / 2, SIZE * 0.38);
	ctx.font = 'italic 20px Georgia';
	ctx.fillText('euchre', SIZE / 2, SIZE * 0.45);
}

function makeCanvas(): HTMLCanvasElement | null {
	if (typeof document === 'undefined') return null; // SSR guard
	const c = document.createElement('canvas');
	c.width = SIZE;
	c.height = SIZE;
	return c;
}

/** The shared felt texture. `null` during SSR; callers render nothing until the scene mounts. */
export function feltTexture(marked = true): CanvasTexture | null {
	const existing = marked ? cached : plainCached;
	if (existing) return existing;
	const canvas = makeCanvas();
	if (canvas === null) return null;
	const ctx = canvas.getContext('2d');
	if (ctx === null) return null;

	paint(ctx, marked);

	const tex = new CanvasTexture(canvas);
	tex.colorSpace = SRGBColorSpace;
	tex.wrapS = ClampToEdgeWrapping;
	tex.wrapT = ClampToEdgeWrapping;
	tex.magFilter = LinearFilter;
	tex.minFilter = LinearMipmapLinearFilter;
	tex.generateMipmaps = true;
	tex.needsUpdate = true;
	if (marked) cached = tex;
	else plainCached = tex;
	return tex;
}

/** Tear down the cached texture — mirrors `cards/cardTexture.ts`'s `disposeCardTextures()`. */
export function disposeFeltTexture(): void {
	cached?.dispose();
	cached = null;
	plainCached?.dispose();
	plainCached = null;
}
