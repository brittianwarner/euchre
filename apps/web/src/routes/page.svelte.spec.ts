import { page } from 'vitest/browser';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';
import { PLATFORM_NAMES, currentDevice, type Platform } from '#lib/home/downloads.ts';
import Page from './+page.svelte';

const startPlaying = vi.hoisted(() => vi.fn<() => Promise<string>>());
vi.mock('#lib/client/start-playing.ts', () => ({ startPlaying }));

const ALL = {
	version: '0.4.0',
	platforms: {
		mac: { file: 'Euchre-0.4.0-mac.dmg', bytes: 41_000_000, sha256: 'a'.repeat(64) },
		windows: { file: 'Euchre-0.4.0-windows.zip', bytes: 38_000_000, sha256: 'b'.repeat(64) },
		linux: { file: 'Euchre-0.4.0-linux.tar.gz', bytes: 40_000_000, sha256: 'c'.repeat(64) }
	}
};
const NONE = { version: null, platforms: {} };
const IPHONE =
	'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1';

/** This browser's own platform, as the page reads it (the tests run on a desktop). */
function ownPlatform(): Platform {
	const device = currentDevice();
	if (device.kind !== 'desktop' || device.platform === null)
		throw new Error('the browser tests need a Mac, Windows or Linux desktop');
	return device.platform;
}

function serveManifest(body: unknown): ReturnType<typeof vi.fn> {
	const fetchMock = vi.fn(async (input: RequestInfo | URL) =>
		String(input).endsWith('/download/manifest.json')
			? Response.json(body)
			: new Response('Not found', { status: 404 })
	);
	vi.stubGlobal('fetch', fetchMock);
	return fetchMock;
}

/** Wait for the page's one entrance to finish (boxes measured mid-rise would be 8px low). */
async function shown(): Promise<void> {
	await expect.element(page.getByRole('heading', { level: 1, name: 'euchre' })).toBeVisible();
	await vi.waitFor(() => {
		const stage = document.querySelector('.stage');
		expect(stage && getComputedStyle(stage).transform).toBe('none');
	});
}

/** Keep test clicks on download links from starting real downloads. */
function blockDownloads(event: MouseEvent): void {
	if ((event.target as Element | null)?.closest('a[download]')) event.preventDefault();
}

beforeEach(() => {
	startPlaying.mockReset();
	document.addEventListener('click', blockDownloads, true);
});

afterEach(() => {
	document.removeEventListener('click', blockDownloads, true);
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe('/+page.svelte', () => {
	it('shows only the wordmark, the picture, Play now and Download', async () => {
		const fetchMock = serveManifest(ALL);
		const own = ownPlatform();
		const screen = await render(Page);
		await shown();

		const download = page.getByRole('link', { name: `Download for ${PLATFORM_NAMES[own]}` });
		await expect.element(download).toBeVisible();
		await expect.element(download).toHaveAttribute('href', `/download/${own}`);
		await expect.element(page.getByRole('button', { name: 'Play now' })).toBeVisible();

		const art = page.getByRole('img');
		await expect.element(art).toHaveAttribute('src', '/art/euchre-hero.webp');
		await expect.element(art).toHaveAttribute('width', '1200');
		await expect.element(art).toHaveAttribute('height', '1000');
		await expect.element(art).toHaveAttribute('fetchpriority', 'high');

		const root = screen.container;
		expect(root.querySelectorAll('nav, footer, section, h2, h3')).toHaveLength(0);
		expect(root.querySelectorAll('button')).toHaveLength(1);
		// The two other builds are offered as links in the quiet line.
		const others = (['mac', 'windows', 'linux'] as const).filter((p) => p !== own);
		const note = root.querySelector('.note');
		expect(note?.textContent?.replace(/\s+/g, ' ').trim()).toBe(
			`Also on ${PLATFORM_NAMES[others[0]]} and ${PLATFORM_NAMES[others[1]]}`
		);
		// The list is not announced; only what answers a press is.
		expect(note?.querySelector('[aria-live] a')).toBeNull();
		expect(note?.querySelector('[aria-live]')?.textContent).toBe('');
		for (const platform of others) {
			await expect
				.element(page.getByRole('link', { name: PLATFORM_NAMES[platform], exact: true }))
				.toHaveAttribute('href', `/download/${platform}`);
		}
		expect(fetchMock).toHaveBeenCalledWith(
			'/download/manifest.json',
			expect.objectContaining({ cache: 'no-cache' })
		);
	});

	it('stands Play now alone when no build is offered', async () => {
		serveManifest(NONE);
		const screen = await render(Page);
		await shown();
		const root = screen.container;
		expect(root.querySelector('a')).toBeNull();
		expect(root.querySelector('.note')?.textContent?.trim()).toBe('');
		const play = root.querySelector('button')!.getBoundingClientRect();
		const art = root.querySelector('.art')!.getBoundingClientRect();
		expect(Math.abs(play.left + play.width / 2 - (art.left + art.width / 2))).toBeLessThan(1);
	});

	it('keeps every box in place when the manifest answers after the page appears', async () => {
		let answer: (body: unknown) => void = () => {};
		const late = new Promise<unknown>((resolve) => {
			answer = resolve;
		});
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => Response.json(await late))
		);
		const screen = await render(Page);
		await shown();
		const boxes = (): number[][] =>
			['.wordmark', '.art', '.actions', 'button', 'a.secondary', '.note'].map((selector) => {
				const rect = screen.container.querySelector(selector)!.getBoundingClientRect();
				return [rect.left, rect.top, rect.width, rect.height];
			});
		const before = boxes();
		const download = screen.container.querySelector<HTMLAnchorElement>('a.secondary')!;
		expect(getComputedStyle(download).visibility).toBe('hidden');

		answer(ALL);
		await expect.element(page.getByRole('link', { name: /^Download for/ })).toBeVisible();
		expect(boxes()).toEqual(before);
	});

	it('closes up out of sight when a late manifest offers nothing', async () => {
		let answer: (body: unknown) => void = () => {};
		const late = new Promise<unknown>((resolve) => {
			answer = resolve;
		});
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => Response.json(await late))
		);
		const shifts: number[] = [];
		const observer = new PerformanceObserver((list) => {
			for (const entry of list.getEntries())
				shifts.push((entry as PerformanceEntry & { value: number }).value);
		});
		observer.observe({ type: 'layout-shift' });
		const screen = await render(Page);
		await shown();
		const download = screen.container.querySelector<HTMLAnchorElement>('a.secondary')!;
		expect(getComputedStyle(download).visibility).toBe('hidden');

		answer(NONE);
		// The empty Download slot closes up out of sight (no layout shift), and
		// Play now ends in the middle.
		await vi.waitFor(() => expect(screen.container.querySelector('a.secondary')).toBeNull());
		await vi.waitFor(() => expect(screen.container.querySelector('.regroup')).toBeNull());
		const play = screen.container.querySelector('button')!.getBoundingClientRect();
		const art = screen.container.querySelector('.art')!.getBoundingClientRect();
		expect(Math.abs(play.left + play.width / 2 - (art.left + art.width / 2))).toBeLessThan(1);
		await new Promise((resolve) => setTimeout(resolve, 100));
		observer.disconnect();
		expect(shifts).toEqual([]);
	});

	it('never lets the club touch the word', async () => {
		serveManifest(NONE);
		const screen = await render(Page);
		await shown();
		const word = screen.container.querySelector('.word')!.getBoundingClientRect();
		const club = screen.container.querySelector('.club')!.getBoundingClientRect();
		const mark = screen.container.querySelector('.wordmark')!;
		const size = parseFloat(getComputedStyle(mark).fontSize);
		expect(club.left - word.right).toBeGreaterThan(0.05 * size);
	});

	it('offers only what the manifest lists', async () => {
		const own = ownPlatform();
		const other = (['mac', 'windows', 'linux'] as const).find((p) => p !== own)!;
		serveManifest({ version: '0.4.0', platforms: { [other]: ALL.platforms[other] } });
		const screen = await render(Page);
		await shown();
		await expect
			.element(page.getByRole('link', { name: PLATFORM_NAMES[other], exact: true }))
			.toBeVisible();
		expect(screen.container.querySelector('.note')?.textContent?.replace(/\s+/g, ' ').trim()).toBe(
			`Download for ${PLATFORM_NAMES[other]}`
		);
		// This computer has no build, so there is no Download button at all.
		expect(screen.container.querySelector('a.secondary')).toBeNull();
	});

	it('shows the one-time opening step after a download', async () => {
		serveManifest(ALL);
		const own = ownPlatform();
		await render(Page);
		await shown();
		await page.getByRole('link', { name: `Download for ${PLATFORM_NAMES[own]}` }).click();
		const step = {
			mac: 'System Settings › Privacy & Security › Open Anyway',
			windows: 'More info › Run anyway',
			linux: 'unpack it'
		}[own];
		await expect.element(page.getByText(step, { exact: false })).toBeVisible();
	});

	it('starts a game from Play now, and lets the player try again after a failure', async () => {
		serveManifest(NONE);
		let fail: (reason: Error) => void = () => {};
		startPlaying.mockImplementationOnce(
			() =>
				new Promise<string>((_, reject) => {
					fail = reject;
				})
		);
		await render(Page);
		await shown();

		await page.getByRole('button', { name: 'Play now' }).click();
		const pending = page.getByRole('button', { name: 'Dealing…' });
		await expect.element(pending).toHaveAttribute('aria-busy', 'true');
		// A second press while the table is on its way does nothing.
		await pending.click({ force: true });
		expect(startPlaying).toHaveBeenCalledTimes(1);

		fail(new Error('503'));
		await expect
			.element(page.getByText('We couldn’t start a game. Please try again.'))
			.toBeVisible();
		startPlaying.mockResolvedValueOnce('0195f0c2-7a3e-7d10-9b1a-2f6c1d8e4a90');
		await page.getByRole('button', { name: 'Play now' }).click();
		expect(startPlaying).toHaveBeenCalledTimes(2);
		await expect
			.element(page.getByText('We couldn’t start a game. Please try again.'))
			.not.toBeInTheDocument();
	});

	it('shows only Play now and the desktop builds on a phone', async () => {
		vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(IPHONE);
		vi.spyOn(navigator, 'maxTouchPoints', 'get').mockReturnValue(5);
		if ('userAgentData' in navigator)
			vi.spyOn(
				navigator as unknown as { userAgentData: unknown },
				'userAgentData',
				'get'
			).mockReturnValue(undefined);
		serveManifest(ALL);
		const screen = await render(Page);
		await shown();
		await expect.element(page.getByText('Also on Mac, Windows and Linux')).toBeVisible();
		expect(screen.container.querySelectorAll('a')).toHaveLength(0);
		expect(screen.container.querySelectorAll('button')).toHaveLength(1);
	});
});
