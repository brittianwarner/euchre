/**
 * `cardTexture.ts` — canvas → `CanvasTexture`, cached per card.
 *
 * Twenty-four faces plus one back is a small, fixed set, so each gets its own
 * texture and every card mesh shares the one belonging to its id. That is 25
 * textures at ~1 MB of VRAM each at 500×700 RGBA — comfortably cheap, and far
 * simpler than an atlas with hand-maintained UV maths, which is the usual reason
 * card rendering ends up subtly misaligned.
 *
 * Textures are created lazily on first request and memoised for the life of the
 * page. Two draw modes (classic / four-colour) are cached separately, because
 * switching decks mid-game must not mutate a texture another mesh is using.
 */

import { CanvasTexture, LinearFilter, LinearMipmapLinearFilter, SRGBColorSpace } from 'three';
import { FACE_H, FACE_W, drawCardBack, drawCardFace, type CardFaceId, type RankId } from './faces';
import type { SuitId } from './suits';

const cache = new Map<string, CanvasTexture>();

/** Anisotropy is set by the consumer, which is the only place that knows the renderer. */
function finish(canvas: HTMLCanvasElement): CanvasTexture {
	const tex = new CanvasTexture(canvas);
	// Card art is authored in sRGB; without this the felt-lit table renders the
	// reds muddy and the whites grey.
	tex.colorSpace = SRGBColorSpace;
	// Mipmaps matter here: a fanned hand is viewed at a steep angle, and without
	// trilinear filtering the pips shimmer as the camera moves.
	tex.magFilter = LinearFilter;
	tex.minFilter = LinearMipmapLinearFilter;
	tex.generateMipmaps = true;
	tex.anisotropy = 8;
	tex.needsUpdate = true;
	return tex;
}

function makeCanvas(): HTMLCanvasElement | null {
	if (typeof document === 'undefined') return null; // SSR guard
	const c = document.createElement('canvas');
	c.width = FACE_W;
	c.height = FACE_H;
	return c;
}

/**
 * The texture for one card face. `null` during SSR, where there is no canvas —
 * callers render nothing until the scene mounts in the browser.
 */
export function faceTexture(id: CardFaceId, fourColor = false): CanvasTexture | null {
	const key = `${id}:${fourColor ? '4' : '2'}`;
	const hit = cache.get(key);
	if (hit !== undefined) return hit;

	const canvas = makeCanvas();
	if (canvas === null) return null;
	const ctx = canvas.getContext('2d');
	if (ctx === null) return null;

	drawCardFace(ctx, id[0] as RankId, id[1] as SuitId, { fourColor });
	const tex = finish(canvas);
	cache.set(key, tex);
	return tex;
}

/** The shared card back. */
export function backTexture(hue?: string): CanvasTexture | null {
	const key = `back:${hue ?? 'default'}`;
	const hit = cache.get(key);
	if (hit !== undefined) return hit;

	const canvas = makeCanvas();
	if (canvas === null) return null;
	const ctx = canvas.getContext('2d');
	if (ctx === null) return null;

	drawCardBack(ctx, hue);
	const tex = finish(canvas);
	cache.set(key, tex);
	return tex;
}

/**
 * Build every texture the game can ask for, once.
 *
 * Lazy creation during render is what produced the intermittent blank card: a
 * texture is only cached on success, so any frame where `getContext('2d')`
 * returned null left `faceTexture()` returning null again next frame. Warming
 * the whole 24-card set plus the back at scene mount makes every later call a
 * pure cache hit, so a card's appearance no longer depends on when it happens to
 * be rendered.
 *
 * Cheap enough to be unconditional: 25 canvases at 500x700, drawn once.
 */
export function warmCardTextures(fourColor = false): number {
	if (typeof document === 'undefined') return 0;
	let n = 0;
	for (const suit of ['S', 'H', 'D', 'C'] as const) {
		for (const rank of ['9', 'T', 'J', 'Q', 'K', 'A'] as const) {
			if (faceTexture(`${rank}${suit}` as CardFaceId, fourColor) !== null) n++;
		}
	}
	if (backTexture() !== null) n++;
	return n;
}

/**
 * Release every cached texture.
 *
 * Only needed when tearing down the whole 3D layer; individual cards must never
 * dispose these, because the textures are shared.
 */
export function disposeCardTextures(): void {
	for (const tex of cache.values()) tex.dispose();
	cache.clear();
}

/** Replace the classic deck's cached canvases with the licensed traditional print artwork.
 * Keeping texture identity stable prevents material churn during the initial load.
 */
let traditionalDeck: Promise<void> | undefined;
export function loadTraditionalDeck(): Promise<void> {
	return (traditionalDeck ??= Promise.all(
		[
			...['S', 'H', 'D', 'C'].flatMap((suit) =>
				['9', 'T', 'J', 'Q', 'K', 'A'].map((rank) => `${rank}${suit}`)
			),
			'back'
		].map(async (id) => {
			const texture = id === 'back' ? backTexture() : faceTexture(id as CardFaceId);
			if (!texture) return;
			const image = new Image();
			image.src = `/art/cards/${id}.png`;
			try {
				await image.decode();
				const canvas = texture.image as HTMLCanvasElement;
				const ctx = canvas.getContext('2d');
				if (!ctx) return;
				ctx.clearRect(0, 0, canvas.width, canvas.height);
				ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
				texture.needsUpdate = true;
			} catch {
				/* The procedural deck remains playable if artwork cannot load. */
			}
		})
	).then(() => undefined));
}
