# The new home page

Lane HOME, 2026-11-11. `apps/web` only, plus this report. Nothing committed.

## What "/" is now

The wordmark ("euchre" and its club, no tagline), the fanned-cards picture, a
**Play now** button and a **Download** button. No nav, sections or footer.

- **Play now** calls `startPlaying()` in `apps/web/src/lib/client/start-playing.ts`.
  It makes the same `POST /api/tables` call that /new makes (default house rules:
  `stick: false`, `natural: false`; the server applies the player's saved
  opponents and mints the guest cookie when there is none), then opens
  `/play/<id>`. A later step can point this one function at the WebGPU Rune build.
  While it waits the button reads "Dealing…" (`aria-busy`). On failure one quiet
  line appears under the buttons ("We couldn’t start a game. Please try again.")
  and the button works again.
- **Download** reads `GET /download/manifest.json` on mount and offers only the
  platforms it lists. The device is read from `navigator.userAgentData` first,
  then the user agent (`apps/web/src/lib/home/downloads.ts`). The button says
  "Download for Mac/Windows/Linux" and links `/download/<platform>` with the
  `download` attribute, so the page stays put. The line under the buttons lists
  the other platforms as links. After a download click it becomes the one-time
  opening step for the unsigned app (Mac: System Settings › Privacy & Security ›
  Open Anyway; Windows: More info › Run anyway; Linux: chosen by file type, for
  example `chmod +x` for an AppImage).
- **Phones and tablets** (iOS, iPadOS including the iPad's desktop-mode Mac user
  agent, Android) get Play now and "Also on Mac, Windows and Linux" (only the
  platforms the manifest lists).
- **No builds:** the page shows Play now alone, centered.
- **Look:** the picture has green felt baked in, and its edges average `#305342`.
  The page uses that same felt, and the picture is feathered with an oval mask,
  so no rectangle shows. The corners darken a little. The type and colours are
  the app's own tokens (Source Serif 4 wordmark, DM Sans buttons, lime primary
  on forest). The layout is one centered column. The picture takes the height
  left over (at most 1040px wide). On a phone it is 116% of the screen width, so
  only plain felt is cropped.
- **Motion:** one entrance. The page fades in and rises 8px once the fonts, the
  picture and the manifest are in, capped at 900 ms after mount. With
  reduced motion it is a 200 ms fade only, and buttons do not move on hover or
  press.

### Zero layout shift, and how

- The image has `width`/`height`, `fetchpriority="high"` and a CSS size worked
  out from the viewport, so its box is known from the first frame.
- The note line under the buttons is a fixed box (two lines, three under 540px).
  A longer line never moves anything.
- The wordmark's boxes are sized to Source Serif 4. A late font changes glyphs,
  never positions (measured: CLS was 0.00017 with fonts delayed 1.5 s before this
  fix, and 0 after it).
- The page stays hidden until fonts, picture and manifest are ready (or 900 ms
  pass), so its layout is final before anyone sees it. If the manifest arrives
  after that, the Download button keeps its reserved place, hidden, and
  appears there or stays hidden. Nothing moves.

### Without JavaScript

`/` cannot be prerendered. The server answers `/` and every extensionless path
with the same `index.html` (adapter-static's SPA fallback, which overwrites a
prerendered `index.html`), and changing that is the server lane's work. So
`apps/web/src/app.html`, the shell every path gets, now carries:

- `<title>Euchre</title>` and the meta description.
- Open Graph and Twitter tags, using `static/art/euchre-og.jpg` (new: a
  1200x630 center crop of the hero, made with macOS `sips`, 141 KB) at
  `https://euchre.sh/art/euchre-og.jpg`.
- The Google Fonts links, moved here from `+layout.svelte`.
- A `<noscript>` copy of the home page: the wordmark, the picture, Play now as a
  link to `/new`, and plain links to `/download/mac`, `/download/windows` and
  `/download/linux`.

The favicon link in `+layout.svelte` is unchanged.

## Files

- New: `apps/web/src/lib/client/start-playing.ts`,
  `apps/web/src/lib/client/start-playing.test.ts`,
  `apps/web/src/lib/home/downloads.ts`, `apps/web/src/lib/home/downloads.test.ts`,
  `apps/web/static/art/euchre-og.jpg`.
- Rewritten: `apps/web/src/routes/+page.svelte` (the old hero, introduction,
  how-to and footer, and all their styles, are deleted) and
  `apps/web/src/routes/page.svelte.spec.ts` (8 browser tests).
- Changed:
  - `apps/web/src/lib/ui/GlobalNav.svelte`: hidden on `/`. The home-only brand
    lockup and its styles are deleted.
  - `apps/web/src/routes/+layout.svelte`: the font links moved to `app.html`.
  - `apps/web/src/app.html`: as above.
  - `apps/web/vite.config.ts`: the dev proxy also forwards `/download`.
  - `apps/web/scripts/e2e-browser.mjs`: see below.
  - Docs: `apps/web/README.md`, `apps/web/src/lib/client/README.md` and
    `apps/web/scripts/README.md`.
- Rebuilt: `apps/web/build/client`.

## Can players still reach /games and /settings?

Yes. The old home was not the only way in.

- The table's settings rail (`#table-settings`) links "Your saved games"
  (`/games`), "Choose player personalities" (`/settings`), "Start a new game"
  (`/new`) and "Back to home".
- The table's loading view also links `/games`.
- `/new`, `/games` and `/settings` keep the global nav (Play, Your games,
  Settings).

The e2e script now checks that all four links are in the table settings.
One gap to know about: a first-time visitor sees neither page from `/` itself.
They reach them from the table. As the task asked, no links were added to the
home page.

## Proof

### Gates (apps/web)

| Gate            | Result                                      |
| --------------- | ------------------------------------------- |
| `bun run check` | 1858 files, 0 errors, 0 warnings            |
| `bun run lint`  | all files pass Prettier                     |
| `bun run test`  | 20 files, 453 tests pass                    |
| `bun run build` | done, `build/client` written                |

`bunx @sveltejs/mcp svelte-autofixer` found no issues in `+page.svelte`,
`GlobalNav.svelte` or `+layout.svelte`. Its one optional suggestion (an attachment
instead of `bind:this`) was not taken.

The home spec ran 5 times in a row with all 8 tests passing. One earlier run
failed because boxes were measured during the 8px entrance rise. The test helper
now waits for the entrance to finish.

### Screenshots, CLS and overflow

Measured in headless Chromium (Playwright 1.62.0 from `apps/web`) with a
`PerformanceObserver` on `layout-shift` (buffered) from before the first byte.
"Fits" means the page height is no more than the viewport height. "No h-scroll"
means `scrollWidth` equals the viewport width.

| Run                                                       | Viewports                                                  | CLS | No h-scroll | Fits |
| --------------------------------------------------------- | ---------------------------------------------------------- | --- | ----------- | ---- |
| All three builds (manifest intercepted)                   | 1440x900, 1280x720, 2560x1440, 768x1024, 390x844, 320x640 | 0   | yes         | yes  |
| No builds                                                 | same six                                                   | 0   | yes         | yes  |
| iPhone user agent                                         | same six                                                   | 0   | yes         | yes  |
| Windows and Linux user agents                             | 1440x900                                                   | 0   | yes         | yes  |
| After a download click (Mac, Windows, Linux)              | 1440x900, 320x640                                          | 0   | yes         | yes  |
| Fonts delayed 1.5 s                                       | 1440x900, 390x844 (iPhone)                                 | 0   | yes         | yes  |
| Manifest delayed 2 s (all, and none)                      | 1440x900, 320x640                                          | 0   | yes         | yes  |
| Reduced motion                                            | 1440x900                                                   | 0   | yes         | yes  |
| No JavaScript                                             | 1440x900, 390x844 (iPhone)                                 | 0   | yes         | yes  |
| Play now refused (503 on `POST /api/tables`)              | 1440x900                                                   | 0   | yes         | yes  |
| Real server, all three builds on disk                     | the six viewports                                          | 0   | yes         | yes  |
| Real server, manifest removed (`{"version":null,...}`)    | the six viewports                                          | 0   | yes         | yes  |

- The largest contentful paint was the picture in every run: 1.44 to 1.82 s
  after navigation on the local server. That is JS start plus the reveal wait,
  which waits for the web fonts from Google over the internet and is capped at
  900 ms.
- Picture size: 1040x867 at 2560x1440, 706x588 at 1440x900, 542x452 at
  1280x720, 849x708 at 768x1024, 452x377 at 390x844 and 371x309 at 320x640.
- Every screenshot was read. Shots and contact sheets are in
  `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/home/shots/`:
  - `intercept/`: 36 shots and `results-intercept.json`.
  - `real/`: 12 shots against the server, 3 real downloads, 1 phone Play now,
    and their results JSON.
  - `e2e/`: 19 shots.
  - The `sheet-*.png` contact sheets.

### Against a production-shaped server

The release `euchre-server` was built in `.cargo-targets/home` (an APFS clone of
`.cargo-targets/w4-web`) and run from `apps/web` so it served the new
`build/client`. Settings: `PORT=7002` (7000 is held by macOS Control Center),
`RIVET_RUN_ENGINE_PORT=7010` (engine on 7010, 7011 and 7015), a scratch
`RIVETKIT_STORAGE_PATH`, `EUCHRE_FAST_TEMPO=1`, `ALLOWED_ORIGINS` and `APP_URL`
set to `http://127.0.0.1:7002`, `NO_PROXY`, the bun-installed engine binary, and
`EUCHRE_DOWNLOADS_DIR` pointing at a scratch folder with three 64 KB fake builds
and a matching manifest.

- **Routes:**
  - `/download/manifest.json` answered 200 with `no-cache`.
  - `/download/{mac,windows,linux}` answered 302 to `/download/files/<file>`.
  - The file answered 200 with `Content-Disposition: attachment` and an
    immutable cache.
  - With the manifest removed, the server answered `{"version":null,"platforms":{}}`
    within the 5 s reload, and `/download/mac` answered 404.
- **Download clicks (real):** Mac, Windows and Linux user agents each saved the
  right file (`Euchre-0.4.0-mac.dmg`, `-windows.zip`, `-linux.AppImage`,
  65536 bytes each). The page stayed put, and the line showed that platform's
  opening step.
- **e2e:** `EUCHRE_URL=http://127.0.0.1:7002 E2E_LABEL=home bun scripts/e2e-browser.mjs`
  exited 0 in 63 s. The run:
  - checked that the home page shows the wordmark and Play now, has no nav or
    footer, and read the manifest (200);
  - opened /new;
  - pressed **Play now** and landed on `/play/1bc5973a-…`;
  - checked the table settings links (`/games`, `/settings`, `/new`, `/`);
  - played a full match: 7 hands, 7 bids, 35 plays, 2 discards, 10 review-gate
    continues and one reload with resume. Final score: Your team 3, Other
    team 10;
  - then visited the scrapbook, match detail, settings and sign-out.

  `errors: []`, `badResponses: []`.
- **Phone:** with an iPhone user agent at 390x844, tapping Play now landed on
  `/play/ba2ebdd5-…` in 267 ms and set the `euchre_guest` cookie. The table and
  its first-run tour rendered.

## The e2e script

`apps/web/scripts/e2e-browser.mjs` changed in three steps:

- **Step 1 (home):** checks the `euchre` heading, the Play now button, no
  nav or footer, and one 200 answer from the manifest.
- **Step 4:** opens `/new` (shot `05-new`) without starting a game there, then
  starts the match from the home page's **Play now**.
- **Table settings:** checks the links to `/games`, `/settings`, `/new` and `/`.

## Tools and leftovers

- No installs. `bunx @sveltejs/mcp` ran from bun's cache. Playwright 1.62.0 and
  its Chromium were already in `apps/web/node_modules` and
  `~/Library/Caches/ms-playwright`. Pillow (already in the user's Python) made
  the contact sheets. macOS `sips` made the OG crop.
- Scratch helpers are in the scratchpad's `home/tools/`:
  - `static-server.ts`: a Bun copy of the server's static rules, on port 7001;
  - `shots.mjs`, `download-check.mjs`, `phone-play.mjs`, `nojs.mjs`, `sheet.py`.
- Everything I started is stopped: the static server, `euchre-server` and its
  engine (stopped by SIGTERM, a clean drain). Nothing of mine listens on
  7000-7019. Port 7000 belongs to macOS Control Center, not to this lane.
