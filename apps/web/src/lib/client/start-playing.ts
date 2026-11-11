/**
 * Play now: start a fresh game and go to its table.
 *
 * This is the call `/new` makes when the player presses Deal me in with the
 * default house rules: `POST /api/tables`, which mints this browser's guest
 * cookie when needed and seats the player's saved opponents (their profile
 * settings). Then it loads `/play/<gameId>` with a full page navigation.
 *
 * It is the one place the home page starts a game, so a later step can send
 * Play now somewhere else (the WebGPU Rune build) without touching the page.
 */
import { createTable } from './api.ts';

export interface StartPlayingOptions {
	/** For tests: the fetch used for `POST /api/tables`. */
	readonly fetch?: typeof fetch;
	/** For tests: how to open the table. Defaults to a full page navigation. */
	readonly navigate?: (href: string) => Promise<void>;
}

/**
 * A full page navigation, not a SvelteKit client one: the server picks the shell
 * for `/play/<id>` (the Rune loader, or the Svelte page for a classic browser).
 * The page is going away, so the promise stays pending and the button stays busy.
 */
function assign(href: string): Promise<void> {
	location.assign(href);
	return new Promise<void>(() => {});
}

/**
 * Create a table and open it. Resolves with the new game id once the table
 * page is open; rejects (an `ApiError` for a refused request) when the table
 * could not be created, and then nothing has navigated.
 */
export async function startPlaying(options: StartPlayingOptions = {}): Promise<string> {
	const { gameId } = await createTable({ fetch: options.fetch, stick: false, natural: false });
	await (options.navigate ?? assign)(`/play/${encodeURIComponent(gameId)}`);
	return gameId;
}
