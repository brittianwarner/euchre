# Rune on the web: the game spike

Lane SPIKE, 2026-11-11. The goal: the real `euchre-desktop` app, built for
`wasm32-unknown-unknown` with Rune's `scripts/web-build.sh` and drawn with WebGPU in Chromium. It
should show Home, New game and a table that plays a hand against the in-process sim, with the
deal, the card flights and the trick sweep.

**Result: it works.** One hand was played end to end in headless Chromium on the Metal adapter:
deal, bid, five tricks, the hand result and the next deal. The run logged zero console errors.
At rest it draws nothing. While it animates, frames hold 60 fps.

All work happened in a scratch copy. Nothing in this repo changed except this file.

- `$S` = `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/webspike`
- Copy of the repo: `$S/repo`. Its `net/` folder belongs to the NET lane and was not touched.
- Proof script: `$S/repo/web-proof/proof.mjs`.
- Build wrapper: `$S/build-web.sh`.
- Builds:
  - `$S/web`: art embedded in the wasm.
  - `$S/web-fetch`: art fetched by URL.
  - `$S/web-opt`: art embedded, plus `wasm-opt -Oz`.
- Screenshots, video and numbers:
  - `$S/shots/`: embedded build.
  - `$S/shots/fetch/`: fetch build.
  - Each folder holds `proof.json`, `console.txt` and `desktop-hand.webm`.
- Logs: `$S/logs/`.

Sibling doc: [network-and-serving.md](network-and-serving.md) (lane NET). It covers the real
server backend (`WebBackend` over the rivetkit JS client), cookies, serving and the Svelte
fallback. This spike plays only the sim and does not repeat that work.

## 1. What works

All of this was checked by screenshot or by the proof script's probe.

| Area | Result |
| --- | --- |
| Home | Hero art, nav and footer draw correctly at 1280×800, at 1000×700 after a live resize, and at DPR 2 (canvas 2560×1600). |
| New game | Rules form and "Deal me in". |
| The deal | Cards fly from the dealer to each seat and to your rack. Shots were taken at 450, 850 and 1350 ms. |
| First-run tour | The modal opens. Escape closes it (the app's `tour_dismiss`). |
| Bidding | The bid panel works, and "Pass" via a mouse click is accepted by the sim. The sim's seats bid with the euchre-ai heuristic (`top_move`). |
| Playing | Clicking a card selects it, and `P` plays it. Card flights work, all four cards land, the "wins the trick" highlight shows, and the sweep runs to the winner's pile. |
| Hand result | The "Point. They score 1." card shows, the score bar updates, and the next hand deals itself. |
| Input | Mouse, keyboard (Escape, P) and touch taps (phone run with `hasTouch`). |
| devicePixelRatio | DPR 2 gives a 2560×1600 canvas for 1280×800 CSS px. DPR 3 gives a 1170×2532 canvas for 390×844 CSS px. Text is sharp. |
| Prefs | Saved to `localStorage["euchre:desktop-prefs"]` and read back after a reload: `tourSeen` stays true. |
| Reduced motion | Rune honours `prefers-reduced-motion`. The table comes to rest (see §5.4). |
| Native build | Still clean after the changes. `cargo check --all-targets` passes, and `cargo test --lib` gives 141 passed. |

Not verified:

- **Sound.** The WebAudio path compiles and is wired in. Sound is off by default, and the run was
  headless, so nothing was heard.
- **Phone layout.** It renders but is not usable at 390 px (§6).
- **The real server backend.** This is lane NET's work.

## 2. How to reproduce

```sh
S=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/webspike
$S/build-web.sh --out $S/web-fetch              # release build (web-build.sh's default profile)
mkdir -p $S/web-fetch/art/cards                 # fetch build only: the art beside the page
cp $S/repo/apps/web/static/art/euchre-hero.webp $S/web-fetch/art/
cp $S/repo/apps/web/static/art/cards/*.png $S/web-fetch/art/cards/
cp $S/repo/apps/desktop/assets/cards/AS.png $S/web-fetch/art/cards/AS.png   # the app's own ace
cd $S/repo && bun web-proof/proof.mjs $S/web-fetch $S/shots/fetch
```

What the wrapper and the proof do:

- **The wrapper** puts Python 3 and wasm-bindgen 0.2.130 on PATH. It sets
  `CARGO_TARGET_DIR=.cargo-targets/webspike` and toolchain 1.99.1, runs Rune's `web-build.sh`,
  and then copies Rune's Inter faces (§4, blocker 4).
- **The proof** serves the folder on 127.0.0.1 on a random port. It launches Playwright's
  Chromium (`channel: 'chromium'`, `--enable-unsafe-webgpu`) and runs four checks: a desktop
  hand with video, three cold loads, a phone at 390×844 DPR 3 with touch, and DPR 2. It writes
  `proof.json`, `console.txt`, the PNGs and `desktop-hand.webm`. When it finishes, it closes its
  browser and its server.

## 3. Every change, with file and reason

13 files in the copy: 11 changed, 2 new. About 330 lines added. Every change is marked
`WEB SPIKE`.

| File | Change | Reason |
| --- | --- | --- |
| `apps/desktop/Cargo.toml` | Adds `[lib] crate-type = ["cdylib", "rlib"]`. | `web-build.sh` builds the package's one cdylib. |
| | Moves `euchre-server`, `euchre-server-core`, `rivetkit-client`, `rivetkit-engine-process`, `directories`, `reqwest`, `rodio` and tokio (workspace `full`) under `cfg(not(target_arch = "wasm32"))`. | These need threads, sockets, files or the OS audio stack. None of them build for `wasm32-unknown-unknown`. |
| | For wasm, adds tokio with `default-features = false` and `["sync", "macros"]`, `uuid` with `js`, `wasm-bindgen 0.2`, `wasm-bindgen-futures 0.4`, `js-sys 0.3`, and `web-sys 0.3` with console, Window, Location, Storage, Performance and the WebAudio types. | The bridge keeps its tokio channels and `select!`, so it needs `macros` as well as `sync`. `uuid::new_v4` needs `js` for randomness. |
| `crates/euchre-ai/Cargo.toml` | tokio and reqwest become native-only. wasm gets tokio `["sync", "macros"]`. | Same reason as above. The sim needs only the heuristic. |
| `crates/euchre-ai/src/lib.rs` | `ai_seat`, `banter`, `clock`, `decide`, `jev`, `model`, `talk` and `testing` are gated native-only. | They pull in reqwest, a tokio runtime and the model client. The sim uses `top_move`, `HeuristicOptions` and `RankParams`, which stay. |
| `apps/desktop/src/net/rt.rs` (new, 220 lines) | The single async-runtime seam: `spawn` returns a `Task` (with `abort` and `join`), plus `sleep` and `timeout`. Native wraps tokio. wasm uses `wasm_bindgen_futures::spawn_local` with `futures::Abortable` for abort, a `setTimeout` future for `sleep`, and `select` for `timeout`. | The browser has no tokio runtime and no threads. Both sides keep the same `Send` bounds, so the bridge and the sim compile unchanged against either. The timer's JS closure lives in a `thread_local` map instead of the future, which keeps the future `Send`. |
| `apps/desktop/src/net/bridge.rs` | Every `tokio::spawn`, `tokio::time::sleep` and `tokio::time::timeout` goes through `rt::`. A wasm `BridgeThread::spawn` runs the loop with `rt::spawn`, and `join` is a no-op. The prefs writer saves directly on wasm and keeps `spawn_blocking` on native. Signals are skipped off unix. | No threads in the browser. The intent/reply loop runs on the page's event loop beside Rune's frames. |
| `apps/desktop/src/net/sim.rs` | Two lines: `rt::sleep` and `drop(rt::spawn(run_table(..)))`. | Same seam. |
| `apps/desktop/src/net/mod.rs` | `embedded`, `fetch`, `local` and `remote` are gated native-only, and `pub mod rt` is added. `use rune::time::Instant` replaces std's. `wall_ms()` uses `js_sys::Date::now()` on wasm. `for_run` returns `Sim` on wasm, and `build()` maps Embedded and Remote to Sim there. `data_dir_from` returns a constant on wasm. `remote_from_env` is native-only. | `std::time::Instant::now()` and `SystemTime::now()` panic on wasm32. The server backends are NET's work, so the spike plays the sim. |
| `apps/desktop/src/prefs.rs` | wasm `platform()` returns a store whose load and save use `localStorage["euchre:desktop-prefs"]`, holding the same JSON. File I/O is native-only. | There is no filesystem. Prefs are per-browser state, which matches the desktop's per-machine file. |
| `apps/desktop/src/sound.rs` | On wasm, `Output::Open` plays each cue through a small `web_audio` module: a thread-local `AudioContext`, the cue's existing `recipe(cue)` samples rendered into an `AudioBuffer`, and an `AudioBufferSourceNode`. The rodio thread is native-only, and `is_app_binary()` is true on wasm. | rodio does not build for wasm32. The cues are already synthesized samples, so WebAudio plays the same sound. |
| `apps/desktop/src/scenes.rs` | `PortInUse { port }` uses the literal 6420 on wasm. | The constant lives in `euchre-server-core`, which is native-only. This only affects a scene fixture. |
| `apps/desktop/src/lib.rs` | `startup` and `run()` are native-only. Adds `#[cfg(wasm32)] pub mod web`. | `run()` boots the embedded server and the native window. `spec()`, `FONTS` and `setup` are shared unchanged. |
| `apps/desktop/src/web.rs` (new, 109 lines) | `#[wasm_bindgen(start)] fn start()`: loads prefs, builds the Sim backend, opens the bridge channel and calls `App::new(spec).init(..)`. In `init` it calls `state.on_ready()`, which sends Identify. It creates a `UiSink` over `handle.update` and spawns the bridge task, then calls `.on_visibility(..).on_quit(..).try_run()`. Errors go to the console. | The browser entry point. Rune's web target refuses `on_ready` (§4, blocker 1), so Identify is sent while the state is built. The default `AppSpec` already fills the viewport, so neither `canvas_id` nor `full_viewport` was needed. |
| | Proof-only hook: when the URL has `?proof`, the view is wrapped to record a JSON summary of the route and table state, read through `#[wasm_bindgen] fn euchre_probe()`. | Lets the script play legal moves and check state without guessing. Remove it before shipping. It costs nothing without `?proof`. |
| `apps/desktop/src/assets.rs` (fetch build only) | On wasm, the `include_bytes!` card faces, back and hero are gated off. `Art::load` calls `ImageCache::file("art/cards/{id}.png")`, `art/cards/back.png` and `art/euchre-hero.webp`, which are URLs relative to the page. Native is unchanged. The embedded version is saved at `$S/logs/assets.rs.embedded`. | It takes 3.39 MB of art out of the wasm. That art barely compresses (PNG and WebP). See §5.1. |
| `Cargo.lock` | 4 lines: `js-sys`, `wasm-bindgen`, `wasm-bindgen-futures` and `web-sys` under `euchre-desktop`. | All were already in the lock at 0.2.130 and its matching versions. |

Areas the brief listed that needed no change:

- **async_trait `Send` bounds.** The `rt` seam keeps every spawned future `Send`, so the
  `#[async_trait]` traits were not touched.
- **Window resize.** Rune handles it, including a live resize.
- **devicePixelRatio, keyboard, mouse and touch.** Rune handles these with no app code.
- **App options.** The defaults worked.
- **The UI code.** Not one view, screen or motion file changed.

## 4. Blockers and how I got past them

1. **`on_ready` is refused on the web.** `platform::web::run` returns "web does not support
   windowless apps or native window hooks" for overlays, extra windows, `on_window` and
   `on_ready`. `on_visibility` and `on_quit` are allowed. Fix: send Identify from the `init`
   closure, since the bridge queues it until the loop starts.
2. **The fallback adapter loses the device.** Playwright's default headless shell picks
   SwiftShader (the "google, swiftshader, fallback adapter", max texture 8192). Rune's device
   was lost right after the first frame, with "A valid external Instance reference no longer
   exists". Fix: launch with `channel: 'chromium'` (new headless). That gives the real Apple
   adapter, "apple metal-3", with max texture 16384. The flags `--use-angle=metal --enable-gpu
   --ignore-gpu-blocklist` also work. Some real users will be on fallback adapters, so the
   loader should treat a lost device like no WebGPU and serve the Svelte site.
3. **`navigator.gpu` needs a secure context.** `about:blank` has none, so the proof serves over
   127.0.0.1.
4. **`Inter-Regular.ttf` returned 404 after the first frame.** Rune fetches the full Inter faces
   from beside `app.js`. `web-build.sh` finds them through the cdylib's `.d` dep-info, which
   lists only path packages, so it finds nothing when Rune is a git dependency. Fix: the wrapper
   copies `crates/rune/assets/web/Inter-*.ttf` from the cargo git checkout. Rune's script should
   fix this upstream.
5. **wasm-bindgen CLI version.** The CLI must equal the lockfile's `wasm-bindgen` (0.2.130).
   None was installed, so I installed it to a user location (§7).
6. **web-build.sh needs `python3`.** The PATH had none, so the wrapper puts a symlink to the
   existing Python 3.13 first on PATH.
7. **The first-run tour hides the table from Rune's DOM mirror.** Rune keeps a semantic DOM
   mirror (`syncA11y`) of its controls. The proof uses it to find buttons by key path, for
   example `.../bid-buttons/bid-pass`. While the tour modal is open the mirror did not list the
   bid buttons, and in general it updates on layout commits, one commit behind. Fix: the probe
   reports `tour_step`, the script presses Escape to close the tour, and it re-reads the mirror
   after each settle.
8. **A native regression from my own edit** (E0107 on `std::thread::JoinHandle`). Fixed. The
   native check and tests are green again.

## 5. Measurements

The setup:

| Item | Value |
| --- | --- |
| Machine | Apple M4 Pro, 14 cores, 64 GB. Other lanes were building at the same time, so treat build times as upper bounds. |
| Browser | Playwright 1.64.1, Chromium 156.0.8078.4 (`channel: 'chromium'`) |
| Adapter | apple metal-3 |
| Server | localhost |
| Frame source | `GPUQueue.submit` and `requestAnimationFrame` are counted by an init script. A "frame" is one submit, and frame time is the gap between submits. |
| Display | 60 Hz, so 16.7 ms is a full-speed frame. |

### 5.1 Size

| Build | `app_bg.wasm` raw | gzip -9 | brotli 11 |
| --- | ---: | ---: | ---: |
| Art embedded (`$S/web`) | 16,358,535 | 9,105,351 | **7,689,931** |
| Art embedded + `wasm-opt -Oz` (`$S/web-opt`, 29.9 s) | 15,046,559 | 9,179,040 | 7,813,200 (larger) |
| Art by URL (`$S/web-fetch`) | 12,969,061 | 5,770,680 | **4,355,138** |

The other files a page needs:

| File | Raw | brotli 11 | When it loads |
| --- | ---: | ---: | --- |
| `app.js` | 125,059 | 16,455 | Before the first frame |
| Inter Regular, Medium and SemiBold `.ttf` | 888,696 | not measured | Rune fetches them after the first frame |
| `art/` in the fetch build (26 files) | 3,389,493 | 3,340,722 | Requested by `Art::load` |

Where the embedded wasm's bytes go:

| Part | Raw | brotli |
| --- | ---: | ---: |
| Card art and hero | 3,439,953 | 3,374,678 |
| App fonts (`FONTS`) | 1,912,208 | 420,306 |
| Rune's built-in Inter cut | 379,260 | 105,937 |
| Code and other data | about 10.6 MB | about 3.8 MB |

The last row is by subtraction, and brotli sizes do not add exactly. I did not break the code
down further.

What the sizes show:

- **Moving the art out of the wasm** cuts the brotli download by 3.33 MB (43%). It moves the
  same bytes to image requests, which can be cached on their own and fetched after the first
  frame.
- **`wasm-opt -Oz`** saves 1.3 MB raw but makes brotli 123 KB worse. Skip it.

### 5.2 Time to first frame

Time to first frame is `performance.now()` at the first `GPUQueue.submit`. Network time is
close to zero on localhost (`app_bg.wasm` finished downloading at 27 to 44 ms).

| Run | Art embedded | Art by URL |
| --- | ---: | ---: |
| Cold load 1, 2, 3 (new context each, no probe) | 327, 324, 322 ms | 120, 108, 109 ms |
| Phone 390×844 at DPR 3 | 338 ms | 111 ms |
| DPR 2 at 1280×800 | 352 ms | 113 ms |
| Desktop run (`?proof`, recording video) | 419 ms | 173 ms |

What the times show:

- **The fetch build reaches its first frame about 210 ms sooner.** The likely cause is that the
  embedded build decodes its art before the first frame, while the fetch build decodes it as it
  arrives. That cause is not proven.
- **In the fetch build, the hero image arrives after the first frame.** I did not measure that
  gap. A real network adds the download of `app_bg.wasm` (4.36 MB brotli for the fetch build).
  NET's doc covers that.

### 5.3 Frame times while animating

| Window | Build | Frames | p50 | p95 | p99 | max | over 20 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Deal, 3 s, no probe | embedded | 180 | 16.7 | 17.6 | 17.7 | 18.7 | 0 |
| Deal, 3 s, no probe | fetch | 179 | 16.7 | 17.4 | 17.6 | 17.6 | 0 |
| Deal, 3 s, probe + video | embedded | 178 | 16.7 | 17.6 | 17.7 | 34.2 | 1 |
| Deal, 3 s, probe + video | fetch | 176 | 16.7 | 17.5 | 33.3 | 83.4 | 2 |
| Trick sweep, 1.2 s | embedded | 71 | 16.6 | 17.3 | 17.6 | 17.6 | 0 |
| Trick sweep, 1.2 s | fetch | 71 | 16.7 | 17.5 | 17.7 | 17.7 | 0 |

All times are in ms.

- **Without the probe**, the deal holds 60 fps with no frame over 18.7 ms.
- **The single long frames appear only in the probe and video runs.** These are the first deal
  after the table opens, while video encoding runs. The fetch build's one 83 ms frame did not
  repeat in its probe-free deal. I did not find its cause.

### 5.4 Idle frames at rest

| Where | Window | Submits | rAFs |
| --- | ---: | ---: | ---: |
| Home (both builds, every run) | 2 to 3 s | **0** | 0 |
| New game | 2 s | **0** | 0 |
| Table, your turn to bid, normal motion | 3 s | 60 to 61 | 120 to 124 |
| Table, your turn to bid, `prefers-reduced-motion: reduce` | 3 s | **1** | not counted |
| Table, tour open | 2 s | 40 to 41 | 80 |
| Right after selecting a card (starts 250 ms after the click, so it includes the lift motion) | 2 s | 108 to 111 | 118 to 121 |
| After the hand result (the next hand starts dealing inside the window) | 3 s | 165 to 167 | 218 to 222 |

The reduced-motion rows come from `$S/logs/reduced-fetch.log`.

- **When nothing moves, nothing draws.** Home and New game are at true zero.
- **At your turn the page is never fully at rest.** The table keeps drawing about 20 frames a
  second. That is the app's `TURN_HALO` breathing `Cycle` (1.6 s period, capped at 30 fps), so
  it is by design. With reduced motion it drops to one frame in 3 s. Whether 20 fps of halo is
  acceptable for a browser tab, on battery, is a product call. On a laptop it is cheap, but it
  is not zero.

### 5.5 Build times

All builds are release builds (`web-build.sh` default: opt-level s, thin LTO, one codegen unit,
`wasm-bindgen --target web --remove-name-section`). Target dir:
`.cargo-targets/webspike`.

| Build | Cargo | Total |
| --- | ---: | ---: |
| Cold (empty target dir) | 1 m 10 s | about 72 s |
| After changing `web-sys` features | 56 s | 57 s |
| Incremental (app code only) | 17 to 22 s | 18 to 23 s |

Other times:

- Native `cargo check --all-targets`: 8 s.
- Native `cargo test --lib`: 104 s, 141 passed.

## 6. Findings to fix before shipping

1. **Suit glyphs draw as tofu.** ♠ ♣ ♦ show as boxes in the wordmark, in "Spades are trump",
   in the "Led" chip, in a seat chip, in the Home footer and in the center watermark. ♥ draws. On the web Rune
   has no system font fallback, and the app's fonts lack those code points. Fix: ship a small
   OFL symbol subset in `FONTS`, or draw suits as images.
2. **Phone widths are broken at the table.** The 390×844 run:
   - Home is fine.
   - The New game title is clipped ("Your table. Your rules").
   - At the table, the score header and the trump line overlap.
   - Seat names are clipped ("M…", "Ve…").
   - "Pass" and "Make spades trump" overlap.
   - The desktop's 720×520 `min_size` is not enforced on the web. The page needs a narrow layout,
     or the loader must send narrow screens to the Svelte site.
3. **The page shell is Rune's placeholder.**
   - `index.html` has the title "Rune".
   - With no WebGPU it shows a text error. NET's loader (§3.4 there) replaces it, and must also
     fall back when the device is lost (blocker 2).
4. **The DOM mirror lags one layout commit, and stays stale while the tour is open.** This
   matters for tests and screen readers. Rune should be asked about it.
5. **Turn halo idle cost.** About 20 fps while it is your turn (§5.4).
6. **`web-build.sh` misses the Inter faces with git dependencies** (blocker 4). Fix it upstream,
   or copy them in the Dockerfile as NET's doc does.
7. **Don't run `wasm-opt`.** It worsens brotli (§5.1).
8. **Console notice.** Chromium logs a verbose "[DOM] Password field is not contained in a
   form" once per load from Rune's DOM layer. It is harmless but noisy.
9. **Remove the `?proof` probe** in `web.rs` before shipping.

## 7. Tools

Installed:

| What | How | Where | Time |
| --- | --- | --- | --- |
| wasm-bindgen-cli 0.2.130 (`wasm-bindgen`, `wasm-bindgen-test-runner`, `wasm2es6js`) | `cargo +1.99.1 install wasm-bindgen-cli --version 0.2.130 --locked --root ~/.local/wasm-bindgen-0.2.130` | `~/.local/wasm-bindgen-0.2.130/bin` | 31 s |

Already present, nothing installed:

- the `wasm32-unknown-unknown` target on 1.99.1;
- Python 3.13, symlinked at `$S/bin/python3`;
- `brotli`;
- `wasm-opt` (binaryen 129);
- bun 1.4.4;
- Playwright 1.64.1, from the layerr checkout's `node_modules`, with its Chromium 1248 and
  ffmpeg 1013 for video.

## 8. What the next lane needs

- Take §3 as the wasm diff for `apps/desktop` and `crates/euchre-ai`.
- NET's §2.5 lists the same gates.
- The two docs differ on tokio: it needs `macros` as well as `sync` for the bridge's `select!`.
- Replace the Sim mapping in `BackendKind::build` with NET's `WebBackend`.
- Fix §6 items 1 and 2 before showing the table on phones.
