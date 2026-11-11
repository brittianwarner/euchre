# r2 - rune-motion: physics, timelines, wiring, and card-game recipes

Audience: the engineer implementing the euchre animation pass. Extends `docs/port/07-rune-cookbook.md` sections 5, 6, 9, 13, 14, 15 (does not repeat them) and serves `docs/port/06-ui-spec.md`.

Revs. All citations are `path:line` at the PINNED rev `85cd8c6` (checkout `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/...`) unless marked HEAD. HEAD = GitHub `0ea75abe` (clone in the session scratchpad `rune-src/rune-head`). Paths below are relative to `crates/`. `motion/` means `rune-motion/src/`.

Status tags: VERIFIED = compiled and run at the pin in a scratch crate (outside the repo) during this research; SOURCE = read from source, not run.

---

## 1. What rune-motion is and how an app gets at it

- `rune-motion` 0.1.0 is pure math plus data: `publish = false`, no default features, no required dependencies; the only feature is optional `serde` (`rune-motion/Cargo.toml`). It never touches the GPU, windows or threads. VERIFIED/SOURCE.
- rune depends on it unconditionally (`rune/Cargo.toml:66`, `path = "../rune-motion"`). The `Runtime` owns the animation `Scene`; app code never touches the `Scene` or `AnimationStore` of the runtime.
- rune re-exports only a subset (`rune/src/lib.rs:259-286`): `Anim, AnimExt, Beat, Blend, Clock, CubicBezier, Curve, Decay, Easing, Edge, Effect, LayoutAnim, LoopKind, LoopSpec, NodeId, Phase as PresencePhase, Presented, Reveal, Sequence, Spring, SpringPair, Stagger, StaggerFrom, StepPosition, Transition, Travel, Vec2, Velocity`, plus `Cycle` (`rune/src/lib.rs:104`) and `rune::tok::motion` (= `rune_motion::motion::*` plus `LEAD`, `rune/src/tok.rs:5-8`).
- NOT re-exported (need a direct dependency): `Animated`, `Animatable`, `AnimationStore`, `Keyframes`, `Choreography`, `Cue`, `PhaseAnimator`, `Release`, `Scene`, `Rect`, `Color`, `Corners`, `prop`, `PropClass`, `VelocityTracker`, `project`, `rubber_band`, `deceleration`, `MotionTrace`, `LoopTrack`.
- How to depend (VERIFIED: resolves with `--offline`, and dedups against the existing lock entry):

```toml
# root Cargo.toml [workspace.dependencies]
rune-motion = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
# apps/desktop/Cargo.toml
rune-motion.workspace = true
```

  Same git URL and rev as `rune` so Cargo unifies to one copy (a second copy would make `Spring` two different types). `Cargo.lock` already holds `rune-motion` at that rev (line 4552 when checked). No feature flag is needed. Prefer `rune::Spring`/`rune::Sequence`/`rune::Stagger` where re-exported and `rune_motion::{Animated, AnimExt, Keyframes, PropClass, ...}` for the rest; both resolve to the same types. `rune_motion::prelude` (`motion/lib.rs:140-174`) bundles the common names.
- `rune_motion::tokens` is the same module as `rune::tok::motion`; in code use `rune::tok::motion::{SETTLE, POP, ...}`.

---

## 2. Physics

### 2.1 Spring (`motion/spring.rs`)
- `Spring::new(duration, bounce)` (`:30`) is the SwiftUI model, mass 1: omega = 2*pi/duration, zeta = 1 - bounce (bounce >= 0), zeta = 1/(1+bounce) for negative bounce (overdamped). Other constructors: `from_response :37`, `from_omega_zeta :42`, `from_stiffness_damping :49`, `from_motion :58`.
- `sample(x0, v0, target, t) -> (x, v)` (`:103`) is closed-form for underdamped, critical and overdamped, valid at any `t`: no integration step, frame-rate independent, retargetable from any exact (x, v). `settling_duration(eps)` `:111`, `settling_duration_from(from, v, target, eps)` `:117`, `css_linear()` `:125` (web only). `Spring::NONE` `:141` means no motion and Reduce Motion leaves it instant. SwiftUI presets `SMOOTH 0.5/0`, `SNAPPY 0.5/0.15`, `BOUNCY 0.5/0.3` `:146-150`.
- `duration` is perceptual (the period), not the settling time. Settling is longer (table below).

Measured at the pin (VERIFIED, unit step 0 to 1, from rest):

| Spring (duration/bounce) | Peak | Peak at | Settles within 0.01 | Within 0.002 |
|---|---|---|---|---|
| tok SNAP 0.25/0 | 1.000 | none | 0.300 s | 0.373 s |
| tok SETTLE 0.3/0 | 1.000 | none | 0.350 s | 0.438 s |
| tok GLIDE 0.32/0 | 1.000 | none | 0.370 s | 0.463 s |
| tok MORPH 0.3/0.16 | 1.0077 | 0.276 s | 0.339 s | 0.430 s |
| tok POP 0.25/0.15 | 1.0063 | 0.237 s | 0.289 s | 0.364 s |
| tok DRIFT 0.85/0 | 1.000 | none | 0.898 s | 1.145 s |
| SMOOTH 0.5/0 | 1.000 | none | 0.537 s | 0.684 s |
| SNAPPY 0.5/0.15 | 1.0063 | 0.475 s | 0.513 s | 0.663 s |
| BOUNCY 0.5/0.3 | 1.046 | 0.350 s | 0.588 s | 0.771 s |
| 0.45/0.35 | 1.068 | 0.296 s | 0.574 s | 0.752 s |
| 0.6/0.5 | 1.163 | 0.346 s | 0.916 s | 1.223 s |

Takeaways for tuning (new facts, not in the cookbook):
- **Token POP overshoots by 0.6 percent. It is a press confirmation, not a visible bounce.** Visible bounce on a 0..1 progress needs bounce >= 0.3 (4.6 percent overshoot); 0.35 gives 6.8 percent; 0.5 gives 16 percent. Pixel overshoot = bounce-dependent fraction x travel, so for a 300 px card flight a bounce of 0.12 overshoots about 1 px (invisible), 0.35 about 20 px.
- The cookbook 06-mapping line "Spring::new(0.7, ~0.1) matches 700 ms" is a perceptual match only: with bounce 0.1 the spring is visually done near 0.8-0.9 x duration of 0.7 and the tail to 0.002 is about 1.5 x duration. For a card flight that "lands" in about 0.45 s use `Spring::new(0.45, 0.25..0.35)`.
- Tokens table (`motion/tokens.rs`): INSTANT 0.1/0 `:12`, SNAP 0.25/0 `:15`, SETTLE 0.3/0 `:18`, REFLOW 0.4/0 `:21`, GLIDE 0.32/0 `:24`, MORPH 0.3/0.16 `:27`, POP 0.25/0.15 `:30`, DRIFT 0.85/0 `:33`, CAMERA `:36`; each has an `_EXIT` (shorter) twin; aliases `:39-45`; `SpringPair` (enter, exit) `motion/spring.rs:176`, `tokens::pair :64`, `swiftui :77`, `ALL :86`.

### 2.2 Decay / inertia / gestures
- `Decay` is exponential: `v(t) = v0*e^(-k t)`, rest point `x + v/k` (`motion/curve.rs:61-76`). `Decay::IOS_NORMAL` rate 2.002002, `IOS_FAST` 10.050336 (`motion/curve.rs:68/70`); `Decay::per_ms(r)` builds from a UIKit per-ms rate `:73`. `project(x, v, rate)` (`motion/gesture.rs:17`) gives the UIKit rest point: VERIFIED `project(0, 1000, deceleration::NORMAL) = 499.006` (matches x + v/1000 x r/(1-r)). `rubber_band(overshoot, dim)` `:23`, `rubber_band_with :~26`, `VelocityTracker :53`.
- `Release<T>` (`motion/store.rs:55`): `Spring{target, spring}` (start from the release velocity), `Decay(Decay)`, `DecayClamped{decay, min, max, spring}` (coast; if the projection leaves the bounds spring onto the bound with the velocity it has there; if already out of bounds, spring straight back). Applied with `AnimationStore::release` (`motion/store/typed.rs:80`) or `Animated::release(velocity, target, curve, now)` (`motion/animated.rs:92`). Tests: fling into a bound `tests/store.rs:216`, rubber band `:236`.
- Game use: swipe-to-play on the rack (release a dragged card with its finger velocity into either the trick target or back to the slot) is the one place Release matters. If the card drags with the pointer, drive it as a `PropClass::Gesture` value (never reduced) and call `release` on pointer-up; the card then continues with the finger's velocity instead of restarting from rest. Not in the 06 spec; optional.

### 2.3 Tracks, layering and retargeting (`motion/store.rs`, `motion/animated.rs`)
- A track is `target + sum(offset layers)`. Springs and decay merge from the exact (x, v); tweens are additive and cap at 8 layers (SOURCE, `tests/store.rs`). Retargeting mid-flight keeps position and velocity: a trick card whose winner changes mid-collect bends toward the new target instead of snapping.
- The first `animate` on a value acts as `set` (no animation from nothing); an unchanged target is a no-op (`motion/animated.rs:46`). So you cannot "replay" by re-animating to the same target; either `set` back to the start then animate, or key a new element.
- Settle thresholds come from `PropClass::eps` (`motion/props.rs:49`): Spatial/Shape/Gesture/Blur about 1/256 pt (at 2x), Scale/Rotation/Presence 1e-5, Opacity/Color 1/1024, **Other 1e-4**. `Animated::new` defaults to `Other`.
- **Gotcha, VERIFIED:** a 0..1 progress in `Animated<f32>` with the default class takes `settle_time` 1.003 s for `Spring::new(0.55, 0.12)` versus 0.777 s with `.with_class(PropClass::Opacity)`; the 1e-4 threshold keeps the poll waking frames about 0.2 s longer than needed. Use `.with_class(PropClass::Opacity)` or `Spatial` for a 0..1 progress.
- Settled tracks leave the active list and `tick` allocates nothing (`tests/alloc.rs:43/:77`).
- `anim.delay` (via `AnimExt::delay`) and `Blend::Additive` only work in `AnimationStore`; `Animated::animate_to` debug-asserts `delay == 0` (`motion/animated.rs:~48`).
- `Anim` (`motion/curve.rs:112`) = curve + delay + blend + velocity + completion token. Builders on anything `Into<Anim>` (`AnimExt`, `:155`): `.delay(s)`, `.blend`, `.additive()`, `.merge()`, `.velocity(Velocity)`, `.completion(token)`. A non-zero token fires `MotionEvent::Completed{token, finished}` when the animation settles or is replaced (`motion/store.rs:45`); only reachable through an app-owned `AnimationStore`.

### 2.4 Tweens, easing, keyframes
- `Curve` (`motion/curve.rs:8`): `Instant`, `Spring`, `Tween{duration, easing}`, `Decay`. `Easing` constants (`motion/easing.rs:26-38`): LINEAR, EASE, EASE_IN, EASE_OUT, EASE_IN_OUT, ENTER `(.2,.8,.2,1)`, EXIT `(.4,0,1,1)`, OVERSHOOT `(.34,1.45,.64,1)` (an eased overshoot with no spring), DRAW_ON `(.45,0,.2,1)`; `Easing::cubic(x1,y1,x2,y2)`, steps and piecewise-linear exist.
- `Keyframes<T>` (`motion/keyframes.rs:28`): builders `linear, eased, spring, spring_for, cubic (Hermite/Catmull-Rom, velocity-continuous), move_to, delay, repeat`; played with `AnimationStore::play_keyframes(node, prop, &kf)` (`:113`). Test: `tests/store.rs:248`. Needs a store you own; the El layer has no keyframes API (only `Sequence`, opacity+scale).

---

## 3. Timelines, sequencing, stagger

### 3.1 Stagger (`motion/choreo.rs:17-76`) - VERIFIED
`Stagger::new(gap).from(StaggerFrom::{First,Last,Center,Index(i)}).start(s).ease(Easing)`; `.delay(index, count) -> f32` seconds; free fn `stagger(index, count, gap, from)` `:71`. Measured: `Stagger::new(0.08).delay(i,5)` = 0, 0.08, 0.16, 0.24, 0.32; `.from(Center)` = 0.16, 0.08, 0, 0.08, 0.16. `ease` bends the delay curve across the ripple (distance normalised, then eased). Re-exported by rune, so no extra dependency to use it with `El::enter_delay` (`rune/src/el/enter_delay.rs:36`, doc example there; test `tests/enter_delay.rs:133` staggered five).

Use for: initial deal-in (First, 0.075 gap per 06 6.5), trick collect (45 ms gap, `from(Index(winner_slot))` if you want the cards to leave nearest-the-winner first), score-row reveal, hand fan reveal (`Center`).

### 3.2 Choreography (SOURCE; `motion/choreo.rs:81-297`)
For an app-owned `AnimationStore`: a `Choreography` holds steps recorded as typed property animations (`Ops`, `:140`), each with a `Cue` (`:92`): `now()`, `at(t)`, `started(step)`, `arrived(step)`, `settled(step)`, `.after(s)`, `.not_before(t)`. `When` (`:81`): `Started`; `Arrived` = the logical duration elapsed (spring `duration`); `Settled` = physical rest. Order per frame: `store.set_time`, `ch.tick`, `store.tick` (tests `tests/choreo.rs:5/:19/:53/:74`). `PhaseAnimator` (`:297`) cycles a value through phases (idle pulse). The El layer exposes none of this; to use it the app must own the store (section 6). For euchre, only worth it for the trick sweep (card 1 arrives -> card 2 starts) if the plain `enter_delay`/`Stagger` is not enough.

### 3.3 El-level timeline sugar (SOURCE)
- `.sequence(Sequence)` (`rune/src/el/motion.rs:259`): `Beat{at, opacity, scale, easing}` list (`motion/sequence.rs:6-39`), `&'static [Beat]`, first beat at 0, strictly increasing times; interpolates opacity+scale piecewise with each beat's easing (target-beat easing); holds the last pose; Reduce Motion lands it at once; a new element key replays it. `Sequence::new(..).delay(s)`, `.duration()`.
- `.looping(LoopSpec)` (`:264`): kinds `Opacity{low,high}`, `OpacityCycle(Cycle)`, `Sequence`, `Rotation` (continuous, constant), `Slide{from,to}` and `SlideEased` (fractions of the node's own size, wraps). `LoopSpec::{opacity, cycle, sequence, rotation, slide}` + `.easing` (`motion/looping.rs:27-53`). `El::cycle(Cycle)` (`rune/src/el/visuals.rs:556`): wall-clock opacity stops, in phase for every cycle with the same period; the window wakes at `Cycle::fps` rather than every frame while only cycles move (`motion/cycle.rs:8`).
- Loops cover opacity, scale (sequence only), rotation (continuous) and slide. **No looping scale pulse and no looping arbitrary-axis motion.** A repeating scale pulse must come from a poll-driven `Animated` (section 6) or from repeated keyed `Sequence`s.

---

## 4. Transitions and presence (`motion/transition.rs`, `motion/scene.rs`)

- `Effect` (`transition.rs:53`) fields: opacity, scale, offset (pts), offset_fraction (of the node size), blur, draw, reveal, anchor. `Transition` presets (`:186-278`): `opacity()`, `scale(k)`, `scale_anchor(k, anchor)`, `offset(x,y)`, `move_edge(Edge)`, `push(Edge)`, `slide()`, `blur_replace()`, `blur(s)`, `draw()`, `reveal(Edge)`, `identity()`, `none()` (no motion at all, even under Reduce Motion), `asymmetric(insertion, removal)`, `.combined(other)`, `.springs(SpringPair)`, `.allow_small_scale()`, `.hold_layout()`. Attached with `El::transition` (`rune/src/el/motion.rs:196`).
- **`kit_rules` clamp** (`:148`, floor const `SMALL_SCALE_FLOOR :86`): transition scale is clamped to [0.9, 1.1], or [0.5, 1.1] with `.allow_small_scale()`. `Transition::scale(0.3)` silently becomes 0.9 without the flag. Use `Transition::scale(0.6).allow_small_scale()` for a card shrinking in. Scale below 0.5 is never allowed through a Transition; for the collect shrink to 0.18 use matched flights (they interpolate the real size), not a Transition scale.
- Presence lifecycle: an element removed from the view keeps drawing in its exit phase (inert, non-hit-testable) until the exit spring settles, then unmounts (`motion/scene/presence.rs:20` `insert_delayed`, `:140` `remove`). `Runtime::presented(key)` (`rune/src/runtime/elements.rs`, `-> Option<([f32;4], f32)>`) reads the drawn rect and effective opacity of an element even while exiting (use in tests to assert a card is mid-flight); `Runtime::find(key)` ignores exiting elements.
- Matched geometry (`El::matched(id)`, `rune/src/el/motion.rs:215`): an element leaving and one mounting with the same id in the same commit fly as one. Internally `LayoutAnim::new(spring).screen()` (`rune/src/runtime/motion/flights.rs:197`) = `Travel::Screen`: x and y share the same curve so the path is a straight line in screen space, with per-axis progress identical (`motion/scene.rs:52-100`, `motion/scene/travel.rs:10/17/51/110`; test `tests/scene.rs:254` straight path, `:289` matched). Under Reduce Motion a flight becomes a crossfade.
- Layout animation APIs on the Scene (`LayoutAnim::axes(x, y)` `:67`, `width_leads :72`, `screen :84`) give separate x/y curves, but the El layer never calls `axes` for matched flights. That is why you cannot get an arc from `.matched` (confirms and sharpens the cookbook's section 6 "arc not expressible").
- `Tx::launch(source_key, destination_key)` (`rune/src/flight.rs:28`) and `fly_text` (`:16`) fly text/an element from another element's text position on `REFLOW`; draws above everything and outside clips until landing. `Tx::unclipped_flights()` (`:36`) draws this commit's `matched` flights outside all clips; call it for cards flying between clipped panels (hand scroll clip to trick area). Does nothing under Reduce Motion or when the destination was already mounted.

---

## 5. Reduced motion, by class (`motion/props.rs:44-75`)

`PropClass::reduce(curve)`: Spatial, Scale, Rotation, Shape, Blur snap to `Instant`; Opacity, Color, Presence take `REDUCED_FADE` (tween 0.18 s ease-in-out); Gesture and Other keep their curve; `Instant` and `Decay` pass through. `match_geometry` becomes a crossfade (test `tests/store.rs:275`). Cookbook 5.5 covers how to set it in tests and per commit. Consequences for euchre: any poll-driven `Animated` you own must apply the same rule yourself (the runtime cannot see it): when `cx.is_reduced_motion()` is true, `set` the end value (or give opacity a 0.18 s tween) instead of `animate_to` a spring. Spatial values you own are not reduced for you.

---

## 6. What the El API exposes vs what needs the direct dependency

El methods (all SOURCE unless cookbook says VERIFIED): `.motion(Spring)` `rune/src/el/motion.rs:89`, `.token_motion :95`, `.offset(x,y) :113`, `.offset_motion(anim) :122`, `.opacity_motion :153`, `.lead(lead, trail) :159`, `.width_leads :164`, `.animate_layout :172`, `.instant_changes :181`, `.transition :196`, `.matched :215`, `.matched_crop :224`, `.hold_landing_width :235`, `.raise :132`, `.paint_order :142`, `.sequence :259`, `.looping :264`, `.enter_delay` (`el/enter_delay.rs:36`), `.scale / .scale_motion` (`el/visuals.rs:364/369`; scale is UNIFORM, no scale_x), `.rotate / .rotate_motion` (`:381/389`; rotates about `.origin`, layout keeps the upright box; rotation changes spring on the element's layout spring unless `rotate_motion` given), `.cycle` (`:556`).
Tx methods: `motion(Spring) tx.rs:527`, `motion_curve(anim) :536`, `instant :513`, `settle :553`, `wake_after(Duration) :226`, `wake_at(f64) :242`, plus `launch`, `fly_text`, `unclipped_flights`. `Cx` has `layout(key) runtime/layout_read.rs:239`, `window_layout(key) :281` (rect `[x, y, w, h]` of a laid-out element in window points: endpoints for a hand-built path), `is_reduced_motion()`, `reduced_motion(on)`.

Not exposed to the El layer: `Keyframes`, `Choreography`, completion tokens, `Release`, per-axis curves, 3D rotation, `scale_x/y`, per-property stagger beyond `enter_delay`. To use any, build the driver in app state with `rune_motion::{Animated, AnimationStore}`.

### The app-owned driver pattern (VERIFIED to compile and sample at the pin)
`AppSpec::poll` is `Option<fn(&mut S, f64, &mut Tx) -> bool>` (`rune/src/tooling.rs:35`), called at the start of every frame with seconds since start (virtual when headless). From `poll`, `tx.wake_after(Duration::ZERO)` keeps frames coming at display rate while anything is still moving; return `true` when state the view reads changed. The view then samples `Animated::sample(now)` and writes the numbers into `.absolute(..)`, `.rotate(..)`, `.opacity(..)`, `.size(..)`. Stop waking when `!Animated::is_settled(now)` becomes false, otherwise the window never idles. `now` must be the same clock the animation was started against: start animations from a handler with the clock you saw in the last `poll` (store `last_now` in state), or start them inside `poll`.

Trade: poll-driven animation costs a view rebuild per frame during flights (the runtime's own flights do not). Fine for one or two cards; do not run 20 at once with large subtrees.

---

## 7. Card-game recipes

Everything in 7.1-7.5 compiled at the pin against `rune`, `rune_motion` (scratch crate); numbers in 7.1 were run. Not run in a real window.

### 7.1 Deal a card along an arc with rotation (poll-driven) - VERIFIED compile + samples
Why not `.matched`: straight line, same curve on both axes, no rotation carried. The arc is derived from one scalar progress `p` (spring) plus a perpendicular lift `sin(pi * clamp(p,0,1)) * lift`; rotation eases from `spin` to 0 with `p`.

```rust
use rune::{el, El, Spring, Tx};
use rune_motion::{Animated, PropClass};
use std::{f32::consts::PI, time::Duration};

pub struct Flight { p: Animated<f32>, from: [f32; 2], to: [f32; 2], lift: f32, spin: f32 }
impl Flight {
    pub fn launch(from: [f32; 2], to: [f32; 2], now: f64) -> Flight {
        let mut p = Animated::new(0.0f32).with_class(PropClass::Opacity); // tight-but-not-1e-4 settle
        p.animate_to(1.0, Spring::new(0.55, 0.12), now);
        Flight { p, from, to, lift: 60.0, spin: 0.35 }
    }
    pub fn pose(&self, now: f64) -> ([f32; 2], f32) {
        let p = self.p.sample(now);
        let q = p.clamp(0.0, 1.0); // sin(pi*p) goes negative for p>1 (overshoot) -> dip below the line
        let x = self.from[0] + (self.to[0] - self.from[0]) * p;
        let y = self.from[1] + (self.to[1] - self.from[1]) * p - (PI * q).sin() * self.lift;
        ([x, y], self.spin * (1.0 - p))
    }
    pub fn moving(&self, now: f64) -> bool { !self.p.is_settled(now) }
}
pub fn poll(f: &mut Option<Flight>, now: f64, tx: &mut Tx) -> bool {
    let Some(fl) = f else { return false };
    if fl.moving(now) { tx.wake_after(Duration::ZERO); } else { *f = None; }
    true
}
pub fn view(f: &Flight, now: f64) -> El {
    let ([x, y], r) = f.pose(now);
    el().size(64.0, 92.0).absolute(x, y).rotate(r).origin(0.5, 1.0)
}
```
Samples (from (0,0) to (300,200), lift 60, spin 0.35): t=0.1 pos (101, 15), rot 0.232; t=0.2 (215, 97), rot 0.099; t=0.3 (274, 166), rot 0.031; t=0.4 (295, 193); lands (300, 200) with a tiny overshoot at about 0.6 s. Start/end from `cx.window_layout("hand-3")` and `cx.window_layout("trick-you")` (`layout_read.rs:281`), converted into the overlay's coordinate space. Hide the real destination card (opacity 0) until `moving` is false so there is no duplicate. Reduced motion: `set(1.0)` on the progress and skip the arc.
Cheaper alternative if the arc is only decorative: keep `.matched` for the straight flight and add `rotate`+`rotate_motion(Spring)` on the destination from the tilt to 0 (rotation changes on the element spring; note the cookbook found rotation is not carried by the flight itself, so the tilt is applied at the destination after landing, not during flight).
Upstream-patch alternative: a per-axis `LayoutAnim::axes(x, y)` already exists at the Scene layer (`scene.rs:67`); a rune fork could expose a `matched_arc`; not needed for the port.

### 7.2 Flip a card (back to face) - width squeeze, no 3D - VERIFIED compile
`scale` is uniform and there is no `rotateY`. A convincing flip is a horizontal squeeze through zero width with the face swapped at the midpoint:

```rust
pub fn flip(p: f32, w: f32, h: f32, back: El, face: El) -> El { // p 0..1 from an Animated<f32>
    let wide = (w * (p * std::f32::consts::PI).cos().abs()).max(1.0);
    let shown = if p < 0.5 { back } else { face };
    el().size(w, h).child(shown.size(wide, h).absolute((w - wide) / 2.0, 0.0))
}
```
Drive `p` with `Spring::new(0.4, 0.0)` (or an `Easing::EASE_IN_OUT` tween of 0.32 s so it passes zero width once and cleanly). Lift 4-8 px at p=0.5 with `.offset(0.0, -lift*sin(pi p))`; add a one-frame brightness drop on the squeezing face if the art supports it. A bouncy spring on `p` would flip back through 0.5 on the overshoot and swap faces twice: use bounce 0 or a tween.

### 7.3 Sweep a trick to the winner (stagger + matched) - extends cookbook 5.3
Cookbook 5.3 already flies each trick card to a mini in the winner's stack via `matched` and `COLLECT`. Add:
- Stagger the departures by mounting the destination minis with `.enter_delay(Stagger::new(0.045).from(StaggerFrom::Index(lead_slot)).delay(i, 4))` (06 6.4: 45 ms). SOURCE for the combination; `enter_delay` alone is tested (`tests/enter_delay.rs:133`).
- A short hold before the sweep so the player reads the trick (06 says 2.2 s read hold; keep it in state logic): schedule the gather with `tx.wake_after(Duration::from_millis(...))` in the handler, not an animation delay.
- Per-commit spring override for the sweep: `tx.motion(Spring::new(0.5, 0.0))` (`tx.rs:527`) so the fast, non-bouncy glide applies only to that commit, without touching the cards' own `.motion`.
- Winner emphasis: a `Sequence` pop on the winning card before the sweep: beats `[0.0 x1.0] -> [0.10 x1.12 EASE_OUT] -> [0.22 x1.0]`, keyed by trick index so it replays.
- Mini size 0.18 is beyond a Transition's scale clamp; the flight itself handles the size interpolation (it interpolates real rects), so no clamp issue.
- Draw cards leaving across the hand's clip with `tx.unclipped_flights()` (`flight.rs:36`).

### 7.4 Score bounce - VERIFIED compile
Key by score so the sequence replays on change (a new key = a new element = replay, `rune/src/el/motion.rs:256-258`):

```rust
use rune::{text, Beat, Easing, Sequence};
static BOUNCE: [Beat; 4] = [
    Beat::new(0.0).scale(1.0),
    Beat::new(0.10).scale(1.35).easing(Easing::EASE_OUT),
    Beat::new(0.24).scale(0.92),
    Beat::new(0.36).scale(1.0),
];
text(n.to_string()).key(format!("score-{n}")).sequence(Sequence::new(&BOUNCE))
```
Must be `&'static [Beat]` (a `static`). Beat easing applies to the segment ending at that beat. The sequence plays on the element's first frame after mount; combine with `.transition(Transition::opacity())` if the old number should crossfade rather than vanish (a keyed replace removes the old element: give the old one an exit). A numeric tick-up (0 to 5 over 0.4 s) is not a built-in: use an `Animated<f32>` driven from `poll` and print `round()`.

### 7.5 Pulse the turn indicator - VERIFIED compile
- Simple: `ring.looping(LoopSpec::opacity(1.2, 0.0, 0.35, 1.0))` (period, delay, low, high). Already used in `apps/desktop/src/screens/table/stage.rs:491` for the thinking dot.
- Shaped: `static BREATHE: Cycle = Cycle { period: 1.4, phase: 0.0, stops: &[(0.0,0.4),(0.5,1.0),(1.0,0.4)], still: 0.8, fps: 30.0 }; ring.cycle(BREATHE)`. Linear stops, wall clock, all cycles of equal period stay in phase (all four seat indicators breathe together), and the window wakes at `fps`, not display rate (battery). `still` is the opacity held under Reduce Motion.
- Scale pulse (ring expanding while fading): not available as a loop. Use two poll-driven `Animated<f32>` or a per-turn `Sequence` (scale 1 to 1.5 with opacity 0.6 to 0) on a ring keyed by turn number, which plays once per turn and holds invisible: that is the usual "ping".

### 7.6 Other quick wins from the same API
- Hover/selection lift: `.offset(0,-8)` + `.offset_motion(SNAP)` (cookbook 6). Press: `.press_scale(0.97)`.
- Trump reveal / up-card: `Transition::scale(0.6).allow_small_scale().combined(Transition::opacity())` with `.springs(SpringPair::new(POP, SNAP_EXIT))` (confirm `SpringPair` constructor `motion/spring.rs:176`: SOURCE).
- Page/screen change (lobby to table): `Transition::blur_replace()` (`transition.rs:231`, 1.5 blur, scale 0.96) is the cheapest "premium" crossfade; `Transition::push(Edge::Trailing)` for drawers.
- Lights-out for a winning hand: `Transition::reveal(Edge::Leading)` wipes a banner in with a 24 pt feather (`:246`).
- Confetti: per piece `LoopSpec::slide(period, [0,0], [dx, fall]).easing(Easing::EASE_OUT)` with a distinct `delay` and key (already used at `stage.rs:966`); slides are fractions of the node's own size, so size each piece large and scale via child.

---

## 8. Goldens and testing

- Pinned rev: `tests/spring.rs:168` asserts a bit-exact FNV hash `15_969_925_884_686_696_054` over a sweep of spring samples. HEAD commit `20f9057d` replaced that with tolerance anchors because libm differs by platform (glibc vs Apple: hash `10396150247033498793` vs `15969925884686696054`, up to 3e-5 at a value of 400). The README and `motion/math.rs` still claim deterministic custom `exp`/`sin_cos`: the claim and the commit message conflict; trust the commit. SOURCE (git log, both clones).
- For our tests: use tolerance assertions (1e-3 relative on samples, or exact only for the rest point), never a hash of floats. Tests worth copying from `tests/spring.rs`: `:24` closed form matches ODE integration in every regime (an independent integrator, the right oracle for our own arcs), `:97` `settle_time` bounds the whole tail, `:114` critically damped from rest never overshoots, `:125` retarget keeps velocity, `:145` retarget golden (tolerance anchors at HEAD, the hash at the pin `:168`), `:172` `css_linear` export, `:179` the `NONE` spring is an instant curve everywhere.
- Headless Runtime drive: cookbook 13/14. Useful addition: `rt.presented(key)` returns the drawn rect and opacity even while exiting, so a test can assert a card is mid-air (`rect.x` between start and end) at a given `frame(dt)` and exactly at the destination after `settle_all()` (`rune/src/runtime/frame.rs:297`; `frame(dt) -> bool` at `:305` returns whether more frames are needed).
- Tick order for app-owned stores in unit tests: `store.set_time(t)`, `ch.tick`, `store.tick`. `Clock` supports manual mode, pause, scale and seek (`motion/clock.rs`) for deterministic tests. `MotionTrace` writes JSONL when `$RUNE_MOTION_TRACE` is set (`motion/trace.rs`): set it to see every started/settled track while debugging a stuck animation.
- Allocation guards (`tests/alloc.rs:43/:77`) show the tick allocates nothing for settled tracks; do not hold clones of `Animated` per frame if you want the same property in the app-owned driver.

---

## 9. Pin vs HEAD, rune-motion only

Differing files: `src/looping.rs`, `src/store.rs`, `src/store/tracks.rs`, `tests/spring.rs`, `tests/store.rs`. Behaviour/refactor commits:
- `047f6f1d`: `AnimationStore` gets an internal `Mode` enum; `LoopTrack` gets a `Phase` enum with a `Done` state (a finished `Sequence` loop stops requesting frames).
- `dde00138`: a due delayed start keeps its immediate wake (no one-frame stall on `.delay(0)` cases).
- `20f9057d`: spring goldens become tolerance anchors.
Everything else is cosmetic. rune itself changed more (a new `marks` module; the `runtime/motion/flights.rs` `Attempt`/`Seq` refactor). No public rune-motion API used in this note changed. Recommendation: stay on the pin for this pass; the only HEAD behaviour that touches us is the finished-Sequence idle wake, which helps the score bounce but is not required.

---

## 10. Gotchas (all new relative to the cookbook)

1. Token `POP` is not a bounce (0.6 percent overshoot). Use bounce >= 0.3 for a visible spring.
2. Duration is the period, not the settle time: allow 1.2-1.5 x duration for the tail; delays that chain on "arrived" use `When::Arrived` (logical duration), on physical rest `When::Settled`.
3. `kit_rules` silently clamps `Transition::scale` to 0.9..1.1 unless `.allow_small_scale()` (then 0.5 floor).
4. `.matched` is a straight line, equal curve on x and y, with no rotation carried. Arcs need the poll-driven driver (7.1).
5. `scale` is uniform; no `rotateY`; flips are width squeezes (7.2). `sin(pi p)` goes negative when a bouncy `p` exceeds 1: clamp it.
6. `Animated::new` uses `PropClass::Other` (1e-4 settle): a 0..1 progress keeps waking frames about 0.2 s longer; set a class.
7. Anything you own is not reduced for you; branch on `cx.is_reduced_motion()`.
8. A poll that does not stop calling `wake_after(ZERO)` pins the window at display rate forever: stop when settled.
9. Animating to an unchanged target is a no-op, and the first `animate` is a `set`: to replay, key a new element (El sugar) or `set` the start then animate (app-owned).
10. `Sequence` and loops take `&'static` data; no runtime-built beat lists (use a few statics, or a poll-driven `Animated` for dynamic ones).
11. `Animated::animate_to` rejects `anim.delay` in debug; use `AnimationStore` for delays and completion tokens.
12. `Runtime::find` skips exiting nodes; use `presented` in tests.

---

## 11. Source index (pinned rev unless noted)

`motion/README.md`, `motion/Cargo.toml`, `src/{spring,tokens,curve,easing,keyframes,sequence,looping,cycle,choreo,props,store,store/typed,store/started,animated,clock,gesture,transition,scene,scene/travel,scene/presence,value,lib}.rs`, `tests/{spring,choreo,store,scene,enter_delay,alloc}.rs`, `examples/*.rs` (not needed for the El layer). rune side: `rune/src/{lib.rs,tok.rs,tooling.rs,flight.rs}`, `rune/src/el/{motion,visuals,tx,enter_delay}.rs`, `rune/src/runtime/{layout_read,elements,frame,cx,system_settings}.rs`, `rune/src/runtime/motion/flights.rs`. Euchre: `apps/desktop/src/screens/table/stage.rs` (read only; another agent is editing apps/desktop). Scratch verification crate (not in the repo): scratchpad `motion-probe/tests/{numbers,snippets,settle}.rs`.
