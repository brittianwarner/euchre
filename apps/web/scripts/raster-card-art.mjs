import { chromium } from 'playwright';
import { readdir, readFile, writeFile } from 'node:fs/promises';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage();
for (const file of (await readdir('static/art/cards')).filter((x) => x.endsWith('.svg'))) {
	const svg = await readFile(`static/art/cards/${file}`, 'utf8');
	const png = await page.evaluate(async (svg) => {
		const im = new Image();
		im.src = 'data:image/svg+xml;base64,' + btoa(unescape(encodeURIComponent(svg)));
		await im.decode();
		const c = document.createElement('canvas');
		c.width = 512;
		c.height = 716;
		c.getContext('2d').drawImage(im, 0, 0, 512, 716);
		return c.toDataURL('image/png').split(',')[1];
	}, svg);
	await writeFile(`static/art/cards/${file.replace('.svg', '.png')}`, Buffer.from(png, 'base64'));
}
await browser.close();
