/**
 * Screenshot the live 3D euchre table.
 *
 * Opens /play, gets past the cut so a hand actually exists (an un-cut table shows
 * only the deck, which looks like a bug and is not), waits for the AI seats to
 * settle, and writes a PNG.
 *
 *   node scripts/shot-table.mjs <outdir> [port] [width] [height]
 *
 * Port defaults to $PORT then 5173 (vite's default), so it works against whatever
 * dev server is already running rather than assuming one.
 */
import { chromium } from 'playwright';

const outDir = process.argv[2] ?? '.';
const port = Number(process.argv[3] ?? process.env.PORT ?? 5173);
const width = Number(process.argv[4] ?? 1440);
const height = Number(process.argv[5] ?? 900);
const base = `http://127.0.0.1:${port}`;

const browser = await chromium.launch({
	// Headless Chromium has no GPU; without SwiftShader the WebGL context fails to
	// create and every screenshot is a blank page.
	args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader']
});
const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1.5 });

const errors = [];
page.on('pageerror', (e) => errors.push(`PAGEERROR ${String(e).slice(0, 220)}`));
page.on('console', (m) => {
	const t = m.text();
	// The Rivet devtools asset 404s in dev and is harmless.
	if (m.type() === 'error' && !t.includes('devtools')) errors.push(`CONSOLE ${t.slice(0, 220)}`);
});

await page.goto(`${base}/play`, { waitUntil: 'networkidle', timeout: 60_000 });
await page.waitForTimeout(1800);
const skip = page.getByRole('button', { name: 'Skip', exact: true });
if (await skip.isVisible()) await skip.click();

for (const label of ['Bump', "Run 'em"]) {
	const btn = page.locator(`button:has-text("${label}")`).first();
	if ((await btn.count()) > 0 && (await btn.isVisible().catch(() => false))) {
		await btn.click().catch(() => {});
		break;
	}
}
await page.waitForTimeout(1800);

const suffix = width < height ? 'portrait' : 'landscape';
const path = `${outDir}/table-${suffix}-${width}x${height}.png`;
await page.screenshot({ path });

const body = (await page.textContent('body').catch(() => '')) ?? '';
console.log(`url        ${page.url()}`);
console.log(`screenshot ${path}`);
console.log(`canvases   ${await page.locator('canvas').count()}`);
console.log(
	`state      ${body
		.replace(/\s+/g, ' ')
		.replace(/.*?Euchre/, '')
		.slice(0, 170)}`
);
console.log(`errors     ${errors.length ? errors.slice(0, 8).join('\n           ') : '(none)'}`);
console.log(
	'\nNow READ the PNG. A clean typecheck has never once caught a visual bug in this project.'
);

await browser.close();
