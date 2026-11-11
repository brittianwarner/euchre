# R4: Rune example apps, motion and visual techniques for the euchre table

Scope: every app under `examples/` plus `demo/` in the Rune repo. Extends `docs/port/07-rune-cookbook.md`; sections 5, 6, 9, 13, 14, 15 of that file are not repeated.

## Revision statement

- Project pin: `rune` rev `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb` (`/Users/brittianwarner/goods/euchre-rune/Cargo.toml:44`). Source at `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6`.
- Paths below are written against GitHub HEAD `0ea75abe` (a clone in the session scratchpad: `.../scratchpad/rune-src/rune-head`).
- `examples/`, `crates/rune-fx`, and `crates/rune-kit` (except markdown) are byte-identical between 85cd8c6 and HEAD, so every example claim below holds at the pin. Path form: `examples/<app>/src/<file>:<line>`.
- HEAD adds `crates/rune/src/flight.rs` launch flights and motion-track Mode changes. No example uses them. They are NOT available at the pin and nothing here depends on them.
- The scratchpad build of the examples succeeded (45.65 s) but I did not need to run them for any claim; claims come from source plus `demo/` images and `demo/fx/timings.md`.

## What the desktop already does (apps/desktop/src/screens/table)

Already in use, so not new: `matched` ids for play/collect/pickup (`stage.rs:585,628,732,778,800`), springs `PLAY`/`PICKUP`/`COLLECT` (`stage.rs:52-56`), `LIFT` via `offset_motion` (`hand.rs:19,168`), deal-in transition with `SpringPair` (`hand.rs:221-228`), header offset+opacity transition (`header.rs:113-115`), `LoopSpec` (imported `stage.rs:17`). Gaps the examples fill: staggered deal timing, arc paths, drag-to-play, scripted timelines, score odometer, trick-win accent effects, turn timer ring, blurred scrims, motion blur.

## Per-example notes

Card-game mapping uses: deal, play, trick sweep, bidding, score, turn flow, dialogs, screen change, hover/press, drag-to-play.

### matched (`examples/matched`)
- Shows: shared-element flight between two parents via `El::matched(id)`. Already verified in cookbook section 5.
- Technique: same id on source and destination element, spring on `.motion`. Use only as the base layer for play/collect flights; nothing new here.

### odometer (`examples/odometer/src/main.rs`)
- Shows: `RollingText` digit-roll counters. `RollingText::new(key, text).value(f64).font(..).color(..).spring(..)` (`main.rs:30`), big count (`main.rs:57`), right-aligned run time with `.trailing()` (`main.rs:58`). Implementation `crates/rune-kit/src/rolling_text.rs`; behaviour tests in `crates/rune-kit/tests/rolling_text.rs:124` (proportional label keeps prefix and springs its width), `:256` (width springs inside a growth row under animate_layout).
- Card game: team scores in the header and score dialog roll up on trick/hand end; "tricks won" counters; bid levels. Cheapest "feels expensive" win. Value must be an `f64` that really changes; keep a stable `key`.

### motion-bento (`examples/motion-bento`; README.md, `src/style.rs`, `tiles_top.rs`, `tiles_mid.rs`, `dock.rs`, `stress/live.rs`, `chaos/src/look.rs`)
- Shows: a dashboard where every tile is a different motion idiom, all tunable.
- Technique A, card deck stack (`tiles_mid.rs:118-187`, card at `:157-173`): `col().key("card{id}").absolute(0, depth*PEEK).scale(1 - depth*SHRINK).motion(layout).transition(Transition::asymmetric(Transition::scale(0.9), Transition::move_edge(Edge::Trailing).combined(Transition::opacity())))`. Items are iterated back to front so the top draws last; removing the front card slides it out the trailing edge while the rest shift up one slot by spring.
  - Card game: the kitty/stock pile, the draw pile after the up-card, a stack of tricks won; pop the top card off with an asymmetric exit.
- Technique B, trim ring (`tiles_top.rs:118`): `canvas(fill).size(RING,RING).absolute(0,0).live_trim(s.ring.smooth(spring))`. A circular arc whose length is a `Live<f32>` smoothed by a spring; no view rebuild per frame.
  - Card game: turn timer ring around the active seat's avatar; drains with `live_trim`, snaps color near zero.
- Technique C, `live_offset(knob.smooth(snap))` (`stress/live.rs:239`): a toggle knob driven by a live value.
  - Card game: Settings toggles (reduced motion, sound), alone-hand toggle.
- Also: segmented selector pill with sliding highlight in `tiles_top.rs`/`dock.rs` (hover/press feedback on dock icons). Card game: bidding bar "Pass / Order up / Pick suit" pill slide.

### live-values (`examples/live-values`, README.md, `src/main.rs`)
- Shows: `Live<T>` values update paint properties without rebuilding the tree. `ring.live_opacity(&level)` (README.md:14), `cursor.live_offset(live.smooth(Spring::new(0.16, 0.0)))` (README.md:15, `main.rs:71`), vs the rebuild path `ring.opacity(...)` (`main.rs:60`).
- Card game: drag position of the held card, hover glow level, timer ring; anything that changes per frame. Rule: if a value changes every frame, make it Live, not state.

### launcher / frosted-bar / glass-card / glass-panel / shapes / background
- `frosted-bar/src/main.rs:66`: `.backdrop_blur(sigma).fill(tint)` produces a frosted bar over scrolling content. Card game: bottom action bar over the felt, score panel.
- `glass-panel/src/main.rs`:
  - `flight(t)` at `:122-131`: a poll-driven absolute card. Smoothstep `p`; width eases ahead of height (`ease(t*1.25)` vs `ease(t*1.25-0.25)`); the centre rides a quadratic Bezier with control point `[lerp(from.x,to.x,0.6), 0.0]` (rises above both endpoints); radius lerps; returns `sin(pi*p)` as a lift factor for shadow. This is the arc technique missing from the cookbook (gap: arc).
  - `:143`: dialog swap transition `opacity().combined(scale(0.96)).combined(blur(3.0))`.
  - `:184`, `:204`: `looping(LoopSpec::rotation(1.0))` and staggered `LoopSpec { delay: k*0.8, ..LoopSpec::slide(4.8, [-1,0], [w/30,0]) }` for ambient shimmer/backdrop loops (cookbook gap `.looping`).
- Card game: play flight along an arc with shadow growing at apex then collapsing on landing; dialog open/close.
- `glass-card`, `shapes`, `background`: `canvas(Shape)` fills/strokes/paths and layered gradients; `LineCap::Round` strokes. Card game: suit pips as `Path`, table felt vignette, card edge highlight.

### showreel (`examples/showreel`; `src/{main,gfx,kit}.rs`, scenes `word, deploy, glass, shock, fire, pricing, ask, keys, tabs, type_`)
- Shows: a scripted 960x540 reel. Fully deterministic: each scene is a pure function of time `t`.
- `kit.rs:59-73` `track(t, from, &[(time, target, Spring)])`: replays a chain of springs by sampling `Spring::sample(x, v, target, dt) -> (x, v)` and carrying velocity across retargets; `kit.rs:52` `ease(t, t0, dur)` smoothstep; `kit.rs:103` `pressed(t, tp)` press pulse. `kit.rs:47` `fx(kind, [f32;4]) -> El` creates an effect element.
- `showreel/Cargo.toml`: depends on `rune-fx` and `wgpu = "30"`; `main.rs:79` installs `.compositor(gfx::Gfx::default())`.
- `scenes/shock.rs:46`: `fx(SHOCK, [x, y, tp+0.03, 22.0]).key(..).effect_backdrop(true).absolute(0,0).size(W,H)`; always in tree because an idle shockwave draws nothing (`shock.rs:41`). Card game: shockwave from the winning card on a trick win or from the table centre on a "Euchred!" / loner win; timing 0.155 ms GPU (timings.md).
- `scenes/deploy.rs:63`: liquid-metal button with glow while hovered; `:148`: scrim `el().backdrop_blur(10*dim).fill(black @ 0.68*dim)` with `dim` animated. Card game: primary "Order it up" button; modal scrim for score and rules dialogs.
- `scenes/pricing.rs`, `scenes/ask.rs`, `scenes/keys.rs`, `scenes/tabs.rs`, `scenes/type_.rs`, `scenes/word.rs`, `scenes/glass.rs`, `scenes/fire.rs`: sparkle bursts, press-key visuals (`demo/motion/press.png` shows rest vs pressed: shadow collapses and scale drops), tab pill slides, text reveal, fluted glass, fire with embers. Card game: sparkles on winning bid, fire/heat behind a "Going alone" bid, tab slide between Scores / Rules / Settings.
- Use the same timeline approach for a hand-start cinematic: `track` for each card's x, y, rotation across the deal.

### site-scenes (`examples/site-scenes`)
- Marketing-site scenes built from the showreel kit: same `track`/`fx` toolbox; confirms the pattern scales to many scenes. Card game: reusable scene functions for splash, "New game" intro.

### lab (`examples/lab/src/{main,stage,gfx,knobs,ui}.rs`)
- Shows: a knob-driven playground for every motion idea. `.compositor(gfx::LabGfx::default())` at `main.rs:189`.
- Drag + fling (`stage.rs:173-282`): Move updates state then `tx.instant()` (`:215`, `:240`, `:246`, `:250`) so the card follows the pointer with no lag; Up computes release velocity and target with `rune_motion::project(c.x, v.x, rate)` clamped to bounds (`:275-276`), then `s.card.release(v, Vec2{x: tx_, y: ty_}, Spring::new(glide, 0.0), now)` (`:278`) and `tx.instant()` at `:279`.
- Motion blur (`gfx.rs:190-212`): `Pose` closure sampled with `MotionSamples::from_track(&p, rest, t, scale, pose_at)`, shutter angle and "soft shutter ends" knobs (`knobs.rs:192-194`), disabled under reduced motion (`gfx.rs:208`).
- `cx.reduced_motion(s.reduced)` at `ui.rs:75`.
- Card game: drag-to-play in the hand; on release over the play zone commit the play and hand the matched id to the table flight; otherwise `release(v, hand_slot, spring)`. Motion blur on fast sweeps (trick collect to the winner).

### web-showcase (`examples/web-showcase/src/lib.rs`)
- Drag with `tx.instant()` at `:102` in the 84-110 block; the same Move/Up pattern as lab in a smaller form. Reference when porting to a minimal drag-to-play.

### drag-out / drop-files / rich-clipboard / sketch / knobs / todo / hello / luau-panel / web-hello
- `Drag` type at `crates/rune/src/el/handlers.rs:117`; `Phase::{Down, Move, Up}`, `d.cancelled()`, `d.data`, `dx`/`dy`. `drag-out` shows dragging data out of the window; `sketch` freehand paths (`Path`); `knobs` slider/knob controls; `todo` list add/remove with enter/exit `Transition`s; `hello` minimal `AppSpec`. Card game: `Drag` cancel handling (Escape or drop outside) returns the card; `todo` list is the simplest reference for score-history row transitions.
- `drop-files`, `rich-clipboard`, `luau-panel`, `web-hello`: not relevant to table motion (file drop, clipboard, scripting, wasm hello).

### settings / cursed-settings (`examples/settings`, `examples/cursed-settings/src/{gags,gfx,ui}.rs`)
- `settings/main.rs:76-77`: `cx.reduced_motion(s.reduce_motion)` and `cx.time_scale(speed_of(s.speed))`: user-facing animation speed (slow/normal/fast) and reduced motion. Card game: implement exactly this in the Settings screen; stage timings (`timing::PLAY` etc.) then scale automatically.
- `cursed-settings/gags.rs:819-846` `flush_pose(u, rest, cc, drain, rm) -> Pose`: wind-up (sin pulse of rotation/scale/y for 0.2 s), then an accelerating spiral (`k = v^2.2`, angle `-pi/2 + k*TAU*1.1`, radius shrinks to the drain, rotation `k*TAU*1.35`, scale to 0.03), fade at the end; reduced-motion branch is a plain fade (`rm`). Card game: trick sweep or end-of-hand discard spiral toward the winner seat, with the reduced-motion fallback.

### playground (`examples/playground/src/main.rs`)
- `Playground`, `Story`, `Play::after`, triggers and variants for replayable choreography (`main.rs:15`, `:235` `upload_replay`). `wake_after` stepping at `:160`. Card game: a dev-only motion lab (replay the deal, play, sweep) to tune timings without playing a game.

### overlay / overlay-lab
- Overlay windows and pointer-following element (`overlay-lab/src/main.rs:67` `live_offset(&s.pointer)`, `overlay/src/main.rs:30`). Card game: hover tooltips following the cursor, "your turn" tags; low priority.

### video-tiles / video-player
- Video playback (`video-player/src/main.rs:27` `clip.set_looping(true)`). Not needed for the table; could host an intro loop.

### kit-gallery (`examples/kit-gallery/src/{stories,layer_timing,main}.rs`)
- `stories.rs:293-307` `motion_stagger_list`: `Stagger::new(0.05)`; each row `.transition(Transition::offset(0.0, 12.0).combined(Transition::opacity())).enter_delay(stagger.delay(i, n))`; all leave together. Card game: the deal. Replace per-card ad-hoc delay with `Stagger` over the five cards, `from` the first card so the deal order matches real order; `Stagger` builder methods `.from/.start/.ease` live in `crates/rune-motion/src/choreo.rs`.
- `main.rs:114`: `cx.reduced_motion(true)` in gallery mode; `layer_timing.rs` shows presence/translation timing; `demo/motion/presence-and-paths.png` plots blur-replace enter/exit/revive curves and a parent-vs-`Travel::Screen` path comparison.

## demo/ folder
- `demo/fx/timings.md`: GPU ms per draw (2x, 10 draws/pass). Relevant: glow soft 0.229, pulse breathe 0.134, pulse ripple 0.086, shockwave 0.155 (backdrop read), lens 0.381, shimmer 0.148, sparkles x9 0.270, grain 0.150, liquid metal animated 0.951 / still 0.217, fire top+16 embers 1.839, aurora 0.781, heat haze 1.153.
- `demo/motion/*.png`: press (rest vs pressed shadow/scale), presence-and-paths curves.

## Cost and risk
- All `canvas`, `Live`, `Stagger`, `Transition`, `RollingText`, `backdrop_blur`, `Drag` techniques are core crates; no new dependency.
- rune-fx (shockwave, sparkles, liquid metal, glow, fire, motion blur) needs: `rune-fx` as a git dependency at the same rev, `wgpu = "30"` with default-features off (`showreel/Cargo.toml`), and `App::compositor(impl Compositor<S>)` (`crates/rune/src/app.rs:539`). Verify the build before committing to it. Effects are GPU cost, not layout cost; keep one-shot effects short, one at a time (rune-fx `OneShots` pool).
- Always provide the reduced-motion branch (`rm` in `flush_pose`, `s.reduced` in lab gfx). Effects with `effect_backdrop(true)` read the framebuffer; avoid more than 2 on screen at once.

## Ranked top 15 reusable techniques (for the euchre table)

1. Poll-driven arc flight with radius and shadow lift (`glass-panel/src/main.rs:122-131`): play and sweep paths that feel thrown, not slid.
2. `track()` spring timeline for scripted choreography (`showreel/src/kit.rs:59-73`): deterministic deal, sweep, and intro sequences, testable with fixed `t`.
3. Drag-to-play: `tx.instant()` while dragging, `release()` with `project()` fling on drop (`lab/src/stage.rs:173-282`), then matched flight to the table.
4. Staggered deal via `Stagger` + `enter_delay` (`kit-gallery/src/stories.rs:293-307`): replace manual delay math.
5. Deck/kitty stack with asymmetric transition (`motion-bento/src/tiles_mid.rs:157-173`): draw pile, kitty, tricks-won stacks.
6. `RollingText` odometer for scores (`odometer/src/main.rs:30,57`): team scores and trick counts.
7. Shockwave on trick win (`showreel/src/scenes/shock.rs:46`): 0.155 ms; needs rune-fx and compositor.
8. Pose + shutter motion blur on fast sweeps (`lab/src/gfx.rs:190-212`): trick collect to the winner; needs rune-fx.
9. Spiral/drain exit with wind-up (`cursed-settings/src/gags.rs:819-846`): trick sweep and end-of-hand discard, reduced-motion fallback included.
10. Live trim ring turn timer (`motion-bento/src/tiles_top.rs:118`): per-seat timer with no rebuilds.
11. Backdrop-blur scrim plus scale+opacity+blur dialog (`showreel/src/scenes/deploy.rs:148`, `glass-panel/src/main.rs:143`): score, rules, and bid dialogs.
12. Sparkles burst and glow for winning bid/hand (`showreel/src/scenes/deploy.rs:63`, `pricing.rs`): cheap accent, 0.27 ms.
13. Liquid-metal / plastic primary button (`showreel/src/scenes/deploy.rs:63`): "Order it up"; animated metal is 0.95 ms, use still variant at rest.
14. Segmented selector pill with sliding highlight (`motion-bento/src/tiles_top.rs`, `showreel/src/scenes/tabs.rs`): bidding bar suit chooser, Scores/Rules tabs.
15. Settings animation speed + reduced motion (`settings/src/main.rs:76-77`) and `Playground` replay for tuning (`playground/src/main.rs:15`): user control and a dev harness.

## Unverified at the pin (flag before relying)
- `LoopSpec` behaviour beyond the two example uses (`glass-panel/src/main.rs:184,204`).
- Exact `Stagger` builder API signatures at 85cd8c6 (`crates/rune-motion/src/choreo.rs`); examples are identical across revs, so usage is safe, but confirm the build before extending.
- rune-fx under wgpu 30 inside the desktop app's existing wgpu surface (not built in this project).
