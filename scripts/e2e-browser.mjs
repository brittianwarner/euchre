import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { createClient } from 'rivetkit/client';

const browser = await chromium.launch({
	args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader']
});
const context = await browser.newContext({
	viewport: { width: 1440, height: 900 },
	deviceScaleFactor: 1
});
const page = await context.newPage();
const errors = [];
const badResponses = [];
page.on('pageerror', (e) => errors.push(e.message));
page.on('response', (r) => {
	if (r.status() >= 400) badResponses.push(`${r.status()} ${new URL(r.url()).pathname}`);
});
await page.addInitScript(() => {
	window.__draws = 0;
	for (const kind of [WebGLRenderingContext, WebGL2RenderingContext]) {
		for (const method of [
			'drawArrays',
			'drawElements',
			'drawArraysInstanced',
			'drawElementsInstanced'
		]) {
			const original = kind.prototype[method];
			if (!original) continue;
			kind.prototype[method] = function (...args) {
				window.__draws++;
				return original.apply(this, args);
			};
		}
	}
});
await page.goto('http://127.0.0.1:5173/play', { waitUntil: 'networkidle', timeout: 60000 });
await page.waitForSelector('canvas', { timeout: 30000 });
const url = page.url();
let moves = 0;
let captured = false;
let complete = false;
const start = Date.now();
while (Date.now() - start < 480000) {
	const skip = page.getByRole('button', { name: 'Skip', exact: true });
	if (await skip.isVisible()) await skip.click();
	if (await page.getByRole('link', { name: 'Play again' }).count()) {
		complete = true;
		break;
	}
	const bids = page.locator('.bids button:not(:disabled)');
	const cards = page.locator('[data-card-id][aria-disabled="false"]');
	if (
		!captured &&
		(await page.locator('[data-card-id]').count()) === 5 &&
		!(await page.locator('dialog[open]').count())
	) {
		await page.waitForTimeout(700);
		await page.screenshot({ path: '/tmp/euchre-table-desktop.png' });
		await page.setViewportSize({ width: 390, height: 844 });
		await page.waitForTimeout(1200);
		await page.screenshot({ path: '/tmp/euchre-table-mobile.png' });
		assert.equal(
			await page.evaluate(() => document.documentElement.scrollWidth > innerWidth),
			false
		);
		await page.setViewportSize({ width: 1440, height: 900 });
		// Reload the same match and confirm the table reconnects without creating a new match.
		await page.reload({ waitUntil: 'networkidle' });
		await page.waitForSelector('canvas');
		assert.equal(page.url(), url);
		await page.waitForTimeout(1500);
		const before = await page.evaluate(() => window.__draws);
		await page.waitForTimeout(1500);
		console.log('idleDrawCallsOver1.5s', (await page.evaluate(() => window.__draws)) - before);
		captured = true;
	}
	if (await bids.count()) {
		// Call trump rather than indefinitely passing. The engine still offers only legal moves.
		const count = await bids.count();
		await bids.nth(count > 1 ? 1 : 0).click();
		moves++;
	} else if (await cards.count()) {
		await cards.first().focus();
		await cards.first().press('Enter');
		assert.equal(await cards.first().getAttribute('aria-pressed'), 'true');
		await page.locator('.confirm-card').click();
		moves++;
	}
	await page.waitForTimeout(180);
}
console.log(
	JSON.stringify({
		url,
		moves,
		complete,
		captured,
		errors,
		badResponses: [...new Set(badResponses)],
		elapsedMs: Date.now() - start
	})
);
await page.screenshot({ path: '/tmp/euchre-end.png' });
await browser.close();
const client = createClient({ endpoint: 'http://127.0.0.1:6420', devtools: false });
const gameId = new URL(url).pathname.split('/').at(-1);
const status = await Promise.all(
	[1, 2, 3].map((seat) => client.aiSeat.get(['table', gameId, 'seat', String(seat)]).getStatus())
);
console.log('AI_METERS', JSON.stringify(status));
assert.ok(
	status.every((s) => s.llmCalls > 0),
	'Live E2E requires configured Jev decisions for all seats'
);
// This legacy counter includes provider timeouts. Legal heuristic failover is
// supported behavior; schema rejection and legal-set checks have unit coverage.
assert.equal(badResponses.length, 0);
assert.equal(errors.length, 0);
assert.equal(complete, true, 'match did not finish within eight minutes');
