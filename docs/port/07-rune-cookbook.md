# 07 - Rune cookbook (verified by compiling)

Everything the native desktop client (`apps/desktop`, crate `euchre-desktop`, bin `euchre`) needs from Rune: dependency lines, app skeleton, card images, shared-element card motion, keyboard, background threads and tokio, the rivetkit-client bridge, felt and wood rail, fonts, window sizing, headless snapshots and scripts, and `rune::Runtime` tests.

Rune is pinned to `github.com/anomalyco/rune` rev `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb`, with toolchain 1.99.1. rivetkit-client is 3.1.0.

**How this was verified.** A throwaway spike crate was built outside the rune repo. It used rune and rune-kit by path at the pinned rev, edition 2024, and the copied `rust-toolchain.toml`. For each recipe below:

- Code marked **VERIFIED** compiled, passed `cargo clippy --all-targets -- -D warnings`, and ran.
- Each `.png` named below was rendered headless and inspected by eye.
- Each test named below passes.
- Code marked **FROM SOURCE** was read from the rune or rivetkit-client source at the pinned versions but not run.

The full verified files are reproduced verbatim in the Appendix. They are the authoritative copies; the inline snippets are excerpts from them.

Related docs: `06-ui-spec.md` covers the animation and timing contract mapped in section 6.4. `02-protocol.md` and `03-table-actor.md` cover the events and actions the bridge in section 8 carries.

---

## 0. Verification matrix

| # | Requirement | Proof (headless PNG, viewed) | Test (`tests/table.rs`, all pass) |
|---|---|---|---|
| a | PNG card (`AS.png`) drawn inside a rounded element | `table.png`, `table-noclip.png` (`--crop hand`) | `card_png_decodes_to_its_pixel_size` |
| b | Row of 5 cards; pressing one springs it to a centre trick area (shared element) | `click-mid.png` (0.15 s after the click: card mid-flight above everything, hand reflowing), `click-settled.png`, `strip.png` (frame strip) | `pressing_a_hand_card_flies_it_to_the_trick` |
| c | Background std::thread and tokio task update state through `AppHandle::update` | `live.png` (`--realtime --at 1.5`: "ticks 14 \| seat 2 played QD", QD moved to the trick by the tokio task); `live-click.png` (UI -> tokio -> UI round trip: "server ack play AS") | `app_handle_updates_reach_a_headless_run`, `background_updates_land_through_runtime_update` |
| d | Keyboard: arrow keys and Enter | `keys.png` (Right, Right, Left, Enter played KH; cursor ring on next card) | `arrow_keys_move_the_cursor_and_enter_plays`, `on_key_can_be_attached_to_a_plain_runtime` |
| e | Felt radial gradient plus rounded wood rail | every snapshot | (visual) |
| f | Scenes plus a headless script that clicks a card and snapshots after it settles | `scene-played.png`, `click-settled.png` (`--at mark:played+1.0`) | (CLI) |
| + | Opponent fan (rotated backs) -> trick; trick -> winner's stack (shrinks) | `opp-mid.png`, `opp-trick.png`, `collect-mid.png`, `collect-settled.png` | `opponent_back_flies_to_the_trick_and_collect_shrinks_into_the_stack` |
| + | Cards scale to fit the minimum window (layout read) | `min-size3.png` (720x520); `min-size2.png` shows the overflow before the fix | `cards_scale_down_to_fit_the_minimum_window`, `set_viewport_resizes_and_the_cards_rescale` |
| + | Reduce Motion lands cards without a flight | - | `reduced_motion_lands_the_card_without_a_flight` |
| + | Image paths: `Pixels::set` vs `ImageCache` (mips) vs `ImageCache::svg` | `quality.png` (`--scene quality --crop trick`) | - |
| + | rivetkit-client 3.1.0 -> `AppHandle` bridge compiles and links with rune | - | `rune-bridge-check/tests/connect.rs`: `connect_outside_a_tokio_runtime_panics`, `connect_inside_a_runtime_handle_enter_works` |
| + | Fresh lockfile (taffy 0.15.1 and 53 other newer patch versions) | `click-settled.png` from the drift build | the 6 core tests pass with the fresh lock |

The final test run was 10 passed, 0 failed, in 0.33 s. Clippy reported 0 warnings in the spike (`-D warnings`). The bridge crate's 2 tests passed and its clippy run was clean.

---

## 1. Dependencies, toolchain, profiles, lockfile

### 1.1 Dependency lines (already in the root `Cargo.toml`)

```toml
[workspace.dependencies]
rune = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
rune-kit = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
rivetkit-client = "3.1.0"
tokio = { version = "1", features = ["full"] }
```

- Both git lines resolve. `cargo generate-lockfile` in a scratch crate fetched the rev, and the `euchre-rune/Cargo.lock` already records `rune 0.0.1 (git+...?rev=85cd8c6...)`.
- The package name is `rune`, and the crate is imported as `rune`.
- rune-kit lives in the same repo, and Cargo finds it by package name.

`apps/desktop/Cargo.toml` needs these additions (rune-kit and png are not there yet):

```toml
[lib]                       # testable library + thin bin (see section 12)
name = "euchre_desktop"
path = "src/lib.rs"

[[bin]]
name = "euchre"
path = "src/main.rs"

[dependencies]
rune.workspace = true
rune-kit.workspace = true     # add: Kit/Button themed controls
rivetkit-client.workspace = true
tokio.workspace = true
serde_json.workspace = true
anyhow.workspace = true
# png = "0.18"                # only if you keep the synchronous Pixels::set path (not recommended, 4.2)
```

### 1.2 Rune feature flags (rev 85cd8c6, `crates/rune/Cargo.toml`)

| Feature | Default | What it does | Euchre needs it? |
|---|---|---|---|
| `accessibility` | yes | AccessKit adapters (NSAccessibility / UIA / AT-SPI). `--tree` reports the tree even without it. | keep |
| `dbus` | yes | Linux desktop services over zbus (theme, tray, Wayland shortcuts); pulls a tokio. No effect on macOS. | keep (harmless) |
| `embedded-fonts` | yes | Web only: bundle Inter + IBM Plex Mono into wasm. Native always bundles Inter. | irrelevant |
| `metal-backend` | yes | macOS/iOS native Metal renderer (default on Mac). Opt out with `App::gpu_backend(GpuBackend::Wgpu)` or `RUNE_GPU_BACKEND=wgpu`. | keep |
| `ogg-audio` | yes | macOS: Ogg/Opus/WebM audio decode in Rust. | keep if card sounds are .ogg; else harmless |
| `svg` | yes | `ImageCache::svg/file/load_mime` and SVG sniffing via resvg. Off: SVGs resolve to `Image::error`. | keep (icons) |
| `native-file-dialog` | no | Open/Save dialogs (rfd). | no |
| `hot` | no | Live editing (`--hot`, `--snap-on-patch`, `--hot-control`). Without it those flags exit with "live editing requires the rune/hot feature". | optional dev |
| `avif` | no | AVIF decode via rav1d (x86 needs NASM). | no |
| `frame-profile` | no | Headless benchmark phase clocks. | no |
| `webgl` | no | wasm32 WebGL2 fallback. | no |
| `macos-background-cursor` | no | Private API cursor in background (App Store risk). | no |
| `baseline-taffy` | no | A/B layout engine parity. | no |

Use the defaults; no `features = [...]` is needed. The spike built with the defaults.

### 1.3 Toolchain

`rust-toolchain.toml` (root, already present) pins `channel = "1.99.1"` with `components = ["clippy", "rustfmt"]`. Rune requires it; the spike copied it unchanged.

### 1.4 Profiles (ADD to the root `Cargo.toml`)

The workspace has `[profile.dev.package."*"] opt-level = 2` but no debug settings. Rune's own workspace notes that full debug info made a rune test build use 18-23 GB of RAM. The spike copied rune's settings. Merge them into what exists:

```toml
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
opt-level = 2        # existing: rune's renderer and text stack are slow unoptimized
debug = false        # add
```

### 1.5 Lockfile drift (measured)

A fresh resolve of the git dependency picks newer patch versions than rune's own `Cargo.lock`:

- **Count:** 54 packages differ, a few of them only by dropping duplicates.
- **Examples:** `taffy 0.15.0 -> 0.15.1`, `icu_* 2.3 -> 2.4`, `uuid 1.28.0 -> 1.29.2`, `zbus 6.1.0 -> 6.2.1`, `cc 1.9.0 -> 1.11.1`, `smallvec 1.16.3 -> 1.16.5`, `syn 3.0.9 -> 3.0.10`, `wasm-bindgen 0.2.129 -> 0.2.130`.
- **Matching:** the render and platform stack is identical. That means `wgpu 30.0.1`, `winit 0.30.13`, `parley 0.12.0`, `image 0.25.10`, `png 0.18.1`, `resvg 0.45.1`, `accesskit 0.25.2` and `tokio 1.54.1`.
- **This workspace:** `euchre-rune/Cargo.lock` already has `taffy 0.15.1`, and 62 packages differ from rune's lock.

The drifted lock was verified: it builds, the 6 core tests pass, and `click-settled.png` is indistinguishable from the rune-lock build. **Recommendation:** accept the drift. If a layout difference ever appears, pin the one crate back with `cargo update -p taffy --precise 0.15.0`. Seeding from rune's lock gives zero drift, but only for a brand-new lockfile: copy `rune/Cargo.lock` in before the first resolve, as the spike did.

### 1.6 Build facts

| Fact | Value |
|---|---|
| Spike first build, warm rune target dir | ~46 s |
| Bridge crate (adds rivetkit-client and its deps), same target dir | 1 m 25 s |
| Incremental rebuild of the spike after an edit | ~2-4 s |
| One headless run with a snapshot (debug build) | ~1-3 s wall |
| First headless frame CPU | ~12 ms; GPU init ~2 ms; animating frames p50 0.3-1.9 ms |
| Rune warnings | A path dependency prints 4 rune warnings (`press_repeat`, `quit`, `redraw`, `frame`). A git dependency caps lints, so `-D warnings` on our crates is unaffected either way. |
| `unsafe_code = "forbid"` (workspace lint) | compatible: none of the recipes use `unsafe` |

---

## 2. App skeleton

### 2.1 Shape of a Rune app

- The **view** is a plain function: `fn view(&S, &mut Cx<S>) -> El`. It re-runs only when state changed (or a layout it read changed).
- **Handlers** (`on_press`, `on_key`, `AppHandle::update`) are queued and run **between frames**. Each one gets `&mut S` (and usually `&mut Tx`), and the next frame commits the change with the motion the `Tx` asked for.
- `S` does not need to be `Send`. Only the closures sent through `AppHandle::update` must be `Send + 'static`.

### 2.2 `AppSpec<S>` fields (`tooling.rs:17`)

| Field | Type | Default (`AppSpec::new`) | Notes |
|---|---|---|---|
| `title` | `&'static str` | arg | window title |
| `size` | `(f32, f32)` | arg | logical points |
| `min_size` | `Option<(f32, f32)>` | `None` = **420x400** (`window/spec.rs:69`) | user resize floor |
| `init` | `fn() -> S` | arg | fn pointer: **cannot capture**. Use `App::init(closure)` to capture channels. |
| `view` | `fn(&S, &mut Cx<S>) -> El` | arg | |
| `scenes` | `&'static [(&'static str, fn(&mut S))]` | `&[]` | `--scene NAME`, `--list-scenes`. Clippy flags the inline type as `type_complexity`, so use a `type Scene` alias. |
| `fonts` | `&'static [&'static [u8]]` | `&[]` (built-in Inter) | **non-empty REPLACES Inter** (section 10) |
| `families` | `(&'static str, &'static str)` | `("Inter", "Inter")` | (sans slot, mono slot) |
| `setup` | `Option<fn(&mut S, Waker)>` | `None` | once, before the first frame; gets the waker for `ImageCache` (no-op headless) |
| `poll` | `Option<fn(&mut S, f64, &mut Tx) -> bool>` | `None` | every frame with seconds since start (virtual headless); return true if state changed |
| `on_key` | `Option<fn(&mut S, &KeyEvent, &mut Tx) -> bool>` | `None` | keys nobody focused took (section 7) |

Always construct with struct update: `AppSpec { min_size: Some(..), ..AppSpec::new(title, size, init, view) }`. New fields then never break the literal.

### 2.3 `App<S>` builder (`app.rs`)

| Method | Line | Use |
|---|---|---|
| `App::new(spec)` | | |
| `.init(FnOnce() -> S + 'static)` | 340 | build the first state with captures (a channel sender, config). Scenes still apply on top. |
| `.args(Vec<String>)` | 348 | read Rune flags from this list instead of the process args (tests, or an app that owns its CLI) |
| `.windowless()` | 354 | start with no window (implies `keep_running`) |
| `.keep_running()` | 362 | closing the window keeps the app alive |
| `.window_options(WindowOptions { .. })` | 196 | native window options (section 11) |
| `.fit_content_height()` | 205 | window height follows content |
| `.on_ready(FnMut(&WindowHandle) + 'static)` | 506 | after the window's first frame, **once per native window creation**. Not called by plain headless tooling; called by `--realtime` headless. |
| `.handle() -> AppHandle<S>` | 545 | Clone + Send + Sync handle for other threads |
| `.run()` | 578 | blocks the main thread until quit; headless flags render offscreen instead; panics if the app cannot start |
| `.try_run() -> Result<(), String>` | 585 | `run` without the panic (no display in CI) |
| `.headless_runtime(size, Env)` | `playground/testing.rs:13` | a `Runtime<S>` with the whole spec wired, for tests (section 13) |

`AppHandle<S>`:

- `update(FnOnce(&mut S, &mut Tx) + Send + 'static)` (`app.rs:750`).
- `update_if(FnOnce(&mut S, &mut Tx) -> bool + Send + 'static)` (`app.rs:762`). Returning false means "nothing the view reads changed": no view re-run and no redraw, but `Tx` requests still happen.
- Both run on the UI thread, in call order, even while no window shows.

### 2.4 Imports used by the spike (VERIFIED)

```rust
use rune::color::hex;
use rune::tok::{Family, TextStyle};
use rune::{
    AppSpec, Color, Cx, El, Image, ImageCache, Key, KeyEvent, Pixels, Spring, SurfaceFit, Tx, Waker, col, el, row,
    text,
};
use rune_kit::{Button, ButtonVariant, Kit, PaletteSeed};
// main.rs / tests:
use rune::{App, AppHandle, Env, Mods, Runtime};
```

---

## 3. (a) A PNG card inside a rounded element - VERIFIED

```rust
pub const CARD_W: f32 = 100.0; // 512x716 art -> 100 x 140 pt (aspect 0.715)
pub const CARD_H: f32 = 140.0;

pub fn card(face: &Pixels, key: String, matched: String, k: f32) -> El {
    el().key(key)
        .matched(matched)
        .size(CARD_W * k, CARD_H * k)
        .radius(8.0)            // clips the picture (and gradients) to the rounded corners
        .fill(Color::WHITE)     // shows behind transparent PNG corners
        .pixels_fit(face, SurfaceFit::Fit)
}
```

- `.pixels(&Pixels)` (`el/visuals.rs:215`) **stretches** the image into the element's rect.
- `.pixels_fit(&Pixels, SurfaceFit::Fit | SurfaceFit::Fill)` (`:223`) keeps the aspect ratio, letterboxing (Fit) or cropping (Fill).
- Both are clipped to `.radius()`. Do **not** add `.clip_rounded(r)` (`:453`) to the card. It clips the element's own shadow and outline too: the first spike snapshot lost the cursor outline and drop shadows. `.radius()` alone already clips the picture.
- **Card chrome that worked:**
  - `.shadow(6.0, 14.0, Color::BLACK.alpha(0.35))` (`:139`)
  - `.outline(width, offset, color)` (`:231`): drawn outside the box, used for the cursor ring
  - `.press_scale(0.97)`
  - `.offset(0, -18)` for the lifted cursor card (section 5.4)

---

## 4. Images: loading, caching, quality

### 4.1 Three paths compared (`quality.png`, `--scene quality --crop trick`)

| Path | API | Decode | Mipmaps | Result when drawn smaller than the source (512x716 -> 50x70 or 100x140 at scale 1) |
|---|---|---|---|---|
| Synchronous `Pixels` | `let p = Pixels::new(); p.set(w, h, rgba_straight_srgb)` (`image/pixels.rs:90`) | your thread (png crate) | **no** | **aliases / moire** (visible in `quality.png` and in `min-size3.png` at `--scale 1`) |
| `ImageCache::load(bytes)` | `cache.load(&'static [u8] or Arc<[u8]>) -> Image` (`image.rs:183`) | up to 4 worker threads, async | **yes** (static images) | clean at every size |
| `ImageCache::svg(bytes, size_pt, scale)` | (`image.rs:196`) rasterised by resvg at `size_pt x scale` px (the square the drawing fits) | async | yes | crisp at the drawn size |

Other `ImageCache` entry points (FROM SOURCE):

- `file(path, size_pt, scale)` (`:218`): a runtime file, sniffed, so PNG or SVG.
- `load_mime(bytes, mime, size_pt, scale)` (`:208`).
- `rgba(w, h, bytes)` (`:234`).
- `app_icon(path, size_pt)` (`:246`).

`Image` methods (`image.rs:54-64`):

- `pixels() -> &Pixels`: feed it to `.pixels()` / `.pixels_fit()`.
- `size() -> Option<[u32; 2]>` and `is_ready()`.
- `error()`.
- `wait() -> Result<[u32; 2], &ImageError>`: blocks until decoded.

Handles dedupe by content. Pixels live in a 1024 px atlas page (`IMAGE_ATLAS_SIZE`, `image/pixels.rs:9`), and larger images get their own pages.

**Recommendation for euchre:** `ImageCache::load` on the **PNG** faces, decoded once in `setup`:

- The original UI uses the PNGs (`src/lib/ui/card-art.ts:12`, `CardRack.svelte:146`). The `.svg` files in `static/art/cards/` are a **different, plainer design** and are unused, so SVG would change the look.
- The art is CC0 (`static/art/cards/LICENSE.txt`).
- 24 faces at ~50 KB each plus the 457 KB back is about 1.7 MB embedded.

### 4.2 `include_bytes!` vs a runtime file

| | `include_bytes!` (VERIFIED) | runtime file (`cache.file(path, pt, scale)`, FROM SOURCE) |
|---|---|---|
| Distribution | single binary, no path lookup | must ship assets next to the binary / in the .app bundle and resolve the path |
| Determinism in tests | perfect | depends on the working directory |
| Build cost | ~1.7 MB in the binary; recompiles the crate when art changes | none |
| Hot swap | no | yes |

Use `include_bytes!`. Paths are relative to the source file: `include_bytes!("../assets/cards/AS.png")` from `src/lib.rs`.

### 4.3 The `setup` recipe (VERIFIED)

```rust
pub fn setup(s: &mut State, waker: Waker) {
    let cache = ImageCache::new(waker);              // the waker redraws the window when a decode lands
    let back = cache.load(BACK);                     // BACK: &'static [u8] from include_bytes!
    let cached: Vec<Image> = CARDS.iter().map(|(_, bytes)| cache.load(*bytes)).collect();
    // size_pt is the square the drawing fits (its long side here); scale = device px per point.
    let svg_small = cache.svg(&include_bytes!("../assets/cards/KH.svg")[..], 70.0, 2.0);
    // Headless renders run on a virtual clock: block so the first snapshot has the image.
    if rune::is_headless() {
        let _ = back.wait();
        for img in &cached {
            let _ = img.wait();
        }
        let _ = svg_small.wait();
    }
    s.svg_small = Some(svg_small);
    s.cached = cached;
    s.back = Some(back);
    s.cache = Some(cache);                           // keep the cache alive in state
}
```

Gotchas:

- **Async decode:**
  - In a window, an image appears a few frames late. The waker redraws when it lands.
  - In a headless run, `setup` gets a **no-op waker** and the clock is virtual. Without `wait()`, a snapshot can render before the picture exists.
- **When `rune::is_headless()` is true** (`tooling/args.rs:236`):
  - It parses the **process** arguments once.
  - It is **false under `cargo test`** and false when you drive a run through `App::args(..)`.
  - In a test that needs pixels, call `wait()` unconditionally, or decode synchronously.
- **Synchronous decode for tests** (VERIFIED, `decode_png` in the Appendix):
  - Uses `png = "0.18"` with `set_transformations(Transformations::normalize_to_color8())`, then expands RGB / gray to RGBA.
  - Clippy 1.99 rejects `chunks_exact(3)`. Use `as_chunks::<3>().0.iter()`.

---

## 5. (b) Shared-element card motion - VERIFIED

### 5.1 Hand -> trick (`click-mid.png`, `click-settled.png`, `strip.png`)

The mechanism is `El::matched(id)` (`el/motion.rs:215`):

- When an element with `id` **leaves** the tree in the same commit that another with `id` **mounts**, the new one starts exactly where the old one was seen. It keeps the old one's velocity, corner radius, fill and opacity, then springs to its own layout.
- While flying, it draws **above the whole window**, inside its destination's clips.
- The spring is the element's `.motion()`, else the commit's `Tx::motion`, else `tok::motion::REFLOW` (0.4 s).

```rust
pub const FLIGHT: Spring = Spring::new(0.45, 0.12);    // const fn: (duration s, bounce)

// State: a card id lives in exactly one of these vectors.
pub fn play(&mut self, id: usize) {
    let Some(at) = self.hand.iter().position(|&c| c == id) else { return };
    self.hand.remove(at);
    self.trick.push(id);
    self.cursor = self.cursor.min(self.hand.len().saturating_sub(1));
}

// View: same matched id in both places, different keys.
let trick = row().key("trick")
    .size(5.0 * CARD_W * k + 4.0 * 12.0 + 32.0, CARD_H * k + 32.0)
    .padding(16.0).gap(12.0).center().radius(16.0).fill(Color::BLACK.alpha(0.12))
    .children(s.trick.iter().map(|&id| card(&s.faces[id], format!("trick-{id}"), format!("card-{id}"), k)));

let hand = row().key("hand").gap(12.0).center().h(CARD_H * k + 24.0).children(s.hand.iter().enumerate().map(
    |(i, &id)| {
        let on = i == s.cursor;
        card(&s.faces[id], format!("hand-{id}"), format!("card-{id}"), k)
            .motion(FLIGHT)                                   // the hand reflow uses the same spring
            .offset(0.0, if on { -18.0 } else { 0.0 })        // lifted cursor card, springs
            .outline(if on { 3.0 } else { 0.0 }, 2.0, c("#ffd75e"))
            .shadow(6.0, 14.0, Color::BLACK.alpha(0.35))
            .press_scale(0.97)
            .on_press(move |s: &mut State| {
                s.play(id);
                if let Some(net) = &s.net {
                    let _ = net.send(Cmd::Play(id));          // non-blocking, see section 8
                }
            })
    },
));
```

The test asserts these points:

- After the press, `trick == [2]` and `hand-2` is gone.
- 0.1 s later, `trick-2` is drawn between the hand and the trick, and `rt.animating` is true.
- After 1.5 s it sits at `trick.y + 16` (the padding), at height `CARD_H`, and `!rt.animating`.

### 5.2 Opponent fan -> trick (`opp-mid.png`, `opp-trick.png`)

An opponent's card comes from a face-down back, so its matched id must be on the back **in the view before the play**: that is the element that will leave. Put the "next play" id on the top back only, and name it with a monotonic counter so ids never repeat.

```rust
// State: opp_left (backs in the fan), opp_played: Vec<(seq, face)>, opp_seq: usize (monotonic)
pub fn opp_play(&mut self, id: usize) {
    if self.opp_left == 0 { return; }
    self.opp_left -= 1;                       // the top back (key opp-back-{n-1}) leaves...
    self.opp_played.push((self.opp_seq, id)); // ...and trick-opp-{seq} mounts with its matched id
    self.opp_seq += 1;
}

// View: a fan of backs turned about their bottom centre.
let n = s.opp_left;
let opp = row().key("opp").gap(4.0).center().h(80.0 * k).children((0..n).map(|i| {
    let tilt = (i as f32 - (n as f32 - 1.0) / 2.0) * 0.09;       // radians, clockwise
    let back = el().key(format!("opp-back-{i}")).size(50.0 * k, 70.0 * k).radius(5.0)
        .fill(Color::WHITE).rotate(tilt).origin(0.5, 1.0);
    let back = match &s.back { Some(img) => back.pixels(img.pixels()), None => back };
    if i + 1 == n { back.matched(format!("opp-play-{}", s.opp_seq)) } else { back }
}));
// in the trick:
.children(s.opp_played.iter().map(|&(seq, id)| card(&s.faces[id], format!("trick-opp-{seq}"), format!("opp-play-{seq}"), k)))
```

What was observed:

- During the flight the element draws the **new** element's picture: the face, not the back. It grows from the 50x70 back's rect to 100x140.
- `rotate(tilt)` (`el/visuals.rs:381`) with `origin(0.5, 1.0)` (`:399`) produced a correct fan. Layout keeps the upright box, and hit testing follows the turned box.

### 5.3 Trick -> winner's stack, shrinking (`collect-mid.png`, `collect-settled.png`)

```rust
pub fn collect(&mut self) {
    for id in self.trick.drain(..) { self.won.push((format!("card-{id}"), id)); }
    for (seq, id) in self.opp_played.drain(..) { self.won.push((format!("opp-play-{seq}"), id)); }
}
// View (in the status bar): minis that the trick cards fly into on collect.
row().key("won").gap(2.0).children(s.won.iter().map(|(m, id)| {
    el().key(format!("won-{m}")).matched(m.clone()).size(24.0, 34.0).radius(3.0)
        .fill(Color::WHITE).pixels_fit(&s.faces[*id], SurfaceFit::Fit)
}))
```

Every trick card leaves and a 24x34 mini mounts in the same commit, so each card shrinks along a straight path into the stack. The test checks that the height is still above 35 at 0.1 s and exactly 34 at rest.

### 5.4 Motion primitives (rev 85cd8c6)

| Need | API | Status |
|---|---|---|
| Fly between places | `.matched(id)` (`el/motion.rs:215`); `.matched_crop()` keeps children unscaled | VERIFIED |
| Spring a layout change (reflow, resize) | `.motion(Spring)` (`:89`); per commit: `tx.motion(Spring)` (`el/tx.rs:527`) overrides it | VERIFIED |
| Lift / nudge without moving layout | `.offset(x, y)` (`:113`), springs; `.offset_motion(..)` | VERIFIED |
| Rotate / scale about a point | `.rotate(rad)` (`visuals.rs:381`), `.scale(k)` (`:364`), `.origin(x, y)` (`:399`), `.rotate_motion/.scale_motion` | rotate+origin VERIFIED; scale FROM SOURCE |
| Press feedback | `.press_scale(0.97)` | VERIFIED |
| Enter/exit | `.transition(Transition)` (`:196`). Default: 0.2 s fade in, 0.15 s fade out. `Transition::offset(0, 8).combined(Transition::opacity())`; `.enter_delay(s)` staggers (deal-in) | FROM SOURCE |
| Keyframed opacity/scale | `.sequence(rune_motion::Sequence)` (`:259`), `.looping(LoopSpec)` | FROM SOURCE |
| Paint above siblings | `.raise()` (`:132`), `.paint_order(..)` | FROM SOURCE |
| No animation for this commit | `tx.instant()` (`tx.rs:513`); `tx.settle()` (`:553`) also lands running tracks | FROM SOURCE |
| Timers without a thread | `tx.wake_after(Duration)` / `tx.wake_at(clock_s)` (`tx.rs:226/242`) + `AppSpec::poll` | FROM SOURCE |
| Tokens | `tok::motion::{SNAP/SNAPPY 0.25, SETTLE 0.3, REFLOW 0.4, GLIDE 0.32, MORPH, POP, DRIFT 0.85, LEAD 0.2}` | FROM SOURCE |

### 5.5 Reduce Motion - VERIFIED

Under the OS "Reduce motion" setting, nothing is matched. The old element exits and the new one enters in place by its own transition, and layout springs land instantly. In tests, use `rt.set_system_reduced_motion(true)` (`runtime/system_settings.rs:65`). Writing `rt.reduced_motion = true` does **not** stick, because each commit recomputes it. A view can override per commit with `cx.reduced_motion(on)` and read it with `cx.is_reduced_motion()`, for example to honour a stored `table.reduceMotion` preference (06-ui-spec section 6.5).

---

## 6. Mapping the 06-ui-spec animations to Rune

| 06 spec item | Rune recipe |
|---|---|
| 6.2 card play flight, 700 ms, from the hand slot (you) or the seat anchor (others) | You: `matched("card-{id}")` hand -> trick (5.1). Others: matched id on the seat's top back (5.2). Use `Spring::new(0.7, ~0.1)` to match the 700 ms. The 82% overshoot keyframe becomes the spring's bounce. The `rotate(-6deg)` start for others: give the back a tilt; the flight carries the radius, fill and opacity but **not** the rotation (it lands on the destination's own transform), so treat a rotation flourish as unverified. |
| 6.2 "skip on reconnect" (pre-seen keys) | Matched flights only happen when an element leaves and one mounts **in the same commit**. A snapshot applied on reconnect mounts trick cards with no leaving partner, so they appear in place with a fade. Apply resync snapshots with `tx.instant()` to suppress even the fade. |
| 6.3 up-card pickup to the dealer, 850 ms, shrink + arc | `matched` from the up-card element to a small element at the dealer seat (like 5.3). The arc (`- sin(p*PI)*24`) is not expressible as one straight matched path; accept straight-line or approximate it with a `.sequence(..)` on the destination. |
| 6.4 trick collect 780 ms, 45 ms stagger, shrink to .18 + fade | `collect()` (5.3), whose destination minis may sit at the winner's seat. For a stagger, mount the minis over successive commits (`tx.wake_after(45ms)` + `poll`), or use `enter_delay` on the minis. `rotateY(90deg)` has no 3D equivalent; use `scale` toward 0 plus opacity. |
| 6.5 rack hover lift -4 px / selection lift -8 px, 180 ms | `.offset(0, -4/-8)` with `.offset_motion(Spring::new(0.18, 0.0))`; the hover state comes from `.on_hover` or a hover style |
| 6.5 deal-in 580 ms, stagger 75 ms | `.transition(Transition::offset(..).combined(Transition::opacity()))` + `.enter_delay(i * 0.075)` |
| 6.5 banter bubble 6.5 s, illegal message 3.2 s, banner 4.5 s | store a deadline in state; `tx.wake_at(due)` + `AppSpec::poll` clears it |
| 6.5 confetti | `.looping(..)` / `.sequence(..)` per piece, or a `poll`-driven offset; unverified |
| Reduce motion | automatic for matched/springs (5.5); keep the 2.2 s read hold in state logic |

---

## 7. (d) Keyboard - VERIFIED (`keys.png`)

```rust
/// Arrow keys move the cursor; Enter (or Up) plays the cursor card; Escape resets.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let n = s.hand.len();
    match &e.key {
        Key::Left if n > 0 => s.cursor = (s.cursor + n - 1) % n,
        Key::Right if n > 0 => s.cursor = (s.cursor + 1) % n,
        Key::Enter | Key::Up if n > 0 => {
            tx.motion(FLIGHT);                  // keys commit with this commit-wide spring
            let id = s.hand[s.cursor];
            s.play(id);
        }
        Key::Escape => s.reset(),
        Key::Text(t) if t == "o" => { /* seat 2 plays */ }
        Key::Text(t) if t == "c" => { /* collect */ }
        _ => return false,                      // not consumed: routing continues
    }
    true
}
// wire it: AppSpec { on_key: Some(on_key), ..AppSpec::new(..) }
```

- `KeyEvent { key: Key, mods: Mods { shift, cmd, alt, ctrl, logo } }`.
- `Key` variants include `Left, Right, Up, Down, Enter, Escape, Tab, Backspace, Delete, Home, End, PageUp, PageDown, Function(u8), Text(String), Command(char)`. Printable keys arrive as `Key::Text("o")`.

**Key routing** (rune `README.md:246`). The first stage that takes the key wins:

1. IME composition.
2. **Return with no modifiers presses the window's `.default_button()`** (`el/focus.rs:177`), unless the focused element takes Return.
3. **Nothing focused: `AppSpec::on_key`** (`tx.focused()` is `None`).
4. The focused field's `on_input_key`.
5. Focus navigation: Tab, Shift-Tab, arrows in a `focus_group`, Space presses the focused control, Escape dismisses the open modal.
6. The text editor.
7. `AppSpec::on_key` again, with `tx.focused()` set.

Implications for euchre:

- The cards in the spike have `on_press`, so the tree reports `role: "button"`, but they report **`focusable: false`**. Tab does not reach them, and arrow keys go to `on_key` at stage 3. That is the behaviour wanted for a card rack.
- If you make cards focusable (`.focusable(true)`, `el/focus.rs:99`), a focused card takes Space and Return at stage 5, before `on_key`.
- If a bid panel has a `.default_button()`, **Return presses it first** (stage 2). Do not mark a default button while the play rack expects Enter.
- Element-scoped keys: `.on_key(|s: &mut S, e, tx| -> bool)` on an element (`el/focus.rs:93`) sees keys while it or a child has focus, before navigation.
- Accessibility labels: `.label("Ace of spades")` (`el/a11y.rs:234`) and `.role(..)`.

---

## 8. (c) Background threads, tokio, and the rivetkit-client bridge

### 8.1 Rules

- **Thread ownership:**
  - Rune owns the **main thread**: `App::run()` blocks it.
  - Build the tokio runtime **yourself, before `run()`**, and keep it alive until `run()` returns.
  - Do not use `#[tokio::main]`. It would park the main thread inside tokio.
- **How to talk to the UI thread:**
  - Producers talk to the UI only through `AppHandle::update` / `update_if`. Each closure is `Send + 'static`, runs on the UI thread in call order, and commits on the next frame with whatever motion its `Tx` asks for.
  - Capture only `Send` data in the closure (cloned `String`s, `serde_json::Value`s).
  - The UI talks to producers through a channel whose sender lives in state, wired with `App::init(closure)` (`AppSpec::init` cannot capture). `UnboundedSender::send` is non-blocking and fine to call from a handler.
- **Where producers start:**
  - Start live producers in **`on_ready`**. Plain headless runs never call it, so `--snap/--tree/--script` stay deterministic and offline.
  - `--realtime` headless runs do call it, which is how `live.png` was made.
  - Guard it with a once flag, because it runs once **per native window creation**.
- **Headless runs:** handle updates queued before `run()` are applied before the first frame (`app_handle_updates_reach_a_headless_run`).
- **Tests:** a test `Runtime` does **not** receive `AppHandle` updates. Drive the same closures with `rt.update(|s, tx| ..)` (`runtime/frame.rs:128`).

### 8.2 `main.rs` - VERIFIED (`live.png`, `live-click.png`)

```rust
fn main() {
    rune::prewarm_gpu();                                   // window/GPU warm-up on a side thread (no-op headless)
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).enable_all().thread_name("spike-tokio").build().expect("tokio runtime");
    let tokio = tokio_rt.handle().clone();

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();
    let mut cmd_rx = Some(cmd_rx);
    let app = App::new(spec()).init(move || State { net: Some(cmd_tx), ..State::new() });
    let handle: AppHandle<State> = app.handle();

    let mut started = false;
    app.on_ready(move |_window| {
        if std::mem::replace(&mut started, true) { return; }
        // (1) a plain std::thread producer
        let h = handle.clone();
        std::thread::Builder::new().name("spike-ticker".into()).spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            h.update(|s, _tx| s.ticks += 1);
        }).expect("ticker thread");
        // (2) the network task: receives UI commands, answers through the handle
        if let Some(mut rx) = cmd_rx.take() {
            let h = handle.clone();
            tokio.spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    let Cmd::Play(id) = cmd;
                    h.update(move |s, _| s.remote = format!("server ack play {}", rune_spike::CARDS[id].0));
                }
            });
        }
        // (3) a server push that animates: the Tx in update() takes motion like a handler's
        let h = handle.clone();
        tokio.spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            h.update(|s, tx| { s.remote = "seat 2 played QD".into(); tx.motion(rune_spike::FLIGHT); s.play(2); });
        });
    })
    .run();
    drop(tokio_rt);                                         // after the event loop returns
}
```

(Abbreviated from the Appendix file: the 50 ms simulated latency is elided.)

### 8.3 rivetkit-client 3.1.0 facts (read from the crate source, then compiled against)

| Fact | Source |
|---|---|
| `ClientConfig::new(endpoint)` defaults to **`EncodingKind::Bare`** and **`TransportKind::WebSocket`** | `client.rs:66` |
| Builders: `.token(..)`, `.token_opt(..)`, `.namespace(..)`, `.encoding(EncodingKind::{Json,Cbor,Bare})`, `.transport(TransportKind::{WebSocket,Sse})`, `.headers(HashMap)` | `client.rs:80-117`, `common.rs:41-51` |
| `Client::new(config) -> Client` is **infallible**. The crate README's `?` is stale. | `client.rs:221` |
| `client.get_or_create(name, key: Vec<String>, GetOrCreateOptions { params, create_in_region, create_with_input, pool_name }) -> anyhow::Result<ActorHandle>`; also `get(..)` | `client.rs:273-299`, `common.rs:76` |
| `handle.action(name, Vec<Value>)` works on the handle **without** a connection (stateless) | `handle.rs:88` |
| `handle.connect() -> ActorConnection` (= `Arc<ActorConnectionInner>`) calls `tokio::spawn` internally: it **panics outside a tokio runtime context** (verified by test) | `handle.rs:395`, `connection.rs:692` |
| `conn.on_event(name, Fn(&Vec<Value>) + Send + Sync).await -> SubscriptionHandle`; `on_event_raw` | `connection.rs:548` |
| `SubscriptionHandle` has **no Drop**: dropping it does not unsubscribe; call `.unsubscribe().await` | `connection.rs:41-75` |
| `conn.action(name, Vec<Value>).await -> anyhow::Result<Value>` | `connection.rs:411` |
| `conn.on_status_change(Fn(ConnectionStatus)).await`; `conn.status_receiver() -> watch::Receiver<ConnectionStatus>`; `conn.conn_status()`; `ConnectionStatus::{Idle, Connecting, Connected, Disconnected}` (Copy + Debug) | `connection.rs:95, 638-652` |
| `conn.disconnect().await` | `connection.rs:656` |
| **Dropping the `Client` broadcasts shutdown to every connection it made.** Keep it alive for the session. | `client.rs:358` |

### 8.4 The bridge - VERIFIED (compiles with rune; 2 tests)

```rust
pub fn client(endpoint: &str) -> Client {
    Client::new(ClientConfig::new(endpoint).transport(TransportKind::WebSocket).encoding(EncodingKind::Json))
}

/// Runs on the tokio runtime for the whole session.
pub async fn run_bridge(client: Client, table_id: String, handle: AppHandle<State>,
                        mut cmds: UnboundedReceiver<Cmd>) -> anyhow::Result<()> {
    let actor = client.get_or_create("table", vec![table_id], GetOrCreateOptions::default())?;
    let conn = actor.connect(); // tokio::spawn inside: must run within a runtime context

    let h = handle.clone();
    conn.on_status_change(move |status| h.update(move |s, _| s.conn = Some(status))).await;

    let h = handle.clone();
    let snapshots = conn.on_event("snapshot", move |args: &Vec<Value>| {
        let v = args.first().cloned().unwrap_or(Value::Null);
        h.update(move |s, tx| { tx.motion(FLIGHT); s.snapshot = Some(v); });
    }).await;

    while let Some(cmd) = cmds.recv().await {
        let Cmd::Play { card } = cmd;
        if let Err(e) = conn.action("playCard", vec![json!({ "card": card })]).await {
            let msg = e.to_string();
            handle.update(move |s, _| s.error = Some(msg));
        }
    }
    snapshots.unsubscribe().await; // dropping a SubscriptionHandle does NOT unsubscribe
    conn.disconnect().await;
    drop(client);
    Ok(())
}
```

Wire it from `on_ready`: `tokio.spawn(run_bridge(client(&url), table_id, handle.clone(), cmd_rx))`. <!-- critic: for the real table use `client.get("euchreTable", vec!["table".into(), game_id], GetOptions { params: Some(json!({"token": token})) })` (or `get_for_id` per 05 section 11.4, item 7), never `get_or_create`, because the TS client connects with `noCreate`. Per 08 G26 never `clone()` the `Client` anywhere (dropping any clone kills reconnection); hold one `Arc<Client>` for the session. Bound every `conn.action` with `tokio::time::timeout` (08 G29). --> Event names, action names and payloads come from `02-protocol.md` and `03-table-actor.md`; `"snapshot"` and `"playCard"` above are placeholders.

- To call `connect()` from a non-async thread, enter the runtime first: `let _g = rt.handle().enter();`. This is VERIFIED by `connect_inside_a_runtime_handle_enter_works`.
- `on_event` callbacks run on tokio threads. Never touch UI state there; always go through `handle.update`.
- To apply server events with the right motion:
  - A play event uses `tx.motion(FLIGHT)` (an animated commit).
  - A full resync or reconnect snapshot uses `tx.instant()`, so nothing flies.

---

## 9. (e) Felt and wood rail - VERIFIED

Colours are **linear sRGB**. `Color { r, g, b, a }` literals from hex digits are wrong. Use `rune::color::hex("#rrggbb") -> Option<Color>` (`color.rs:78`); `oklch` also exists.

```rust
fn c(h: &str) -> Color { hex(h).unwrap_or(Color::BLACK) }

// Felt: a lighter centre to a dark edge, clipped to its radius.
let felt = col().key("felt").w_full().h_full().radius(22.0).fill(c("#1b3f30"))
    .radial_gradient([0.5, 0.42], [560.0, 380.0], [(0.0, c("#3a7a5a")), (0.55, c("#285641")), (1.0, c("#163626"))])
    .padding(24.0).gap(16.0)
    .child(status).child(opp).child(col().grow(1.0).center().child(trick)).child(hand);

// Wood rail: a rounded frame with a vertical wood gradient and the felt inset.
let rail = col().key("rail").w_full().h_full().radius(34.0).fill(c("#6b3f1f"))
    .linear_gradient(std::f32::consts::PI, [(0.0, c("#8a5a32")), (0.5, c("#6b3f1f")), (1.0, c("#4a2a12"))])
    .border(1.0, c("#2e1a0b")).shadow(10.0, 30.0, Color::BLACK.alpha(0.5)).padding(14.0).child(felt);

col().w_full().h_full().fill(c("#14110e")).padding(20.0).child(rail)
```

- `.radial_gradient(centre [x,y] normalised 0..1, radius: f32 or [rx, ry] in points, stops)` (`el/visuals.rs:164`) paints **over** the fill, clipped to the radius. At most 16 stops.
- `.linear_gradient(angle_rad, stops)` (`:190`) uses the CSS angle convention: `PI` is top to bottom.
- `.gradient(top, bottom)` is a two-stop shortcut.
- The original palette (06-ui-spec section 1.2) is felt `#285641`, felt border `#1b3f30`, top `#44684e`, ink `#fffdf3`, radius 18. <!-- critic: those are the base `.felt` rules in TableStage.svelte:500-511, but TableStage.svelte:1336-1342 overrides them on the live table (`background: transparent; border: 0; border-radius: 0; box-shadow: none`). The shipped surface is the `.table-layout` background `#17452f` (play/[gameId]/+page.svelte:904) plus the decorative `#245a49 -> #10382c` felt canvas (06 sections 1.2 and 16). The spike colours above are illustrative, not the shipped palette. -->

---

## 10. Fonts

```rust
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
];
// AppSpec { fonts: FONTS, families: ("Inter", "IBM Plex Mono"), .. }
let label = TextStyle::new(15.0, 600.0);                                          // size pt, weight
let mono = TextStyle::new(13.0, 500.0).family(Family::Mono).features("'tnum' 1"); // tabular digits
text("Spike table").font(label).color(ink)
```

| Fact | Detail |
|---|---|
| A non-empty `AppSpec::fonts` **replaces** the built-in Inter faces (native and headless). | `text.rs:286-289`. List every weight you use; a missing weight is synthesised from the nearest. |
| The text system is built with `system_fonts: false` | Installed fonts are not discovered implicitly. |
| `families: (sans, mono)` names the two slots | The default is `("Inter", "Inter")`, so **`Family::Mono` renders Inter** unless you set the mono slot. |
| `Family::{Sans, Mono, Named("DM Sans"), SystemMono}` (`tok.rs:26`) | `Named` takes a registered **or installed** family (loaded on first use) and falls back to sans. It accepts a CSS list. `SystemMono` = SF Mono on macOS, never bundled. |
| `TextStyle::new(size, weight).family(..).features("'tnum' 1")` (`tok.rs:63`) | OpenType features use CSS syntax. |
| Verified | IBM Plex Mono rendered the status text (`status.png`, `--crop "#0/rail/felt/#0"`) |

**Euchre:** the original's app font is **DM Sans** (`src/app.css:6`), loaded from the web. Bundle `DMSans-{Regular,Medium,SemiBold,Bold}.ttf` (OFL; the files are not in either repo) with `families: ("DM Sans", "<mono>")`. <!-- critic: the app also uses `--font-serif: 'Source Serif 4'` (src/app.css:7) for headings, the score numerals and every dark scrapbook page (06 sections 1.1 and 1.3). Bundle Source Serif 4 (OFL, weights 400/500/600/700 + italic 400) too and select it with `Family::Named("Source Serif 4")`; the mono slot is barely used by the web app. --> Snapshots then match across machines because headless uses the same bundled faces.

---

## 11. Window size, minimum size, responsive layout

```rust
pub const WINDOW: (f32, f32) = (960.0, 640.0);
AppSpec { min_size: Some((720.0, 520.0)), ..AppSpec::new("Euchre spike", WINDOW, State::new, view) }
```

**`WindowOptions`** (`window/spec.rs:72`, FROM SOURCE) is set via `App::window_options(..)`:

- `transparent`, `borderless`, `click_through`, `always_on_top`, `visible`.
- `position: Option<(i32, i32)>` (physical).
- `physical_size: Option<(u32, u32)>`: overrides `AppSpec::size`.
- `min_size: Option<(f32, f32)>`: `None` inherits `AppSpec::min_size`; `Some((0, 0))` removes the floor.
- `max_size: Option<(f32, f32)>` and `resizable: bool`.
- Use `..WindowOptions::default()`.

Headless runs **do not enforce** `min_size`. Test small sizes with `--size WxH`.

**`radius()` does not clip children, and overflow is silent.** At 720x520 with fixed card sizes, the hand row spilled past the felt and rail (`min-size2.png`), and the tree's `warnings` was empty.

**Responsive scale** (VERIFIED, `min-size3.png`):

- `Cx` has **no viewport accessor**. Read a keyed element's laid-out size instead.
- `cx.layout_size("felt") -> Option<[f32; 2]>` (`runtime/layout_read.rs:272`) re-runs the view when that size changes.
- On the very first frame it is `None`, but the commit re-runs the view within the same frame. A `--trace` at 720x520 showed the hand cards already at the fitted 97 pt height on frame 0, with "animating frames 0", so there is no startup jiggle.

```rust
/// Card scale for the felt's size: fixed rows take 202 pt, the card rows 360 pt at full size.
pub fn card_scale(felt: Option<[f32; 2]>) -> f32 {
    felt.map(|[w, h]| ((h - 202.0) / 360.0).min(w / 892.0).clamp(0.5, 1.0)).unwrap_or(1.0)
}
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = card_scale(cx.layout_size("felt"));
    // ... every card/row size multiplies by k
}
```

**Alternatives** (FROM SOURCE):

- `.on_layout(|s: &mut S, [x, y, w, h]| ..)` (`el/layout_watch.rs:36`) stores the rect in state.
- `.aspect_fit(ratio)` (`el/layout.rs:62`) gives the largest box at a ratio that fits a cell.
- Live resize animates on each element's `.motion()` spring; the test `set_viewport_resizes_and_the_cards_rescale` uses `rt.set_viewport(..)`.

---

## 12. rune-kit (themed controls) - VERIFIED

```rust
let kit = Kit::from_palette(PaletteSeed { bg: c("#285641"), fg: ink, accent: c("#ffd75e") }); // or Kit::neutral(dark)
Button::new("Reset")
    .variant(ButtonVariant::Contrast)
    .on_press(|s: &mut State| s.reset())
    .build(&kit)           // -> El
    .key("reset")          // El methods after build
```

`rune_kit::root(body, layer)` wraps a page with kit layers (FROM SOURCE). Building a `Kit` per view run is cheap enough for the spike. For the real app, build it once and keep it in state or a `thread_local`.

---

## 13. Testing with `rune::Runtime` - VERIFIED

### 13.1 Two ways to get a runtime

| | `Runtime::new(state, view, viewport, Env::new(dark, scale))` (`runtime.rs:820`) | `App::new(spec()).headless_runtime(viewport, Env)` (`playground/testing.rs:13`) |
|---|---|---|
| Wires | the view only | the whole `AppSpec`: `on_key`, `poll`, `setup` (no-op waker), fonts, families |
| Fonts | built-in Inter | your `AppSpec::fonts` |
| Add hooks by hand | `rt.on_key = Some(on_key)`; `rt.poll = Some(..)` | - |
| Use for | pure view/motion tests | keyboard, setup, font-dependent layout |

### 13.2 API used

| Call | Meaning |
|---|---|
| `rt.frame(dt_seconds)` (`runtime/frame.rs:305`) | advance the virtual clock and render one frame. **Handlers queued by input run before the next frame:** always `rt.frame(DT)` after input before asserting. `rt.frame(0.0)` first to lay out. |
| `rt.find(key) -> Option<(x, y)>` (`runtime/elements.rs:385`) | centre of the element as drawn (feed it to the pointer) |
| `rt.presented(key) -> Option<([x, y, w, h], opacity)>` (`:402`) | the rect **drawn this frame**, mid-spring included |
| `rt.pointer_down(x, y)` / `rt.pointer_up(x, y)` (`runtime/input/pointer.rs:177/299`) | a click |
| `rt.key(KeyEvent { key, mods: Mods::default() })` (`runtime/input/keys.rs:261`) | a key through the full routing |
| `rt.update(|s, tx| ..)` (`runtime/frame.rs:128`) | what the app loop does with each queued `AppHandle` update |
| `rt.set_viewport((w, h), scale)` (`runtime/frame.rs:11`) | resize |
| `rt.set_system_reduced_motion(bool)` | the OS preference |
| `rt.state`, `rt.animating`, `rt.on_key`, `rt.poll`, `rt.viewport`, `rt.t` | public fields |

### 13.3 Recipes (excerpts; full file in the Appendix)

```rust
const DT: f64 = 1.0 / 60.0;
fn settle(rt: &mut Runtime<State>, seconds: f64) { for _ in 0..(seconds / DT).ceil() as usize { rt.frame(DT); } }
fn press(rt: &mut Runtime<State>, key: Key) { rt.key(KeyEvent { key, mods: Mods::default() }); rt.frame(DT); }

#[test]
fn pressing_a_hand_card_flies_it_to_the_trick() {
    let mut rt = Runtime::new(State::new(), view, WINDOW, Env::new(true, 2.0));
    rt.frame(0.0);
    let (hand_rect, _) = rt.presented("hand-2").expect("hand card drawn");
    let trick = rt.presented("trick").expect("trick area").0;
    let (x, y) = rt.find("hand-2").expect("hand-2 has a centre");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT); // handlers run between frames: this frame commits the press
    assert_eq!(rt.state.trick, vec![2]);
    settle(&mut rt, 0.1);
    let mid = rt.presented("trick-2").expect("trick card").0;
    assert!(mid[1] < hand_rect[1] - 1.0 && mid[1] > trick[1] + 16.0 + 1.0); // in flight
    assert!(rt.animating);
    settle(&mut rt, 1.5);
    assert!(!rt.animating);
}

#[test]
fn app_handle_updates_reach_a_headless_run() {
    // Integration: a real headless App run, driven by args (no process args involved).
    let dir = std::env::temp_dir().join(format!("rune-spike-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tree = dir.join("tree.json");
    let app = App::new(spec());
    let handle = app.handle();
    std::thread::spawn(move || handle.update(|s, _| s.remote = "from a thread".into())).join().unwrap();
    app.args(vec!["--tree".into(), tree.to_str().unwrap().into()]).run();
    assert!(std::fs::read_to_string(&tree).unwrap().contains("from a thread"));
    std::fs::remove_dir_all(&dir).unwrap();
}
```

- Tests run without a window or GPU, in about 0.3 s for 10 tests.
- `App::args([...]).run()` inside a test performs a real headless render (`--tree`, `--snap`) and returns. This is the way to assert on tree JSON or pixels from `cargo test`.

---

## 14. (f) Headless tooling: scenes, scripts, snapshots, tree, trace

Any binary built on `App::run()` accepts these flags (`tooling/args.rs:241` prints the full help with `--help`). Through cargo: `cargo run -p euchre-desktop -- --snap out/table.png`. Every headless run prints one stats line to stderr, for example `rune: animating frames 60 (cpu p50 0.342 p95 2.431 ms), at-rest frames 18 ..., layout nodes 19, glyphs 38`.

### 14.1 Flags

| Flag | Meaning |
|---|---|
| `--snap out.png` | render offscreen (no window) at `--at` |
| `--at SECONDS` / `--at mark:NAME` / `--at mark:NAME+0.5` / `mark:NAME-0.05` | virtual time of the snapshot/tree. Default: end of frames or script (the script's `end` mark, else 1 s after its last event). |
| `--scene NAME` / `--list-scenes` | start from an `AppSpec::scenes` entry / list them |
| `--size WxH` | viewport in points (default `AppSpec::size`) |
| `--scale N` | device scale (default **2**). Use `--scale 1` to see aliasing honestly. |
| `--theme dark\|light` | system appearance |
| `--crop KEY` (or a path like `"#0/rail/felt/#0"`) | crop `--snap` to that element; `#N` is the Nth unkeyed child |
| `--script FILE.json` | replay timestamped input (14.2) |
| `--frames N --fps 60 --start S --out DIR` | dump N frames from S seconds |
| `--strip out.png --crop x,y,w,h --every K --cols N` | tile dumped frames into one image (repeat `--crop` to stack regions) |
| `--video out.mp4` (`.mov` ProRes, `.webm`) `--duration S\|auto` | stream frames into **ffmpeg (must be on PATH; it is not installed here, so this was unverified)** |
| `--keys [CORNER]`, `--keys-theme`, `--pointer` | draw keycaps / press ripples on dumped frames |
| `--tree out.json` | element tree with animated values (14.3) |
| `--trace out.jsonl`, `--trace-all` | per-frame rects/opacity (14.4); `--trace-all` includes unkeyed elements |
| `--native` | trace the native window's clock instead of the virtual one |
| `--realtime` | hold each frame until the wall clock reaches it, print `rune-mark NAME T`, and **run `on_ready` after the first frame** (live producers / a live server) |
| `--html out.html [--html-widths 360,640,1000] [--html-template F]` | static HTML/CSS of the page at rest |
| `--first-pixels`, `--chrome`, `--window KEY`, `--list-windows`, `--desktop BACKDROP.png`, `--playground [NAME]`, `--variant`, `--whole-window`, `--scroll-bars MODE` | other tooling |
| `--hot`, `--snap-on-patch`, `--hot-control` | need the `hot` feature |
| anything else | passed to the app (`rune::tooling::extra_args`) |

**Exact commands used** (from the spike directory, `B=../spike-target/debug/rune-spike`), all VERIFIED:

```sh
$B --snap out/table.png
$B --snap out/table-noclip.png --crop hand
$B --script scripts/click.json --snap out/click-mid.png --at 0.45
$B --script scripts/click.json --snap out/click-settled.png --at mark:played+1.0
$B --script scripts/keys.json --snap out/keys.png --at 1.8
$B --realtime --snap out/live.png --at 1.5                 # on_ready producers ran: "ticks 14 | seat 2 played QD"
$B --snap out/nolive.png --at 1.5                          # same time, no producers: deterministic
$B --realtime --script scripts/click-live.json --snap out/live-click.png --at mark:clicked+0.8
$B --list-scenes                                           # table, cursor, quality, played
$B --scene played --snap out/scene-played.png --size 720x520 --scale 1
$B --scene quality --snap out/quality.png --crop trick
$B --scene played --size 720x520 --scale 1 --snap out/min-size3.png
$B --snap out/status.png --crop "#0/rail/felt/#0"
$B --tree out/tree.json
$B --script scripts/click.json --strip out/strip.png --crop 180,170,600,420 --start 0.3 --frames 30 --every 3 --cols 5 --scale 1
$B --script scripts/click.json --trace out/trace.jsonl --frames 40
$B --size 720x520 --trace out/trace-min.jsonl --frames 6
$B --script scripts/opp.json --snap out/opp-mid.png --at mark:opp+0.12
$B --script scripts/opp.json --snap out/opp-trick.png --at mark:collect-0.05
$B --script scripts/opp.json --snap out/collect-mid.png --at mark:collect+0.12
$B --script scripts/opp.json --snap out/collect-settled.png --at mark:collect+1.2
```

### 14.2 Script JSON

The top level is either `{"events": [...]}` or a bare array (both VERIFIED). Each event has `"t"` (seconds) and `"type"`.

| Event | Fields | Status |
|---|---|---|
| `click` / `tap` | `"key": "<element key>"` (its centre) or `"x"`, `"y"` (points); optional `"button": "secondary"\|"middle"\|"back"\|"forward"` | VERIFIED |
| `key` / `key-up` | `"name"`: `Enter, Backspace, Delete, Left, Right, Up, Down, Home, End, PageUp, PageDown, Escape, Tab, ContextMenu, Insert, PrintScreen, ScrollLock, Pause, F1..F24`, or **any single character** (`"o"` -> `Key::Text("o")`, `" "` for Space). Modifiers: `"shift"`, `"cmd"` (Super on macOS, Ctrl elsewhere), `"alt"`, `"ctrl"`, `"logo"` (booleans); `"repeat": true` | `Right/Left/Enter/o/c` VERIFIED |
| `mark` | `"name"` (target for `--at mark:NAME`) | VERIFIED |
| `move`, `down`, `up`, `double-click`, `triple-click`, `secondary-click`, `drag`, `wheel`, `scroll`, `fling`, `magnify`, `rotate`, `resize` | position as above | FROM SOURCE (`tooling/script.rs:113-221`) |
| `type` | `"text"`, `"every"` (s, default 0.035) | FROM SOURCE |
| `shortcut` | `"chord": "cmd+k"` (the registered global shortcut, never `on_key`) | FROM SOURCE |
| `paste`, `menu-pick` (`"id"`), `menu-cancel`, `file-dialog` (`"path"`), `file-dialog-cancel`; `{"focus": false}`; `{"hold": "alt"}` | | FROM SOURCE |

The spike's scripts:

```json
{"events": [
  {"t": 0.3, "type": "click", "key": "hand-2"},
  {"t": 0.3, "type": "mark", "name": "played"}
]}
```

```json
[
  {"t": 0.2, "type": "key", "name": "Right"},
  {"t": 0.4, "type": "key", "name": "Right"},
  {"t": 0.6, "type": "key", "name": "Left"},
  {"t": 0.8, "type": "key", "name": "Enter"},
  {"t": 0.8, "type": "mark", "name": "enter"}
]
```

### 14.3 `--tree` JSON (VERIFIED)

Top-level keys:

- `accessibility`
- `anim_t`
- `animating`
- `dark_system`
- `elements` (a flat array)
- `reduced_motion`
- `scale`
- `t`
- `time_scale`
- `title`
- `viewport`
- `warnings`

An element:

```json
{"blur": 0.0, "fill": "#ffffffff", "focusVisible": false, "focusable": false, "focused": false, "inert": false,
 "key": "hand-0", "loopPhase": null, "opacity": 1.0, "path": "#0/rail/felt/hand/hand-0",
 "rect": [206.0, 412.0, 100.0, 140.0], "role": "button", "scale": 1.0, "target": [206.0, 430.0, 100.0, 140.0]}
```

- `rect` is what is drawn, including `.offset()` and springs. `target` appears when it differs from layout: the cursor card is lifted -18.
- `fill` and `color` are sRGB `#rrggbbaa`. Text elements carry `"text"` and `"color"`.
- `path` joins keys, with `#N` for unkeyed nodes.

### 14.4 `--trace` JSONL (VERIFIED)

Line 1 is a header:

```json
{"clock":"virtual","height":640.0,"hz":60.0,"launch":"rune","platform":"rune","scale":2.0,"type":"header","units":"pt","width":960.0}
```

Each frame line looks like this:

```json
{"e": [[id, x, y, w, h, opacity, radius, flags], ...], "m": [{"cls":"rune","host":"rune","id":1,"nm":"#0/rail","role":"group"}, ...]}
```

`m` maps ids to names, and only lists ids not seen before. Other line types exist (a summary), so filter on `"e"`.

---

## 15. Gotchas (all observed or read at rev 85cd8c6)

| # | Gotcha | Fix / rule |
|---|---|---|
| 1 | `.clip_rounded(r)` on a card hid its own shadow and outline | Use `.radius(r)` only: it clips the element's picture and gradients |
| 2 | `.radius()` does **not** clip children; overflow is silent (no tree warning) | Size content to fit (section 11) or clip a container on purpose |
| 3 | `Color` is linear sRGB | `rune::color::hex("#..")` (returns `Option`) |
| 4 | Non-empty `AppSpec::fonts` replaces Inter; the default mono slot is Inter | Bundle every face; set `families` |
| 5 | `Pixels::set` has no mipmaps: downscaled cards alias (moire at `--scale 1`) | `ImageCache::load` for card faces |
| 6 | `ImageCache` decodes asynchronously; headless `setup` has a no-op waker | `img.wait()` when headless |
| 7 | `rune::is_headless()` reads the process args once; false under `cargo test` and with `App::args` | Wait unconditionally in tests, or decode synchronously |
| 8 | `on_ready` is not called in plain headless runs; it is in `--realtime` and windowed runs, once per window creation | Start network and timers there with a once-guard; snapshots stay offline |
| 9 | `AppHandle::update` never reaches a test `Runtime` | `rt.update(closure)` in tests; `App::args(..).run()` for end-to-end |
| 10 | `AppSpec::init` is a fn pointer (cannot capture) | `App::new(spec).init(move || ..)` |
| 11 | `AppHandle::update` closures must be `Send + 'static` (state need not be) | Clone `Value`/`String` into the closure |
| 12 | Handlers run between frames | `rt.frame(DT)` after input before asserting |
| 13 | `Runtime::new` wires only the view | `headless_runtime` or set `rt.on_key` / `rt.poll` |
| 14 | `rt.reduced_motion = true` is overwritten every commit | `rt.set_system_reduced_motion(true)` |
| 15 | A matched flight needs the leaving element to carry the id **in the previous view** | Put `opp-play-{seq}` on the top back ahead of time; monotonic ids |
| 16 | Duplicate matched ids: the last in tree order wins; ids are per window | Never reuse an id while its old element can still be in the tree |
| 17 | The flight draws the **new** element's picture (a back -> face flight shows the face throughout) and carries radius/fill/opacity, not rotation | Design flights accordingly |
| 18 | Removing elements in an animated commit fades them over 0.15 s (default transition) | `tx.instant()` / `tx.settle()` for snap removal and resync snapshots |
| 19 | Under Reduce Motion nothing is matched | Expected; the state logic keeps the hold timings |
| 20 | Return presses a `default_button()` before `on_key` | Do not set a default button while the rack expects Enter |
| 21 | Pressable elements report `role: "button"` but `focusable: false` | `.focusable(true)` + `.label(..)` if you want Tab/VoiceOver on cards (then Space and Return go to the card) |
| 22 | `Cx` has no viewport size | `cx.layout_size("felt")`; resolved within frame 0 |
| 23 | Headless ignores `min_size` | Test with `--size 720x520` |
| 24 | `--video` needs ffmpeg | `brew install ffmpeg` on dev machines, or use `--strip` / `--out` frames |
| 25 | `--hot*` flags exit unless the `hot` feature is on | Optional dev feature |
| 26 | Clippy 1.99: `chunks_exact(N)` -> `as_chunks::<N>()`; the inline scenes type trips `type_complexity` | `type Scene = (&'static str, fn(&mut State));` |
| 27 | rivetkit-client: `connect()` panics outside a tokio context | Call it inside a task or after `handle.enter()` |
| 28 | rivetkit-client: dropping `Client` kills every connection; `SubscriptionHandle` drop does not unsubscribe | Keep the `Client` in the bridge task; `unsubscribe().await` |
| 29 | rivetkit-client: the default encoding is `Bare`; the README's `Client::new(..)?` is stale; the variant is `TransportKind::WebSocket` (not `Ws`) | Set `.encoding(EncodingKind::Json)` to match the server |
| 30 | Full debug info: 18-23 GB RAM building rune tests | `debug = "line-tables-only"`, deps `debug = false` (1.4) |
| 31 | Fresh lock drift (54 packages) | Verified harmless; `cargo update -p taffy --precise 0.15.0` if needed |
| 32 | `#[tokio::main]` would own the main thread | Build the runtime by hand; Rune runs on main |
| 33 | `App::run()` panics without a display (CI) | `try_run()` or only use headless flags in CI |

---

## 16. Proposed `apps/desktop` layout

```
apps/desktop/
  Cargo.toml            # [lib] euchre_desktop + [[bin]] euchre; rune, rune-kit, rivetkit-client, tokio, serde_json, anyhow
  assets/cards/*.png    # 24 faces + back.png, copied from euchre/static/art/cards (CC0, LICENSE.txt alongside) <!-- critic: ARCHITECTURE "Assets" wins: embed from apps/web/static/art/cards/*.png via include_bytes! (one copy in the monorepo), not a second copy under apps/desktop/assets -->
  assets/fonts/*.ttf    # DM Sans + Source Serif 4 (both OFL) + a mono face <!-- critic: added Source Serif 4, see section 10 -->
  scripts/*.json        # headless scripts: play-card.json, bid.json, collect.json, reconnect.json
  src/main.rs           # tokio runtime, App::new(spec()).init(..).on_ready(start producers).run()
  src/lib.rs            # pub fn spec() -> AppSpec<State>; pub mod ...
  src/state.rs          # State: protocol view (from euchre-protocol) + local UI (cursor, flights, conn, error)
  src/art.rs            # include_bytes! card table, setup(): ImageCache + headless wait
  src/theme.rs          # hex() palette, TextStyles, FONTS, Kit
  src/keys.rs           # on_key
  src/net.rs            # Cmd enum, client(), run_bridge() (rivetkit-client -> AppHandle)
  src/scenes.rs         # SCENES: lobby, bidding, mid-trick, trick-won, hand-over, reconnect, small-window
  src/view/{mod,table,seats,hand,trick,scoreboard,bid,settings}.rs
  tests/*.rs            # Runtime tests (motion, keys, layout at min size, reduce motion, bridge mapping)
```

- **Lib plus bin:** tests and `headless_runtime` need `spec()`, `view` and `State` from a library.
- **Keep `src/view` pure:** `fn(&State, &mut Cx<State>) -> El`. All I/O goes to `net.rs` and arrives through `AppHandle::update`.
- **Scenes are the visual regression harness:** run `--scene X --snap` per scene in CI, plus `cargo test`.

---

## 17. Source index (rev 85cd8c6, `crates/rune/src/`)

| Topic | Location |
|---|---|
| AppSpec | `tooling.rs:17`; runtime wiring of setup/poll/on_key `tooling.rs:135` |
| App builder | `app.rs` 196 `window_options`, 205 `fit_content_height`, 340 `init`, 348 `args`, 354 `windowless`, 362 `keep_running`, 506 `on_ready`, 545 `handle`, 578 `run`, 585 `try_run`, 750 `update`, 762 `update_if` |
| Headless flags / help / `is_headless` | `tooling/args.rs:127-200`, `:236`, `:241`; realtime `tooling/headless.rs:76` |
| Script events / key names | `tooling/script.rs:113-232`; `tooling/key_names.rs:4` |
| Images | `image.rs:54-64` (`Image`), `:183` load, `:196` svg, `:208` load_mime, `:218` file, `:234` rgba; `image/pixels.rs:9` atlas size, `:90` `Pixels::set` |
| Visuals | `el/visuals.rs:139` shadow, `:164` radial_gradient, `:190` linear_gradient, `:215` pixels, `:223` pixels_fit, `:231` outline, `:364` scale, `:381` rotate, `:399` origin, `:453` clip_rounded |
| Motion | `el/motion.rs:89` motion, `:113` offset, `:132` raise, `:196` transition, `:215` matched, `:259` sequence; `el/tx.rs:226` wake_after, `:513` instant, `:527` motion, `:553` settle |
| Focus / keys / a11y | `el/focus.rs:93` on_key, `:99` focusable, `:177` default_button; `el/a11y.rs:234` label; rune `README.md:246` key routing |
| Layout reads | `runtime/layout_read.rs:239` layout, `:272` layout_size; `el/layout_watch.rs:36` on_layout; `el/layout.rs:62` aspect_fit |
| Runtime (tests) | `runtime.rs:820` new; `runtime/frame.rs:11` set_viewport, `:128` update, `:305` frame; `runtime/elements.rs:385` find, `:402` presented; `runtime/input/pointer.rs:177/299`; `runtime/input/keys.rs:261`; `runtime/system_settings.rs:65`; `playground/testing.rs:13` headless_runtime |
| Text | `tok.rs:26` Family, `:63` TextStyle; `text.rs:286-289` fonts replace defaults |
| Colour | `color.rs:78` hex |
| Window | `window/spec.rs:69` default min size, `:72` WindowOptions |
| GPU warm-up | `render.rs:660` prewarm_gpu |

rivetkit-client 3.1.0 (`~/.cargo/registry/src/*/rivetkit-client-3.1.0/src/`):

- `client.rs` 20/25 options, 66 `ClientConfig::new`, 221 `Client::new`, 273 `get`, 299 `get_or_create`, 358 `Drop`.
- `handle.rs` 88 stateless `action`, 395 `connect`.
- `connection.rs` 41 `SubscriptionHandle`, 95 `ConnectionStatus`, 411 `action`, 548 `on_event`, 638 `on_status_change`, 652 `status_receiver`, 656 `disconnect`, 692 `tokio::spawn`.
- `common.rs` 41 `TransportKind`, 47 `EncodingKind`, 76 `ActorKey`.

---

## Appendix A - verified spike files (verbatim)

These compiled with the pinned rune at edition 2024, passed `cargo clippy --all-targets -- -D warnings`, and passed every test. The path dependencies were used only because the spike built against a local checkout; the port uses the git lines in section 1.1.

### A.1 `Cargo.toml`

```toml
[package]
name = "rune-spike"
version = "0.0.0"
edition = "2024"
publish = false

# Standalone: not a member of the rune workspace.
[workspace]

[lib]
path = "src/lib.rs"

[[bin]]
name = "rune-spike"
path = "src/main.rs"

[dependencies]
# Spike uses the local clone; the port uses the git line:
# rune = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
rune = { path = "/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rune-src/rune/crates/rune" }
rune-kit = { path = "/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rune-src/rune/crates/rune-kit" }
png = "0.18"
tokio = { version = "1", features = ["rt-multi-thread", "time", "sync", "macros"] }

# Copied from the rune workspace: full debug info made a rune test build use 18-23 GB of RAM.
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = false
```

### A.2 `src/lib.rs`

```rust
//! Rune spike for the euchre desktop port: card images, hand -> trick shared-element motion,
//! background-thread updates, arrow/Enter keys, felt + wood rail, headless scripts.

use rune::color::hex;
use rune_kit::{Button, ButtonVariant, Kit, PaletteSeed};
use rune::tok::{Family, TextStyle};
use rune::{
    AppSpec, Color, Cx, El, Image, ImageCache, Key, KeyEvent, Pixels, Spring, SurfaceFit, Tx, Waker, col, el, row,
    text,
};

/// Card faces, embedded at compile time (paths are relative to this file).
pub const CARDS: [(&str, &[u8]); 5] = [
    ("AS", include_bytes!("../assets/cards/AS.png")),
    ("KH", include_bytes!("../assets/cards/KH.png")),
    ("QD", include_bytes!("../assets/cards/QD.png")),
    ("JC", include_bytes!("../assets/cards/JC.png")),
    ("TS", include_bytes!("../assets/cards/TS.png")),
];
pub const BACK: &[u8] = include_bytes!("../assets/cards/back.png");

/// 512x716 art -> 100 x 140 points (aspect 0.715).
pub const CARD_W: f32 = 100.0;
pub const CARD_H: f32 = 140.0;
pub const WINDOW: (f32, f32) = (960.0, 640.0);

/// The card flight spring (shared-element + reflow).
pub const FLIGHT: Spring = Spring::new(0.45, 0.12);

/// UI -> network commands (the rivetkit-client task owns the receiver).
#[derive(Debug, Clone)]
pub enum Cmd {
    Play(usize),
}

pub struct State {
    /// Sender into the tokio network task; `None` in tests and plain headless runs.
    pub net: Option<tokio::sync::mpsc::UnboundedSender<Cmd>>,
    /// Synchronously decoded faces (png crate -> Pixels::set): deterministic, no mips.
    pub faces: Vec<Pixels>,
    /// Card ids still in hand, left to right.
    pub hand: Vec<usize>,
    /// Card ids played to the centre, in play order.
    pub trick: Vec<usize>,
    /// Keyboard cursor: index into `hand`.
    pub cursor: usize,
    /// Written by background producers (std::thread / tokio) through AppHandle::update.
    pub ticks: u32,
    pub remote: String,
    /// The cache-loaded card back (ImageCache path: async decode, mips). Set in `setup`.
    pub cache: Option<ImageCache>,
    pub back: Option<Image>,
    /// The same faces through the ImageCache (mipmapped), for the quality comparison scene.
    pub cached: Vec<Image>,
    /// KH.svg rasterized by the cache at its drawn size (vector source, crisp at any size).
    pub svg_small: Option<Image>,
    pub quality: bool,
    /// Seat 2's face-down cards left in its fan.
    pub opp_left: usize,
    /// Seat 2's plays this trick: (play sequence number, face index).
    pub opp_played: Vec<(usize, usize)>,
    /// Monotonic count of seat 2's plays; names the matched id of its next card.
    pub opp_seq: usize,
    /// The winner's stack: (matched id the card flew in on, face index).
    pub won: Vec<(String, usize)>,
}

impl State {
    pub fn new() -> State {
        State {
            net: None,
            faces: CARDS.iter().map(|(_, bytes)| decode_png(bytes)).collect(),
            hand: (0..CARDS.len()).collect(),
            trick: Vec::new(),
            cursor: 0,
            ticks: 0,
            remote: "waiting for server".into(),
            cache: None,
            back: None,
            cached: Vec::new(),
            svg_small: None,
            quality: false,
            opp_left: 5,
            opp_played: Vec::new(),
            opp_seq: 0,
            won: Vec::new(),
        }
    }

    /// Moves card `id` from the hand to the trick. One commit removes `hand-{id}` and mounts
    /// `trick-{id}`; both carry `matched("card-{id}")`, so the trick card flies from the hand.
    pub fn play(&mut self, id: usize) {
        let Some(at) = self.hand.iter().position(|&c| c == id) else { return };
        self.hand.remove(at);
        self.trick.push(id);
        self.cursor = self.cursor.min(self.hand.len().saturating_sub(1));
    }

    /// Seat 2 plays face `id`: its top back (matched `opp-play-{seq}`) leaves and the trick card
    /// mounts with the same matched id, so it flies out of the opponent's fan.
    pub fn opp_play(&mut self, id: usize) {
        if self.opp_left == 0 {
            return;
        }
        self.opp_left -= 1;
        self.opp_played.push((self.opp_seq, id));
        self.opp_seq += 1;
    }

    /// The trick goes to the winner's stack: every trick card leaves and a mini mounts in the
    /// stack with the same matched id, so each shrinks into the stack on one commit.
    pub fn collect(&mut self) {
        for id in self.trick.drain(..) {
            self.won.push((format!("card-{id}"), id));
        }
        for (seq, id) in self.opp_played.drain(..) {
            self.won.push((format!("opp-play-{seq}"), id));
        }
    }

    pub fn reset(&mut self) {
        self.hand = (0..CARDS.len()).collect();
        self.trick.clear();
        self.cursor = 0;
        self.opp_left = 5;
        self.opp_played.clear();
        self.won.clear();
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

/// PNG bytes -> straight-alpha sRGB RGBA8 in a `Pixels`.
pub fn decode_png(bytes: &[u8]) -> Pixels {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => unreachable!("normalize_to_color8 expands palettes"),
    };
    let pixels = Pixels::new();
    pixels.set(info.width, info.height, rgba);
    pixels
}

/// `AppSpec::setup`: runs once, before the first frame, with the waker the cache workers call.
pub fn setup(s: &mut State, waker: Waker) {
    let cache = ImageCache::new(waker);
    let back = cache.load(BACK);
    let cached: Vec<Image> = CARDS.iter().map(|(_, bytes)| cache.load(*bytes)).collect();
    // size_pt is the square the drawing fits (its long side here); scale = device px per point.
    let svg_small = cache.svg(&include_bytes!("../assets/cards/KH.svg")[..], 70.0, 2.0);
    // Headless renders run on a virtual clock: block so the first snapshot has the image.
    if rune::is_headless() {
        let _ = back.wait();
        for img in &cached {
            let _ = img.wait();
        }
        let _ = svg_small.wait();
    }
    s.svg_small = Some(svg_small);
    s.cached = cached;
    s.back = Some(back);
    s.cache = Some(cache);
}

/// Colours are LINEAR sRGB in rune; `hex` converts from the usual sRGB hex.
fn c(h: &str) -> Color {
    hex(h).unwrap_or(Color::BLACK)
}

/// A card face at `k` times its full size (k comes from the felt's laid-out size).
pub fn card(face: &Pixels, key: String, matched: String, k: f32) -> El {
    el().key(key)
        .matched(matched)
        .size(CARD_W * k, CARD_H * k)
        .radius(8.0)
        .fill(Color::WHITE)
        .pixels_fit(face, SurfaceFit::Fit)
}

/// Card scale for the felt's size: fixed rows take 202 pt, the card rows 360 pt at full size.
/// Cx has no viewport accessor; reading a keyed element's laid-out size re-runs the view when
/// it changes (the first frame reads None and lays out at full size).
pub fn card_scale(felt: Option<[f32; 2]>) -> f32 {
    felt.map(|[w, h]| ((h - 202.0) / 360.0).min(w / 892.0).clamp(0.5, 1.0)).unwrap_or(1.0)
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = card_scale(cx.layout_size("felt"));
    let ink = c("#fffdf3");
    let label = TextStyle::new(15.0, 600.0);
    let mono = TextStyle::new(13.0, 500.0).family(Family::Mono).features("'tnum' 1");
    let kit = Kit::from_palette(PaletteSeed { bg: c("#285641"), fg: ink, accent: c("#ffd75e") });

    // Status bar: background-producer output + the ImageCache card back.
    let back = match &s.back {
        Some(img) => el().key("back").size(36.0, 50.0).radius(4.0).pixels(img.pixels()),
        None => el().key("back").size(36.0, 50.0),
    };
    let status = row()
        .w_full()
        .center_y()
        .justify_between()
        .child(row().gap(12.0).center_y().child(back).child(text("Spike table").font(label).color(ink)))
        .child(
            row()
                .gap(16.0)
                .center_y()
                .child(text(format!("ticks {}  |  {}", s.ticks, s.remote)).key("status").font(mono).color(ink.alpha(0.8)))
                .child(
                    // The winner's stack: minis that the trick cards fly into on collect.
                    row().key("won").gap(2.0).children(s.won.iter().map(|(m, id)| {
                        el().key(format!("won-{m}"))
                            .matched(m.clone())
                            .size(24.0, 34.0)
                            .radius(3.0)
                            .fill(Color::WHITE)
                            .pixels_fit(&s.faces[*id], SurfaceFit::Fit)
                    })),
                )
                .child(
                    // rune-kit: a themed button from three seed colours.
                    Button::new("Reset")
                        .variant(ButtonVariant::Contrast)
                        .on_press(|s: &mut State| s.reset())
                        .build(&kit)
                        .key("reset"),
                ),
        );

    // Seat 2's fan: backs turned about their bottom centre. Only the top back carries the
    // matched id of seat 2's next play, so that one card is the one that flies.
    let n = s.opp_left;
    let opp = row().key("opp").gap(4.0).center().h(80.0 * k).children((0..n).map(|i| {
        let tilt = (i as f32 - (n as f32 - 1.0) / 2.0) * 0.09;
        let back = el().key(format!("opp-back-{i}")).size(50.0 * k, 70.0 * k).radius(5.0).fill(Color::WHITE).rotate(tilt).origin(0.5, 1.0);
        let back = match &s.back {
            Some(img) => back.pixels(img.pixels()),
            None => back,
        };
        if i + 1 == n { back.matched(format!("opp-play-{}", s.opp_seq)) } else { back }
    }));

    // Trick: the played cards, centred. Each mounts with the same matched id as its hand card.
    let trick = row()
        .key("trick")
        .size(5.0 * CARD_W * k + 4.0 * 12.0 + 32.0, CARD_H * k + 32.0)
        .padding(16.0)
        .gap(12.0)
        .center()
        .radius(16.0)
        .fill(Color::BLACK.alpha(0.12))
        .children(s.trick.iter().map(|&id| card(&s.faces[id], format!("trick-{id}"), format!("card-{id}"), k)))
        .children(s.opp_played.iter().map(|&(seq, id)| card(&s.faces[id], format!("trick-opp-{seq}"), format!("opp-play-{seq}"), k)));
    // Scene "quality": Pixels::set (no mips) vs ImageCache (mips) at 50x70 and 100x140.
    let trick = if s.quality && !s.cached.is_empty() {
        let small = |p: &Pixels, k: &str| el().key(k.to_string()).size(50.0, 70.0).radius(4.0).fill(Color::WHITE).pixels_fit(p, SurfaceFit::Fit);
        row().key("trick").gap(12.0).center().padding(16.0)
            .child(small(&s.faces[1], "q-raw-small"))
            .child(small(s.cached[1].pixels(), "q-mip-small"))
            .children(s.svg_small.as_ref().map(|i| small(i.pixels(), "q-svg-small")))
            .child(card(&s.faces[1], "q-raw".into(), "q-raw".into(), k))
            .child(card(s.cached[1].pixels(), "q-mip".into(), "q-mip".into(), k))
    } else {
        trick
    };

    // Hand: keyed cards; pressing one plays it. The cursor card lifts on an offset spring.
    let hand = row().key("hand").gap(12.0).center().h(CARD_H * k + 24.0).children(s.hand.iter().enumerate().map(
        |(i, &id)| {
            let on = i == s.cursor;
            card(&s.faces[id], format!("hand-{id}"), format!("card-{id}"), k)
                .motion(FLIGHT)
                .offset(0.0, if on { -18.0 } else { 0.0 })
                .outline(if on { 3.0 } else { 0.0 }, 2.0, c("#ffd75e"))
                .shadow(6.0, 14.0, Color::BLACK.alpha(0.35))
                .press_scale(0.97)
                .on_press(move |s: &mut State| {
                    s.play(id);
                    // Non-blocking send from the UI thread; the tokio task answers via AppHandle.
                    if let Some(net) = &s.net {
                        let _ = net.send(Cmd::Play(id));
                    }
                })
        },
    ));

    // Felt: radial gradient (lighter centre to dark edge), clipped to its radius.
    let felt = col()
        .key("felt")
        .w_full()
        .h_full()
        .radius(22.0)
        .fill(c("#1b3f30"))
        .radial_gradient([0.5, 0.42], [560.0, 380.0], [(0.0, c("#3a7a5a")), (0.55, c("#285641")), (1.0, c("#163626"))])
        .padding(24.0)
        .gap(16.0)
        .child(status)
        .child(opp)
        .child(col().grow(1.0).center().child(trick))
        .child(hand);

    // Wood rail: a rounded frame with a linear wood gradient and an inner felt.
    let rail = col()
        .key("rail")
        .w_full()
        .h_full()
        .radius(34.0)
        .fill(c("#6b3f1f"))
        .linear_gradient(std::f32::consts::PI, [(0.0, c("#8a5a32")), (0.5, c("#6b3f1f")), (1.0, c("#4a2a12"))])
        .border(1.0, c("#2e1a0b"))
        .shadow(10.0, 30.0, Color::BLACK.alpha(0.5))
        .padding(14.0)
        .child(felt);

    col().w_full().h_full().fill(c("#14110e")).padding(20.0).child(rail)
}

/// Arrow keys move the cursor; Enter (or Up) plays the cursor card; Down/Escape reset.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let n = s.hand.len();
    match &e.key {
        Key::Left if n > 0 => s.cursor = (s.cursor + n - 1) % n,
        Key::Right if n > 0 => s.cursor = (s.cursor + 1) % n,
        Key::Enter | Key::Up if n > 0 => {
            tx.motion(FLIGHT);
            let id = s.hand[s.cursor];
            s.play(id);
        }
        Key::Escape => s.reset(),
        // "o": seat 2 plays (stands in for a server event); "c": collect the trick.
        Key::Text(t) if t == "o" => {
            tx.motion(FLIGHT);
            let face = (s.opp_seq * 2 + 1) % CARDS.len();
            s.opp_play(face);
        }
        Key::Text(t) if t == "c" => {
            tx.motion(FLIGHT);
            s.collect();
        }
        _ => return false,
    }
    true
}

/// A named starting state for `--scene NAME` (and the playground).
pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("table", |_| {}),
    ("cursor", |s| s.cursor = 2),
    ("quality", |s| s.quality = true),
    ("played", |s| {
        s.play(2);
        s.play(0);
    }),
];

/// Non-empty `fonts` REPLACES the built-in Inter (native and headless), so list every face.
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
];

pub fn spec() -> AppSpec<State> {
    AppSpec {
        min_size: Some((720.0, 520.0)),
        scenes: SCENES,
        fonts: FONTS,
        families: ("Inter", "IBM Plex Mono"),
        setup: Some(setup),
        on_key: Some(on_key),
        ..AppSpec::new("Euchre spike", WINDOW, State::new, view)
    }
}
```

### A.3 `src/main.rs`

```rust
//! `cargo run` opens the window. Headless:
//!   rune-spike --snap out/table.png
//!   rune-spike --script scripts/click.json --snap out/click.png --at 1.2
//!   rune-spike --realtime --snap out/live.png --at 1.5     (runs on_ready -> producers)

use std::time::Duration;

use rune::{App, AppHandle};
use rune_spike::{Cmd, State, spec};

fn main() {
    // Window/GPU warm-up on a side thread while state builds (no-op headless).
    rune::prewarm_gpu();

    // A tokio runtime that outlives `app.run()`; rune owns the main thread.
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("spike-tokio")
        .build()
        .expect("tokio runtime");
    let tokio = tokio_rt.handle().clone();

    // UI -> network channel. App::init takes a closure, so state can capture runtime values
    // (AppSpec::init is a plain fn pointer and cannot).
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();
    let mut cmd_rx = Some(cmd_rx);
    let app = App::new(spec()).init(move || State { net: Some(cmd_tx), ..State::new() });
    let handle: AppHandle<State> = app.handle();

    // on_ready runs after the window's first frame (and in `--realtime` headless runs; never in
    // plain headless runs), so live producers start here and snapshots stay deterministic.
    let mut started = false;
    app.on_ready(move |_window| {
        if std::mem::replace(&mut started, true) {
            return;
        }
        // (c1) a plain std::thread
        let h = handle.clone();
        std::thread::Builder::new()
            .name("spike-ticker".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    h.update(|s, _tx| s.ticks += 1);
                }
            })
            .expect("ticker thread");
        // (c3) the network task: receives UI commands, answers through the handle.
        if let Some(mut rx) = cmd_rx.take() {
            let h = handle.clone();
            tokio.spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let Cmd::Play(id) = cmd;
                    h.update(move |s, _| s.remote = format!("server ack play {}", rune_spike::CARDS[id].0));
                }
            });
        }
        // (c2) a tokio task (stands in for a rivetkit-client event stream)
        let h = handle.clone();
        tokio.spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            h.update(|s, tx| {
                s.remote = "seat 2 played QD".into();
                tx.motion(rune_spike::FLIGHT);
                s.play(2);
            });
        });
    })
    .run();

    drop(tokio_rt);
}
```

### A.4 `tests/table.rs`

```rust
//! Runtime tests: no window, no GPU, deterministic frames.

use rune::{App, Env, Key, KeyEvent, Mods, Runtime};
use rune_spike::{CARDS, CARD_H, State, WINDOW, decode_png, on_key, spec, view};

const DT: f64 = 1.0 / 60.0;

fn settle(rt: &mut Runtime<State>, seconds: f64) {
    for _ in 0..(seconds / DT).ceil() as usize {
        rt.frame(DT);
    }
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent { key, mods: Mods::default() });
    rt.frame(DT);
}

#[test]
fn card_png_decodes_to_its_pixel_size() {
    let px = decode_png(CARDS[0].1);
    assert_eq!(px.size(), (512, 716));
}

#[test]
fn pressing_a_hand_card_flies_it_to_the_trick() {
    // Plain Runtime::new: view only (no on_key/poll/setup/fonts from the AppSpec).
    let mut rt = Runtime::new(State::new(), view, WINDOW, Env::new(true, 2.0));
    rt.frame(0.0);
    let (hand_rect, _) = rt.presented("hand-2").expect("hand card drawn");
    let trick = rt.presented("trick").expect("trick area").0;

    let (x, y) = rt.find("hand-2").expect("hand-2 has a centre");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT); // handlers run between frames: this frame commits the press
    assert_eq!(rt.state.trick, vec![2]);
    assert!(rt.find("hand-2").is_none(), "the hand card left the tree");

    // A few frames in, the trick card is between the hand and the trick (it is flying).
    settle(&mut rt, 0.1);
    let mid = rt.presented("trick-2").expect("trick card").0;
    assert!(mid[1] < hand_rect[1] - 1.0, "rose from the hand: {mid:?} vs {hand_rect:?}");
    assert!(mid[1] > trick[1] + 16.0 + 1.0, "but has not landed: {mid:?}");
    assert!(rt.animating);

    settle(&mut rt, 1.5);
    let landed = rt.presented("trick-2").unwrap().0;
    assert!((landed[1] - (trick[1] + 16.0)).abs() < 0.5, "landed inside the trick padding: {landed:?}");
    assert!((landed[3] - CARD_H).abs() < 0.01);
    assert!(!rt.animating, "at rest after the spring settles");
}

#[test]
fn arrow_keys_move_the_cursor_and_enter_plays() {
    // headless_runtime wires the whole AppSpec: on_key, poll, setup (no-op waker), fonts.
    let mut rt = App::new(spec()).headless_runtime(WINDOW, Env::new(true, 2.0));
    rt.frame(0.0);
    press(&mut rt, Key::Right);
    press(&mut rt, Key::Right);
    press(&mut rt, Key::Left);
    assert_eq!(rt.state.cursor, 1);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.trick, vec![1], "Enter played the KH under the cursor");
    assert_eq!(rt.state.hand, vec![0, 2, 3, 4]);
    press(&mut rt, Key::Escape);
    assert!(rt.state.trick.is_empty());
}

#[test]
fn on_key_can_be_attached_to_a_plain_runtime() {
    let mut rt = Runtime::new(State::new(), view, WINDOW, Env::new(true, 2.0));
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.cursor, 1);
}

#[test]
fn background_updates_land_through_runtime_update() {
    // AppHandle::update is not delivered to a test Runtime; drive the same closure with
    // Runtime::update (what the app loop does with each queued handle update).
    let mut rt = Runtime::new(State::new(), view, WINDOW, Env::new(true, 2.0));
    rt.frame(0.0);
    let (tx, rx) = std::sync::mpsc::channel::<u32>();
    std::thread::spawn(move || {
        for n in 1..=3 {
            tx.send(n).unwrap();
        }
    })
    .join()
    .unwrap();
    for n in rx.try_iter() {
        rt.update(move |s, _| s.ticks = n);
    }
    rt.frame(DT);
    assert_eq!(rt.state.ticks, 3);
}

#[test]
fn app_handle_updates_reach_a_headless_run() {
    // Queue a handle update before run: a headless run applies it before the first frame.
    let dir = std::env::temp_dir().join(format!("rune-spike-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tree = dir.join("tree.json");
    let app = App::new(spec());
    let handle = app.handle();
    std::thread::spawn(move || handle.update(|s, _| s.remote = "from a thread".into())).join().unwrap();
    app.args(vec!["--tree".into(), tree.to_str().unwrap().into()]).run();
    let json = std::fs::read_to_string(&tree).unwrap();
    assert!(json.contains("from a thread"), "status text in the tree");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opponent_back_flies_to_the_trick_and_collect_shrinks_into_the_stack() {
    let mut rt = App::new(spec()).headless_runtime(WINDOW, Env::new(true, 2.0));
    rt.frame(0.0);
    let back = rt.presented("opp-back-4").expect("top back").0;
    press(&mut rt, Key::Text("o".into()));
    assert!(rt.find("opp-back-4").is_none(), "the top back left");
    settle(&mut rt, 0.1);
    let mid = rt.presented("trick-opp-0").expect("seat 2 card").0;
    assert!(mid[1] < back[1] + 70.0 && mid[3] > 70.0 && mid[3] < CARD_H, "growing out of the fan: {mid:?}");
    settle(&mut rt, 1.5);
    assert!((rt.presented("trick-opp-0").unwrap().0[3] - CARD_H).abs() < 0.01);

    press(&mut rt, Key::Text("c".into()));
    assert!(rt.state.opp_played.is_empty() && rt.state.won.len() == 1);
    settle(&mut rt, 0.1);
    let shrinking = rt.presented("won-opp-play-0").expect("stack mini").0;
    assert!(shrinking[3] > 34.0 + 1.0, "still shrinking: {shrinking:?}");
    settle(&mut rt, 1.5);
    let at_rest = rt.presented("won-opp-play-0").unwrap().0;
    assert!((at_rest[3] - 34.0).abs() < 0.01, "{at_rest:?}");
    assert!(!rt.animating);
}

#[test]
fn cards_scale_down_to_fit_the_minimum_window() {
    let mut rt = App::new(spec()).headless_runtime((720.0, 520.0), Env::new(true, 2.0));
    rt.frame(0.0); // first layout: the view read None for "felt" and laid out at full size
    rt.frame(DT); // the read changed, so the view re-ran at the fitted scale
    let felt = rt.presented("felt").unwrap().0;
    let hand = rt.presented("hand").unwrap().0;
    let card = rt.presented("hand-1").unwrap().0;
    assert!(card[3] < CARD_H, "scaled: {card:?}");
    assert!(hand[1] + hand[3] <= felt[1] + felt[3] - 24.0 + 0.5, "hand inside the felt padding: {hand:?} {felt:?}");
}

#[test]
fn reduced_motion_lands_the_card_without_a_flight() {
    let mut rt = Runtime::new(State::new(), view, WINDOW, Env::new(true, 2.0));
    // The host preference (what macOS "Reduce motion" sets). Writing `rt.reduced_motion`
    // directly does not stick: each commit recomputes it from this.
    rt.set_system_reduced_motion(true);
    rt.frame(0.0);
    let trick = rt.presented("trick").unwrap().0;
    let (x, y) = rt.find("hand-2").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(DT);
    rt.frame(DT);
    let at = rt.presented("trick-2").unwrap().0;
    assert!((at[1] - (trick[1] + 16.0)).abs() < 0.5, "no flight under Reduce Motion: {at:?}");
}

#[test]
fn set_viewport_resizes_and_the_cards_rescale() {
    let mut rt = Runtime::new(State::new(), view, WINDOW, Env::new(true, 2.0));
    rt.frame(0.0);
    assert!((rt.presented("hand-1").unwrap().0[3] - CARD_H).abs() < 0.01);
    rt.set_viewport((720.0, 520.0), 2.0);
    settle(&mut rt, 1.5);
    assert!(rt.presented("hand-1").unwrap().0[3] < CARD_H - 10.0);
}
```

### A.5 `scripts/click.json`, `scripts/keys.json`, `scripts/click-live.json`, `scripts/opp.json`

```json
{"events": [
  {"t": 0.3, "type": "click", "key": "hand-2"},
  {"t": 0.3, "type": "mark", "name": "played"}
]}
```

```json
[
  {"t": 0.2, "type": "key", "name": "Right"},
  {"t": 0.4, "type": "key", "name": "Right"},
  {"t": 0.6, "type": "key", "name": "Left"},
  {"t": 0.8, "type": "key", "name": "Enter"},
  {"t": 0.8, "type": "mark", "name": "enter"}
]
```

```json
{"events": [
  {"t": 0.8, "type": "click", "key": "hand-0"},
  {"t": 0.8, "type": "mark", "name": "clicked"}
]}
```

```json
{"events": [
  {"t": 0.3, "type": "key", "name": "o"},
  {"t": 0.3, "type": "mark", "name": "opp"},
  {"t": 1.4, "type": "click", "key": "hand-0"},
  {"t": 2.6, "type": "key", "name": "c"},
  {"t": 2.6, "type": "mark", "name": "collect"}
]}
```

## Appendix B - verified rivetkit-client bridge (verbatim)

### B.1 `Cargo.toml`

```toml
[package]
name = "rune-bridge-check"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
rune = { path = "/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/rune-src/rune/crates/rune" }
rivetkit-client = "3.1.0"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
anyhow = "1"

[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = false
opt-level = 2
```

### B.2 `src/lib.rs`

```rust
//! Type-checks the rivetkit-client -> rune AppHandle bridge used by the cookbook.

use rivetkit_client::{Client, ClientConfig, ConnectionStatus, EncodingKind, GetOrCreateOptions, TransportKind};
use rune::{AppHandle, Spring};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

pub const FLIGHT: Spring = Spring::new(0.45, 0.12);

#[derive(Debug)]
pub enum Cmd {
    Play { card: String },
}

#[derive(Default)]
pub struct State {
    pub conn: Option<ConnectionStatus>,
    pub snapshot: Option<Value>,
    pub error: Option<String>,
}

pub fn client(endpoint: &str) -> Client {
    // ClientConfig defaults: TransportKind::WebSocket + EncodingKind::Bare. Client::new is infallible.
    Client::new(ClientConfig::new(endpoint).transport(TransportKind::WebSocket).encoding(EncodingKind::Json))
}

/// Runs on the tokio runtime for the whole session. `client` must outlive the connection:
/// dropping a `Client` broadcasts shutdown to every connection it made.
pub async fn run_bridge(
    client: Client,
    table_id: String,
    handle: AppHandle<State>,
    mut cmds: UnboundedReceiver<Cmd>,
) -> anyhow::Result<()> {
    let actor = client.get_or_create("table", vec![table_id], GetOrCreateOptions::default())?;
    let conn = actor.connect(); // tokio::spawn inside: must run within a runtime context

    let h = handle.clone();
    conn.on_status_change(move |status| h.update(move |s, _| s.conn = Some(status))).await;

    let h = handle.clone();
    let snapshots = conn
        .on_event("snapshot", move |args: &Vec<Value>| {
            let v = args.first().cloned().unwrap_or(Value::Null);
            h.update(move |s, tx| {
                tx.motion(FLIGHT);
                s.snapshot = Some(v);
            });
        })
        .await;

    while let Some(cmd) = cmds.recv().await {
        let Cmd::Play { card } = cmd;
        if let Err(e) = conn.action("playCard", vec![json!({ "card": card })]).await {
            let msg = e.to_string();
            handle.update(move |s, _| s.error = Some(msg));
        }
    }
    snapshots.unsubscribe().await; // dropping a SubscriptionHandle does NOT unsubscribe
    conn.disconnect().await;
    drop(client);
    Ok(())
}
```

### B.3 `tests/connect.rs`

```rust
use rivetkit_client::{ConnectionStatus, GetOrCreateOptions};
use rune_bridge_check::client;

#[test]
fn connect_outside_a_tokio_runtime_panics() {
    let c = client("http://127.0.0.1:9");
    let actor = c.get_or_create("table", vec!["t1".into()], GetOrCreateOptions::default()).unwrap();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _conn = actor.connect();
    }));
    assert!(r.is_err(), "connect() must be called inside a tokio runtime context");
}

#[test]
fn connect_inside_a_runtime_handle_enter_works() {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    // From a non-async thread (e.g. rune's on_ready), enter the runtime before connect().
    let _guard = rt.handle().enter();
    let c = client("http://127.0.0.1:9");
    let actor = c.get_or_create("table", vec!["t1".into()], GetOrCreateOptions::default()).unwrap();
    let conn = actor.connect();
    let status = conn.status_receiver();
    let st = *status.borrow();
    assert!(matches!(st, ConnectionStatus::Idle | ConnectionStatus::Connecting | ConnectionStatus::Disconnected), "{st:?}");
    drop(_guard);
    rt.shutdown_background();
}
```
