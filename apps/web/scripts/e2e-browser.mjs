/**
 * Full-match browser proof against the Rust euchre-server (D4 static SPA).
 *
 * Walks one fresh browser through the whole product: home, the identity gate,
 * settings (edit + save), the /new page, a game started from the home page's
 * Play now with the trick-review gate on, bidding,
 * every trick, hand results and the hand-review dialog, a reload mid-hand
 * (resume), game over, the scrapbook and match detail pages, settings again,
 * and sign-out. Exits nonzero on any page error, any 4xx/5xx response, or a
 * match that does not finish in time.
 *
 *   EUCHRE_URL=http://127.0.0.1:3000 E2E_SHOTS=/tmp/euchre-e2e bun scripts/e2e-browser.mjs
 *
 * Start the server first (README.md, "Browser proof"). `EUCHRE_FAST_TEMPO=1` on
 * the server keeps a full match to a few minutes.
 */
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const BASE = (process.env.EUCHRE_URL ?? 'http://127.0.0.1:3000').replace(/\/$/, '');
const SHOTS = process.env.E2E_SHOTS ?? '/tmp/euchre-e2e';
const LABEL = process.env.E2E_LABEL ?? 'run';
const MATCH_BUDGET_MS = Number(process.env.E2E_MATCH_BUDGET_MS ?? 900_000);
mkdirSync(SHOTS, { recursive: true });

const email = `e2e-${Date.now()}@example.com`;
const westName = 'Westie';
const log = (...args) => console.log(`[${LABEL}]`, ...args);

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

const shots = [];
async function shot(name) {
	const path = join(SHOTS, `${LABEL}-${name}.png`);
	await page.screenshot({ path });
	shots.push(path);
}

/* 1. Home: the wordmark, the picture, Play now and Download; no nav ------ */
const manifestAnswers = [];
page.on('response', (r) => {
	if (new URL(r.url()).pathname === '/download/manifest.json') manifestAnswers.push(r.status());
});
await page.goto(`${BASE}/`, { waitUntil: 'networkidle', timeout: 60_000 });
await page.getByRole('heading', { level: 1, name: 'euchre' }).waitFor();
await page.getByRole('button', { name: 'Play now' }).waitFor();
await page.locator('.stage.shown').waitFor();
assert.equal(await page.locator('nav, footer').count(), 0, 'the home page shows a nav or footer');
assert.deepEqual(manifestAnswers, [200], 'the home page did not read the download manifest');
await shot('01-home');

/* 2. Identity gate on /games --------------------------------------------- */
await page.goto(`${BASE}/games`, { waitUntil: 'networkidle' });
await page.getByRole('heading', { name: 'Whose scrapbook is this?' }).waitFor();
await shot('02-games-gate');
await page.getByLabel('Your email').fill(email);
await page.getByRole('button', { name: 'Remember me here' }).click();
await page.getByText(`Playing as ${email}`).waitFor({ timeout: 15_000 });
await shot('03-games-identified');

/* 3. Settings: edit a persona and save ----------------------------------- */
await page.goto(`${BASE}/settings`, { waitUntil: 'networkidle' });
await page.getByRole('heading', { level: 1, name: 'Settings' }).waitFor();
const westInput = page.locator('input[name="seat1_name"]');
await westInput.waitFor();
await westInput.fill(westName);
await page.getByRole('button', { name: 'Save changes' }).click();
await page.getByText('Saved. Your next match will use these.').waitFor({ timeout: 15_000 });
assert.equal(await westInput.inputValue(), westName);
await shot('04-settings-saved');
await page.reload({ waitUntil: 'networkidle' });
await westInput.waitFor();
assert.equal(await westInput.inputValue(), westName, 'saved persona name did not persist');

/* 4. New game: /new still opens, and the home page's Play now deals ------ */
await page.goto(`${BASE}/new`, { waitUntil: 'networkidle' });
await page.getByRole('heading', { name: 'Your table. Your rules.' }).waitFor();
await shot('05-new');
await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
await page.locator('.stage.shown').waitFor();
await page.getByRole('button', { name: 'Play now' }).click();
await page.waitForURL(/\/play\/[0-9a-f-]{36}$/, { timeout: 30_000 });
const url = page.url();
const gameId = new URL(url).pathname.split('/').at(-1);
await page.waitForSelector('canvas', { timeout: 30_000 });
log('table', gameId);

/** Dismiss the first-run tour; with `waitMs`, first give it that long to open. */
async function skipTour(waitMs = 0) {
	const skip = page.getByRole('button', { name: 'Skip', exact: true });
	if (waitMs) await skip.waitFor({ state: 'visible', timeout: waitMs }).catch(() => {});
	if (await skip.isVisible()) await skip.click();
}

async function openTableSettings() {
	const trigger = page.locator('.settings-trigger');
	if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
	await page.locator('#table-settings').waitFor({ state: 'visible' });
}

async function closeTableSettings() {
	const trigger = page.locator('.settings-trigger');
	if ((await trigger.getAttribute('aria-expanded')) === 'true')
		await page.getByRole('button', { name: 'Close settings' }).click();
}

async function setReviewGate(enabled) {
	await openTableSettings();
	const toggle = page.getByRole('checkbox', { name: /Wait for me after each trick/ });
	await toggle.waitFor();
	for (let i = 0; i < 50 && (await toggle.isDisabled()); i++) await page.waitForTimeout(100);
	if ((await toggle.isChecked()) !== enabled) await toggle.click();
	await page.waitForFunction(
		(want) => {
			const input = [...document.querySelectorAll('.review-setting input')][0];
			return input && input.checked === want && !input.disabled;
		},
		enabled,
		{ timeout: 15_000 }
	);
}

// The table opens on the cut or the first bid. Wait until the player holds a
// card or has a move (their own cut). The first-run tour opens once cards are
// dealt, which at production tempo comes seconds after the table, so give it a
// moment before touching settings. When the player cuts first, nothing is dealt
// until they do; the loop below then dismisses the tour.
await page.locator('.hand-area').waitFor({ timeout: 30_000 });
await page
	.locator('.hand-area [data-card-id], .bids button:not(:disabled)')
	.first()
	.waitFor({ timeout: 60_000 });
await skipTour(5_000);
await setReviewGate(true);
await page.getByText(westName).first().waitFor();
// With no nav on the home page, the table is the way to the scrapbook and the
// opponents' settings.
for (const href of ['/games', '/settings', '/new', '/']) {
	assert.ok(
		await page.locator(`#table-settings a[href="${href}"]`).count(),
		`the table settings have no link to ${href}`
	);
}
await shot('06-table-settings');
await closeTableSettings();

/* 5. Play the whole match ------------------------------------------------- */
const stats = {
	bids: 0,
	plays: 0,
	discards: 0,
	reviewContinues: 0,
	handResults: 0,
	reloads: 0
};
const seen = new Set();
let inHandScore = false;
let reviewedHand = false;
let resumed = false;
let gateTurnedOff = false;
let mobileChecked = false;
let complete = false;
const start = Date.now();

/** Click when the target settles within a moment; `false` when it went away. */
async function clickSoon(locator) {
	return locator.click({ timeout: 3_000 }).then(
		() => true,
		() => false
	);
}

async function rackCount() {
	return page.locator('.hand-area [data-card-id]').count();
}

async function scoreText() {
	return (await page.locator('section.score .scoreline').innerText()).replace(/\s+/g, ' ').trim();
}

while (Date.now() - start < MATCH_BUDGET_MS) {
	await skipTour();
	if (await page.getByRole('link', { name: 'Play again' }).count()) {
		complete = true;
		break;
	}
	const instruction =
		(await page
			.locator('p.instruction')
			.innerText()
			.catch(() => '')) ?? '';

	// Hand results: the table holds `hand_score` while it adds the points.
	if (instruction.startsWith('Hand complete')) {
		if (!inHandScore) {
			stats.handResults++;
			if (!seen.has('hand-result')) {
				seen.add('hand-result');
				await shot('10-hand-result');
			}
		}
		inHandScore = true;
	} else inHandScore = false;

	// After the first hand: read it back through the hand-review dialog.
	if (stats.handResults >= 1 && !reviewedHand && !inHandScore) {
		await openTableSettings();
		await page.getByRole('button', { name: '↶ Review hands' }).click();
		const dialog = page.locator('dialog[open]');
		await dialog.waitFor();
		await dialog.locator('select').selectOption('0');
		await dialog
			.getByText(/were trump\.|Everyone passed/)
			.first()
			.waitFor({ timeout: 15_000 });
		await shot('11-hand-review');
		await dialog.getByRole('button', { name: 'Close trick review' }).click();
		await closeTableSettings();
		reviewedHand = true;
		continue;
	}

	// The review gate: Continue after every trick while it is on.
	const cont = page.locator('.trick-result button:not(:disabled)');
	if (await cont.count()) {
		if (!seen.has('review')) {
			seen.add('review');
			await shot('09-review-gate');
		}
		// The button stays up while the trick sweeps away, so a second pass can
		// find it mid-outro; a click that misses just loops again.
		if (await clickSoon(cont.first())) stats.reviewContinues++;
		await page.waitForTimeout(150);
		continue;
	}

	// Turn the gate off after two hands so both table modes are exercised.
	if (stats.handResults >= 2 && !gateTurnedOff && !inHandScore) {
		await setReviewGate(false);
		await closeTableSettings();
		gateTurnedOff = true;
		continue;
	}

	const bids = page.locator('.bids button:not(:disabled)');
	const cards = page.locator('[data-card-id][aria-disabled="false"]');
	const inRack = await rackCount();

	// Desktop + phone layout once a fresh hand is on the table.
	if (!mobileChecked && inRack === 5 && !(await page.locator('dialog[open]').count())) {
		await page.waitForTimeout(500);
		await shot('07-table-desktop');
		await page.setViewportSize({ width: 390, height: 844 });
		await page.waitForTimeout(1000);
		await shot('07-table-mobile');
		assert.equal(
			await page.evaluate(() => document.documentElement.scrollWidth > innerWidth),
			false,
			'horizontal scroll at phone width'
		);
		await page.setViewportSize({ width: 1440, height: 900 });
		mobileChecked = true;
	}

	// Reload mid-hand, in a later hand, and resume the same table.
	if (
		!resumed &&
		stats.handResults >= 1 &&
		inRack >= 1 &&
		inRack <= 4 &&
		instruction !== '' &&
		!inHandScore
	) {
		const before = { cards: inRack, score: await scoreText() };
		await page.reload({ waitUntil: 'networkidle' });
		await page.waitForSelector('canvas');
		assert.equal(page.url(), url, 'reload changed the table');
		await page.locator('.hand-area [data-card-id]').first().waitFor({ timeout: 30_000 });
		await skipTour();
		const after = { cards: await rackCount(), score: await scoreText() };
		assert.deepEqual(after, before, 'resumed table differs from the table before reload');
		await page.waitForTimeout(1500);
		const drawsBefore = await page.evaluate(() => window.__draws);
		await page.waitForTimeout(1500);
		log('idleDrawCallsOver1.5s', (await page.evaluate(() => window.__draws)) - drawsBefore);
		await shot('12-resumed');
		stats.reloads++;
		resumed = true;
		continue;
	}

	if (await bids.count()) {
		if (!seen.has('bid')) {
			seen.add('bid');
			await shot('08-bidding');
		}
		// Call trump rather than indefinitely passing. The engine still offers only legal moves.
		const count = await bids.count();
		if (await clickSoon(bids.nth(count > 1 ? 1 : 0))) stats.bids++;
	} else if (await cards.count()) {
		const discard = (await page.locator('.confirm-card').innerText()).startsWith('Discard');
		await cards.first().focus();
		await cards.first().press('Enter');
		assert.equal(await cards.first().getAttribute('aria-pressed'), 'true');
		if (await clickSoon(page.locator('.confirm-card'))) {
			if (discard) stats.discards++;
			else stats.plays++;
		}
		if (!seen.has('trick')) {
			seen.add('trick');
			await page.waitForTimeout(400);
			await shot('08-trick');
		}
	}
	await page.waitForTimeout(150);
}

/* 6. Game over ------------------------------------------------------------ */
assert.equal(complete, true, `match did not finish within ${MATCH_BUDGET_MS} ms`);
await page.getByRole('heading', { name: 'A good game, well played.' }).waitFor();
const finalScore = await scoreText();
await page.waitForTimeout(800);
await shot('13-game-over');
log('final score', finalScore);

/* 7. Scrapbook and match detail ------------------------------------------ */
await page.goto(`${BASE}/games`, { waitUntil: 'networkidle' });
await page.getByText(`Playing as ${email}`).waitFor();
await page.locator(`a[href="/play/${gameId}"]`).waitFor();
const matchLink = page.locator('a[href^="/games/"]').first();
await matchLink.waitFor({ timeout: 15_000 });
assert.equal(
	(await page.locator('.stats .value').first().innerText()).trim(),
	'1',
	'scrapbook should list exactly this match'
);
await shot('14-games');
await matchLink.click();
await page.waitForURL(/\/games\/[^/?]+/);
await page
	.getByRole('heading', { level: 1, name: /^(You won|They won|Left unfinished)$/ })
	.waitFor();
const hands = await page.locator('ol.hands li.hand').count();
assert.ok(hands >= 1, 'match detail lists no hands');
await shot('15-match-detail');

/* 8. Settings again, then sign out --------------------------------------- */
await page.goto(`${BASE}/settings`, { waitUntil: 'networkidle' });
await westInput.waitFor();
assert.equal(await westInput.inputValue(), westName);
await shot('16-settings');
await page.getByRole('button', { name: 'Not you? Switch profile' }).click();
await page
	.getByRole('heading', { name: 'Whose opponents are these?' })
	.waitFor({ timeout: 15_000 });
await shot('17-signed-out');

await browser.close();

const summary = {
	label: LABEL,
	url,
	email,
	finalScore,
	...stats,
	handsInReplay: hands,
	complete,
	resumed,
	errors,
	badResponses: [...new Set(badResponses)],
	elapsedMs: Date.now() - start,
	shots: shots.length
};
console.log(JSON.stringify(summary));
assert.equal(errors.length, 0, `page errors: ${errors.join(' | ')}`);
assert.equal(badResponses.length, 0, `bad responses: ${[...new Set(badResponses)].join(', ')}`);
assert.ok(stats.reviewContinues > 0, 'the review gate never held a trick');
assert.ok(stats.handResults >= 2, 'fewer than two hand results seen');
assert.equal(resumed, true, 'never reloaded mid-hand');
