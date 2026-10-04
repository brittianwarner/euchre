import { chromium } from 'playwright';
const browser = await chromium.launch({
	args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader']
});
const errors = [];
for (const [width, height] of [
	[1440, 900],
	[390, 844]
]) {
	const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
	page.on('pageerror', (e) => errors.push(e.message));
	await page.goto('http://127.0.0.1:5173/', { waitUntil: 'networkidle' });
	await page.screenshot({ path: `/tmp/euchre-home-${width}.png`, fullPage: true });
	console.log(
		'home',
		width,
		await page.title(),
		'overflow',
		await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
	);
	await page.close();
}
console.log('errors', errors);
await browser.close();
