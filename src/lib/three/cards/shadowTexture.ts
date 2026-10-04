import { CanvasTexture } from 'three';
let cached: CanvasTexture | undefined;
/** One shared soft contact texture. No lights, shadow maps or blur render passes. */
export function cardShadow(): CanvasTexture | undefined {
	if (cached || typeof document === 'undefined') return cached;
	const canvas = document.createElement('canvas');
	canvas.width = 128;
	canvas.height = 176;
	const ctx = canvas.getContext('2d');
	if (!ctx) return;
	ctx.filter = 'blur(6px)';
	ctx.fillStyle = '#000';
	ctx.beginPath();
	ctx.roundRect(12, 12, 104, 152, 9);
	ctx.fill();
	cached = new CanvasTexture(canvas);
	return cached;
}
