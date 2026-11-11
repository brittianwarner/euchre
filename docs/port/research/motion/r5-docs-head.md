# R5: Rune documentation at HEAD, and the change since the app's pin

Revs. PIN = 85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb (apps/desktop and the workspace Cargo.toml:44-45 use it). HEAD = 0ea75abef0a6caa014067a88f89518b9a04cce2b (26 commits, 82 files, +5404/-807 lines). Every "HEAD" citation below is a path:line in a checkout of HEAD. Every "PIN" citation is a path:line in the cargo checkout of 85cd8c6. Paths are relative to the rune repo root unless stated.

Scope note: this is read-only research. Nothing in apps/desktop or the cookbook was edited. Items marked [recall] in the Rune research notes were not verified by their author; I repeat that mark where it matters.

## 1. Bottom line

1. Bumping the pin to HEAD is low risk. I found no breaking public API change in the 26 commits. The changes are state-machine refactors (the "r" rounds), a handful of fixes, and three small additions (`rune::marks`, `MarkdownStyle::code_margin`, a Windows occlusion tracker).
2. Nothing in the bump adds a new motion feature the app needs for the card flights. "Launch" flights (`Tx::launch`, `Tx::fly_text`, `Tx::unclipped_flights`) are NOT new. They exist at the PIN (crates/rune/src/flight.rs:16, :28, :36, PIN). HEAD only reworks their internals (commits 9e537376, bc0d36fc).
3. The largest gap is not the pin. It is that the cookbook (docs/port/07-rune-cookbook.md) never mentions `rune::motion_gate`, which exists at the PIN (crates/rune/src/lib.rs:46, PIN), nor `Tx::launch`, `Tx::unclipped_flights`, `El::dissolve`, `blur_replace`, or `Live` smoothing. Those are the tools for "badass transitions", and they work on the current pin.
4. Recommendation: bump to HEAD, but as its own change after the current apps/desktop fix lands, then re-run the three verifications listed in section 7. Do not bump in the middle of someone else's apps/desktop edit.

## 2. Official motion guidance (HEAD docs)

### 2.1 What rune-motion is

- Analytic, closed-form tracks. `Spring::new(duration, bounce)` is SwiftUI-style, bounce 0 by default, closed form in every damping regime (crates/rune-motion/README.md:45-52, HEAD; table of pieces).
- Curves: `Spring`, `Tween` (cubic-bezier, steps, linear), `Decay`, `Keyframes` (Hermite, keep velocity across keys), `Instant`. `Anim` adds delay, additive/merge, velocity, completion token (README.md:45-52, HEAD).
- `AnimationStore`: every animated value is a track. Retargeting keeps position and velocity. `Scene`: keyed nodes, presence (exits stay mounted, can revive mid-exit), `Transition` constructors `opacity scale offset move push slide blur_replace reveal asymmetric combined` (README.md:45-52, HEAD).
- Choreography: `Cue::now/at/started/arrived/settled(step).after(s).not_before(t)`, `Stagger` (from start, end, centre or index, eased), `PhaseAnimator`, `Keyframes` (README.md:45-52, HEAD).
- `LoopTrack`/`LoopSpec`: periodic eased opacity or clockwise rotation sampled at clock time. `retime` preserves phase. The renderer owns visibility and scheduling (README.md:45-52, HEAD).
- Tokens: `INSTANT SNAP SETTLE REFLOW GLIDE MORPH POP DRIFT CAMERA`, each with an `_EXIT` partner 10 to 25 percent shorter (README.md:45-52, HEAD).

### 2.2 Rules Rune applies by default (crates/rune-motion/README.md:60-71, HEAD)

- Springs are duration plus bounce. Bounce 0 by default. Exits run 10 to 25 percent shorter than entrances.
- Interrupting keeps position and velocity.
- Nothing animates on first mount. A track's first `animate` acts as `set`. Setting an unchanged target does nothing.
- Direct manipulation is never animated. Gestures write with `set` and animate only on `release`.
- Reduced motion: travel, scale, shape and blur change instantly. Opacity, colour and presence take an ease-in-out fade. `match_geometry` becomes a crossfade in place.
- Rect edges snap to 1/64 device px. Settle thresholds scale with pixel density.
- Blur at most 4 px and only for things entering or leaving in place. A transition that travels drops its blur. Scale stays in [0.9, 1.1] and never goes to 0. `Transition::allow_small_scale()` lowers the floor to 0.5 for small glyphs.
- `.width_leads()` runs the X spring at 0.78 of the duration.
- A stationary edge gets no motion.

Doc inconsistency to know about: the reduced-motion fade is 0.18 s in code and in rune-motion's README (crates/rune-motion/src/props.rs:44, HEAD), and 0.15 s in crates/rune/README.md:52-53 and :442 (HEAD). Treat the rune-motion value as the primitive and the rune value as what the app-level runtime documents. For the app, either is a fade; do not depend on the exact number.

### 2.3 Renderer contract (crates/rune-motion/README.md:152-184, HEAD)

- Sample at the timestamp the frame will be shown, not at now: `scene.set_time(target_timestamp)`.
- Per frame: apply commits, then `scene.frame()` returns changed nodes, exited nodes, events, and an `animating` flag. When `animating` is false, pause the display link.
- Presentation values: rect (already snapped), opacity, transform + anchor, blur sigma, radius, fill, mask, phase, `in_flow`, flags.
- `RUNE_MOTION_TRACE=/path.jsonl` writes motion traces.

### 2.4 App-level motion features (crates/rune/README.md, HEAD)

- Matched geometry, `El::matched(id)` (README.md:1097-1146). Behaviour that matters for the card game:
  - A flight starts when an element with an id leaves in the same commit that another with the same id mounts elsewhere. The new one starts where the old one was seen, with its velocity, radius, fill and opacity.
  - Children keep their final layout and are scaled uniformly to cover the moving frame, clipped to it. Text keeps its size.
  - A flight paints above everything, outside ancestors' opacity, scale, blur and rounded clips. It keeps its destination's rectangular clips (scroll viewports included) unless `Tx::unclipped_flights` is set.
  - Interruptible: matching back mid-flight starts from the presented rect with velocity.
  - Spring is `tok::motion::REFLOW` unless `Tx::motion` or the element's `.motion()` sets one.
  - Under reduced motion nothing is matched; the old element exits and the new one enters by their own transitions. Settle and first frames swap.
  - Ids are per window. If two present elements share an id, the last in tree order wins.
- Finite choreography (README.md:1074-1096): `El::sequence(Sequence::new(&[Beat..]).delay(..))`. Each `Beat::new(t)` is a full opacity and scale pose at elapsed seconds, omitted values are 1, `Beat::easing` sets the arriving segment. Unchanged specs keep progress across commits. Changing the spec or mounting a new key replays. Reduced motion lands the final pose at once. Scale is presentation only, not layout. It holds the final pose and sleeps, so no `poll` or `wake_after` needed.
- `El::looping(LoopSpec)`: render-clock loops (the app already uses it for the thinking dot).
- `El::dissolve()` (README.md:2195-2231): keeps the parent isolated even at opacity 1 and composites each child's subtree additively, for full-state crossfades. Blur, dissolve, native video and image atlas pages share one isolated-scope path.
- Subtree blur and isolated opacity (README.md:2158, 2220), and compositor effect slots via rune-fx (README.md:2086). Ordinary slots stay unaffected.
- `Tx::fly_text(source, destination)` (README.md:161, 1395): hands a visible plain-text field to a newly mounted element. `Tx::launch` and `Tx::unclipped_flights` are listed only in the source docs (PIN crates/rune/src/flight.rs:16-38), not in any README paragraph I found.
- High-frequency values (README.md:1146+): `Live<T>` and `LiveText` for values that change per frame without re-running the view.

### 2.5 Performance and pacing rules

- Sample at presentation time, never `Instant::now()` (research/15-research-motion-perception.md:14-17, :53, HEAD). Frame-time variance matters more than average fps. The research cites NVIDIA/UCSB: 12 ms of frame-time variation measurably degrades smoothness and 4 ms barely does (research/15:16-17, marked [read]).
- Hitch-time ratio thresholds are attributed to Apple's WWDC20/21 sessions (research/15:65) and are [recall]. Do not quote them as verified. Judder threshold maths and the Jota/Ng CHI 2013 latency paper are also [recall] (research/15:83).
- Low Power Mode holds a ProMotion panel at 60 Hz while display links and `NSScreen` still report 120 Hz (scripts/native-pacing/README.md:71-84, HEAD). Rune paces submissions at that cap (crates/rune/src/platform/display_link.rs). Implication: never assume 120 fps; design for 60 and let 120 be a bonus. Keep the half-refresh admission guard (do not try to remove it).
- Launch performance (crates/rune/README.md:688-760, HEAD):
  - `rune::prewarm_gpu()` first in `main` (README.md:695).
  - The first frame draws during window creation on macOS for visible windows (README.md:719).
  - Defer everything else to `on_ready` (README.md:722).
  - Keep the first second still. Rune animates nothing then. Land state that arrives right after launch with `tx.instant()` or do not animate it. One settings window's launch fade drew about 85 frames in its first second instead of 4 to 8 (README.md:733-737).
  - Measure idle too: an idle window draws nothing (README.md:752).
- Event cost: a handler that only moves live values says `tx.live_only()` so the view is not re-run (README.md:434, 483-527; the table at :527 gives 0 view runs, 2.0 ms, for `live_only`). Layout handlers that follow a scroll should be `live_only`. Read `Cx::layout_size`, not `Cx::window_layout` (README.md:777-780).
- Smooth scrolling and fling are analytic (README.md:764).
- Logging frame timing: README.md:927.

### 2.6 What Rune forbids or discourages

- Animating on first mount, animating direct manipulation, bounce on kit components, blur above 4 px, scale outside [0.9, 1.1] (rune-motion README.md:60-71).
- Per-VM or ad hoc timers in place of `wake_after` or tracks. Do not poll for motion; use tracks, `sequence` or `looping`.
- Animating anything while the window is still starting (launch rule above).
- Trusting a test pass for motion. Rune's answer is the motion gate (below).

### 2.7 Tooling

- `rune::motion_gate` (exists at the PIN, crates/rune/src/lib.rs:46; documented at scripts/motion-audit/README.md:66-116, HEAD). A `cargo test` gate on the app's real `Runtime` and real view. `Run` has `idle`, `step`, `play`, `act`, `quiet`. `check` flags: `pop-in pop-out cut-exit jump offset-jump appear-then-jump jerk reversal overshoot jiggle text-swap fade-snap flicker load-motion never-settles loop-jump wakes-after-end lands-unseen remount unpinned twice scroll-commit`, plus colour/region findings `snap flash changes-twice`. `gate_rows(cells, ALLOW)` fails on any finding not covered by an `AllowRow::new(kind, element, cells, why)`. Mark authored motion with `El::motion_intent` (`BOUNCE KICK STEP CUT PULSE TYPING`) or `El::instant_changes` instead of allowing it. Runner: `scripts/motion-audit/gate [cell] [--allowed|--dump|--sheet] -- -p my-app motion_gate`. Headless: `RUNE_MOTION_GATE=<path>`.
- `scripts/trace-quality.py` and the pacing check (crates/rune/README.md:2557-2570, HEAD): scene-edge error against analytic samples, inherited velocity at retarget, cadence. Virtual timestamps do not measure real display hitches.
- `rune::marks` (new at HEAD, crates/rune/src/marks.rs:1-63): `mark(name)`, `marks()`, `metrics()`, `notes()`. Records first-reached times from event loop start to first frame presented, and out through quit. Useful for a launch trace. Not a motion feature.

## 3. Commit review, PIN..HEAD (26 commits, newest first)

Legend: F feature, B behaviour change, R refactor with no intended behaviour change, T test-only, P platform-specific.

| SHA | Subject | Kind | Note for the app |
|---|---|---|---|
| 0ea75abe | Predict the overlay sequence test's callbacks... (r2) | T | Overlay input only. No effect. |
| e555ea17 | Check every short overlay pointer sequence (r2) | R/T | Overlay pointer ordering fixes. App has no overlay windows. |
| 02f469bc | Keep a lost release's point (r2) | B (overlay) | Overlay only. |
| 3cfb39b2 | End a lost overlay release before the next edge (r2) | B (overlay) | Overlay only. |
| 61ec31d2 | Overlay pointer capture is one Hold machine (r2) | R | Overlay only. |
| 14e3c5d5, 52e20219, be24e577 | Clipboard secret clear machine (r19) | R/B | Clipboard secret timing. The app does not copy secrets. No effect. |
| db69ea31 | Backoff test reference (r4) | T | None. |
| 627c2b3e | One backoff state for a window's draw scheduling (r4) | R, touches window/{draw,pacing,create,events,first_frame}.rs | Changes how a window retries a missed or occluded draw. Intended no-change. This is the riskiest internal refactor for frame pacing; see section 6. |
| d8f89f83 | Motion: what mounts inside a stuck sticky element enters as it would in sight (#572) | B (fix) | crates/rune/src/runtime/motion/unseen_presence.rs. Sticky headers with a ticking number now roll in rather than appearing. App has no sticky headers. |
| e3b6edf4 | `MarkdownStyle::code_margin` (#571) | F (additive) | rune-kit markdown. Not used by the app. |
| 0add0d57 | Windows: a fully covered window rests (#570) | P/F | Windows only; adds `window::Sight`. Desktop app on macOS unaffected. A Windows build would stop drawing when covered and call `App::on_visibility`. |
| 6b03df67 | `rune::marks` (#569) | F (additive) | New public module, crates/rune/src/lib.rs:45 (HEAD). Optional. |
| 3686e94a | Compare what a dispatch's caller uses after a close (r7) | R/T | Internal. |
| 57b87f54 | Wayland layer surface enums (r7) | P | Linux only. |
| 9674cb32 | Switch a panel's occlusion override at its fade (r3) | P (macOS panels) | Panels, not normal windows. |
| 28fc1d01 | macOS panel Phase for prewarm, shown, fading (r3) | P/R (macOS panels) | `platform/macos/presentation.rs`. Panels only. |
| bc0d36fc | Route a launch's phases through one step, prove a launch replaces its node's flight (r31) | R | crates/rune/src/runtime/motion/flights.rs: `Launches::step` (line 123, HEAD). No API change. |
| 9e537376 | Name a launch's waiting and airborne phases, with unclipped on the flight (r31) | R | `LaunchPhase::{Waiting, Airborne}`, `Airborne::{Launched, Unclipped}` (flights.rs:21-35, HEAD). Internal types. `launch_elements` (flights.rs:393) reports a miss if the commit is instant, uses `tx.settle`, is under reduced motion, or the destination was already mounted, which matches the PIN doc comment at crates/rune/src/flight.rs:20-27. |
| 20f9057d | rune-motion: golden samples pinned with a tolerance, not a bit hash (r64) | T | Only changes a test. Notes that springs read exp/sin/cos from the platform libm and macOS and glibc differ in the last bits (about 3e-5 at value 400). Not a runtime change. |
| d8f1ffed | Layout: a picture under a min or max width reserves the box it draws (#568) | B (layout) | `Picture::{Styled, Measured}`, `picture_side` in crates/rune/src/runtime/commit/flatten.rs and text_layout.rs. A picture with `max_w`/`min_w` and a ratio used to reserve a larger box than it drew (176 by 40 drawn, 158 reserved in the example commit). Now it reserves what it draws. Affects card faces only if they combine percentage width with min/max width. See section 6. |
| dde00138 | Keep a due delayed start's immediate wake before the tick applies it (r33) | B (fix) | crates/rune-motion/src/store/tracks.rs:7, :74-89 (HEAD). `waiting_until` now agrees with the tick about whether a delayed start is due, so a track whose delay has just elapsed is not reported as asleep. Relevant to the app's `enter_delay` deal-in and stagger: it removes a possible one-frame stall. Good change, no API change. |
| 047f6f1d | One `Mode` to hold in for motion tracks, `LoopTrack` a `Phase` (r33) | R | `Mode::{Moving, Due, Waiting{due}, Settled, Frozen}` (tracks.rs:27-46, HEAD); looping.rs `Phase`. Internal. |
| 2b20250c | Text: WindfoilChromium default everywhere but macOS and a Mac's browser (#566) | B (non-mac) | Glyph coverage on Windows/Linux. macOS keeps CoreText-style. Advances unchanged, so no reflow. |
| 17e7ffa9 | X11 repeat and pointer mailbox are one enum each (r37) | P | Linux only. |

Breaking public API changes: none found. I diffed the public surface touched by these commits (flight.rs, motion README, kit markdown style, lib.rs). New public items are `rune::marks` and `MarkdownStyle::code_margin`. No removed or renamed public items surfaced in the motion, flight, or el paths the app uses.

New motion or visual features at HEAD relative to the PIN: effectively none. Visible differences are (a) sticky-element entrance (#572), (b) picture min/max layout (#568), (c) delayed-start wake (dde00138), (d) non-mac text gamma (#566). The motion API surface the app can use is the same.

Caveat on completeness: I read the full diffs for motion, flights, commit/flatten, window draw/pacing, markdown style, marks and the panel Phase. I read subjects and stats only for the overlay pointer, clipboard, Wayland and X11 commits, which are platform paths the macOS app does not build. If apps/desktop later adds an overlay or panel window, re-read those.

## 4. Features present at the PIN that the cookbook omits (usable today, no bump)

All cited at the PIN unless noted.

- `Tx::launch(source, destination)`: PIN crates/rune/src/flight.rs:28. The destination mounts in this commit and starts with its first text on the source text's position, at its own size, travelling one straight screen path on `REFLOW` (or `Tx::motion`). Draws above the whole window and outside every clip. Does nothing under reduced motion or when the destination already existed. Use for: a played card flying out of the hand into the trick area when the hand card and trick card are different elements, or a dealt card flying from the deck.
- `Tx::unclipped_flights()`: PIN crates/rune/src/flight.rs:36. Makes every `El::matched` flight in this commit draw outside every clip until landing. Use when the hand tray or a scroll area clips the flying card. The cookbook gotcha list may mention clipping; this is the supported remedy.
- `Tx::fly_text`: PIN crates/rune/src/flight.rs:16. Text hand-off. Probably not needed for cards.
- `rune::motion_gate`: PIN crates/rune/src/lib.rs:46. See 2.7.
- `.sequence(Sequence)` and `.looping(LoopSpec)`: the cookbook lists them (07-rune-cookbook.md:423, 446) but marks the arc and confetti rows as unverified.
- `El::dissolve()`: PIN crates/rune/src/el/visuals.rs. Use for the trump-suit or banner crossfade, and for scoreboard swaps.
- `Transition::blur_replace`: used inside rune-kit at PIN (crates/rune-kit/src/notes.rs). Peaks at 1.5 pt blur, so it stays inside the 4 px cap. Use for score-number changes.
- `Live<T>`/`LiveText` and `tx.live_only()`: for per-frame values without re-running the view.
- `Stagger` and `Cue` choreography in rune-motion: the app uses `enter_delay` by hand for the deal-in. Stagger gives the same 75 ms stagger with ease control.

## 5. Mapping to docs/port/06-ui-spec.md (gameplay animations)

These are proposals, each citing what Rune provides. Verify each with a `motion_gate` cell before calling it done.

| Spec animation (06-ui-spec.md section 6) | Rune tool | Notes |
|---|---|---|
| Play flight, 700 ms, 0/82/100% keyframes | `El::matched` with a custom `.motion()` spring (the app does this: PLAY spring) or `Tx::launch` if source and destination are different elements | The 82% overshoot beat is not expressible as one matched path. A bounce above 0 on a flight breaks the Kit rule; if wanted, mark it `motion_intent(BOUNCE)` so the gate accepts it. |
| Up-card pickup, 850 ms, arc | Matched flight plus a `.sequence(..)` on the destination for the arc, or accept the straight path | Cookbook already says the arc is not one straight path (07-rune-cookbook.md:441). |
| Collect, 780 ms, 45 ms stagger | `Stagger` from start, or per-card `enter_delay`; `Tx::unclipped_flights` if the trick area clips | The app already flies `COLLECT`. |
| Rack lift | `offset_motion(LIFT)` (already used) | Fine. |
| Deal-in, 580 ms, 75 ms stagger | `enter_delay` (already used). dde00138 at HEAD makes delayed starts wake reliably | Keep the first second still: do not start the deal in the first second after launch unless it is the intended first-run motion. |
| Confetti | `.looping(..)` per piece or `.sequence(..)` | Cheap per piece, but each looping element keeps the window drawing. Stop them when the banner leaves. The idle rule (README.md:752) requires the window to go back to drawing nothing. |
| Banners | `Transition::combined` (opacity + scale 0.96) or `El::dissolve` | Stay within scale [0.9, 1.1]. |
| Score tick | `Transition::blur_replace` or kit `RollingText` (crates/rune-kit/README.md:121) | `RollingText` rolls changed characters and is the supported "number ticks up" effect. |
| Thinking indicator | `LoopSpec` (already used) | Held static under reduced motion. |

Reduced motion: keep the app's `cx.reduced_motion(prefs || system)`. Under reduced motion matched flights do not run, so every flight must have a sensible enter/exit transition on its own elements. That is a test to add to the gate: one cell per flight with reduced motion on.

## 6. Pin-bump recommendation

Recommendation: bump to HEAD (0ea75abe) as a separate, reviewable change.

Why:
- No breaking public API changes, no removed motion API, and the only motion-path behaviour changes are fixes (delayed-start wake, stuck-shift presence).
- It brings the picture min/max layout fix, which matters if card images use percentage sizing under a max width.
- It gets the repo onto a tree where the r-round state machines are consolidated, which lowers future merge cost.

Risks and what to watch:
1. Window draw scheduling (627c2b3e). Backoff replaces three fields. The commit says the old retry rule is spelled out in a reference test, so the intended behaviour is the same. Still, this is the code that decides when a missed or occluded window redraws. Run the app, minimise, cover and restore the window, and confirm it repaints the settled state.
2. Picture layout (d8f1ffed). Any card face using `max_w`/`min_w` with a ratio could change its reserved box, so a row of cards could shift by a few points. Re-run the cookbook's card-in-rounded-element check (section 3) and the hand fan layout.
3. Flight internals (9e537376, bc0d36fc). Behaviour is meant to be identical, and `flights_tests.rs` grew by 97 lines. A regression would show as a flight that fails to start, never lands, or does not replace a previous flight on the same node. The three app flights (`play-`, `pickup-`, `collect`) are the check.
4. A Cargo.toml feature change for `windows-sys` came with the Windows occlusion commit. macOS builds are unaffected, but `cargo update`/lockfile will change. Re-check that the lock still resolves, and keep the cookbook's build-memory profile advice (07-rune-cookbook.md:99).
5. Docs drift: the reduced-motion fade is documented as 0.18 s and 0.15 s in two places at HEAD (section 2.2). No action.
6. Lockfile: the cookbook pins by rev in two lines (Cargo.toml:44-45). Both rune and rune-kit must move together. Mixed revs will break.

What apps/desktop would need to change: nothing for compilation, as far as I can tell. I grepped apps/desktop for motion API use. It uses `matched`, `looping`, `enter_delay`, `offset_motion`, `press_scale`, `tx.instant()` and `cx.reduced_motion`. None are removed or renamed. It uses no `Tx::launch`, `fly_text`, `unclipped_flights`, or `motion_gate`. I did not run a build against HEAD (read-only task, and another agent is editing apps/desktop).

Cookbook verifications that could be invalidated by the bump (re-run after the bump):
- 07-rune-cookbook.md section 5 (shared-element card motion, VERIFIED): flights are the code that was refactored. Re-run the matched-flight spike and the trace check in section 14.
- Section 5.5 (Reduce Motion, VERIFIED): flights and reduced motion interact in `launch_elements`. Re-run.
- Section 3 (PNG card in a rounded element, VERIFIED) and section 9 (felt and wood rail, VERIFIED): the picture layout change can alter reserved size. Re-run the snapshots and compare.
- Section 13 (testing with `Runtime`) and section 14 headless tooling: the `marks` module is added to `lib.rs`; trace output format is unchanged as far as the diffs show, but the trace header (07-rune-cookbook.md:936) should be diffed once.
- Section 15 gotchas 14 to 19 were "observed or read at rev 85cd8c6". The flight-rotation gotcha has no confirmation in any Rune doc; do not assume HEAD fixes it.

## 7. Suggested order of work

1. Land the current apps/desktop fix first.
2. Add `rune::motion_gate` cells for the existing flights on the current pin (it exists there). This gives a baseline.
3. Bump both Cargo.toml lines to 0ea75abe and rebuild. Re-run the baseline gate and the cookbook checks above. Diff results.
4. Add: `Tx::unclipped_flights` if any flight is clipped, `El::sequence` for the pickup arc, `Transition::blur_replace` or `RollingText` for score ticks, `El::dissolve` for banner crossfades.
5. Add reduced-motion gate cells for every flight.

## 8. Facts I could not confirm

- The Apple hitch thresholds and the CHI 2013 numbers in research/15 are [recall] by the author (research/15:65, :83).
- I did not build or run anything at HEAD.
- I did not read WEB.md, CHANGELOG.md in full, rune-perf, rune-light, rune-luau, or the www app. A skim of the site's `content.tsx`, `pages.tsx` and `FastApps.tsx` repeated the README guidance above and added no rule I found missing.
- I read the overlay pointer, clipboard, Wayland and X11 commits by subject and stat only.
