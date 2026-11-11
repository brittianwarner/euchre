import { chromium } from 'playwright';
import assert from 'node:assert/strict';

const browser = await chromium.launch({
	args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader']
});
const page = await browser.newPage({
	viewport: { width: 390, height: 844 },
	isMobile: true,
	hasTouch: true
});
await page.goto('http://127.0.0.1:5173/play', { waitUntil: 'domcontentloaded', timeout: 60000 });
await page.waitForSelector('canvas', { timeout: 60000 });
const until = Date.now() + 90000;
let tapped = false;
while (Date.now() < until && !tapped) {
	const skip = page.getByRole('button', { name: 'Skip', exact: true });
	if (await skip.isVisible()) await skip.tap();
	const bids = page.locator('.bids button:not(:disabled)');
	const cards = page.locator('[data-card-id][aria-disabled="false"]');
	if (await bids.count()) await bids.nth((await bids.count()) > 1 ? 1 : 0).tap();
	else if (await cards.count()) {
		const before = await page.locator('[data-card-id]').count();
		for (const card of await cards.all()) {
			const box = await card.boundingBox();
			assert.ok(box.width >= 44 && box.height >= 44);
			await card.tap();
			assert.equal(await card.getAttribute('aria-pressed'), 'true');
			assert.deepEqual(await card.boundingBox(), box, 'selection must not move the tap target');
		}
		await page.locator('.confirm-card').tap();
		await page.waitForFunction(
			(count) => document.querySelectorAll('[data-card-id]').length < count,
			before
		);
		tapped = true;
	}
	await page.waitForTimeout(180);
}
await page.screenshot({ path: '/tmp/euchre-touch.png' });
console.log({ tappedCard: tapped, url: page.url() });
await browser.close();
assert.ok(tapped, 'A touch selection followed by confirmation must play the selected card');
