/**
 * Desktop downloads, as the home page offers them.
 *
 * The server and this page share one contract (docs/port/downloads/):
 *
 * - `GET /download/manifest.json` is
 *   `{ version, platforms: { mac?, windows?, linux? } }`, each entry
 *   `{ file, bytes, sha256 }`. It lists only the files the server can send. With
 *   nothing to offer it is `{ version: null, platforms: {} }`.
 * - `GET /download/{mac|windows|linux}` redirects to the file, and answers 404
 *   when that platform is absent.
 *
 * The page offers only what the manifest lists. The builds are unsigned for now,
 * so after a download the page shows the one-time step to open the app.
 */

/** The platform keys of the manifest, in the order the page lists them. */
export const PLATFORMS = ['mac', 'windows', 'linux'] as const;

export type Platform = (typeof PLATFORMS)[number];

/** How a platform reads in a sentence. */
export const PLATFORM_NAMES: Readonly<Record<Platform, string>> = {
	mac: 'Mac',
	windows: 'Windows',
	linux: 'Linux'
};

/** One downloadable build. */
export interface DownloadFile {
	readonly file: string;
	readonly bytes: number;
	readonly sha256: string;
}

/** `GET /download/manifest.json`. */
export interface DownloadManifest {
	readonly version: string | null;
	readonly platforms: Readonly<Partial<Record<Platform, DownloadFile>>>;
}

/** What the page offers when the manifest is missing or unreadable. */
export const EMPTY_MANIFEST: DownloadManifest = { version: null, platforms: {} };

/**
 * What the visitor is using. A phone or tablet cannot run the desktop app. A
 * desktop whose system the page cannot name (ChromeOS, say) has `platform: null`.
 */
export type Device =
	{ readonly kind: 'desktop'; readonly platform: Platform | null } | { readonly kind: 'handheld' };

/** What {@link detectDevice} reads: parts of `navigator`, plus one media query. */
export interface NavigatorLike {
	readonly userAgent: string;
	readonly maxTouchPoints?: number;
	/** User-Agent Client Hints (Chromium only). */
	readonly userAgentData?: { readonly platform?: string; readonly mobile?: boolean } | null;
	/**
	 * `(any-pointer: fine)`: a mouse or trackpad is attached. Unknown reads as
	 * yes, so only a touch-only device is ever taken for a tablet.
	 */
	readonly finePointer?: boolean;
}

const HANDHELD: Device = { kind: 'handheld' };

function desktop(platform: Platform | null): Device {
	return { kind: 'desktop', platform };
}

/**
 * Detect the device: User-Agent Client Hints first (`navigator.userAgentData`),
 * then the user-agent string. iOS, iPadOS and Android count as phone or tablet.
 *
 * Tablets that ask for the desktop site name a desktop system, so touch decides:
 * - iPadOS sends a Mac user agent, and no Mac has a touch screen.
 * - Android sends Linux (Chrome: `X11; Linux` with `platform: "Linux"` hints;
 *   Firefox: the same user agent, no hints). A Linux computer with a touch
 *   screen still has a mouse or trackpad, so only touch with no fine pointer
 *   reads as a tablet.
 */
export function detectDevice(nav: NavigatorLike): Device {
	const touch = (nav.maxTouchPoints ?? 0) > 1;
	const touchOnly = touch && nav.finePointer === false;
	const hints = nav.userAgentData;
	if (hints?.mobile === true) return HANDHELD;
	switch ((hints?.platform ?? '').toLowerCase()) {
		case 'android':
		case 'ios':
		case 'ipados':
			return HANDHELD;
		case 'macos':
			return touch ? HANDHELD : desktop('mac');
		case 'windows':
			return desktop('windows');
		case 'linux':
			return touchOnly ? HANDHELD : desktop('linux');
		case 'chrome os':
		case 'chromeos':
			return desktop(null);
	}
	const ua = nav.userAgent;
	if (/iPhone|iPad|iPod|Android|Windows Phone|Mobile/i.test(ua)) return HANDHELD;
	if (/Macintosh|Mac OS X/.test(ua)) return touch ? HANDHELD : desktop('mac');
	if (/Windows/.test(ua)) return desktop('windows');
	if (/CrOS/.test(ua)) return desktop(null);
	if (/Linux|X11/.test(ua)) return touchOnly ? HANDHELD : desktop('linux');
	return desktop(null);
}

/** {@link detectDevice} for this browser. */
export function currentDevice(): Device {
	const nav = navigator as Navigator & Pick<NavigatorLike, 'userAgentData'>;
	return detectDevice({
		userAgent: nav.userAgent,
		maxTouchPoints: nav.maxTouchPoints,
		userAgentData: nav.userAgentData,
		finePointer:
			typeof matchMedia === 'function' ? matchMedia('(any-pointer: fine)').matches : undefined
	});
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** A bare file name: no path, no leading dot. */
const FILE_NAME = /^[A-Za-z0-9][A-Za-z0-9._+-]*$/;

/**
 * Read a manifest body. Unknown keys and malformed entries are dropped, so a bad
 * entry hides that one platform and nothing else.
 */
export function parseManifest(input: unknown): DownloadManifest {
	if (!isRecord(input)) return EMPTY_MANIFEST;
	const version = typeof input.version === 'string' && input.version !== '' ? input.version : null;
	const raw = isRecord(input.platforms) ? input.platforms : {};
	const platforms: Partial<Record<Platform, DownloadFile>> = {};
	for (const platform of PLATFORMS) {
		const entry = raw[platform];
		if (
			isRecord(entry) &&
			typeof entry.file === 'string' &&
			FILE_NAME.test(entry.file) &&
			typeof entry.bytes === 'number' &&
			Number.isSafeInteger(entry.bytes) &&
			entry.bytes >= 0 &&
			typeof entry.sha256 === 'string'
		) {
			platforms[platform] = { file: entry.file, bytes: entry.bytes, sha256: entry.sha256 };
		}
	}
	return { version, platforms };
}

/** How long the page waits for the manifest before it offers nothing. */
export const MANIFEST_TIMEOUT_MS = 5000;

/**
 * `GET /download/manifest.json`. Never throws: a missing route, a network error,
 * a bad body or no answer within `timeoutMs` all read as nothing to offer.
 */
export async function fetchManifest(
	params: {
		readonly fetch?: typeof fetch;
		readonly signal?: AbortSignal;
		readonly timeoutMs?: number;
	} = {}
): Promise<DownloadManifest> {
	const controller = new AbortController();
	const stop = (): void => controller.abort();
	const timer = setTimeout(stop, params.timeoutMs ?? MANIFEST_TIMEOUT_MS);
	if (params.signal?.aborted) stop();
	params.signal?.addEventListener('abort', stop, { once: true });
	try {
		const response = await (params.fetch ?? fetch)('/download/manifest.json', {
			cache: 'no-cache',
			credentials: 'same-origin',
			headers: { Accept: 'application/json' },
			signal: controller.signal
		});
		if (!response.ok) return EMPTY_MANIFEST;
		return parseManifest(await response.json());
	} catch {
		return EMPTY_MANIFEST;
	} finally {
		clearTimeout(timer);
		params.signal?.removeEventListener('abort', stop);
	}
}

/** The platforms the manifest offers, in {@link PLATFORMS} order. */
export function availablePlatforms(manifest: DownloadManifest): Platform[] {
	return PLATFORMS.filter((platform) => manifest.platforms[platform] !== undefined);
}

/** Where the Download button points. */
export function downloadHref(platform: Platform): string {
	return `/download/${platform}`;
}

/** The separator before item `index` of `count` in "Mac, Windows and Linux". */
export function listSeparator(index: number, count: number): string {
	if (index === 0) return '';
	return index === count - 1 ? ' and ' : ', ';
}

/** "Mac", "Mac and Linux", "Mac, Windows and Linux". */
export function platformList(platforms: readonly Platform[]): string {
	return platforms
		.map((platform, i) => listSeparator(i, platforms.length) + PLATFORM_NAMES[platform])
		.join('');
}

/** The one-time step to open an unsigned build, and the command it needs, if any. */
export interface OpeningStep {
	readonly text: string;
	/** A shell command, shown on its own line in a monospace face. */
	readonly command?: string;
}

/**
 * The one-time step to open the unsigned app, shown after a download starts.
 * Linux depends on the file the manifest names.
 */
export function openingStep(platform: Platform, file?: string): OpeningStep {
	switch (platform) {
		case 'mac':
			return {
				text: 'First time: open Euchre once, then go to System Settings › Privacy & Security › Open Anyway.'
			};
		case 'windows':
			return {
				text: 'First time: when “Windows protected your PC” appears, choose More info › Run anyway.'
			};
		case 'linux': {
			const name = file ?? '';
			if (/\.AppImage$/i.test(name))
				return { text: 'First time: make it runnable, then open it.', command: `chmod +x ${name}` };
			if (/\.deb$/i.test(name))
				return {
					text: 'First time: install it, then open Euchre.',
					command: `sudo apt install ./${name}`
				};
			if (/\.(tar\.gz|tgz|tar\.xz|zip)$/i.test(name))
				return { text: 'First time: unpack it, then run euchre from the folder.' };
			return { text: 'First time: make the file runnable (chmod +x), then open it.' };
		}
	}
}
