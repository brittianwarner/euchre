import { describe, expect, it, vi } from 'vitest';
import {
	EMPTY_MANIFEST,
	availablePlatforms,
	detectDevice,
	downloadHref,
	fetchManifest,
	openingStep,
	parseManifest,
	platformList,
	type NavigatorLike
} from './downloads.ts';

const UA = {
	macSafari:
		'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15',
	windowsFirefox:
		'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0',
	linuxFirefox: 'Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0',
	ubuntuFirefox: 'Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0',
	chromeOs:
		'Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
	iphone:
		'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1',
	ipadOld:
		'Mozilla/5.0 (iPad; CPU OS 12_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/12.1 Mobile/15E148 Safari/604.1',
	android:
		'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36',
	androidTablet:
		'Mozilla/5.0 (Linux; Android 14; SM-X910) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
	chromeReduced:
		'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
	// Chrome on an Android tablet asking for the desktop site (the default on large tablets).
	chromeAndroidDesktopSite:
		'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'
};

const FILES = {
	mac: { file: 'Euchre-0.4.0-mac.dmg', bytes: 41_000_000, sha256: 'a'.repeat(64) },
	windows: { file: 'Euchre-0.4.0-windows.zip', bytes: 38_000_000, sha256: 'b'.repeat(64) },
	linux: { file: 'Euchre-0.4.0-linux.tar.gz', bytes: 40_000_000, sha256: 'c'.repeat(64) }
};

describe('detectDevice', () => {
	const cases: [string, NavigatorLike, ReturnType<typeof detectDevice>][] = [
		['Safari on a Mac', { userAgent: UA.macSafari, maxTouchPoints: 0 }, desktop('mac')],
		['Firefox on Windows', { userAgent: UA.windowsFirefox }, desktop('windows')],
		['Firefox on Linux', { userAgent: UA.linuxFirefox }, desktop('linux')],
		['Firefox on Ubuntu', { userAgent: UA.ubuntuFirefox }, desktop('linux')],
		['a Chromebook', { userAgent: UA.chromeOs }, desktop(null)],
		['an unknown system', { userAgent: 'curl/8.7.1' }, desktop(null)],
		['an iPhone', { userAgent: UA.iphone, maxTouchPoints: 5 }, { kind: 'handheld' }],
		['an older iPad', { userAgent: UA.ipadOld, maxTouchPoints: 5 }, { kind: 'handheld' }],
		[
			'an iPad asking for the desktop site',
			{ userAgent: UA.macSafari, maxTouchPoints: 5 },
			{ kind: 'handheld' }
		],
		['an Android phone', { userAgent: UA.android }, { kind: 'handheld' }],
		['an Android tablet', { userAgent: UA.androidTablet }, { kind: 'handheld' }]
	];
	for (const [name, nav, want] of cases) {
		it(`reads ${name}`, () => {
			expect(detectDevice(nav)).toEqual(want);
		});
	}

	it('prefers client hints over the user-agent string', () => {
		expect(
			detectDevice({
				userAgent: UA.chromeReduced,
				userAgentData: { platform: 'Windows', mobile: false }
			})
		).toEqual(desktop('windows'));
		expect(
			detectDevice({ userAgent: UA.chromeReduced, userAgentData: { platform: 'Linux' } })
		).toEqual(desktop('linux'));
		expect(
			detectDevice({ userAgent: UA.chromeReduced, userAgentData: { platform: 'macOS' } })
		).toEqual(desktop('mac'));
		expect(
			detectDevice({ userAgent: UA.chromeOs, userAgentData: { platform: 'Chrome OS' } })
		).toEqual(desktop(null));
	});

	it('treats Android and mobile client hints as handheld', () => {
		expect(
			detectDevice({ userAgent: UA.android, userAgentData: { platform: 'Android', mobile: true } })
		).toEqual({ kind: 'handheld' });
		expect(
			detectDevice({
				userAgent: UA.androidTablet,
				userAgentData: { platform: 'Android', mobile: false }
			})
		).toEqual({ kind: 'handheld' });
	});

	it('reads an Android tablet asking for the desktop site as a tablet', () => {
		// Chrome: a Linux user agent and Linux client hints, on a touch-only screen.
		expect(
			detectDevice({
				userAgent: UA.chromeAndroidDesktopSite,
				userAgentData: { platform: 'Linux', mobile: false },
				maxTouchPoints: 5,
				finePointer: false
			})
		).toEqual({ kind: 'handheld' });
		// Firefox: the same user agent with no hints.
		expect(
			detectDevice({ userAgent: UA.linuxFirefox, maxTouchPoints: 5, finePointer: false })
		).toEqual({ kind: 'handheld' });
	});

	it('keeps a touch-screen Linux laptop on the desktop build', () => {
		expect(
			detectDevice({
				userAgent: UA.chromeAndroidDesktopSite,
				userAgentData: { platform: 'Linux', mobile: false },
				maxTouchPoints: 10,
				finePointer: true
			})
		).toEqual(desktop('linux'));
		expect(detectDevice({ userAgent: UA.linuxFirefox, maxTouchPoints: 10 })).toEqual(
			desktop('linux')
		);
	});

	it('reads macOS hints with a touch screen as an iPad, like the user agent', () => {
		expect(
			detectDevice({
				userAgent: UA.chromeReduced,
				userAgentData: { platform: 'macOS', mobile: false },
				maxTouchPoints: 5
			})
		).toEqual({ kind: 'handheld' });
	});

	it('falls back to the user-agent string when the hints name no platform', () => {
		expect(
			detectDevice({ userAgent: UA.linuxFirefox, userAgentData: { platform: '', mobile: false } })
		).toEqual(desktop('linux'));
	});
});

describe('parseManifest', () => {
	it('keeps every well-formed platform', () => {
		const manifest = parseManifest({ version: '0.4.0', platforms: FILES });
		expect(manifest).toEqual({ version: '0.4.0', platforms: FILES });
		expect(availablePlatforms(manifest)).toEqual(['mac', 'windows', 'linux']);
	});

	it('reads the empty manifest the server sends with nothing to offer', () => {
		expect(parseManifest({ version: null, platforms: {} })).toEqual(EMPTY_MANIFEST);
		expect(availablePlatforms(EMPTY_MANIFEST)).toEqual([]);
	});

	it('drops unknown keys and malformed entries, and only those', () => {
		const manifest = parseManifest({
			version: '0.4.0',
			platforms: {
				mac: FILES.mac,
				windows: { file: '../etc/passwd', bytes: 1, sha256: 'x' },
				linux: { file: 'Euchre-0.4.0-linux.tar.gz', bytes: '40', sha256: 'c' },
				ios: FILES.mac
			}
		});
		expect(manifest.platforms).toEqual({ mac: FILES.mac });
		expect(availablePlatforms(manifest)).toEqual(['mac']);
	});

	it('reads anything else as nothing to offer', () => {
		for (const body of [null, 'downloads', [], { platforms: [] }, { version: 3 }]) {
			expect(availablePlatforms(parseManifest(body))).toEqual([]);
		}
	});
});

describe('fetchManifest', () => {
	it('asks for the manifest without a stale cache', async () => {
		const fetchFn = vi.fn(
			async () =>
				new Response(JSON.stringify({ version: '0.4.0', platforms: { linux: FILES.linux } }))
		);
		const manifest = await fetchManifest({ fetch: fetchFn as typeof fetch });
		expect(availablePlatforms(manifest)).toEqual(['linux']);
		expect(fetchFn).toHaveBeenCalledWith(
			'/download/manifest.json',
			expect.objectContaining({ cache: 'no-cache' })
		);
	});

	it('offers nothing when the route is missing or the network fails', async () => {
		const missing = vi.fn(async () => new Response('Not found', { status: 404 }));
		expect(await fetchManifest({ fetch: missing as typeof fetch })).toEqual(EMPTY_MANIFEST);
		const down = vi.fn(async () => {
			throw new TypeError('Failed to fetch');
		});
		expect(await fetchManifest({ fetch: down as typeof fetch })).toEqual(EMPTY_MANIFEST);
		const html = vi.fn(async () => new Response('<!doctype html>'));
		expect(await fetchManifest({ fetch: html as typeof fetch })).toEqual(EMPTY_MANIFEST);
	});

	it('gives up on a manifest that never answers', async () => {
		const hung = vi.fn(
			(_input: RequestInfo | URL, init?: RequestInit) =>
				new Promise<Response>((_, reject) => {
					init?.signal?.addEventListener('abort', () =>
						reject(new DOMException('The operation was aborted.', 'AbortError'))
					);
				})
		);
		const started = Date.now();
		expect(await fetchManifest({ fetch: hung as typeof fetch, timeoutMs: 20 })).toEqual(
			EMPTY_MANIFEST
		);
		expect(Date.now() - started).toBeLessThan(1000);
	});

	it('stops when the page that asked goes away', async () => {
		const hung = vi.fn(
			(_input: RequestInfo | URL, init?: RequestInit) =>
				new Promise<Response>((_, reject) => {
					init?.signal?.addEventListener('abort', () =>
						reject(new DOMException('The operation was aborted.', 'AbortError'))
					);
				})
		);
		const leave = new AbortController();
		const answer = fetchManifest({ fetch: hung as typeof fetch, signal: leave.signal });
		leave.abort();
		expect(await answer).toEqual(EMPTY_MANIFEST);
	});
});

describe('copy', () => {
	it('points each platform at its download route', () => {
		expect(downloadHref('mac')).toBe('/download/mac');
		expect(downloadHref('windows')).toBe('/download/windows');
		expect(downloadHref('linux')).toBe('/download/linux');
	});

	it('lists platforms the way a sentence does', () => {
		expect(platformList(['mac'])).toBe('Mac');
		expect(platformList(['windows', 'linux'])).toBe('Windows and Linux');
		expect(platformList(['mac', 'windows', 'linux'])).toBe('Mac, Windows and Linux');
	});

	it('names the one-time step to open each unsigned build', () => {
		expect(openingStep('mac').text).toContain('System Settings › Privacy & Security › Open Anyway');
		expect(openingStep('windows').text).toContain('More info › Run anyway');
		expect(openingStep('linux', 'Euchre-0.4.0-linux.AppImage')).toEqual({
			text: 'First time: make it runnable, then open it.',
			command: 'chmod +x Euchre-0.4.0-linux.AppImage'
		});
		expect(openingStep('linux', 'Euchre-0.4.0-linux.deb').command).toBe(
			'sudo apt install ./Euchre-0.4.0-linux.deb'
		);
		expect(openingStep('linux', 'Euchre-0.4.0-linux.tar.gz')).toEqual({
			text: 'First time: unpack it, then run euchre from the folder.'
		});
		expect(openingStep('linux').text).toContain('chmod +x');
	});
});

function desktop(platform: 'mac' | 'windows' | 'linux' | null): ReturnType<typeof detectDevice> {
	return { kind: 'desktop', platform };
}
