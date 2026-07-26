import { chromium } from 'playwright';
const b = await chromium.launch({ args: ['--use-gl=swiftshader','--enable-unsafe-swiftshader'] });
const p = await b.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2 });
await p.goto('http://localhost:5199/play', { waitUntil:'networkidle', timeout:60000 });
await p.waitForTimeout(4000);
for (const label of ['Bump', "Run 'em"]) {
  const btn = p.locator(`button:has-text("${label}")`).first();
  if (await btn.count() && await btn.isVisible().catch(()=>false)) { await btn.click().catch(()=>{}); break; }
}
await p.waitForTimeout(6000);
await p.screenshot({ path: '/tmp/shots/table-portrait.png' });
console.log('state:', (await p.textContent('body')).replace(/\s+/g,' ').replace(/.*?Euchre/,'').slice(0,150));
await b.close();
