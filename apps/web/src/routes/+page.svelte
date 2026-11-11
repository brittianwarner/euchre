<!--
  Home: the wordmark, the fanned cards, Play now and Download. Nothing else.

  - Play now starts a game with the player's saved settings and opens its table
    (`startPlaying`).
  - Download offers the desktop build for this computer, and only the builds
    `/download/manifest.json` lists. Phones and tablets get Play now and one
    line naming the desktop builds.
  - The page fades in once, when the fonts, the picture and the manifest are in
    (at most `REVEAL_WAIT_MS` after mount), so its layout is final before anyone
    sees it. If the manifest names no build for this computer by then, Play now
    stands alone in the middle. If the manifest is later than that, the Download
    button keeps its place, hidden, and appears there when the manifest offers a
    build. When it offers none, the buttons fade out, close up and fade back in,
    so nothing moves while it can be seen.
  - The wordmark's face must be in at the reveal, or the wordmark stays in the
    fallback serif for this visit: a late swap would move the club.

  The page is a client-rendered SPA route like every other. `src/app.html`
  carries the title, description, social tags and a `<noscript>` copy of this
  page for visitors without JavaScript.
-->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { startPlaying } from '#lib/client/start-playing.ts';
	import {
		PLATFORM_NAMES,
		availablePlatforms,
		currentDevice,
		downloadHref,
		fetchManifest,
		listSeparator,
		openingStep,
		platformList,
		type DownloadManifest,
		type Platform
	} from '#lib/home/downloads.ts';

	/** The longest the page waits for fonts, picture and manifest before it shows anyway. */
	const REVEAL_WAIT_MS = 900;
	/** How long the buttons take to fade out before they close up (matches the CSS). */
	const REGROUP_FADE_MS = 160;
	/** Below this width the buttons stack, so closing up moves the whole page. */
	const STACKED = '(max-width: 539px)';

	const device = currentDevice();
	const handheld = device.kind === 'handheld';
	const here = device.kind === 'desktop' ? device.platform : null;

	let manifest = $state.raw<DownloadManifest | null>(null);
	let starting = $state(false);
	let failed = $state(false);
	let downloaded = $state<Platform | null>(null);
	let shown = $state(false);
	/** Play now alone: a phone, or no build for this computer. */
	let single = $state(handheld);
	/** What is hidden while the buttons close up: the button row, or the whole page. */
	let regrouping = $state<'actions' | 'stage' | null>(null);
	/** The faded part is hidden outright while the slot closes up. */
	let gone = $state(false);
	/** The wordmark's web font missed the reveal, so the fallback serif stays. */
	let plainSerif = $state(false);
	let art = $state<HTMLImageElement>();

	const available = $derived(manifest ? availablePlatforms(manifest) : []);
	/** This computer's build, when the manifest has it. */
	const own = $derived(here !== null && available.includes(here) ? here : null);
	const others = $derived(available.filter((platform) => platform !== own));
	const step = $derived(
		downloaded ? openingStep(downloaded, manifest?.platforms[downloaded]?.file) : null
	);

	async function play(): Promise<void> {
		if (starting) return;
		starting = true;
		failed = false;
		try {
			await startPlaying();
		} catch {
			failed = true;
		} finally {
			starting = false;
		}
	}

	function wait(ms: number): Promise<void> {
		return new Promise((resolve) => setTimeout(resolve, ms));
	}

	function nextFrame(): Promise<void> {
		return new Promise((resolve) => requestAnimationFrame(() => resolve()));
	}

	/**
	 * The web fonts' stylesheet (added by the root layout, so it never holds back
	 * the first paint), then the faces this page uses. True when the wordmark's
	 * face is in.
	 */
	async function webFontsIn(): Promise<boolean> {
		const sheets = document.querySelectorAll<HTMLLinkElement>(
			'link[rel="stylesheet"][href^="https://fonts.googleapis.com/"]'
		);
		await Promise.all(
			[...sheets]
				.filter((link) => link.sheet === null)
				.map(
					(link) =>
						new Promise((resolve) => {
							link.addEventListener('load', resolve, { once: true });
							link.addEventListener('error', resolve, { once: true });
						})
				)
		);
		const fonts = document.fonts;
		if (!fonts) return false;
		const [serif] = await Promise.all([
			fonts.load('600 64px "Source Serif 4"', 'euchre'),
			fonts.load('650 15px "DM Sans"', 'Play now'),
			fonts.load('400 14px "DM Sans"', 'Also on')
		]);
		return serif.length > 0;
	}

	/**
	 * A late manifest with no build for this computer: fade out what would move,
	 * hide it, close up the empty Download slot, then show it again. A box that
	 * moves while hidden is not a layout shift.
	 */
	async function regroup(signal: AbortSignal): Promise<void> {
		regrouping = matchMedia(STACKED).matches ? 'stage' : 'actions';
		await wait(REGROUP_FADE_MS);
		await nextFrame();
		if (signal.aborted) return;
		gone = true;
		await tick();
		await nextFrame();
		single = true;
		await tick();
		await nextFrame();
		await nextFrame();
		gone = false;
		regrouping = null;
	}

	onMount(() => {
		const abort = new AbortController();
		let serifIn = false;
		const offered = fetchManifest({ signal: abort.signal }).then((result) => {
			if (abort.signal.aborted) return;
			manifest = result;
			if (shown && !single && own === null) void regroup(abort.signal);
		});
		const fonts = webFontsIn().then((ok) => {
			serifIn = ok;
		});

		// Reveal once the fonts, the picture and the manifest are in, so a font
		// swap or a late answer never moves anything on screen.
		const ready = Promise.all([fonts, art?.decode(), offered]).catch(() => undefined);
		void Promise.race([ready, wait(REVEAL_WAIT_MS)]).then(() => {
			if (abort.signal.aborted) return;
			if (manifest !== null && own === null) single = true;
			plainSerif = !serifIn;
			shown = true;
		});

		return () => abort.abort();
	});
</script>

<svelte:head>
	<title>Euchre</title>
</svelte:head>

<main class="home" class:single class:plain-serif={plainSerif}>
	<div class="stage" class:shown class:regroup={regrouping === 'stage'} class:gone>
		<h1 class="wordmark">
			<span class="word">euchre</span><span class="club" aria-hidden="true">♣</span>
		</h1>
		<img
			bind:this={art}
			class="art"
			src="/art/euchre-hero.webp"
			width="1200"
			height="1000"
			alt="Classic playing cards and a dealer button on a green felt table, with both red jacks at the front"
			fetchpriority="high"
			decoding="async"
		/>
		<div class="actions" class:regroup={regrouping === 'actions'} class:gone>
			<button
				type="button"
				class="button primary"
				aria-busy={starting}
				aria-disabled={starting}
				onclick={play}>{starting ? 'Dealing…' : 'Play now'}</button
			>
			{#if !single}
				<!-- Hidden, but holding its place, until the manifest offers this computer's build. -->
				<a
					class="button secondary"
					class:unavailable={own === null}
					href={downloadHref(own ?? here ?? 'mac')}
					download
					onclick={() => {
						if (own) downloaded = own;
					}}>Download for {PLATFORM_NAMES[own ?? here ?? 'mac']}</a
				>
			{/if}
		</div>
		<p class="note">
			<!-- Only what answers a press is announced; the platform list is not. -->
			<span aria-live="polite"
				>{#if failed}We couldn’t start a game. Please try again.{:else if step}{step.text}{#if step.command}<code
							>{step.command}</code
						>{/if}{/if}</span
			>
			{#if !failed && !step}
				{#if handheld}
					{#if available.length > 0}Also on {platformList(available)}{/if}
				{:else if others.length > 0}
					{own ? 'Also on' : 'Download for'}
					{#each others as platform, i (platform)}{listSeparator(i, others.length)}<a
							href={downloadHref(platform)}
							download
							onclick={() => (downloaded = platform)}>{PLATFORM_NAMES[platform]}</a
						>{/each}
				{/if}
			{/if}
		</p>
	</div>
</main>

<style>
	/* The picture has a felt table baked in. Its edges average #305342 (each
	   side within a few steps of it), so the page is that same felt: the
	   picture's feathered edge disappears into it, and only the page corners
	   darken. */
	:global(html:has(main.home)) {
		background: #305342;
	}
	.home {
		--gutter: 16px;
		--pad: clamp(24px, 5svh, 64px);
		/* Tall tablets read the height, so the wordmark keeps up with the picture. */
		--mark: clamp(40px, min(max(6vw, 5.5svh), 7svh), 88px);
		--gap: clamp(12px, 2.4svh, 28px);
		/* The buttons grow a little with the wordmark on big screens. */
		--button-h: clamp(52px, calc(var(--mark) * 0.66), 60px);
		--button-w: clamp(15rem, calc(var(--mark) * 3.1), 17rem);
		--button-font: clamp(15px, calc(var(--mark) * 0.19), 17px);
		--line: 21px;
		--note-lines: 2;
		--buttons-h: var(--button-h);
		--art-min: 240px;
		box-sizing: border-box;
		min-height: 100svh;
		display: grid;
		/* One column exactly as wide as the page, so the wider picture on a phone
		   overflows both sides evenly instead of widening the column. */
		grid-template-columns: minmax(0, 1fr);
		place-items: center;
		padding: max(var(--pad), env(safe-area-inset-top))
			max(var(--gutter), env(safe-area-inset-right)) max(var(--pad), env(safe-area-inset-bottom))
			max(var(--gutter), env(safe-area-inset-left));
		overflow-x: clip;
		color: var(--club-cream);
		background: radial-gradient(ellipse 100% 100% at 50% 50%, #305342 60%, #213e30 100%);
	}
	.stage {
		/* Everything but the picture, so the picture takes the height that is left. */
		--chrome: calc(
			2 * var(--pad) + var(--mark) + 3 * var(--gap) + var(--buttons-h) + var(--note-lines) *
				var(--line)
		);
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--gap);
		text-align: center;
		visibility: hidden;
		opacity: 0;
		transform: translateY(8px);
	}
	.stage.shown {
		visibility: visible;
		opacity: 1;
		transform: none;
		transition:
			opacity 480ms ease-out,
			transform 640ms cubic-bezier(0.22, 0.7, 0.25, 1);
	}
	/* Closing up after a late manifest: fade out, then hide, so the move is unseen. */
	.stage.shown.regroup,
	.actions.regroup {
		opacity: 0;
		transition: opacity 160ms ease-out;
	}
	.stage.shown.regroup.gone,
	.actions.regroup.gone {
		visibility: hidden;
	}
	/* One optical size at every width: Source Serif 4 widens as it gets smaller,
	   so a pinned axis keeps the lockup's proportions. The club follows the word
	   with a gap, so no face can push them together. */
	.wordmark {
		display: flex;
		align-items: baseline;
		justify-content: center;
		gap: 0.17em;
		height: 1em;
		margin: 0;
		font: 600 var(--mark) / 1 var(--font-serif);
		font-variation-settings: 'opsz' 60;
		white-space: nowrap;
		color: var(--club-cream);
	}
	.home.plain-serif .wordmark {
		font-family: 'Iowan Old Style', Georgia, serif;
	}
	.word {
		letter-spacing: -0.07em;
	}
	.club {
		font-size: 0.62em;
		color: var(--club-lime);
	}
	.art {
		display: block;
		flex: none;
		/* The height that is left, as wide as 108% of a narrow screen: the
		   picture's side margins are plain felt, so a phone crops felt, not
		   cards, and the cards still clear the 16px gutter. */
		width: min(108vw, max(var(--art-min), (100svh - var(--chrome)) * 1.2), 1040px);
		max-width: none;
		height: auto;
		/* Feather the picture into the page with an oval, so no corner or edge of
		   the rectangle shows. Every card and the dealer button sit inside 86% of
		   it. */
		mask-image: radial-gradient(ellipse 52% 51% at 48% 49%, #000 86%, transparent 100%);
		user-select: none;
	}
	.actions {
		display: flex;
		justify-content: center;
		gap: 12px;
		height: var(--buttons-h);
		transition: opacity 200ms ease-out;
	}
	.button {
		box-sizing: border-box;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: var(--button-w);
		height: var(--button-h);
		padding: 0 20px;
		border-radius: 10px;
		font: 650 var(--button-font) / 1 var(--font-sans);
		letter-spacing: 0.01em;
		text-decoration: none;
		white-space: nowrap;
		cursor: pointer;
		transition:
			transform 160ms ease-out,
			background-color 160ms,
			border-color 160ms,
			opacity 200ms;
	}
	.button:active {
		transform: scale(0.96);
	}
	.primary {
		/* Transparent, so forced colours (Windows High Contrast) draw an edge. */
		border: 1px solid transparent;
		background: var(--club-lime);
		color: var(--club-forest);
	}
	.secondary {
		border: 1px solid #f4f3e966;
		color: var(--club-cream);
		background: #15352e33;
	}
	@media (hover: hover) and (pointer: fine) {
		.primary:hover {
			background: #e3f6be;
			transform: translateY(-1px);
		}
		.secondary:hover {
			border-color: #d4ed9bb3;
			background: #d4ed9b14;
			transform: translateY(-1px);
		}
		.note a:hover {
			text-decoration-color: currentColor;
		}
	}
	.primary[aria-busy='true'] {
		cursor: progress;
		background: #c3dc8a;
		transform: none;
	}
	.secondary.unavailable {
		visibility: hidden;
		opacity: 0;
	}
	.note {
		box-sizing: border-box;
		width: min(100%, 34rem);
		/* A fixed box: a longer line never moves anything. */
		height: calc(var(--note-lines) * var(--line));
		margin: 0;
		font: 400 14px / var(--line) var(--font-sans);
		color: #b9cabd;
		text-wrap: balance;
	}
	.note a {
		border-radius: 3px;
		color: var(--club-cream);
		text-decoration-line: underline;
		text-decoration-color: #f4f3e980;
		text-decoration-thickness: 1px;
		text-underline-offset: 3px;
	}
	.note a:focus-visible {
		outline-width: 2px;
		outline-offset: 2px;
	}
	.note code {
		display: block;
		font:
			500 12.5px / var(--line) ui-monospace,
			SFMono-Regular,
			Menlo,
			Consolas,
			monospace;
		color: var(--club-cream);
		overflow-wrap: anywhere;
	}
	@media (max-width: 539px) {
		.home {
			--note-lines: 3;
			--buttons-h: calc(2 * var(--button-h) + 12px);
		}
		.home.single {
			--buttons-h: var(--button-h);
		}
		.actions {
			flex-direction: column;
			align-items: center;
			width: 100%;
		}
		.button {
			width: min(100%, 20rem);
		}
	}
	/* Short screens (a phone on its side): tighter spacing and a smaller
	   picture, so Play now and the line under it stay on screen. */
	@media (max-height: 430px) {
		.home {
			--pad: 12px;
			--gap: 8px;
			--mark: clamp(28px, 9svh, 40px);
			--button-h: 44px;
			--line: 18px;
			--art-min: 160px;
		}
		.note {
			font-size: 13px;
		}
	}
	/* Windows High Contrast: the felt is forced away, so drop the oval feather
	   and show the picture whole. */
	@media (forced-colors: active) {
		.art {
			mask-image: none;
			border-radius: 10px;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.stage,
		.stage.shown {
			transform: none;
		}
		.stage.shown {
			transition: opacity 200ms ease-out;
		}
		.button,
		.button:hover,
		.button:active {
			transition: none;
			transform: none;
		}
	}
</style>
