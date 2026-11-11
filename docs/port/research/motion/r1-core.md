# R1 - Rune core motion and animation API (rune + rune-motion)

Scope: every motion primitive in `crates/rune` and `crates/rune-motion`. Extends `docs/port/07-rune-cookbook.md`; it does not repeat sections 5 (matched card flights), 6 (06-ui-spec mapping), 9, 13 (Runtime tests), 14 (headless flags), 15 (gotchas). Read those first.

## 0. Revs and citation form

- **P** = pinned rev `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb`, read at `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/`.
- **H** = GitHub HEAD `0ea75abe` (26 commits newer), read from a scratch clone.
- Cites are `crate-path:line` relative to `crates/`. Bare `rune/src/...` means P. `H:` prefix means HEAD only.

### Key finding: the public motion API is the same at P and H

`diff` of these files between P and H is empty: `rune/src/el/motion.rs`, `flight.rs`, `el/tx.rs`, `el/visuals.rs`, `el/enter_delay.rs`, `live.rs`, `runtime/commit/apply.rs`, `runtime/frame.rs`, `app.rs`, and in rune-motion `transition.rs`, `spring.rs`, `tokens.rs`, `curve.rs`, `sequence.rs`, `choreo.rs`, `keyframes.rs`.

Three files differ, all internal refactors with no signature change:

| File | H change |
| --- | --- |
| `rune/src/runtime/motion/flights.rs` (P 461 lines) | Launch handling becomes a phase state machine: `enum LaunchPhase` (`H:rune/src/runtime/motion/flights.rs:21`), `Launches::step` (`:123`). A launch that takes off now drops any existing flight for the same node: `self.launches.retain(\|l\| l.airborne_id() != Some(to))` (`H:...flights.rs:146`). New `flights_tests.rs`. |
| `rune-motion/src/looping.rs` | `LoopTrack` gets a private `enum Phase` (`H:rune-motion/src/looping.rs:64`). |
| `rune-motion/src/store.rs`, `store/tracks.rs` | Track `Mode` enum with a `Due` state: a delayed start that has come due but is not yet applied wakes the store now (`H:rune-motion/src/store/tracks.rs:10,15,84-85`). A wake fix, not an API. |

Also differing at H, internal only: `rune/src/runtime/motion/unseen_presence.rs` (adds a sticky-element shift correction around `H:...unseen_presence.rs:130-142`, affecting whether a mount inside a stuck header counts as "unseen" and so lands at once), plus `runtime/commit/flatten.rs` and `text_layout.rs` (not motion, not read). Sticky headers are not used by the euchre table.

**Everything below exists at both P and H** unless marked. In particular `Tx::launch`, `Tx::unclipped_flights`, `El::matched_crop`, `El::enter_delay`, `El::paint_order`, and "an instant commit keeps running flights" are all already at P. Nothing in this note requires moving the pin. The only user-visible reasons to bump are the `Due` wake fix (a delayed track might fail to wake an idle window at P; unreproduced) and the launch-replaces-flight rule (flights.rs:146 at H).

---

## 1. Mental model

1. Rune is retained-by-diff: the view fn rebuilds an `El` tree each commit. The runtime diffs by key/path against the previous tree. A property that differs from the last commit **springs** to the new target. Nothing animates on the first frame (`rune/README.md:48`).
2. Default layout/colour spring is `tok::motion::DEFAULT = SETTLE` (0.3 s, bounce 0) (`rune-motion/src/tokens.rs:18,41`). Not `Spring::DEFAULT` (0.5 s) (`rune-motion/src/spring.rs:152`); the two names differ.
3. Per-property curve overrides exist for offset, opacity, scale, rotate, blur (all `*_motion(impl Into<Anim>)`). Per-commit overrides: `Tx::motion`, `Tx::motion_curve`, `Tx::color_motion`.
4. Appearing/disappearing elements play a `Transition`. Default is a fade.
5. Two systems run off the **render clock** with no view commits: `Live<T>`/`Tween` bindings and `LoopTrack` (loops, sequences). These are what you want for per-frame or free-running motion.

---

## 2. Springs, curves, easing, Anim

### Spring (`rune-motion/src/spring.rs`)

```rust
Spring::new(duration: f32, bounce: f32) -> Spring   // const; panics if duration <= 0 or bounce not in (-1, 1)   spring.rs:30
```
- `duration` = perceptual duration, seconds (undamped period). `bounce`: `0` critically damped, `> 0` overshoots, `< 0` overdamped (`spring.rs:8-18`). omega = 2pi/duration, zeta = 1 - bounce.
- Not CSS: motion.dev's `visualDuration` = this duration / 1.2. Use `Spring::from_motion(visual_duration, bounce)` (`spring.rs:58`) to port numbers from motion.dev.
- Other constructors: `from_response(response, damping_fraction)` `:37`, `from_omega_zeta` `:42`, `from_stiffness_damping(k, c, mass)` `:49`. Readers: `settling_duration(eps)` `:111`, `sample(x0, v0, target, t)` `:103`.
- Presets: `NONE` (1e-6 s, lands at once) `:141`, `SMOOTH` 0.5/0 `:146`, `SNAPPY` 0.5/0.15 `:148`, `BOUNCY` 0.5/0.3 `:150`, `DEFAULT` 0.5/0 `:152`; `smooth/snappy/bouncy(duration, extra_bounce)` `:155-163`.
- `SpringPair::new(enter, exit)` `:182`, `with_exit_ratio(enter, ratio)` `:188`.

### Tokens (`rune-motion/src/tokens.rs`, re-exported as `rune::tok::motion`)

| Token | enter (dur, bounce) | exit |
| --- | --- | --- |
| INSTANT | 0.10, 0 | 0.10 |
| SNAP | 0.25, 0 | 0.20 |
| SETTLE (default) | 0.30, 0 | 0.24 |
| REFLOW (matched/launch default) | 0.40, 0 | 0.32 |
| GLIDE | 0.32, 0 | 0.26 |
| MORPH | 0.30, 0.16 | 0.27 / 0.08 |
| POP | 0.25, 0.15 | 0.20 / 0.05 |
| DRIFT | 0.85, 0 | 0.60 |
| CAMERA | 1.20, 0 | (none) |

Lines `tokens.rs:12-36`. Pair consts in `pub mod pair` `:66-73` (e.g. `pair::POP`). Aliases `QUICK/SNAPPY=SNAP`, `CALM=REFLOW`, `SMOOTH=GLIDE`, `PLAYFUL=POP`, `SLOW=DRIFT` `:39-45`. `REDUCED_FADE` curve `:48` (0.15 s). `CSS_ENTER` tween 0.15 s `:50`, `CSS_EXIT` 0.11 s `:52`.

**Euchre note:** only POP, MORPH have bounce. Bounce is the only overshoot a spring gives; for a "badass" landing slam use `Spring::new(0.35, 0.3)` or a tween with `Easing::OVERSHOOT` (below).

### Curve / Anim (`rune-motion/src/curve.rs`)

```rust
enum Curve { Spring(Spring), Tween { duration: f32, easing: Easing }, Decay(Decay), Instant, /*hidden*/ Hermite{..} }   curve.rs:8
Curve::tween(d, easing) / Curve::linear(d) / Curve::ease_in_out(d)     :23-29
struct Anim { curve, delay: f32, blend: Blend, velocity: Velocity, token: u64 }   :112
Anim::INSTANT   :127
impl From<Spring|Curve|SpringPair> for Anim
trait AnimExt (blanket): .delay(s) .blend(b) .additive() .merge() .velocity(v) .completion(token)   :155-175
```
- Blend: `Auto` (springs/decay merge, tweens add), `Merge` (restart from current pos+vel), `Additive` (stack a new delta segment) (`curve.rs:86-97`). Velocity: `Inherit` (default, interruptible), `Zero`, `Uniform(f32)` `:100-108`.
- `Anim.delay` = seconds before the new target takes effect; a newer animate on the same value cancels the pending one (`curve.rs:112-116`).
- `Anim.token != 0` emits `MotionEvent::Completed { token }` on settle or replace. That event is on the `AnimationStore` (rune-motion level); I found no route from `rune::Tx` to receive it, so treat it as engine-internal for an app.
- `Curve::Spring` retargets merge (keep velocity), `Tween` retargets add (Core Animation) (`curve.rs:9-12`).

### Easing (`rune-motion/src/easing.rs`)
`Easing::{LINEAR, EASE, EASE_IN, EASE_OUT, EASE_IN_OUT, ENTER, EXIT, OVERSHOOT, DRAW_ON}` `:26-38`; `Easing::cubic(x1,y1,x2,y2)` `:41`. `OVERSHOOT = cubic(0.34, 1.45, 0.64, 1.0)` is the only built-in easing that exceeds 1.0: use it in `Curve::tween` for a settle-with-overshoot that is exact-duration (springs are not). `StepPosition` enum `:18` for stepped easing.

### Snippet: per-property curves

```rust
use rune::{AnimExt, Curve, Easing, Spring};
el().offset(0.0, if lifted { -14.0 } else { 0.0 })
    .offset_motion(Spring::new(0.22, 0.25))                       // bouncy lift
    .scale(if lifted { 1.06 } else { 1.0 })
    .scale_motion(Curve::tween(0.18, Easing::OVERSHOOT))
    .opacity_motion(Curve::ease_in_out(0.2).delay(0.05))          // delay is an Anim field
```
(Setters: `offset_motion` `rune/src/el/motion.rs:122`, `scale_motion` `el/visuals.rs:369`, `rotate_motion` `:389`, `blur_motion` `:256`, `opacity_motion` `el/motion.rs:153`.)

**Gotcha:** an `*_motion` curve with its own velocity "would restart each commit", so opacity only re-animates when the target value changes (`runtime/commit/apply.rs:530-538`, comment at the opacity branch). Do not rely on re-issuing an identical target to retrigger.

---

## 3. What animates implicitly on a property change

All driven in `rune/src/runtime/commit/apply.rs`. Property -> curve used:

| Property (El setter) | Animates? | Curve | Apply site |
| --- | --- | --- | --- |
| layout rect (x, y, w, h) | yes, per edge | layout spring (inherited `El::motion`, else `Tx::motion`, else SETTLE); `El::lead(lead, trail)`, `El::width_leads()` change edge timing | apply.rs:441-502 |
| corner radius | yes | layout spring | apply.rs:~511 (`set_or_anim(.., prop::RADIUS ..)`) |
| `opacity` | yes (unless `live_opacity` bound) | `opacity_motion` if set, else layout spring | apply.rs:~512-526 |
| `offset(x,y)` | yes (unless `live_offset` or a sliding loop owns it) | `offset_motion`, else layout spring | apply.rs:~530-560 |
| `scale(k)` | yes | `scale_motion` else layout spring | apply.rs:~566 |
| `rotate(rad)` | yes | `rotate_motion` else layout spring | apply.rs:~570 |
| `blur(sigma)` | yes | `blur_motion` else layout spring | apply.rs:~575-583 |
| fill, border, text colour | yes, in premultiplied OKLab; text colour leads fill by 0.5x duration | colour spring (`Tx::color_motion`, else layout) | apply.rs:~585-600; README:41-42 |
| shadows (up to 4: colour and x/y/blur/spread) | yes | layout spring | `runtime/motion/props.rs:9` (`P_SHADOWS`), `:68` (`P_SHADOW_GEO`) |
| trim (`live_trim`), icon turn | yes / via live | | `props.rs:66`; apply.rs `P_ICON_TURN` |

Re: line numbers marked `~`: they are within the 509-600 block I read; exact lines drift by a few. The call names are exact.

Rules that matter:
- `rest_quiet = lands || !was_present || was_exiting || reduced_motion` (apply.rs:~527): a node that just mounted, or is coming back from exit, **snaps** its offset/scale/rotate to the target rather than animating them. Its entrance is the transition's job. So to animate a *new* element's scale/offset you need `transition(...)`, or `sequence(..)`, or a `matched` flight, not a plain prop.
- `Spring::NONE` on `El::motion` lands this element's own changes at once and marks that as authored (`rune/src/el/motion.rs:89-93`).
- Two commits in a row retarget from the current pose+velocity (merge blend), so rapid state flips are smooth.

---

## 4. Presence: enter/exit

### `El::transition` (`rune/src/el/motion.rs:196`)
```rust
pub fn transition(self, t: Transition) -> Self
```
Default `Transition::opacity()` = fade in 0.2 s / out 0.15 s via `pair::SETTLE` (`rune-motion/src/transition.rs:198,166-170`; doc `el/motion.rs:188-196`). `pop_layout = true` default: an exiting node leaves layout flow at once (siblings reflow while it fades in place) (`transition.rs:160-163`).

### Constructors (`rune-motion/src/transition.rs`)
| Fn | Line | Effect |
| --- | --- | --- |
| `identity()` | 186 | no visual change but still holds exit until settle |
| `none()` | 194 | no motion at all, survives Reduce Motion (does not turn into a fade) |
| `opacity()` | 198 | fade |
| `scale(k)` / `scale_anchor(k, Vec2)` | 202 / 206 | scale about centre / anchor + fade |
| `offset(x, y)` | 210 | translate in points |
| `move_edge(Edge)` | 214 | in from / out to edge by own size |
| `push(Edge)` | 218 | in from edge, out through opposite, fading |
| `slide()` | 223 | in from leading, out trailing |
| `blur_replace()` | 231 | sigma 1.5, scale 0.96, fade (a whole-element swap) |
| `blur(sigma)` | 235 | blur + fade |
| `draw()` | 242 | canvas strokes trace on (own shape only) |
| `reveal(Edge)` | 246 | soft-edge wipe, 24 pt feather |
| `asymmetric(ins, rem)` | 250 | different in/out |
| `combined(other)` | 254 | multiplies opacity and scale, adds offsets, max blur (`Effect::combined` `:118`) |
| `.springs(SpringPair)` | 261 | enter/exit springs, default `pair::SETTLE` |
| `.allow_small_scale()` | 269 | lets scale go to 0.5 floor |
| `.hold_layout()` | 275 | exiting node keeps its slot instead of popping out |

`Edge::{Leading, Trailing, Top, Bottom}` `:13`. `Effect` fields (what an *absent* node looks like): `opacity, scale, anchor, offset, offset_fraction, blur, reveal, small_scale, draw` (`:53-84`).

### Clamping (kit rules) - confirms the open item
`Effect::kit_rules(removal)` (`transition.rs:148-157`) is applied to **every** presence transition: `scene/presence.rs:63,153,236,259`. It clamps scale to [0.9, 1.1] (floor 0.5 only with `allow_small_scale`), forces blur to 0 if the effect travels (offset/offset_fraction/reveal), clamps blur <= 4, clamps opacity/draw to [0,1], and **forces removal opacity to 0** (`:154-156`). Consequences:
- `Transition::scale(0.0)` becomes 0.9. `Transition::scale(1.3)` becomes 1.1. A big "pop in from nothing" needs `Transition::scale(0.5).allow_small_scale()`, and still never reaches 0.
- You cannot exit at full opacity (e.g. a card that slides off but stays opaque): removal opacity is forced to 0, so it fades. If the card must stay opaque while flying away, move it with `matched` into another element instead of removing it.
- Blur never combines with offset in a transition ("sliding content stays sharp").
- Matched/launch flights and plain props are **not** clamped (kit_rules only runs in presence.rs). `El::scale(2.0)` works.

### Snippet
```rust
use rune::{Transition, Edge, tok, Stagger};
// deal-in: rises 16pt, fades in, per-card delay; exit slides off right
let t = Transition::asymmetric(
    Transition::offset(0.0, 16.0).combined(Transition::opacity()),
    Transition::push(Edge::Trailing),
).springs(tok::motion::pair::POP);
card.transition(t).enter_delay(Stagger::new(0.075).delay(i, n))
```

### Gotchas
- Every element has the default fade, so an animated commit that removes an element fades it out over 0.15 s (`el/motion.rs:188-193`). The removed element stays drawn and **inert** until it settles. `Tx::instant()`/`settle()` unmount at once. For elements that must vanish this frame (e.g. a card that already flew via `matched`) use `Transition::none()`.
- Rows inserted into a **growth stack** only grow open unless they set their own transition (`el/motion.rs:192-194`); virtual-list remounts never replay it.
- A removed node that is re-added while exiting is revived mid-flight (README:43-44), not restarted.
- Presence phase is exposed as `rune::PresencePhase` (`lib.rs:274`) and `ElementInfo.exiting` (`runtime/elements.rs:17`). Lookups by key treat exiting nodes as gone.

---

## 5. Stagger, delays, choreography

| Primitive | Sig | Notes |
| --- | --- | --- |
| `El::enter_delay(seconds)` | `rune/src/el/enter_delay.rs:36` | holds the **whole** entrance at the insertion effect (offset/scale/blur/opacity/reveal all hold together) then plays on the transition's enter spring. Fresh mount only. Ignored for first frame, `instant`, `settle`, hidden window, out-of-sight, and revive-while-leaving. Kept under Reduce Motion (becomes the fade, same timing). Changing it after mount does nothing. Runs on the element clock, so `time_scale` stretches it. |
| `Stagger::new(gap)` | `rune-motion/src/choreo.rs:35` | `.from(StaggerFrom::{First,Last,Center,Index(n)})` `:38`, `.start(s)` `:42`, `.ease(Easing)` `:46`, `.delay(index, count) -> f32` `:52`. Pure arithmetic; free fn `stagger(i, n, gap, from)` `:71`. Re-exported `rune::Stagger`, `rune::StaggerFrom`. |
| `Anim::delay(s)` | `curve.rs:156` | delays one property change (`opacity_motion`, `offset_motion`, ...). |
| `Choreography`, `Cue`, `PhaseAnimator` | `choreo.rs:192,92,297` | cue-based sequences over an `AnimationStore`. **rune-motion level only**: not re-exported from `rune` (`lib.rs:259-286` re-exports Anim, Beat, Blend, Clock, CubicBezier, Curve, Decay, Easing, Edge, Effect, LayoutAnim, LoopKind, LoopSpec, NodeId, PresencePhase, Presented, Reveal, Sequence, Spring, SpringPair, Stagger, StaggerFrom, StepPosition, Transition, Travel, Vec2, Velocity). `Keyframes<T>` (`keyframes.rs`) is the same. An app would need `rune_motion` as a direct dep and has no handle to the runtime's `AnimationStore`, so treat these as unusable from `apps/desktop`. Use `Sequence`/`LoopSpec`/`enter_delay`/`wake_after` instead. |

Gotcha: `enter_delay` x `matched`. A matched arrival is inserted through `insert_delayed(.., enter_delay)` (`runtime/commit/apply.rs:401`) and then `match_geometry_offset` runs (`flights.rs:~200`). I did **not** verify whether the delay also holds the flight start. Test before relying on it (recipe in section 14). Fallback for a staggered collect: stagger **commits** with `Tx::wake_after` and move one card per poll.

---

## 6. Shared-element flights (matched / launch)

Cookbook 5 covers basic `matched`. Additional facts:

### El::matched (`rune/src/el/motion.rs:215`) - semantics from source
- Matching happens in `Runtime::match_elements` (`rune/src/runtime/motion/flights.rs:163-221` at P; same logic in the H refactor). Requirements per pair: old node with id X **not present in the new tree** (`self.node_index.contains_key(&from)` skips it, `:177`), not already exiting, and a new node with X that **arrived** this commit (`arrivals` flag `!was_present || was_exiting`, `apply.rs:370`). So the same keyed element that merely moves inside one parent is *not* a flight; it is ordinary layout motion on its own spring.
- Carries across: drawn rect and velocity, corner radius, **fill colour** (`:196-199`) and **opacity** (`:200-203`) of the old node. Does not carry scale or rotation (not read in that block: inferred, unverified in practice).
- Spring: `Tx::motion` > the element's own `El::motion` (stored in `nodes[i].matched.1`) > `tok::motion::REFLOW` 0.4 s (`:190`). So a **per-element spring is a free way to vary flight duration per card** without separate commits.
- While flying: drawn above the whole window at its own opacity, **inside its destination's clips** by default; children keep final layout scaled uniformly to cover the moving frame and are clipped (so images zoom, never distort). Text keeps its size. (`el/motion.rs:204-214`)
- All descendants of the arrival are `enter_now`'d (`flights.rs:~208-212`): no inner fades while it flies.
- Last duplicate id in tree order wins; `warn_duplicate_match` logs (`flights.rs:~167`).
- Under Reduce Motion, `instant`, or `settle`: **no match at all** (`flights.rs:~172`); old exits, new enters by their own transitions.

### El::matched_crop (`el/motion.rs:224`)
Children keep their final layout at **own size** from the frame's leading edge, centred in height, cropped, instead of scaling to cover. For rows whose width changes during flight. For a card whose face is one `Pixels` image, leave it off (cover-scale is what you want).

### Tx::unclipped_flights (`rune/src/flight.rs:36`)
`tx.unclipped_flights()` makes this commit's matched flights draw outside every clip until they land. **Needed when a card flies out of a clipped container** (e.g. a hand fan inside a rounded felt that has `clip`, to a trick area outside it). Without it the card is cut at the clip for most of the path.

### Tx::launch (`flight.rs:28`) and Tx::fly_text (`:16`)
```rust
tx.launch(source: impl Into<String>, destination: impl Into<String>)
tx.fly_text(source, destination)
```
Both are **text-sourced**: the destination's first text starts on the source's text, at its own size, and travels one straight path on REFLOW (or `Tx::motion`), drawing above the window and outside every clip. Keys or key paths (`"msg-42/body"`, `flight.rs:103` `selects`). Does nothing under Reduce Motion or if the destination was already mounted. Created for chat sends (test: `rune/tests/launch.rs:1-30`). **Not useful for card images** (it launches *text*). For a face-down-card-to-trick flight use `matched`. Possible use: score or "Trump: Hearts" chip flying from the call banner to the scoreboard.
At P vs H: identical public API; H adds the phase state machine and the replace-existing-flight rule (`H:...flights.rs:146`). At P a launch for a node that is already in a `matched` flight is not specified in what I read; avoid pointing both at the same node.

### Flight lifecycle gotchas
- Flights survive `tx.instant()` if the instant change leaves the flying node's own layout alone (`apply.rs:~455-458`, `keeps_flights`). A re-layout of the flying node retargets it: `retarget_drawn` with the drawn pose and speed (`:~484-490`), so a resize mid-flight re-aims rather than restarts.
- `Runtime::elements()` reports `moving = true` for a node while it has a flight (`runtime/elements.rs:~210-215`) and substitutes the flight's presented rect: assert on `moving` to detect "still flying" in tests.
- `El::paint_order(i16)` (`el/motion.rs:142`) orders siblings like CSS z-index, stays inside the parent's layers, takes the pointer first. `El::raise()` (`:132`) paints above the whole window and takes pointer first. For trick stacking use `paint_order` (hand cards fanning over each other); use `raise` only for a dragged card.

---

## 7. Transforms, opacity, shadow, blur, colour

| Need | API | Notes |
| --- | --- | --- |
| translate without layout change | `El::offset(x, y)` `el/motion.rs:113` | hit tests follow; ignores `origin`; ancestors' offsets add. Animates unless `live_offset`/loop owns it |
| uniform scale | `El::scale(k)` `el/visuals.rs:364` | about `origin`; layout keeps unscaled size; text re-rasterizes crisp after rest |
| 2D rotate | `El::rotate(radians)` `:381` | clockwise about `origin`; subtree draws into **its own layer** while turned (cost, section 12); hit test follows the turned box; reported rect = upright bounds |
| pivot | `El::origin(x, y)` `:399` | unit coords of own size; `(0.5, 1.0)` = bottom-centre. Applies to scale, rotate, `live_scale`, `press_scale`, transition scale. Not offset |
| press feedback | `El::press_scale(k)` `:350` | iOS-style 0.96-0.97; spring back; off under Reduce Motion; put on the element with `on_press` |
| hover fades | `El::hover_motion(enter, exit)` `:517`, `hover_motion_within` `:529`, `group_opacity(rest, hovered)` `:538` | |
| subtree opacity | `El::opacity(o)` `:236` | partial opacity creates a **group layer** (cost) |
| per-leaf dim | `El::dim(alpha)` `:266` | static (not animated) dim of each paint leaf; no group layer; overlapping draws keep coverage; call after adding children |
| blur | `El::blur(sigma)` `:248` | sigma in points; zero costs nothing; renderer caps at 64 device px; bounds grow 3 sigma |
| shadow | `El::shadow(y, blur, color)` `:139`, `El::box_shadow(Shadow{x,y,blur,spread,color,inset})` `:148` | `blur = 2*sigma` (CSS). Up to 4 shadows; colour and geometry animate |
| fill gradient | `El::gradient(top, bottom)` `:153` | |
| z-order | `paint_order`, `raise` | section 6 |

### Not in Rune (verified by grep of el/visuals.rs, rune-motion props)
- **No 3D**: no perspective, `rotate_x/y`, skew, matrix transform, or card-flip primitive. Search for `perspective|rotate_x|skew|flip` finds nothing in `rune/src` visuals or `rune-motion/src/props.rs`.
- **Scale is uniform only** (one `f32`): no `scale_x`. A fake flip via scaleX-to-0 is **impossible**. Substitutes: (1) crossfade back->face with a brief uniform-scale dip using `El::sequence` (`Beat::new(0.0).scale(1.0)`, `Beat::new(0.12).scale(0.92).opacity(0.0)`, ...) on two stacked images; (2) `Transition::reveal(Edge::Leading)` to wipe the face in over the back (24 pt feather, `transition.rs:246`); (3) swap the two images under a `rotate` of a few degrees for a "snap" feel.
- **No keyframe API on `El`** beyond `sequence` (opacity+scale only) and `looping`. `Keyframes<T>` exists only at rune-motion level (`keyframes.rs`) with no runtime handle (section 5).
- Rotation animates on any element via `rotate`; the loop `Rotation` kind (below) is for icon quads only.

### Colour animation
Fill/border/text colour changes spring automatically between commits. `Tx::color_motion(Spring)` (`el/tx.rs:558`) sets that commit's colour spring (a theme change uses CALM = REFLOW). Colours are interpolated in premultiplied OKLab (README:41).

### Snippet: card hover-lift with shadow and press
```rust
el().key(k).on_press(|s: &mut S, tx| { s.play(id); })
    .offset(0.0, if hot { -10.0 } else { 0.0 })
    .scale(if hot { 1.05 } else { 1.0 }).origin(0.5, 1.0)
    .shadow(if hot { 14.0 } else { 3.0 }, if hot { 28.0 } else { 6.0 }, shadow_col)
    .press_scale(0.97)
    .motion(Spring::new(0.28, 0.15))       // inherited by this subtree; drives offset/scale/shadow
```

---

## 8. Loops and finite sequences (render-clock, zero commits)

### El::sequence / El::looping (`el/motion.rs:259,264`)
```rust
pub fn sequence(self, frames: Sequence) -> Self          // = looping(LoopSpec::sequence(frames))
pub fn looping(self, spec: LoopSpec) -> Self
```
- `Beat::new(at_seconds).opacity(f).scale(f).easing(Easing)` (`rune-motion/src/sequence.rs:14-17`). A beat is a **full pose** at `at`; omitted values are 1.0 (not "unchanged"). `Beat.easing` shapes the segment *arriving* at that beat (default EASE_IN_OUT).
- `Sequence::new(&'static [Beat])` (`:29`): must be `'static` (so use a `const`), >= 2 beats, first at 0.0, strictly increasing times, scale >= 0. Panics otherwise (`:30-31`). `.delay(secs)` `:35`; `.duration()` `:39`.
- Holds the final pose, then sleeps. An unchanged spec keeps progress across commits; **a changed spec or a new element key replays it** (README:1093). Reduce Motion and `settle` land the final pose at once.
- Only **opacity and uniform scale**. No offset/rotation in a Sequence.
- Because `'static`, you cannot build a sequence with runtime numbers (e.g. a different delay per card). Options: a small fixed `const` table of N variants, or put per-card timing in `Sequence::delay` (also takes a runtime `f64`: `Sequence::new(CONST).delay(i as f64 * 0.045)` works because only the frames slice is static).

### LoopSpec (`rune-motion/src/looping.rs:20-50`)
```rust
LoopSpec::sequence(Sequence)                 :27
LoopSpec::cycle(Cycle)                       :30   // wall-clock opacity polyline: Cycle{period, phase, stops:&'static [(f32,f32)], still, fps}  cycle.rs:8
LoopSpec::opacity(period, delay, low, high)  :33   // const
LoopSpec::rotation(period)                   :36   // constant spin
LoopSpec::slide(period, from:[f32;2], to:[f32;2]) :40  // eased sweep that wraps, in fractions of own size; descendants + hit tests follow
.easing(Easing)                              :45   // slides only; Easing::LINEAR = shimmer
```
- Loops with the same period stay in phase (shared clock) (README:1066-1068). While **only** loops move, the window wakes at the cycle's `fps`, not every display frame, and a still window does no work (`cycle.rs:3-6`).
- `LoopKind::Rotation`: README names rotation as an icon-quad feature; I did not find it spinning an arbitrary subtree. Use `El::rotate` driven from state/`Live` for a non-icon element.
- `rotate_motion` etc. are ignored while a loop owns the property (`apply.rs:~540`, `nodes[i].looping.is_none()` guard).
- `LoopTrack` API (`looping.rs:54-160`: `phase, retime, enabled, shift, advance, finish, active, pending_start, frame_interval, sample, pose`) is engine-internal; `ElementInfo.loop_phase` (`elements.rs:10`) exposes the phase to tests.

### Snippet: confetti / score "bump"
```rust
use rune::{Beat, Sequence, Easing, LoopSpec};
const BUMP: &[Beat] = &[
    Beat::new(0.00).opacity(0.0).scale(0.6),
    Beat::new(0.18).scale(1.25).easing(Easing::OVERSHOOT),
    Beat::new(0.40),
];
el().key(format!("score-{}", s.score))              // new key replays it
    .sequence(Sequence::new(BUMP).delay(0.05))
```
A **keyed-by-value** element (`score-3` -> `score-4`) replays on every change: the idiom for "pop on change".

---

## 9. Live values and Tween (no view commit)

`rune/src/live.rs`, `el.rs:368-398`.

```rust
Live<T: LiveValue>   // T in { f32, [f32;2], [f32;4], Color }  (live.rs:102-125)
Live::new(v)  .set(v) -> bool   // lock-free, thread-safe, coalesced wake; equal bits don't wake  :142,152
.snap(v)                         // lands at once, cancels running/delayed motion :170
.pulse(active, rest, hold: Duration)  // go to active, hold while window sleeps, then rest :181
.get()  .smooth(Spring) -> Binding<T>   // interruptible spring on the render clock :191,193
Binding::rest(Spring)            // spring for returning to rest / moving down :205
Tween::<T>::linear(from, to, Duration) -> Tween<T>   // live.rs:16
```
Bind with `El::live_opacity / live_blur / live_offset / live_scale / live_trim / live_fill / live_rect` (`el.rs:368-398`). Pass `&Live` (raw) or `live.smooth(spring)`.

- `Tween::linear`: render-time linear track; starts on **first attachment to each runtime**; rebuilds and remounts keep elapsed time; replacing the `Tween` handle restarts it. Needs no ticks, no `wake_after`, no layout. Hidden windows stop sampling and resume at the elapsed value. **Not affected by `settle` or Reduce Motion** (README:1274-1276, "a timed value"). Uses the window clock (not `El::time_scale`). Use for a turn timer ring or countdown bar (`live_rect`/`live_trim`).
- `Live` is the right tool for an **animation driven by an async/bridge thread** (e.g. rivetkit event) or per-frame values: `live.set()` from any thread wakes the window without a commit and without a view re-run.
- Caveat: `live_*` bindings **own** the property. The normal `offset/opacity/scale/blur` setter is skipped for that node (`apply.rs:512,527,577`), including across unrelated commits.
- Debug builds warn if a handler calls `Tx::live_only()` but state the view reads changed (`el/tx.rs:485-492`). Do not mix `live_only` with view-visible state.
- `El::cache()` plus `live_offset`/`live_scale` on an ancestor moves a cached raster without re-emitting draws (`el/visuals.rs:296-303`): use for panning a whole static table layer.
- At H: unchanged.

---

## 10. Layout animation and growth

- Default: moved/resized elements spring to the new rect **per edge** (stationary edges never move), on the inherited layout spring (README:36). `El::lead(lead, trail)` / `width_leads()` skew edge timing (`el/motion.rs:159-165`).
- `El::animate_layout()` (`:172`): spring horizontal layout inside a growth-stack row (rows otherwise snap and the row height carries the motion).
- `El::hold_landing_width()` (`:235`): on an animated width change the element and its contents take the final layout immediately (text wraps once) and the whole thing slides as one.
- `El::instant_changes()` (`:181`): this subtree lands changes at once even in an animated commit; own entrance/exit and colours still animate.
- `Tx::follow_edge()` (`el/tx.rs:548`), `Tx::relayout` (growth). Not covered here.
- **Reordering** (hand re-sort after a play, a dragged card with others stepping aside): rows slide aside via `offset` springs (`el/motion.rs:100-112` doc; `apply.rs:~540-556` `reorder_jump`/`slide_reordered`). A keyed reorder inside one parent animates by default; use stable keys per card id.

---

## 11. Per-commit controls, wake model, time

### Tx (`rune/src/el/tx.rs`)
| Method | Line | Effect |
| --- | --- | --- |
| `instant()` | 513 | no animation; mounts/unmounts at once; **also suppresses matched flights and launches**; keeps running flights not re-laid-out |
| `animated()` | 523 | cancels an earlier `instant()` in this handler (last call wins); does not override `settle()` |
| `settle()` | 553 | instant + lands every running track |
| `motion(Spring)` | 527 | layout spring for this commit; clears `motion_curve` |
| `motion_curve(impl Into<Anim>)` | 536 | any curve (tween ok); matched flights/scrolls only take a zero-bounce spring of the curve's duration |
| `color_motion(Spring)` | 558 | |
| `wake_after(Duration)` | 226 | wake `poll` after unscaled duration; due deadlines collapse into one poll/frame; schedules a frame, not a commit |
| `wake_at(f64)` | 242 | same, absolute `Tx::clock` seconds |
| `clock()` | 234 | logical seconds, virtual headless |
| `live_only()` | 493 | no view re-run (heuristic debug check) |
| `scroll_to_with(key, x, y, Spring)` | `el/tx/scroll.rs:93` | animated scroll with spring; `scroll_to` `:79`, `_instant` `:99`, `scroll_to_bottom` `:45` |

Automatic `instant`: typing in a text input, focusing on pointer-down, drag moves (README:35 area; `tx.rs:513-521` doc). `on_key`, `on_submit`, `on_press` commits animate by default.

### Poll and wake
- `AppSpec::poll: Option<fn(&mut S, f64, &mut Tx) -> bool>` (`rune/src/tooling.rs:35`; runtime field `runtime.rs:436`). Return `true` only when state the view reads changed; `false` leaves an idle window idle (README:738-741: 77 presents/20 s idle -> 0).
- `wake_after(Duration::ZERO)` from poll = "next frame". Needs a display link (macOS). Hidden/covered/unpaced: a wake sooner than a frame waits ~1 frame (`tx.rs:221-225`).
- `AppHandle` (thread-safe): `wake()` `rune/src/app.rs:645`, `update(f)` `:750`, `update_if(f -> bool)` `:762`. `update` closure runs on the UI thread in order, "the next frame commits it with the motion its `Tx` asks for", so you can call `tx.motion(..)`/`tx.unclipped_flights()` inside `update` (the cookbook bridge uses this).
- **Nothing animates on the first frame.** Prime state, call `rt.frame(0.0)` (tests) then change state.

### Time scale / Reduce Motion
- `Cx::time_scale(k)` (`runtime/cx.rs:266`): window clock scale, clamped [0, 8], 0 pauses.
- `El::time_scale(k)` (`el/visuals.rs:419`): subtree clock; nested multiply; frozen subtree requests no frames; scroll, growth and `Tween` ignore it.
- Reduce Motion: `Cx::system_reduced_motion()` host pref, `Cx::reduced_motion(bool)` overrides **this commit only** (`runtime/system_settings.rs:~70`; README:1226-1229); `Runtime::set_system_reduced_motion(bool)` (`system_settings.rs:65`) for tests. Effects: spatial travel snaps; opacity/presence/colour take `REDUCED_FADE` 0.15 s; matched/launch disabled; `press_scale` off; `scale/rotate/offset` land at once; `Sequence` lands its final pose; `enter_delay` kept (becomes fade); `Transition::none()` stays instant; `Tween` and `Live::smooth` unaffected.

---

## 12. Per-frame cost (from docs and code; **not measured by me**)

| Feature | Cost note | Source |
| --- | --- | --- |
| Settled tree | no frames requested; windows draw nothing when idle | README:738-741 |
| `Live`/`Tween`, loops, sequence | no commit, no layout, no timer thread; loop-only windows wake at `fps` | `live.rs:8-12`; `cycle.rs:3-6` |
| `blur(sigma)` | zero cost at 0; Gaussian pass, bounds grow 3 sigma, capped 64 device px; nested blur / dissolve / native video fall back | `el/visuals.rs:246-249`; README:2220-2231 |
| `rotate` | subtree draws into own layer while turned | `el/visuals.rs:376-380` |
| partial `opacity` | composites as a group (layer); static `El::dim(alpha)` per-leaf dim avoids it but does not animate | `el/visuals.rs:236`, `:266` |
| `cache()` | rasterizes a settled subtree once, composites a quad; **draws live while any spring/transition/loop/flight inside is active**, re-caches on rest | `el/visuals.rs:296-308`; README:219-228 |
| text under scale | rasterizes at scaled size once resting; re-crisps ~0.1 s after motion stops | README:~216-218 |
| matched flights | zero cost without matched elements; else O(matched) plus one pass over old nodes when a pair matches | `flights.rs:160-162` |
| view commit | whole view fn re-runs on any state change; memo reuse exists (`runtime/commit/memo_reuse.rs`) | |
| new window | held to two drawables until it rests / first second | README:~731-733 |

Euchre guidance: keep the card layer's blur at 0 except one-shot moments; avoid animating `blur` on 25+ elements; use `cache()` on the static felt/wood rail so only moving cards redraw (matches cookbook 9 felt/rail); prefer `Live` + `Tween` for the turn timer.

---

## 13. Scroll-linked motion

- `El::on_scroll_offset(Fn(ScrollPos))` (`el/layout_watch.rs:148`): callback cannot change state; set `Live` values from it (header shrink, shadow fade-in; doc example at `:140-146`).
- Sticky: `sticky_top(inset)` `el/sticky.rs:46`, `sticky_top_showing(show)` `:57`, `sticky_bottom` `:64`, `sticky_left` `:78`.
- Scroll glide: `Tx::scroll_to_with(key, x, y, spring)`.
- Fade edges: `scroll_edge_fade(w)` `el/scroll.rs:115`, `scroll_fade` `:123`.
- Not relevant to a fixed-viewport euchre table beyond the score history/log pane.

---

## 14. Headless time and tests (extends cookbook 13/14)

Facts specific to motion:
- `Runtime::new(state, view, (w, h), Env::new(dark, scale))` (`runtime.rs:820`). Real virtual clock: `rt.frame(dt: f64) -> bool` (`runtime/frame.rs:305`) advances `dt` seconds, runs handlers, commits if dirty, steps the scene. Returns whether the last frame went stale (a redraw was warranted): `assert!(!rt.frame(..))` proves **idleness** (`tests/enter_delay.rs:68-72`).
- Always `rt.frame(0.0)` first (first frame never animates), then `rt.update(|s, tx| ..)` (`frame.rs:128`; closure gets `&mut Tx`, so `tx.motion(..)` works), then loop `rt.frame(1.0/120.0)`.
- `rt.update_if(f -> bool)` `:142`. `rt.settle_all()` `:297` lands everything.
- Waits: `wake_after` deadlines are honoured by headless steps; "coarse headless steps stop at deadlines at least a frame interval apart" (README:465). Advance by the deadline (`rt.frame(wake - 1/HZ)` then one frame) as `tests/enter_delay.rs:75-77` does.
- Reads: `rt.elements()` (`runtime/elements.rs:201`) returns `ElementInfo { key, path, rect (drawn), target, opacity, scale, blur, exiting, loop_phase, moving, .. }`; `rt.presented(key) -> ([x,y,w,h], opacity)` `:402`; `rt.presented_pose(key)` `:493` returns rotation, scale, shadows and whether any spring still moves. Opacity is effective (ancestors included).
- `rt.set_system_reduced_motion(true)` (`system_settings.rs:65`) then drive the same script to assert snapping.
- `Cx::time_scale(0.0)` freezes mid-flight for a deterministic probe.
- Patterns to copy: `rune/tests/launch.rs`, `matched.rs` (`:34` flight between parents), `enter_delay.rs`, `expressive_motion.rs`, `motion_trace.rs`.
- Headless CLI (cookbook 14): `--snap --at T`, `--frames N --every K --strip`, `--tree [--at T]` with `moving`, `--trace run.jsonl` (README:2422-2438).

### Recipe: assert a matched flight with a stagger delay (the unverified item in section 5)
```rust
let mut rt = Runtime::new(state, view, (960.0, 640.0), Env::new(false, 2.0));
rt.frame(0.0);
rt.update(|s, tx| { s.start_collect(); /* tx.unclipped_flights(); */ });
let mut starts = vec![];
for step in 0..120 {
    rt.frame(1.0 / 120.0);
    for e in rt.elements().iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("won-"))) {
        if e.moving { starts.push((e.key.clone(), step)); }
    }
}
// If enter_delay holds flights, first-moving step differs per card.
```

---

## 15. Euchre-specific recommendations

Mapped to 06-ui-spec timings (cookbook 6 already maps the base cases). Additions that the cookbook did not have:

| Wanted effect | Primitive | Availability |
| --- | --- | --- |
| Card-play slam (overshoot landing) | `Tx::motion(Spring::new(0.38, 0.25))` on the play commit, or per-card `.motion(..)` on the arriving trick card (flight spring order: `Tx::motion` > `El::motion` > REFLOW, flights.rs:190) | P and H |
| Hover lift + shadow + press-down | `offset`+`scale`+`origin`+`shadow`+`press_scale`+`motion(Spring)` (section 7) | P and H |
| Staggered deal | `Transition::offset().combined(opacity())` + `enter_delay(Stagger)` | P and H |
| Trick sweeps off the table (cards must stay opaque) | `matched` to a destination element; **not** a removal transition (removal forces opacity 0, `transition.rs:154`) | P and H |
| Cards leaving a clipped felt | `tx.unclipped_flights()` | P and H |
| Fake card flip | no 3D, no scaleX: use stacked back/face + `Transition::reveal(Edge)` or a `Sequence` scale-dip + crossfade | n/a |
| Score pop / "Euchred!" stamp | key-by-value element + `sequence(Sequence)` with `Easing::OVERSHOOT` beats (section 8) | P and H |
| Winner glow pulse | `looping(LoopSpec::opacity(period, delay, low, high))` on a glow `El`, or a `Live<f32>` `live_opacity(live.smooth(spring))` | P and H |
| Turn timer ring | `Tween::linear(..)` + `live_trim`/`live_rect` | P and H |
| Dealer-button / turn marker slides between seats | `matched` between seat slots, or plain same-key layout move (springs automatically) | P and H |
| Trump suit banner | `launch("call-btn", "trump-chip")` is text-sourced: ok for a text chip only | P and H |
| Confetti | per-piece `Sequence` (opacity+scale) + `offset` driven by `Live` or `LoopSpec::slide`; unverified at scale (count of simultaneous layers) | P and H |
| Reduce Motion | nothing to do for matched/springs/transitions; add a `Cx::reduced_motion()` branch only where you pick different motion | P and H |

Constraints to design around:
1. **No stagger primitive for matched flights** (delay unverified). Plan on staggering commits via `poll` + `wake_after`, or per-card `El::motion` durations.
2. **Transitions cannot go below 0.9 scale** (0.5 with `allow_small_scale`) or exit opaque; matched flights are not clamped.
3. **Rotation exists, no 3D**; a rotated subtree gets its own layer (cost with many cards).
4. **Choreography/Keyframes are not reachable from the `rune` crate**; plan with `Sequence`, `LoopSpec`, `Live`, `Tween`, `enter_delay`, and per-property `Anim` delays.
5. `Anim` completion tokens are not delivered to apps; sequence game logic off state/`wake_after`, not animation completion.
6. Pinned vs HEAD: no motion API reason to bump. Re-evaluate only if a delayed-start track fails to wake an idle window at P (HEAD's `Mode::Due` fix, `H:rune-motion/src/store/tracks.rs:84-85`).

## 16. Not covered / open

- Scene-level APIs (`rune_motion::Scene`, `LayoutAnim`, `Travel`, `match_geometry`, `gesture.rs`, `clock.rs`) were only read as far as callers in `apply.rs`/`flights.rs`; they are internal to the runtime.
- `examples/` (expressive_press.rs, growth_reorder.rs, gpu_springs.rs) not read line by line; `expressive_press.rs` and `growth_reorder.rs` are the likeliest sources of hover/press and reorder snippets.
- Per-frame cost numbers are from the docs, not measured here.
- Unverified: `enter_delay` on a matched arrival; scale/rotation carry-over in a matched flight; `LoopKind::Rotation` on non-icon elements.
