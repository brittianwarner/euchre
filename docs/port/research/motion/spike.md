# Motion spike: what Rune can actually do for the table

Stage: SPIKE. Every technique below was built as a small compiled Rune program and rendered headless. I looked at the frames before writing anything down. This file promises only what rendered.

- **Rev.** Everything here is verified at the app's pinned Rune rev `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb` (short `85cd8c6`).
  - Every `path:line` citation is at `85cd8c6`. Paths are relative to the checkout's `crates/` dir: `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/`.
  - No technique needed a HEAD-only API, so nothing was built at HEAD `0ea75abe`. r5-docs-head.md found the public motion surface unchanged between the two revs.
- **Spike crate.** A standalone cargo workspace: toolchain 1.99.1; rune, rune-kit and rune-fx from git at the pin. Full source is in the appendix.
  - It used the app's art (`apps/web/static/art/cards/*.png`, `back.png`) and fonts (DM Sans, Source Serif 4, IBM Plex Mono from `apps/desktop/assets/fonts/`).
  - Window 1000x640 pt. Cards 90x126; seat backs 46x64.
  - Scratch build: `scratchpad/motion-spike`, target `scratchpad/motion-spike-target`. Shots are in `scratchpad/motion-spike/shots/`; that is session scratch and may not survive.
- **Cost numbers.** These come from the headless stats line: `rune: animating frames N (cpu p50 .. p95 .. ms)`.
  - They are CPU per frame for the app's debug profile, with dependencies at opt-level 2, on this Mac (Apple Silicon, Metal).
  - GPU time is **not** measured.
  - The app's release build will be cheaper on CPU.
  - Every scene's idle frames draw nothing ("the window draws none"), except the breathing turn indicator, which wakes at its `fps`.

## Summary table

| # | Technique | Verified @85cd8c6 | How | Timing that looked right | CPU p50 / p95 per animating frame |
|---|---|---|---|---|---|
| 1 | Curved flight + rotate + lift + settle | yes | poll-driven pose, `instant_changes` | 0.55 s travel, settle spring 0.42/0.3 | 0.12-0.14 / 0.25-0.27 ms |
| 2 | Staggered deal, 5 to 4 seats | yes | `Transition::offset` from deck + `enter_delay(Stagger)` | 55 ms gap, spring 0.42/0.12 (about 1.5 s total) | 0.91 / 6.1 ms (20 mounts) |
| 3 | Face-down to face-up flip | yes, two ways | width squeeze (poll) or two-commit width tween | 0.36 s total, swap at mid | 0.17 / 0.31 ms |
| 4 | Trick sweep to winner, fading | yes | `matched` flights into 46x64 minis at opacity 0 | pop 0.3 s, gather 0.3/0.12, sweep 0.5/0, 60 ms stagger | 0.33 / 0.75 ms (one commit), 0.63 / 1.19 ms (stagger commits) |
| 5 | Hover lift + shadow + glow, press, illegal shake | yes | `.motion` spring on offset/scale + plate shadows; poll shake | spring 0.26/0.32, lift -16 pt, scale 1.05; shake 0.36 s | 0.45 / 0.61 ms; shake 0.54 / 1.09; fx glow 0.75 / 2.2 |
| 6 | Score count-up + pulse | yes | `RollingText` + re-keyed `Sequence` ring | one point per 0.14 s, ring 0.5 s | 0.15 / 0.23 ms |
| 7 | Breathing turn indicator | yes | `Cycle` on a halo + re-keyed `Sequence` ping; marker on a layout spring | period 1.6 s, ping 0.9 s, move spring 0.5/0.2 | 0.12 / 0.36 ms, wakes 30 fps while shown |
| 8 | Spark / sparkle / confetti burst | yes | core `canvas` (VShape) per frame; rune-fx sparkles + ripple | sparks 0.9 s, fx 1.1 s, confetti 3.7 s | sparks 0.10 / 0.15; confetti 0.14 / 0.23 ms |
| 9 | Route cross-fade + slide, shared card | yes, with one gap | keyed screen + asymmetric `Transition`; `matched` hero card | enter 0.5/0, exit 0.4/0, flight 0.6/0.12 | 0.11 / 0.22 ms |
| 10 | Dialog spring-in + backdrop blur | yes | `backdrop_blur(10)` scrim + asymmetric scale/offset/opacity panel | in 0.45/0.3, out 0.2/0 | 0.23 / 0.48 ms open, 0.27 / 0.68 close |
| 11 | Felt vignette + texture, shadows deepen on lift | yes | rail + radial felt + vignette child + texture child + inset child; 3-shadow stack | lift spring 0.26/0.32 | 0.41 / 0.75 ms; felt at rest draws nothing |

Spring notation: `Spring::new(duration_s, bounce)` (`rune-motion/src/spring.rs:30`).

## Shared building blocks (used by several techniques)

### Poll-driven motion: the core pattern

- **Driving per-frame motion.** Anything Rune's springs can't express is computed in the view from `s.now`. Examples: arcs, a rotation that settles, a width squeeze, a shake, particles.
- **The poll** (`AppSpec.poll`, field at `rune/src/runtime.rs:436`) stores `now` and returns `true` while something moves.
  - While moving, it calls `tx.wake_after(Duration::ZERO)` (`rune/src/el/tx.rs:226`), which asks for the next frame.
  - For scheduled triggers it calls `tx.wake_at(t)` (`rune/src/el/tx.rs:242`).
- **The driven element** carries `.instant_changes()` (`rune/src/el/motion.rs:181`), so layout, transform and opacity land at once and don't spring behind the view. Without it, the element lags a spring behind the computed pose.
- **Mount fade.** A driven element that mounts mid-motion also needs `.transition(Transition::none())` (`rune-motion/src/transition.rs:194`). Otherwise the default 0.2 s mount fade makes it semi-transparent at launch. Seen in the first flight render.
- **Sampling helpers:**
  - `Spring::sample(x0, v0, target, t) -> (x, v)` (`rune-motion/src/spring.rs:103`)
  - `Easing::eval(x)` (`rune-motion/src/easing.rs:47`)
  - `Easing::cubic(x1,y1,x2,y2)` (`rune-motion/src/easing.rs:41`)

```rust
fn at(s: &State, t: f64, step: u32) -> bool {
    s.now >= t && s.step < step
}
```

The poll's frame (the per-technique arms are shown in each section):
```rust
fn poll(s: &mut State, now: f64, tx: &mut Tx) -> bool {
    s.now = now;
    let mut changed = false;
    let mut next: Option<f64> = None;
    let mut due = |t: f64, next: &mut Option<f64>| {
        if now < t {
            *next = Some(next.map_or(t, |n: f64| n.min(t)));
        }
    };
    match s.tech {
        // ... per-technique arms ...
    }
    if let Some(t) = next {
        tx.wake_at(t);
    }
    changed
}
```


### Card shadow stack

- **Always emit 3 shadows.** Shadow geometry only springs when the shadow count matches between states (r3 1.1).
  - Fade a shadow out with alpha; never drop it.
  - `box_shadow` is at `rune/src/el/visuals.rs:148`. `Shadow` (with `inset: bool`) is at `rune/src/el/visuals.rs:569`.
- **The three layers:** a contact edge, a soft drop that deepens with `lift`, and a gold glow driven by `glow`.

```rust
/// Shadow stack for a card: contact edge + soft drop (+ optional gold glow). Always 3 shadows
/// so the counts match between states and the geometry springs (r3 1.1).
fn card_shadows(e: El, lift: f32, glow: f32) -> El {
    let l = lift.clamp(0.0, 1.0);
    e.box_shadow(Shadow { y: 1.0 + 0.0 * l, blur: 0.0, color: c("#c4c7b9").alpha(1.0 - l), ..Default::default() })
        .box_shadow(Shadow { y: 4.0 + 16.0 * l, blur: 10.0 + 26.0 * l, color: Color::BLACK.alpha(0.28 + 0.2 * l), ..Default::default() })
        .box_shadow(Shadow { y: 0.0, blur: 28.0, spread: 2.0, color: c(GOLD).alpha(0.75 * glow), ..Default::default() })
}
```


### Card art

- **Face:** `pixels_fit(.., SurfaceFit::Fit)` (`rune/src/el/visuals.rs:223`).
- **Back:** `pixels` (`rune/src/el/visuals.rs:215`), which stretches the image to the element rect. This is what lets a width squeeze squash it.

```rust
fn img(s: &State, i: usize) -> Option<&Image> {
    s.faces.get(i % FACES.len())
}

/// A face-up card, art fitted into a rounded white card.
fn face(s: &State, i: usize) -> El {
    let e = el().size(CW, CH).radius(7.0).fill(c(CARD_WHITE));
    match img(s, i) {
        Some(im) => e.pixels_fit(im.pixels(), SurfaceFit::Fit),
        None => e,
    }
}
/// The back, stretched (so a squeezed width squashes it, for the flip).
fn back_px(s: &State, e: El) -> El {
    match &s.back {
        Some(im) => e.pixels(im.pixels()),
        None => e.fill(c("#8a2b2b")),
    }
}
```


### The rune-fx compositor (grain, glow, burst)

- **Registration.** Registered with `App::compositor` (`rune/src/app.rs:539`).
- **Effect slots.** Elements made with `effect(mark)` (`rune/src/el.rs:85`) are effect slots. `effect_bleed(pt)` (`rune/src/el/effect.rs:52`) lets an effect paint outside the slot.
- **`animating()`** returns true **only** while a burst is alive. Grain and glow are static, so the table keeps its zero-draw idle floor.
- **Line cited.** The relevant line is in `fx.rs` below (`fn animating`).

```rust
//! rune-fx compositor: grain on the felt, glow under a hovered card, sparkle + ripple burst.
use super::{BURST, GLOW, GRAIN, State};
use rune::compose::{Compositor, Frame};
use rune::render::{EffectPhase, EffectSite, KIND_EFFECT};
use rune_fx::*;

pub struct TableFx {
    pub fx: Option<FxRenderer>,
    /// True only while a time-varying effect is on screen (keeps the idle floor).
    pub alive: bool,
}

impl Compositor<State> for TableFx {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        let (device, queue, size) = (f.device, f.queue, f.size);
        let t = f.rt.t as f32;
        let reduced = f.rt.reduced_motion;
        let marks = &f.rt.marks;
        let slots: Vec<(u32, rune::render::Instance, u32)> = f
            .rt
            .instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.params[1] == KIND_EFFECT)
            .map(|(n, i)| (n as u32, *i, marks.get(n).copied().unwrap_or(0)))
            .collect();
        let burst = f.rt.state.burst;
        let mut alive = false;
        let mut layers: Vec<FxLayer> = Vec::new();
        let mut owner: Vec<usize> = Vec::new();
        let mut placements = Vec::new();
        for (k, (_, i, m)) in slots.iter().enumerate() {
            let sc = i.params[3];
            let [x, y, w, h] = i.rect.map(|v| v / sc);
            let r = i.radii.iter().fold(0.0f32, |a, v| a.max(v.abs())) / sc;
            let shape = Shape::rounded(x, y, w, h, r);
            let mut add = |l: FxLayer| {
                layers.push(l);
                owner.push(k);
                placements.push(FxSlot::default().scale(sc).clip(i.clip, i.fade));
            };
            match *m {
                GRAIN => add(Fx::grain(GrainParams { amount: 0.045, ..Default::default() }).on(shape).at(t).reduced(reduced)),
                GLOW => add(
                    Fx::glow(GlowParams {
                        style: GlowStyle::Soft,
                        color: rgb(0xf5d98e),
                        core: rgb(0xfff2c2),
                        radius: 14.0,
                        intensity: 0.8,
                        breathe: 0.0,
                        occlusion: 0.0,
                        ..Default::default()
                    })
                    .on(shape)
                    .at(t)
                    .reduced(reduced),
                ),
                BURST => {
                    if let Some(t0) = burst {
                        let sp = SparkleParams { count: 28, size: 11.0, life: 1.1, margin: 34.0, burst: Some(t0 as f32), color: rgb(0xffe6a8), ..Default::default() };
                        if t - (t0 as f32) < sp.burst_life() {
                            alive = true;
                            add(Fx::sparkles(sp).on(Shape::circle(x + w / 2.0, y + h / 2.0, 40.0)).at(t).reduced(reduced));
                            add(Fx::pulse(PulseParams { style: PulseStyle::Ripple { at: [w / 2.0, h / 2.0] }, color: rgb(0xf5d98e), t0: t0 as f32, duration: 0.8, reach: 110.0, ..Default::default() })
                                .on(shape)
                                .at(t)
                                .reduced(reduced));
                        }
                    }
                }
                _ => {}
            }
        }
        self.alive = alive;
        let fx = self.fx.get_or_insert_with(|| FxRenderer::new(device, f.format, FxOptions::default()));
        fx.set_srgb_blend(f.srgb_blend());
        fx.add_format(device, wgpu::TextureFormat::Rgba16Float);
        let _ = fx.prepare_with_slots(device, queue, &FxFrame { viewport: [size.0, size.1], scale: 1.0, layers: &layers, backdrop: None, sources: &[] }, &placements);
        let clear = f.rt.clear_color();
        f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, size);
        f.ui.draw_with_backdrop(f.encoder, f.target, clear, &mut |phase, s: &EffectSite<'_>| {
            if let EffectPhase::Draw(pass) = phase
                && let Some(k) = slots.iter().position(|(at, _, _)| *at == s.index)
            {
                for (li, o) in owner.iter().enumerate() {
                    if *o == k {
                        fx.draw_layer_in(pass, s.format, s.origin, li);
                    }
                }
            }
        });
    }

    fn animating(&self) -> bool {
        self.alive
    }
}

```


---

## 1. Curved seat-to-centre flight with rotation, scale lift and landing settle

- **Verified** at `85cd8c6`.
  - Shots: `shots/flight-strip.png` (West seat to West slot) and `shots/flight-you-strip.png` (rack to South slot).
  - In both: the card leaves fully opaque, rises on an arc, grows about 14% at mid-flight, swings from the seat tilt (-0.7 rad) past the slot tilt, and settles with a small 3.5% touchdown squash.

**Why poll-driven and not `matched`.**
- `El::matched` (`rune/src/el/motion.rs:215`) flies in a straight line and carries rect, velocity, radius, fill and opacity. It does **not** carry rotation (see technique 9, where the rotation visibly snaps).
- A Bezier arc, a rotation settle and a scale lift therefore need the view to compute the pose each frame.

```rust
const FLY_D: f32 = 0.55; // travel, seconds
const FLY_EASE: Easing = Easing::cubic(0.22, 0.7, 0.25, 1.0); // the web's play curve
const SETTLE: Spring = Spring::new(0.42, 0.3); // rotation settle wobble
const FLY_END: f32 = FLY_D + 0.4;

struct Pose {
    x: f32,
    y: f32,
    rot: f32,
    scale: f32,
    lift: f32,
}

fn pose(f: &Fl, now: f64) -> Pose {
    let t = ((now - f.t0) as f32).max(0.0);
    let u = FLY_EASE.eval((t / FLY_D).min(1.0));
    // quadratic Bezier, control point above the midpoint
    let cx = (f.from[0] + f.to[0]) * 0.5;
    let cy = (f.from[1] + f.to[1]) * 0.5 - f.arc;
    let a = 1.0 - u;
    let x = a * a * f.from[0] + 2.0 * a * u * cx + u * u * f.to[0];
    let y = a * a * f.from[1] + 2.0 * a * u * cy + u * u * f.to[1];
    let lift = (PI * u).sin();
    // rotation: spring from r0 to r1 so it swings past and settles (the "landing")
    let (rp, _) = SETTLE.sample(0.0, 0.0, 1.0, t);
    let rot = f.r0 + (f.r1 - f.r0) * rp;
    // scale: start size -> 1, plus a 14% lift at mid-flight, plus a 3.5% squash on touchdown
    let land = ((t - FLY_D) / 0.16).clamp(0.0, 1.0);
    let squash = if t > FLY_D { 0.035 * (PI * land).sin() } else { 0.0 };
    let scale = (f.s0 + (1.0 - f.s0) * u) * (1.0 + 0.14 * lift) * (1.0 - squash);
    Pose { x, y, rot, scale, lift }
}

fn flight_card(s: &State, k: usize, f: &Fl) -> El {
    let p = pose(f, s.now);
    let card = face(s, f.face).key(format!("fly-face-{k}"));
    card_shadows(
        el().key(format!("fly-{k}"))
            .absolute(p.x - CW / 2.0, p.y - CH / 2.0)
            .size(CW, CH)
            .radius(7.0)
            .rotate(p.rot)
            .scale(p.scale)
            .origin(0.5, 0.5)
            .instant_changes() // the view drives every frame: no layout spring lag
            .transition(Transition::none()) // no default 0.2 s fade on mount
            .child(card),
        p.lift,
        0.0,
    )
}
```

Poll arm (the flights are data; the view reads `s.flights`):
```rust
        Tech::Flight => {
            if at(s, 0.4, 1) {
                s.step = 1;
                s.flights.push(Fl { from: SEATS[1], to: SLOTS[1], arc: 110.0, r0: -0.7, r1: SLOT_ROT[1], s0: 0.55, t0: now, face: 4 });
            }
            if at(s, 1.6, 2) {
                s.step = 2;
                s.flights.push(Fl { from: [500.0, 520.0], to: SLOTS[0], arc: 40.0, r0: 0.0, r1: SLOT_ROT[0], s0: 1.0, t0: now, face: 0 });
            }
            due(0.4, &mut next);
            due(1.6, &mut next);
            let moving = s.flights.iter().any(|f| ((now - f.t0) as f32) < FLY_END);
            if moving {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
```

```rust
fn view_flight(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1)).child(seat_plate(1, "West", false)).child(seat_plate(0, "You", false));
    for (k, f) in s.flights.iter().enumerate() {
        r = r.child(flight_card(s, k, f));
    }
    r
}
```


**Timings.**
- Travel `FLY_D = 0.55 s` on `cubic(0.22, 0.7, 0.25, 1.0)` (the web app's play curve).
- Rotation settle `Spring::new(0.42, 0.3)`.
- Touchdown squash over 0.16 s after landing.
- Arc height: 110 pt from a side seat; 40 pt from your own rack.
- Start scale: 0.55 from a seat (seat cards are small); 1.0 from the rack.
- An earlier try at 0.46 s travel with settle 0.32/0.45 read as rushed then wobbly.

**Cost.**
- Animating frames: p50 0.12-0.14 ms, p95 0.25-0.27 ms (29 instances).
- The poll returns `false` once `t > FLY_D + 0.4`, and idle frames then draw nothing.

**Gotchas.**
- **Mount fade.** Without `.transition(Transition::none())`, the default mount fade shows the card at partial opacity for its first ~0.2 s.
- **Spring lag.** Without `.instant_changes()`, the element springs toward each frame's pose and lags behind the computed arc.
- **Stopping the poll.** Stop waking when nothing moves (`FLY_END`); otherwise the app redraws forever.
- **Handing off at landing.** In the real game the flying element is a temporary overlay keyed per play. At landing, swap it for the static trick-slot card with the same rect and rotation, and give that card `Transition::none()` too, so there is no fade at the swap.
- **Reduced motion.** Read `rt.reduced_motion` (or the app's setting) and jump `t` to the end.

## 2. Staggered deal of 5 cards to 4 seats

- **Verified** at `85cd8c6`. Shot: `shots/deal-strip.png`.
  - Twenty cards leave one deck spot (East, the dealer) round-robin, 55 ms apart.
  - Each springs to its seat: South face-up in a row, West/East stacked backs, North a slight fan.
  - The last card lands about 1.5 s after the deal starts.
- **No poll motion.** One commit mounts all 20.
  - Each card's insertion `Transition::offset(dx, dy)` (`rune-motion/src/transition.rs:210`) starts it at the deck.
  - `enter_delay` (`rune/src/el/enter_delay.rs:36`) with `Stagger::new(gap).delay(i, n)` (`rune-motion/src/choreo.rs:35`, `:52`) holds each one.

```rust
/// Final rect for card `n` (0..5) at seat `seat`.
fn deal_rect(seat: usize, n: usize) -> ([f32; 4], f32) {
    let [x, y] = SEATS[seat];
    let off = n as f32 - 2.0;
    match seat {
        0 => ([x - CW / 2.0 + off * (CW + 10.0), y - 120.0 - CH / 2.0 + 40.0, CW, CH], 0.0),
        2 => ([x - BW / 2.0 + off * 22.0, y + 34.0, BW, BH], off * 0.06),
        1 => ([x - BW / 2.0 + 16.0 + off * 4.0, y - BH / 2.0 - 80.0 + n as f32 * 22.0, BW, BH], 0.0),
        _ => ([x - BW / 2.0 - 16.0 - off * 4.0, y - BH / 2.0 - 80.0 + n as f32 * 22.0, BW, BH], 0.0),
    }
}
const DECK: [f32; 2] = [820.0, 470.0]; // dealer (East) puts the deck down here
const DEAL_GAP: f32 = 0.055;
const DEAL_SPRING: Spring = Spring::new(0.42, 0.12);

fn view_deal(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for (i, n) in ["You", "West", "North", "East (D)"].iter().enumerate() {
        r = r.child(seat_plate(i, n, false));
    }
    if s.step == 0 {
        // the deck at rest before the deal
        r = r.child(back_px(s, el().key("deck").absolute(DECK[0] - BW / 2.0, DECK[1] - BH / 2.0).size(BW, BH).radius(4.0)));
        return r;
    }
    // deal order: left of dealer first, round robin, 5 rounds
    let order = [0usize, 1, 2, 3];
    let total = 20;
    for round in 0..5 {
        for (k, &seat) in order.iter().enumerate() {
            let i = round * 4 + k;
            let ([x, y, w, h], rot) = deal_rect(seat, round);
            // offset from the card's own rect so its centre starts on the deck
            let dx = DECK[0] - (x + w / 2.0);
            let dy = DECK[1] - (y + h / 2.0);
            let t = Transition::offset(dx, dy).springs(SpringPair::new(DEAL_SPRING, Spring::new(0.2, 0.0)));
            let card = el().key(format!("deal-{seat}-{round}")).absolute(x, y).size(w, h).radius(if seat == 0 { 7.0 } else { 4.0 }).rotate(rot);
            let card = if seat == 0 { card.fill(c(CARD_WHITE)).pixels_fit(img(s, round).unwrap().pixels(), SurfaceFit::Fit) } else { back_px(s, card) };
            r = r.child(
                card.box_shadow(Shadow { y: 3.0, blur: 8.0, color: Color::BLACK.alpha(0.3), ..Default::default() })
                    .transition(t)
                    .enter_delay(Stagger::new(DEAL_GAP).delay(i, total))
                    .paint_order((total - i) as i16), // first dealt paints on top of the waiting deck
            );
        }
    }
    r
}
```

Poll arm:
```rust
        Tech::Deal => {
            if at(s, 0.4, 1) {
                s.step = 1;
                changed = true;
            }
            due(0.4, &mut next);
        }
```


**Timings.**
- `DEAL_GAP = 0.055 s`.
- Travel spring `Spring::new(0.42, 0.12)`: a small overshoot reads as a toss.
- Opacity spring 0.2/0.

**Cost.**
- p50 0.91 ms, p95 6.1 ms. The p95 is the first frames, when 20 elements and their images mount.
- A cold single snapshot at 0.55 s reported p95 17.9 ms, which includes first-use image upload.
- **Pre-upload the card images at setup** (the spike `wait()`s every image in `setup`).

**Gotchas.**
- **Offset is from the card's own rect.** Compute `DECK - own centre` per card. `offset` is not clamped; scale is (`kit_rules`, `rune-motion/src/transition.rs:148`), so the deal cannot also grow cards from 0.5x to 1x via a scale transition. Clamp is [0.9, 1.1], or a 0.5 floor with `allow_small_scale`.
- **Paint order.** `.paint_order((total - i) as i16)` (`rune/src/el/motion.rs:142`) keeps the next card to leave on top of those still waiting at the deck.
- **South faces are visible as they fly** (the spike mounts them face-up). For a face-down deal, mount backs, then run technique 3 per card after it lands (or flip all five together).
- **Insertion only.** `enter_delay` holds fresh mounts only. Re-dealing needs new keys (for example `deal-{hand}-{seat}-{n}`).
- **Real deal shape.** The real Euchre deal is 3-2 / 2-3 packets. Change the order array and gaps; the mechanism is the same.

## 3. Face-down to face-up flip

There is no 3D and no `scale_x` (r1/r2). A flip is a width squeeze about the centre, with the art swapped at the midpoint. **Both variants are verified** at `85cd8c6`; shot `shots/flip-strip.png` (A left, B right).

- **A, poll-driven.**
  - Width is `CW * |cos(pi*u)|` with `u` eased over 0.36 s.
  - Back until `u >= 0.5`, then face.
  - The wrapper lifts `scale(1 + 0.1*lift)`, and the shadows deepen with it.
- **B, two commits.**
  - Commit 1 sets width to 1 under `tx.motion_curve(Curve::tween(0.18, EASE_IN))`.
  - Commit 2, at +0.18 s, sets the face and full width under `Curve::tween(0.18, EASE_OUT)`.
  - Citations: `tx.motion_curve` at `rune/src/el/tx.rs:536`; `Curve::tween` at `rune-motion/src/curve.rs:23`.
  - No per-frame view runs except at the two commits.
  - **Yes, the art squeezes:** `pixels` stretches to the tweened layout width.

```rust
const FLIP_D: f32 = 0.36;

fn view_flip(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    // A: poll-driven width squeeze, face swaps at the midpoint, lift scale + shadow
    let u = match s.flip_t0 {
        Some(t0) => Easing::EASE_IN_OUT.eval((((s.now - t0) as f32) / FLIP_D).clamp(0.0, 1.0)),
        None => 0.0,
    };
    let wide = (CW * (PI * u).cos().abs()).max(1.0);
    let lift = (PI * u).sin();
    let showing_face = u >= 0.5;
    let inner = el().key("flip-a-art").absolute((CW - wide) / 2.0, 0.0).size(wide, CH).radius(7.0);
    let inner = if showing_face {
        inner.fill(c(CARD_WHITE)).pixels(img(s, 0).unwrap().pixels())
    } else {
        back_px(s, inner)
    };
    let inner = card_shadows(inner, lift, 0.0);
    r = r.child(
        el().key("flip-a").absolute(300.0 - CW / 2.0, 320.0 - CH / 2.0).size(CW, CH).scale(1.0 + 0.1 * lift).instant_changes().child(inner),
    );
    // B: two commits, layout tween on width (no per-frame view runs)
    let wb = if s.flip_b_narrow { 1.0 } else { CW };
    let inner_b = el().key("flip-b-art").absolute((CW - wb) / 2.0, 0.0).size(wb, CH).radius(7.0);
    let inner_b = if s.flip_b_face { inner_b.fill(c(CARD_WHITE)).pixels(img(s, 1).unwrap().pixels()) } else { back_px(s, inner_b) };
    r = r.child(
        el().key("flip-b")
            .absolute(700.0 - CW / 2.0, 320.0 - CH / 2.0)
            .size(CW, CH)
            .child(inner_b.box_shadow(Shadow { y: 4.0, blur: 10.0, color: Color::BLACK.alpha(0.3), ..Default::default() })),
    );
    r.child(el().absolute(240.0, 420.0).child(label("A: poll squeeze", 13.0, "#d9e6cb")))
        .child(el().absolute(630.0, 420.0).child(label("B: two-commit tween", 13.0, "#d9e6cb")))
}
```

Poll arm:
```rust
        Tech::Flip => {
            if at(s, 0.5, 1) {
                s.step = 1;
                s.flip_t0 = Some(now);
                s.flip_b_narrow = true;
                tx.motion_curve(Curve::tween(FLIP_D / 2.0, Easing::EASE_IN));
                changed = true;
            }
            if at(s, 0.5 + (FLIP_D / 2.0) as f64, 2) {
                s.step = 2;
                s.flip_b_narrow = false;
                s.flip_b_face = true;
                tx.motion_curve(Curve::tween(FLIP_D / 2.0, Easing::EASE_OUT));
                changed = true;
            }
            due(0.5, &mut next);
            due(0.5 + (FLIP_D / 2.0) as f64, &mut next);
            if s.flip_t0.is_some_and(|t0| now - t0 < FLIP_D as f64 + 0.02) {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
```


**Timings.**
- 0.36 s total; 0.18 s per half.
- EASE_IN into the edge and EASE_OUT out of it reads as a card turning, not a door.

**Cost.**
- p50 0.17 ms, p95 0.31 ms.
- B costs nothing between commits.

**Gotchas.**
- **Fit vs stretch.** Use `.pixels` (stretch), not `pixels_fit(Fit)`, for the art during the squeeze. Fit letterboxes inside the narrow rect and the art stops squashing.
- **Inner element.** Keep the squeezed child centred inside a fixed-size outer, so siblings don't reflow.
- **Prefer A for a lift.** A gives the lift and shadow arc in one place. B is cheaper and fine for many cards at once (flipping a whole hand).
- **Width floor.** Width is floored at 1 pt (`max(1.0)`), because a 0-width element may be culled and lose its key's animation state.

## 4. Trick sweep to the winner's seat, fading

- **Verified** at `85cd8c6`.
  - Shots: `shots/sweep-strip.png` (pop + gather), `shots/sweep-strip2.png` (one-commit sweep), `shots/sweep-commits-strip2.png` (staggered sweep), `shots/sweep-pop-strip.png` (pop with no cross-fade).
- **Three beats:**
  1. **Pop at t=0.4.** The winning card gets a `WIN_POP` `Sequence` (scale 1 to 1.09 to 1 over 0.3 s), and a gold ring fades in with `RING_IN`.
     - The element API is `.sequence()` (`rune/src/el/motion.rs:259`); the `Sequence` type is at `rune-motion/src/sequence.rs:23`.
     - It replays only on a **new element key**.
  2. **Gather at t=1.0.** Cards spring to a tight pile at the centre with `.motion(Spring::new(0.3, 0.12))` (`rune/src/el/motion.rs:89`).
  3. **Sweep at t=1.5.**
     - Each trick card leaves the tree.
     - A 46x64 mini with the same `matched("m-trick-i")` mounts at the winner's seat with `opacity(0)`.
     - The flight carries rect and opacity, so the card shrinks, travels and fades together.
     - The commit sets `tx.unclipped_flights()` (`rune/src/flight.rs:36`) and `tx.motion(SWEEP)` (`rune/src/el/tx.rs:527`).

```rust
const WINNER: usize = 2; // North
const GATHER: Spring = Spring::new(0.3, 0.12);
const SWEEP: Spring = Spring::new(0.5, 0.0);

const WIN_POP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.12).scale(1.09).easing(Easing::EASE_OUT),
    Beat::new(0.3).scale(1.0),
];
const RING_IN: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(1.18),
    Beat::new(0.22).opacity(1.0).scale(1.0).easing(Easing::EASE_OUT),
];

fn view_sweep(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for (i, n) in ["You", "West", "North (wins)", "East"].iter().enumerate() {
        r = r.child(seat_plate(i, n, i == WINNER && s.step >= 1));
    }
    let gathered = s.step >= 2;
    let commits = s.tech == Tech::SweepCommits;
    for i in 0..4 {
        let id = format!("trick-{i}");
        let sent = if commits { i < s.sweep_sent } else { s.step >= 3 };
        if sent {
            continue;
        }
        let [x, y] = if gathered { [500.0 + (i as f32 - 1.5) * 6.0, 300.0 - i as f32 * 2.0] } else { SLOTS[i] };
        let rot = if gathered { (i as f32 - 1.5) * 0.05 } else { SLOT_ROT[i] };
        // Transition::none(): the re-key below would otherwise cross-fade old and new art
        let mut card = card_shadows(face(s, i + 2), 0.0, 0.0).transition(Transition::none());
        if i == WINNER && s.step >= 1 {
            card = card.key(format!("{id}-pop")).sequence(Sequence::new(WIN_POP));
        }
        let mut e = el().key(id.clone())
            .absolute(x - CW / 2.0, y - CH / 2.0)
            .size(CW, CH)
            .rotate(rot)
            .origin(0.5, 0.5)
            .motion(GATHER)
            .matched(format!("m-{id}"))
            .paint_order(if i == WINNER { 10 } else { i as i16 })
            .child(card);
        if i == WINNER && s.step >= 1 && !gathered {
            e = e.child(
                el().key("win-ring").absolute(-6.0, -6.0).size(CW + 12.0, CH + 12.0).radius(11.0).border(3.0, c(GOLD)).sequence(Sequence::new(RING_IN)),
            );
        }
        r = r.child(e);
    }
    // the winner's pile: minis that the trick cards fly into, fading to 0
    let [wx, wy] = SEATS[WINNER];
    for i in 0..4 {
        let sent = if commits { i < s.sweep_sent } else { s.step >= 3 };
        if !sent {
            continue;
        }
        let mut mini = el().key(format!("mini-{i}"))
            .absolute(wx - 23.0 + i as f32 * 3.0, wy + 26.0)
            .size(46.0, 64.0)
            .radius(4.0)
            .rotate(0.0)
            .opacity(0.0)
            .matched(format!("m-trick-{i}"))
            .child(face(s, i + 2).size(46.0, 64.0).radius(4.0));
        if !commits {
            mini = mini.enter_delay(Stagger::new(0.06).delay(i, 4));
        }
        r = r.child(mini);
    }
    r
}
```

Poll arm (the `SweepCommits` branch is the staggered version):
```rust
        Tech::Sweep | Tech::SweepCommits => {
            for (t, st) in [(0.4, 1), (1.0, 2), (1.5, 3)] {
                if at(s, t, st) {
                    s.step = st;
                    changed = true;
                    if st == 3 {
                        tx.unclipped_flights();
                        tx.motion(SWEEP);
                        if s.tech == Tech::SweepCommits {
                            s.sweep_sent = 1;
                        }
                    }
                }
                due(t, &mut next);
            }
            if s.tech == Tech::SweepCommits && s.step >= 3 && s.sweep_sent < 4 {
                let t = 1.5 + 0.06 * s.sweep_sent as f64;
                if now >= t {
                    s.sweep_sent += 1;
                    tx.unclipped_flights();
                    tx.motion(SWEEP);
                    changed = true;
                }
                due(1.5 + 0.06 * s.sweep_sent as f64, &mut next);
            }
        }
```


**Timings.**
- Pop 0.3 s. Gather spring 0.3/0.12. Sweep spring 0.5/0 (about 1.1 s to fully gone).
- Stagger 60 ms between cards.
- 0.5 s reads slightly slow next to the 0.55 s play flight; 0.4 is probably better (not rendered).

**Cost.**
- One commit: p50 0.33 ms, p95 0.75 ms.
- Staggered commits: p50 0.63 ms, p95 1.19 ms.

**Gotchas.**
- **`enter_delay` does NOT stagger a matched flight.** With `enter_delay(Stagger 0.06)` on the minis, all four cards still fly together (`sweep-strip2.png`).
  - To stagger, send one card per commit, 60 ms apart, from the poll with `wake_at` (`sweep-commits-strip2.png` shows a clean trail).
  - So r2's open question is answered: the delay only holds the mini's own mount, not the flight.
- **Re-keying cross-fades.** Re-keying an element to replay its `Sequence` makes the old and new art cross-fade through the default mount/unmount fade. Frame 4 of `sweep-strip.png` showed a ghosted, doubled queen.
  - Fix: put `.transition(Transition::none())` on the re-keyed element (`sweep-pop-strip.png` is clean).
  - The same bug and fix applied to the score pop (technique 6).
- **Flight paint order** inside a one-commit sweep does not follow `paint_order` on the source. The top card of the pile changed mid-flight. Staggered commits avoid this, because each flight is alone.
- **Unclipped flights.** Without `unclipped_flights()`, a flight crossing a clipping ancestor is cut at the clip edge.
- **Reduced motion.** Under reduced motion, `matched` does not fly; the cards just leave and the minis appear.

## 5. Hover/press spring lift, deeper shadow and glow, illegal-card shake

- **Verified** at `85cd8c6`, with the scripted pointer `scripts/hover.json` (headless `--script`; the parser is `rune/src/tooling/script.rs:19`). Shots:

  | Shot | Shows |
  |---|---|
  | `shots/hover-strip.png` | Hovered card lifts 16 pt, scales 1.05 from its bottom edge, shadow deepens, gold glow fades in |
  | `shots/hover-press-strip.png` | Hover moves to the next card; the press dips 0.96 |
  | `shots/hover-shake-strip.png` | The dimmed illegal card shakes three times with a slight rotation |
  | `shots/hover-fx-0.7.png` | rune-fx Soft glow instead of the shadow glow |

- **Hover lift.** `.motion(HOVER_SPRING)` springs the offset and scale.
- **Plate child.** A child plate carries the 3-shadow stack on the same spring (r3 1.2), so shadow geometry and lift move together.
- **APIs:**

  | API | Citation |
  |---|---|
  | `on_hover_change` | `rune/src/el/layout.rs:676` |
  | `on_press_tx` | `rune/src/el/handlers.rs:10` |
  | `press_scale` | `rune/src/el/visuals.rs:350` |
  | `tx.clock()` (shake start time) | `rune/src/el/tx.rs:234` |
  | `origin(0.5, 1.0)` (lift from the bottom edge) | `rune/src/el/visuals.rs:399` |

```rust
const HOVER_SPRING: Spring = Spring::new(0.26, 0.32);

fn shake_x(t: f32) -> f32 {
    // damped sine: 3 swings in 0.36 s
    if !(0.0..0.36).contains(&t) {
        return 0.0;
    }
    11.0 * (-t / 0.11).exp() * (2.0 * PI * t / 0.12).sin()
}

fn view_hover(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    let n = 5;
    let illegal = 4;
    let fxglow = s.tech == Tech::HoverFx;
    let mut hand = row().key("rack").absolute(500.0 - (n as f32 * (CW + 14.0)) / 2.0, 430.0).gap(14.0).items_end().h(CH + 40.0);
    for i in 0..n {
        let hot = s.hovered == Some(i) && i != illegal;
        let lift = if hot { 1.0 } else { 0.0 };
        let shake = match s.shake {
            Some((k, t0)) if k == i => shake_x((s.now - t0) as f32),
            _ => 0.0,
        };
        // plate carries the shadows on the same spring as the lift (r3 1.2)
        let plate = card_shadows(el().key(format!("plate-{i}")).absolute(0.0, 0.0).size(CW, CH).radius(7.0).motion(HOVER_SPRING), lift, if fxglow { 0.0 } else { lift });
        let mut art = face(s, i + 6).absolute(0.0, 0.0);
        if i == illegal {
            art = art.opacity(0.55);
        }
        let mut card = el().key(format!("rack-{i}"))
            .size(CW, CH)
            .motion(HOVER_SPRING)
            .offset(shake, if hot { -16.0 } else { 0.0 })
            .scale(if hot { 1.05 } else { 1.0 })
            .rotate(shake * 0.004)
            .origin(0.5, 1.0)
            .press_scale(0.96)
            .on_hover_change(move |s: &mut State, on, _tx| {
                if on {
                    s.hovered = Some(i)
                } else if s.hovered == Some(i) {
                    s.hovered = None
                }
            })
            .on_press_tx(move |s: &mut State, tx| {
                if i == illegal {
                    s.shake = Some((i, tx.clock()));
                }
            });
        if fxglow && hot {
            card = card.child(
                effect(GLOW).key(format!("glow-{i}")).effect_bleed(64.0).absolute(0.0, 0.0).size(CW, CH).radius(7.0).transition(Transition::opacity()),
            );
        }
        card = card.child(plate).child(art);
        if s.shake.is_some_and(|(k, t0)| k == i && s.now - t0 < 0.36) {
            card = card.instant_changes(); // the view drives the shake per frame
        }
        hand = hand.child(card);
    }
    r.child(hand)
        .child(el().absolute(40.0, 40.0).child(label(if fxglow { "hover: rune-fx glow" } else { "hover: core shadow glow" }, 13.0, "#d9e6cb")))
}
```

Poll arm (only the shake needs frames):
```rust
        Tech::Hover | Tech::HoverFx => {
            if s.shake.is_some_and(|(_, t0)| now - t0 < 0.4) {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
```

Script used:
```json
[{"t":0.3,"type":"move","key":"rack-1"},
 {"t":0.9,"type":"move","key":"rack-2"},
 {"t":1.2,"type":"down","key":"rack-2"},
 {"t":1.45,"type":"up","key":"rack-2"},
 {"t":2.0,"type":"click","key":"rack-4"},
 {"t":2.6,"type":"move","x":500,"y":120}]
```

**Timings.**
- Spring 0.26/0.32: quick, with a little bounce.
- Lift -16 pt, scale 1.05.
- Press 0.96.
- Shake: `11*exp(-t/0.11)*sin(2*pi*t/0.12)` for 0.36 s, rotation `shake*0.004` rad.

**Cost.**

| Case | p50 | p95 |
|---|---|---|
| Hover | 0.45 ms | 0.61 ms |
| Press | 0.73 ms | 3.6 ms (one spike) |
| Shake | 0.54 ms | 1.09 ms |
| rune-fx glow | 0.75 ms | 2.2 ms |

**Gotchas.**
- **Shake vs spring.** `instant_changes()` only while shaking. Otherwise the spring smooths the shake into mush; outside the shake, the hover spring must stay.
- **rune-fx glow bleed.**
  - The glow is clipped hard to its slot rect plus `effect_bleed`. At bleed 40 with radius 20 / intensity 1.1 the clip showed as a visible rectangle.
  - Bleed 64, radius 14, intensity 0.8 fixed it (the values in `fx.rs` above).
  - The glow also paints over neighbouring cards. On green felt the gold reads slightly yellow-green.
  - The core 3rd shadow (gold, blur 28, spread 2) looked as good and costs less. **Prefer the core shadow glow** and keep rune-fx for bursts.
- **Illegal card.** The illegal card is dimmed with `opacity(0.55)` on the art only; the white plate shows through and reads as "washed out", which is the intent.
- **Press feedback.** `press_scale(0.96)` on an already-lifted card is subtle. If the press should read, drop the lift by 4 pt on press as well.

## 6. Score odometer / count-up with a pulse

- **Verified** at `85cd8c6`. Shot: `shots/score-strip.png`.
  - Us counts 4, 5, 6 with each digit rolling up.
  - A gold ring pulses out once around the box.
  - The number pops 1.18x.
  - A "+2" chip rises in, then leaves upward.
  - Them rolls 7 to 8 at t=1.6.
- **APIs:**
  - `RollingText::new(key, text).value(f64).font(..).color(..).spring(..).build(&Kit)`: struct at `rune-kit/src/rolling_text.rs:93`; `.spring` at `:131`; `.value` at `:145`; `.build` at `:159`.
  - Tabular figures: `TextStyle::features("'tnum' 1")`.

```rust
const PULSE: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(0.9),
    Beat::new(0.12).opacity(0.9).scale(1.12).easing(Easing::EASE_OUT),
    Beat::new(0.5).opacity(0.0).scale(1.35).easing(Easing::EASE_IN_OUT),
];
const NUM_POP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.1).scale(1.18).easing(Easing::EASE_OUT),
    Beat::new(0.32).scale(1.0).easing(Easing::OVERSHOOT),
];

fn view_score(s: &State, root: El) -> El {
    let k = Kit::neutral(false);
    let big = TextStyle::new(56.0, 600.0).features("'tnum' 1");
    let team = |key: &str, name: &str, v: u32, pulse: u32| -> El {
        let num = RollingText::new(format!("{key}-n"), v.to_string()).value(v as f64).font(big).color(c(INK)).spring(Spring::new(0.42, 0.18)).build(&k);
        let mut box_ = el().key(format!("{key}-box")).size(180.0, 150.0).radius(18.0).fill(c(CREAM)).center().box_shadow(Shadow {
            y: 6.0,
            blur: 18.0,
            color: Color::BLACK.alpha(0.3),
            ..Default::default()
        });
        if pulse > 0 {
            // keyed by the pulse count: a new key replays the sequence
            box_ = box_.child(
                el().key(format!("{key}-pulse-{pulse}")).absolute(-8.0, -8.0).size(196.0, 166.0).radius(24.0).border(3.0, c(GOLD)).sequence(Sequence::new(PULSE)),
            );
        }
        let num = if pulse > 0 { el().key(format!("{key}-pop-{pulse}")).sequence(Sequence::new(NUM_POP)).transition(Transition::none()).child(num) } else { el().key(format!("{key}-pop-0")).child(num) };
        box_.child(rune::col().center().gap(4.0).child(label(name, 14.0, "#58695b")).child(num))
    };
    let mut r = root.child(felt(s, 1)).child(
        row().key("scores").absolute(500.0 - 200.0, 220.0).gap(40.0).child(team("us", "Us", s.us, s.pulse)).child(team("them", "Them", s.them, 0)),
    );
    if s.pulse > 0 && s.step < 3 {
        r = r.child(
            el().key(format!("delta-{}", s.pulse))
                .absolute(390.0, 180.0)
                .padding(8.0)
                .radius(12.0)
                .fill(c(GOLD))
                .child(label("+2", 18.0, INK))
                .transition(Transition::asymmetric(
                    Transition::offset(0.0, 14.0).combined(Transition::opacity()),
                    Transition::offset(0.0, -18.0),
                )
                .springs(SpringPair::new(Spring::new(0.4, 0.3), Spring::new(0.3, 0.0)))),
        );
    }
    r
}
```

Poll arm (one point per 0.14 s):
```rust
        Tech::Score => {
            if at(s, 0.5, 1) {
                s.step = 1;
                s.us_target = 6;
                s.pulse = 1;
                changed = true;
            }
            // count up one point per 0.14 s
            if s.us < s.us_target {
                let t = 0.5 + 0.14 * (s.us - 4) as f64;
                if now >= t {
                    s.us += 1;
                    changed = true;
                }
                due(0.5 + 0.14 * (s.us - 4) as f64, &mut next);
            }
            if at(s, 1.6, 2) {
                s.step = 2;
                s.them = 8;
                changed = true;
            }
            if at(s, 2.2, 3) {
                s.step = 3;
                changed = true;
            }
            due(0.5, &mut next);
            due(1.6, &mut next);
            due(2.2, &mut next);
        }
```


**Timings.**
- Roll spring 0.42/0.18.
- 0.14 s per point. 2 points in 0.28 s; a 4-point march would be 0.56 s.
- Ring 0.5 s. Number pop 0.32 s.
- Chip: in 0.4/0.3, out 0.3/0.

**Cost.**
- p50 0.15 ms, p95 0.23 ms.
- An earlier run, with the cross-fade bug, measured 0.58 / 1.53 ms.

**Gotchas.**
- **Re-key cross-fade.** The pop wrapper is re-keyed per pulse to replay its `Sequence`. Without `.transition(Transition::none())` on it, the whole box contents ("Us" label and number) cross-fade and look ghosted. Fixed in the code above.
- **Roll spill.** The rolling digits visibly spill a little below the number line mid-roll. That is RollingText's look; to contain it, clip the number cell.
- **`value()` matters.** `value()` sets the direction of the roll (up for an increase).

## 7. Breathing turn indicator

- **Verified** at `85cd8c6`. Shots: `shots/turn-strip.png` (the halo breathes on West), `shots/turn-move-strip.png` (the marker springs West to North at t=3.4, with a slight overshoot).
- **Mechanism:**
  - The halo is a gold `box_shadow` with `.cycle(BREATHE)`.
    - Element API: `rune/src/el/visuals.rs:556`.
    - `Cycle` fields `period/phase/stops/still/fps`: `rune-motion/src/cycle.rs:8`.
  - A thin ring with a re-keyed `PING` `Sequence` expands and fades once per breath.
  - One keyed marker moves between seats with `.motion(Spring::new(0.5, 0.2))`.

```rust
static BREATHE: Cycle = Cycle { period: 1.6, phase: 0.0, stops: &[(0.0, 0.35), (0.5, 1.0), (1.0, 0.35)], still: 0.8, fps: 30.0 };
const PING: &[Beat] = &[
    Beat::new(0.0).opacity(0.7).scale(1.0),
    Beat::new(0.9).opacity(0.0).scale(1.45).easing(Easing::EASE_OUT),
];

fn view_turn(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for (i, n) in ["You", "West", "North", "East"].iter().enumerate() {
        r = r.child(seat_plate(i, n, i == s.turn));
    }
    let [x, y] = SEATS[s.turn];
    let (w, h) = (130.0, 40.0);
    // one keyed marker that moves between seats on a layout spring
    let marker = el()
        .key("turn-marker")
        .absolute(x - w / 2.0, y - h / 2.0)
        .size(w, h)
        .motion(Spring::new(0.5, 0.2))
        .child(
            el().key("halo").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).box_shadow(Shadow {
                blur: 30.0,
                spread: 4.0,
                color: c(GOLD).alpha(0.75),
                ..Default::default()
            })
            .cycle(BREATHE),
        )
        .child(
            el().key(format!("ping-{}", s.ping))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(20.0)
                .border(2.0, c(GOLD))
                .sequence(Sequence::new(PING)),
        );
    r.child(marker)
}
```

Poll arm (it only re-keys the ping per period and moves the turn):
```rust
        Tech::Turn => {
            // ping once per breath period while the seat waits
            let k = ((now - 0.2) / 1.6).floor().max(0.0) as u32 + 1;
            if now >= 0.2 && k != s.ping {
                s.ping = k;
                changed = true;
            }
            due(0.2 + 1.6 * s.ping as f64, &mut next);
            if at(s, 3.4, 1) {
                s.step = 1;
                s.turn = 2;
                changed = true;
            }
            due(3.4, &mut next);
        }
```


**Timings.**
- Period 1.6 s, opacity 0.35 to 1.0.
- Ping 0.9 s, scale 1 to 1.45.
- Seat move 0.5/0.2.

**Cost.**
- p50 0.12 ms, p95 0.36 ms.
- `Cycle` wakes at its `fps` (30) the whole time it is on screen: 179 animating frames of about 250 in a 4 s run.
- That is the only technique that breaks the zero-draw idle floor. Keep `fps` at 30 or lower, and show the halo only on the seat whose turn it is.

**Gotchas.**
- **Ping key.** The ping is re-keyed from the poll on a `wake_at` schedule. It must be keyed off a counter, not `now`, or it re-mounts every frame.
- **Reduced motion.** `still` (0.8) is the opacity used under reduced motion.
- **Paired halo.** If the halo also lives on a seat plate that springs, keep it a child of the moving marker (as here), so halo and marker move together.

## 8. Particle, confetti and spark bursts

- **Verified** at `85cd8c6`. Shots: `shots/burst-zoom-strip.png` (left: core canvas sparks; right: rune-fx sparkles + ripple), `shots/burst-2.4.png` and `shots/burst-3.0.png` (confetti cannons).
- **Core canvas:**
  - `canvas(shape)` (`rune/src/el.rs:359`) is rebuilt per frame from `t`.
  - Spark streaks use `Shape::stroke` (`rune/src/shape.rs:62`).
  - Confetti uses filled quads.
- **rune-fx:** `Fx::sparkles` with `burst: Some(t0)`, plus `Fx::pulse` Ripple (see `fx.rs`).

```rust
fn sparks(t: f32, cx: f32, cy: f32) -> VShape {
    let mut sh = VShape::new();
    let n = 28;
    for i in 0..n {
        let a = i as f32 / n as f32 * 2.0 * PI + (i as f32 * 1.7).sin() * 0.12;
        let speed = 260.0 + 140.0 * ((i * 7919 % 13) as f32 / 13.0);
        let drag = (-4.0 * t).exp();
        let d = speed * (1.0 - drag) / 4.0;
        let g = 160.0 * t * t; // gravity
        let (dx, dy) = (a.cos(), a.sin());
        let head = [cx + dx * (40.0 + d), cy + dy * (56.0 + d) + g]; // start at the card edge
        let tail_len = 26.0 * drag + 2.0;
        let tail = [head[0] - dx * tail_len, head[1] - dy * tail_len];
        let life = (1.0 - t / 0.9).clamp(0.0, 1.0);
        if life <= 0.0 {
            continue;
        }
        let col = if i % 3 == 0 { c("#ffffff") } else { c("#ffd75e") };
        sh = sh.stroke(vec![tail, head], 3.0 * life + 0.5, col.alpha(life));
    }
    sh
}

fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

pub fn confetti(t: f64, w: f32, h: f32) -> VShape {
    let mut sh = VShape::new();
    let cols = [c("#e0c579"), c("#497d69"), c("#ba5e46"), c("#fff8df"), c("#f5d98e")];
    for i in 0..120u32 {
        let left = i % 2 == 0;
        let (ox, dir) = if left { (0.06 * w, 1.0) } else { (0.94 * w, -1.0) };
        let oy = h * 0.95;
        let life = 2.6 + hash(i * 7 + 1);
        let tt = (t as f32 - hash(i * 3 + 5) * 0.25).max(0.0);
        if tt > life {
            continue;
        }
        let speed = 760.0 + 420.0 * hash(i * 11 + 2);
        let ang = (60.0 + 22.0 * hash(i * 13 + 3)).to_radians();
        let (vx, vy) = (dir * speed * ang.cos(), -speed * ang.sin());
        let drag = (-1.6 * tt).exp();
        let x = ox + vx * (1.0 - drag) / 1.6 + 30.0 * (tt * 3.0 + i as f32).sin() * (1.0 - drag);
        let y = oy + vy * (1.0 - drag) / 1.6 + 420.0 * tt * tt * 0.5 * 0.55 + 90.0 * tt;
        let spin = tt * (4.0 + 6.0 * hash(i * 17 + 4)) + i as f32;
        let (sw, shh) = (9.0 * spin.cos().abs().max(0.15), 5.0);
        let (cs, sn) = (spin.sin(), spin.cos());
        let pts: Vec<[f32; 2]> = [[-sw, -shh], [sw, -shh], [sw, shh], [-sw, shh]].iter().map(|p| [x + p[0] * sn - p[1] * cs, y + p[0] * cs + p[1] * sn]).collect();
        let fade = ((life - tt) / 0.5).clamp(0.0, 1.0);
        sh = sh.fill(pts, cols[(i % 5) as usize].alpha(fade));
    }
    sh
}

fn view_burst(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    // left: core canvas sparks around the winning card; the canvas goes BEFORE the card so
    // the card paints over the streaks (tree order is paint order)
    if let Some(t0) = s.burst {
        let t = (s.now - t0) as f32;
        if t < 0.9 {
            r = r.child(canvas(sparks(t, 300.0, 300.0)).key("sparks").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).instant_changes());
        }
    }
    r = r.child(card_shadows(face(s, 0).key("win-l").absolute(300.0 - CW / 2.0, 300.0 - CH / 2.0), 0.0, 0.0));
    // right: rune-fx sparkles + ripple slot behind the card
    r = r.child(
        el().key("win-r").absolute(700.0 - 80.0, 300.0 - 60.0).size(160.0, 120.0).child(
            effect(BURST).key("burst-slot").effect_bleed(80.0).absolute(0.0, 0.0).size(160.0, 120.0).radius(60.0),
        ),
    );
    r = r.child(card_shadows(face(s, 1).key("win-r-card").absolute(700.0 - CW / 2.0, 300.0 - CH / 2.0), 0.0, 0.0));
    if let Some(t0) = s.won {
        if s.now - t0 < 3.7 {
            r = r.child(canvas(confetti(s.now - t0, W, H)).key("confetti").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).instant_changes());
        }
    }
    r
}
```

Poll arm:
```rust
        Tech::Burst => {
            if at(s, 0.4, 1) {
                s.step = 1;
                s.burst = Some(now);
                changed = true;
            }
            if at(s, 1.6, 2) {
                s.step = 2;
                s.won = Some(now);
                changed = true;
            }
            due(0.4, &mut next);
            due(1.6, &mut next);
            let alive = s.burst.is_some_and(|t0| now - t0 < 0.95) || s.won.is_some_and(|t0| now - t0 < 3.75);
            if alive {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
```


**Timings.**
- Sparks 0.9 s: drag `exp(-4t)`, gravity 160 pt/s^2, 28 streaks.
- rune-fx sparkles life 1.1 s, 28 stars; ripple 0.8 s, reach 110.
- Confetti: 120 pieces from both bottom corners at 760-1180 pt/s, 60-82 degrees, drag 1.6, 2.6-3.6 s life, 0.5 s fade.

**Cost.**
- Sparks: p50 0.10 ms, p95 0.15 ms.
- Confetti: p50 0.14 ms, p95 0.23 ms.
- Both drive frames only while alive.
- The rune-fx burst sets `alive` and `animating()` only until `sp.burst_life()` passes.

**Gotchas.**
- **Tree order is paint order.** The first spark render drew streaks over the card face. Mount the spark canvas *before* the card it surrounds, and start each spark at the card edge (`40 + d`, `56 + d`).
- **Confetti layering.** Confetti is mounted last on purpose, so it falls over the cards.
- **First confetti speed was too low.** At 520-900 pt/s it barely reached mid-table; raised to 760-1180.
- **Small shapes and scale.** Canvas shapes are in pt; at 1x scale the 3 pt streaks are thin, so look at 2x before judging.
- **Clipping.** rune-fx sparkles sit on a circle shape (`Shape::circle`), are capped at 64, and are clipped to the slot plus `effect_bleed` (80 here).
- **Reduced motion.** Skip both bursts under reduced motion; `.reduced(reduced)` already damps the rune-fx ones.

## 9. Route cross-fade + slide, with a shared-element card

- **Verified** at `85cd8c6`. Shots: `shots/route-strip.png` (slide + fade) and `shots/route-blur-strip.png` (`blur_replace`).
- **Screens:**
  - The two screens are keyed full-window elements: `screen-home` and `screen-table`.
  - Switching the key removes one and inserts the other.
  - Transition: asymmetric (`rune-motion/src/transition.rs:250`). Insert is `offset(70,0) + opacity`; removal is `offset(-70,0)` (removal opacity is forced to 0 by `kit_rules`).
  - Springs: `.springs(SpringPair::new(enter, exit))` (`rune-motion/src/transition.rs:261`, `spring.rs:182`).
- **Shared hero card.** `matched("hero-JS")` on both screens. It flies from the big tilted home card to the table centre, opaque and sharp, **above both screens** for the whole transition (shots frames 2-6).

```rust
fn view_route(s: &State, root: El) -> El {
    let blur = s.tech == Tech::RouteBlur;
    let t = if blur {
        Transition::blur_replace()
    } else {
        // forward: new screen enters from the right, old leaves to the left; both fade
        Transition::asymmetric(Transition::offset(70.0, 0.0).combined(Transition::opacity()), Transition::offset(-70.0, 0.0))
            .springs(SpringPair::new(Spring::new(0.5, 0.0), Spring::new(0.4, 0.0)))
    };
    let screen = if s.route == 0 {
        el().key("screen-home").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(c(CREAM))
            .child(el().absolute(90.0, 180.0).child(text("Pull up a chair.").font(TextStyle::new(64.0, 500.0).family(SERIF)).color(c(INK))))
            .child(el().absolute(94.0, 280.0).child(label("Euchre for four. You and three good players.", 18.0, "#58695b")))
            .child(
                el().key("home-card").absolute(700.0, 200.0).size(CW * 1.6, CH * 1.6).radius(11.0).rotate(0.12).matched("hero-JS")
                    .fill(c(CARD_WHITE)).pixels_fit(img(s, 0).unwrap().pixels(), SurfaceFit::Fit)
                    .box_shadow(Shadow { y: 14.0, blur: 30.0, color: Color::BLACK.alpha(0.3), ..Default::default() }),
            )
    } else {
        el().key("screen-table").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(c(TABLE_BG))
            .child(felt(s, 1))
            .child(seat_plate(0, "You", true))
            .child(
                el().key("table-card").absolute(500.0 - CW / 2.0, 300.0 - CH / 2.0).size(CW, CH).radius(7.0).rotate(0.03).matched("hero-JS")
                    .fill(c(CARD_WHITE)).pixels_fit(img(s, 0).unwrap().pixels(), SurfaceFit::Fit)
                    .box_shadow(Shadow { y: 4.0, blur: 10.0, color: Color::BLACK.alpha(0.3), ..Default::default() }),
            )
    };
    root.child(screen.transition(t))
}
```

Poll arm:
```rust
        Tech::Route | Tech::RouteBlur => {
            if at(s, 0.5, 1) {
                s.step = 1;
                s.route = 1;
                tx.motion(Spring::new(0.6, 0.12));
                tx.unclipped_flights();
                changed = true;
            }
            due(0.5, &mut next);
        }
```


**Timings.**
- Enter 0.5/0, exit 0.4/0.
- Hero flight `tx.motion(Spring::new(0.6, 0.12))`.
- `blur_replace` (`rune-motion/src/transition.rs:231`) is softer and reads well for menu to table.

**Cost.**
- Slide: p50 0.11 ms, p95 0.22 ms.
- Blur: p50 0.08 ms, p95 0.26 ms.

**Gotchas (the gap).**
- **Rotation snaps.** `matched` does not carry rotation. The hero card snaps from 0.12 rad to 0.03 rad on the first flight frame, which is visible in `route-strip.png` frame 1 to 2.
  - Fix A: give both ends the same rotation.
  - Fix B: run the hero as a technique-1 poll flight across the route change, as an overlay outside both screens.
- **Unclipped flights.** Use `tx.unclipped_flights()`, or the hero is clipped by the exiting screen's bounds.
- **Exit ghosting.** The exiting home screen is visibly ghosted over the felt for about 0.3 s. That is the cross-fade; shorten the exit spring if it reads muddy.
- **Blur vs travel.** Per `kit_rules`, blur is dropped when the effect travels: you cannot combine `offset` and blur in one effect.

## 10. Dialog springing in with backdrop blur

- **Verified** at `85cd8c6`. Shots: `shots/dialog-strip.png` (open) and `shots/dialog-close-strip.png` (close).
- **Scrim.**
  - `backdrop_blur(10)` (`rune/src/el/backdrop.rs:34`) plus a dark green tint at 0.45, with an opacity transition.
  - **The blur fades with the element's opacity:** frames go sharp, partly blurred, then fully blurred, and reverse on close. So the scrim's opacity transition alone animates the blur.
- **Panel.** Scale 0.9 + offset 30 + opacity in on a bouncy 0.45/0.3; scale 0.96 + opacity out on 0.2/0.

```rust
fn view_dialog(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for i in 0..4 {
        let [x, y] = SLOTS[i];
        r = r.child(card_shadows(face(s, i + 3).key(format!("t-{i}")).absolute(x - CW / 2.0, y - CH / 2.0).rotate(SLOT_ROT[i]), 0.0, 0.0));
    }
    if s.open {
        r = r.child(
            el().key("scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).backdrop_blur(10.0).fill(c("#0b1d16").alpha(0.45))
                .transition(Transition::opacity().springs(SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.22, 0.0)))),
        );
        let panel = el().key("panel")
            .absolute(500.0 - 210.0, 320.0 - 130.0)
            .size(420.0, 260.0)
            .radius(18.0)
            .fill(c(CREAM))
            .border(1.0, c("#ffffff").alpha(0.6))
            .box_shadow(Shadow { y: 24.0, blur: 60.0, color: Color::BLACK.alpha(0.45), ..Default::default() })
            .padding(28.0)
            .child(rune::col().gap(10.0).child(text("Hand 4").font(TextStyle::new(34.0, 500.0).family(SERIF)).color(c(INK))).child(label("You made it. +1 point.", 17.0, "#58695b")))
            .transition(
                Transition::asymmetric(
                    Transition::scale(0.9).combined(Transition::offset(0.0, 30.0)).combined(Transition::opacity()),
                    Transition::scale(0.96).combined(Transition::opacity()),
                )
                .springs(SpringPair::new(Spring::new(0.45, 0.3), Spring::new(0.2, 0.0))),
            );
        r = r.child(panel);
    }
    r
}
```

Poll arm:
```rust
        Tech::Dialog => {
            if at(s, 0.4, 1) {
                s.step = 1;
                s.open = true;
                changed = true;
            }
            if at(s, 1.8, 2) {
                s.step = 2;
                s.open = false;
                changed = true;
            }
            due(0.4, &mut next);
            due(1.8, &mut next);
        }
```


**Timings.**
- In: 0.45 s with 0.3 bounce.
- Out: 0.2 s.
- Scrim: 0.3 in, 0.22 out.

**Cost.**
- Open: p50 0.23 ms, p95 0.48 ms.
- Close: p50 0.27 ms, p95 0.68 ms. CPU only; the blur's GPU cost is not measured.
- At most 4 backdrop blurs per frame (r1).

**Gotchas.**
- **Panel see-through.** While the panel fades in, the cards behind show through it. That is expected; if it reads messy, start the panel at opacity 0.6 instead of 0 (not tested).
- **Blur reach.** The blur also blurs the rail, so frame the whole window.
- **Scale clamp.** Panel scale is clamped to [0.9, 1.1] by `kit_rules`; 0.9 is the floor without `allow_small_scale`.

## 11. Felt with vignette and texture; card shadows that deepen on lift

- **Verified** at `85cd8c6`. Shots: `shots/felt-1.2.png` (rune-fx grain), `shots/felt-noise-1.2.png` (core noise image), and `shots/felt-lift-strip.png` (middle card lifts and its drop shadow deepens).
- **Layers:**
  1. **Rail:** linear gradient (`rune/src/el/visuals.rs:190`) and a big drop shadow.
  2. **Felt:** radial gradient (`rune/src/el/visuals.rs:164`) with a lit centre, and a brass hairline.
  3. **Vignette child:** a second radial (one gradient slot per element, so the vignette needs its own element).
  4. **Texture child:** either the rune-fx `GRAIN` slot or `pixels` of a 1024x640 procedural noise RGBA from `ImageCache::rgba` (`rune/src/image.rs:234`).
  5. **Inset edge-shadow child:** `inset: true`.

```rust
/// Wood rail + felt: radial lit centre, vignette child, inset edge shadow, brass hairline,
/// and an optional texture child (rune-fx grain slot or a core noise image).
pub fn felt(s: &State, texture: u8) -> El {
    let mut felt = el()
        .key("felt")
        .pin(Some(14.0), Some(14.0), Some(14.0), Some(14.0))
        .radius(20.0)
        .fill(c(FELT_EDGE))
        .radial_gradient([0.5, 0.46], [560.0, 340.0], [(0.0, c("#2d6b56")), (0.45, c(FELT_CENTER)), (1.0, c(FELT_EDGE))])
        .border(1.0, c(BRASS).alpha(0.55))
        // vignette: a second gradient needs its own element (one gradient slot per element)
        .child(
            el().key("vignette").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).radial_gradient(
                [0.5, 0.5],
                [620.0, 400.0],
                [(0.0, Color::BLACK.alpha(0.0)), (0.62, Color::BLACK.alpha(0.0)), (1.0, Color::BLACK.alpha(0.42))],
            ),
        );
    felt = match texture {
        1 => felt.child(effect(GRAIN).key("grain").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0)),
        2 => match &s.noise {
            Some(n) => felt.child(el().key("noise").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).pixels(n.pixels())),
            None => felt,
        },
        _ => felt,
    };
    // inner edge shadow on top of the felt paint
    felt = felt.child(
        el().key("felt-inset").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).box_shadow(Shadow {
            y: 3.0,
            blur: 22.0,
            spread: 2.0,
            color: Color::BLACK.alpha(0.55),
            inset: true,
            ..Default::default()
        }),
    );
    el().key("rail")
        .pin(Some(10.0), Some(10.0), Some(10.0), Some(10.0))
        .radius(32.0)
        .fill(c(RAIL_BOTTOM))
        .linear_gradient(PI, [(0.0, c("#7a5232")), (0.12, c(RAIL_TOP)), (1.0, c(RAIL_BOTTOM))])
        .box_shadow(Shadow { y: 12.0, blur: 34.0, color: Color::BLACK.alpha(0.5), ..Default::default() })
        .child(felt)
}
```

```rust
fn noise_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..w * h {
        let n = hash(i.wrapping_mul(2_654_435_761));
        let light = n > 0.5;
        let a = ((n - 0.5).abs() * 2.0).powf(2.2) * 30.0; // sparse, soft speckle
        let g = if light { 255 } else { 0 };
        v.extend_from_slice(&[g, g, g, a as u8]);
    }
    v
}
```

```rust
fn view_felt(s: &State, root: El, texture: u8) -> El {
    let mut r = root.child(felt(s, texture));
    for i in 0..3 {
        let lifted = s.step >= 1 && i == 1;
        let l = if lifted { 1.0 } else { 0.0 };
        let plate = card_shadows(el().key(format!("fplate-{i}")).absolute(0.0, 0.0).size(CW, CH).radius(7.0).motion(HOVER_SPRING), l, 0.0);
        r = r.child(
            el().key(format!("fcard-{i}"))
                .absolute(500.0 - 160.0 + i as f32 * 110.0 - CW / 2.0, 330.0 - CH / 2.0)
                .size(CW, CH)
                .motion(HOVER_SPRING)
                .offset(0.0, -22.0 * l)
                .scale(1.0 + 0.05 * l)
                .child(plate)
                .child(face(s, i + 3).absolute(0.0, 0.0)),
        );
    }
    r
}
```


**Timings.** Lift spring 0.26/0.32 (the hover spring); offset -22 pt, scale 1.05.

**Cost.**
- Lift: p50 0.41 ms, p95 0.75 ms.
- The felt itself is static: at rest the window draws nothing, with either texture (grain is drawn only when a frame is drawn for another reason).
- Grain needs the compositor. Noise costs one 2.6 MB texture and no compositor.

**Gotchas.**
- **Grain vs noise.** Grain is fine and subtle, and reads as cloth. The noise image reads as visible speckle at 2x. Prefer grain, at amount about 0.045. If you skip rune-fx, use noise but drop its alpha (30 to about 18).
- **Inset shadow unconfirmed.** The inset edge shadow is hard to tell apart from the vignette in the shots. It is not separately verified; treat it as optional.
- **Gradient slots.** One gradient per element; stacking a vignette needs a child.
- **Shadow count.** Card shadows deepen only because the count stays 3 in both states (see "Card shadow stack").

---

## Appendix: build and render recipe

- **Build:** `cd scratchpad/motion-spike && CARGO_TARGET_DIR=../motion-spike-target cargo build --offline`.
  - The cold build was about 82 s, reusing a target dir seeded from `.cargo-targets/preview`. Incremental builds take about 2-3 s.
- **Snapshot:** `motion-spike --scene <name> --snap out.png --at <t> --scale 1`.
- **Strip:** `--strip out.png --crop x,y,w,h --start <t> --frames N --every K --cols C`.
- **Pointer script:** add `--script scripts/hover.json`.

```sh
#!/bin/sh
# usage: snap.sh scene t1 t2 ... ; writes shots/<scene>-<t>.png at scale 1, logs stats
B=/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/motion-spike-target/debug/motion-spike
cd /private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/motion-spike
sc=$1; shift
for t in "$@"; do
  $B --scene $sc --snap shots/$sc-$t.png --at $t --scale ${SCALE:-1} $EXTRA 2>&1 | grep -E 'rune:|panick|error' | sed "s/^/[$sc@$t] /"
done
```

### Cargo.toml (spike)
```toml
[package]
name = "motion-spike"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
rune = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
rune-kit = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
rune-fx = { git = "https://github.com/anomalyco/rune", rev = "85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb" }
wgpu = { version = "30", default-features = false, features = ["std", "parking_lot", "metal", "wgsl"] }

[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
opt-level = 2
debug = false
```

### src/main.rs (complete, as rendered)
```rust
//! Motion spike: eleven card-game techniques, one scene each, driven by `poll` on the virtual
//! clock so `--scene X --snap out.png --at T` is deterministic. Rune rev: see Cargo.toml.
use std::f32::consts::PI;
use std::time::Duration;

use rune::color::hex;


use rune::{
    App, AppSpec, Beat, Color, Curve, Cx, Cycle, Easing, El, Image, ImageCache, Sequence, Shadow, Shape as VShape,
    Spring, SpringPair, Stagger, SurfaceFit, TextStyle, Transition, Tx, Waker, canvas, effect, el, row, text,
};
use rune_kit::{Kit, RollingText};

mod fx;

// ---------- art, fonts, palette ----------

macro_rules! face {
    ($id:literal) => {
        ($id, include_bytes!(concat!("/Users/brittianwarner/goods/euchre-rune/apps/web/static/art/cards/", $id, ".png")).as_slice())
    };
}
const FACES: [(&str, &[u8]); 12] = [
    face!("JS"), face!("JH"), face!("AS"), face!("KD"), face!("QC"), face!("TH"),
    face!("9S"), face!("AH"), face!("KS"), face!("QD"), face!("JC"), face!("TD"),
];
const BACK: &[u8] = include_bytes!("/Users/brittianwarner/goods/euchre-rune/apps/web/static/art/cards/back.png");
const FD: &str = "/Users/brittianwarner/goods/euchre-rune/apps/desktop/assets/fonts/";
const FONTS: &[&[u8]] = &[
    include_bytes!("/Users/brittianwarner/goods/euchre-rune/apps/desktop/assets/fonts/DMSans-Regular.ttf"),
    include_bytes!("/Users/brittianwarner/goods/euchre-rune/apps/desktop/assets/fonts/DMSans-SemiBold.ttf"),
    include_bytes!("/Users/brittianwarner/goods/euchre-rune/apps/desktop/assets/fonts/SourceSerif4-Medium.ttf"),
    include_bytes!("/Users/brittianwarner/goods/euchre-rune/apps/desktop/assets/fonts/IBMPlexMono-Regular.ttf"),
];
const SERIF: rune::tok::Family = rune::tok::Family::Named("Source Serif 4");

pub fn c(h: &str) -> Color {
    hex(h).unwrap_or(Color::BLACK)
}
const FELT_CENTER: &str = "#245a49";
const FELT_EDGE: &str = "#10382c";
const TABLE_BG: &str = "#17452f";
const RAIL_TOP: &str = "#5a3a22";
const RAIL_BOTTOM: &str = "#3b2415";
const BRASS: &str = "#b4a57a";
const CREAM: &str = "#f4f3e9";
const INK: &str = "#233e32";
const GOLD: &str = "#f5d98e";
const CARD_WHITE: &str = "#fffdf6";

pub const W: f32 = 1000.0;
pub const H: f32 = 640.0;
const CW: f32 = 90.0;
const CH: f32 = 126.0;
const BW: f32 = 46.0; // seat backs
const BH: f32 = 64.0;

// marks for the compositor
pub const GRAIN: u32 = 1;
pub const GLOW: u32 = 2;
pub const BURST: u32 = 3;

// ---------- state ----------

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tech {
    #[default]
    Flight,
    Deal,
    Flip,
    Sweep,
    SweepCommits,
    Hover,
    HoverFx,
    Score,
    Turn,
    Burst,
    Route,
    RouteBlur,
    Dialog,
    Felt,
    FeltNoise,
}

#[derive(Clone, Copy)]
pub struct Fl {
    from: [f32; 2], // card centre
    to: [f32; 2],
    arc: f32, // control point lift, pt
    r0: f32,
    r1: f32,
    s0: f32, // start scale
    t0: f64,
    face: usize,
}

#[derive(Default)]
pub struct State {
    pub tech: Tech,
    pub now: f64,
    pub step: u32,
    pub faces: Vec<Image>,
    pub back: Option<Image>,
    pub noise: Option<Image>,
    _cache: Option<ImageCache>,
    // flight
    pub flights: Vec<Fl>,
    // flip
    pub flip_t0: Option<f64>,
    pub flip_b_face: bool,
    pub flip_b_narrow: bool,
    // sweep
    pub sweep_sent: usize,
    // hover
    pub hovered: Option<usize>,
    pub shake: Option<(usize, f64)>,
    // score
    pub us: u32,
    pub them: u32,
    pub us_target: u32,
    pub pulse: u32,
    // turn
    pub turn: usize,
    pub ping: u32,
    // burst
    pub burst: Option<f64>,
    pub won: Option<f64>,
    // dialog / route
    pub open: bool,
    pub route: u8,
}

fn img(s: &State, i: usize) -> Option<&Image> {
    s.faces.get(i % FACES.len())
}

/// A face-up card, art fitted into a rounded white card.
fn face(s: &State, i: usize) -> El {
    let e = el().size(CW, CH).radius(7.0).fill(c(CARD_WHITE));
    match img(s, i) {
        Some(im) => e.pixels_fit(im.pixels(), SurfaceFit::Fit),
        None => e,
    }
}
/// The back, stretched (so a squeezed width squashes it, for the flip).
fn back_px(s: &State, e: El) -> El {
    match &s.back {
        Some(im) => e.pixels(im.pixels()),
        None => e.fill(c("#8a2b2b")),
    }
}

/// Shadow stack for a card: contact edge + soft drop (+ optional gold glow). Always 3 shadows
/// so the counts match between states and the geometry springs (r3 1.1).
fn card_shadows(e: El, lift: f32, glow: f32) -> El {
    let l = lift.clamp(0.0, 1.0);
    e.box_shadow(Shadow { y: 1.0 + 0.0 * l, blur: 0.0, color: c("#c4c7b9").alpha(1.0 - l), ..Default::default() })
        .box_shadow(Shadow { y: 4.0 + 16.0 * l, blur: 10.0 + 26.0 * l, color: Color::BLACK.alpha(0.28 + 0.2 * l), ..Default::default() })
        .box_shadow(Shadow { y: 0.0, blur: 28.0, spread: 2.0, color: c(GOLD).alpha(0.75 * glow), ..Default::default() })
}

// ---------- felt (technique 11) ----------

/// Wood rail + felt: radial lit centre, vignette child, inset edge shadow, brass hairline,
/// and an optional texture child (rune-fx grain slot or a core noise image).
pub fn felt(s: &State, texture: u8) -> El {
    let mut felt = el()
        .key("felt")
        .pin(Some(14.0), Some(14.0), Some(14.0), Some(14.0))
        .radius(20.0)
        .fill(c(FELT_EDGE))
        .radial_gradient([0.5, 0.46], [560.0, 340.0], [(0.0, c("#2d6b56")), (0.45, c(FELT_CENTER)), (1.0, c(FELT_EDGE))])
        .border(1.0, c(BRASS).alpha(0.55))
        // vignette: a second gradient needs its own element (one gradient slot per element)
        .child(
            el().key("vignette").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).radial_gradient(
                [0.5, 0.5],
                [620.0, 400.0],
                [(0.0, Color::BLACK.alpha(0.0)), (0.62, Color::BLACK.alpha(0.0)), (1.0, Color::BLACK.alpha(0.42))],
            ),
        );
    felt = match texture {
        1 => felt.child(effect(GRAIN).key("grain").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0)),
        2 => match &s.noise {
            Some(n) => felt.child(el().key("noise").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).pixels(n.pixels())),
            None => felt,
        },
        _ => felt,
    };
    // inner edge shadow on top of the felt paint
    felt = felt.child(
        el().key("felt-inset").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).box_shadow(Shadow {
            y: 3.0,
            blur: 22.0,
            spread: 2.0,
            color: Color::BLACK.alpha(0.55),
            inset: true,
            ..Default::default()
        }),
    );
    el().key("rail")
        .pin(Some(10.0), Some(10.0), Some(10.0), Some(10.0))
        .radius(32.0)
        .fill(c(RAIL_BOTTOM))
        .linear_gradient(PI, [(0.0, c("#7a5232")), (0.12, c(RAIL_TOP)), (1.0, c(RAIL_BOTTOM))])
        .box_shadow(Shadow { y: 12.0, blur: 34.0, color: Color::BLACK.alpha(0.5), ..Default::default() })
        .child(felt)
}

fn label(t: &str, size: f32, col: &str) -> El {
    text(t).font(TextStyle::new(size, 600.0)).color(c(col))
}

// ---------- 1. curved flight (poll-driven) ----------

const FLY_D: f32 = 0.55; // travel, seconds
const FLY_EASE: Easing = Easing::cubic(0.22, 0.7, 0.25, 1.0); // the web's play curve
const SETTLE: Spring = Spring::new(0.42, 0.3); // rotation settle wobble
const FLY_END: f32 = FLY_D + 0.4;

struct Pose {
    x: f32,
    y: f32,
    rot: f32,
    scale: f32,
    lift: f32,
}

fn pose(f: &Fl, now: f64) -> Pose {
    let t = ((now - f.t0) as f32).max(0.0);
    let u = FLY_EASE.eval((t / FLY_D).min(1.0));
    // quadratic Bezier, control point above the midpoint
    let cx = (f.from[0] + f.to[0]) * 0.5;
    let cy = (f.from[1] + f.to[1]) * 0.5 - f.arc;
    let a = 1.0 - u;
    let x = a * a * f.from[0] + 2.0 * a * u * cx + u * u * f.to[0];
    let y = a * a * f.from[1] + 2.0 * a * u * cy + u * u * f.to[1];
    let lift = (PI * u).sin();
    // rotation: spring from r0 to r1 so it swings past and settles (the "landing")
    let (rp, _) = SETTLE.sample(0.0, 0.0, 1.0, t);
    let rot = f.r0 + (f.r1 - f.r0) * rp;
    // scale: start size -> 1, plus a 14% lift at mid-flight, plus a 3.5% squash on touchdown
    let land = ((t - FLY_D) / 0.16).clamp(0.0, 1.0);
    let squash = if t > FLY_D { 0.035 * (PI * land).sin() } else { 0.0 };
    let scale = (f.s0 + (1.0 - f.s0) * u) * (1.0 + 0.14 * lift) * (1.0 - squash);
    Pose { x, y, rot, scale, lift }
}

fn flight_card(s: &State, k: usize, f: &Fl) -> El {
    let p = pose(f, s.now);
    let card = face(s, f.face).key(format!("fly-face-{k}"));
    card_shadows(
        el().key(format!("fly-{k}"))
            .absolute(p.x - CW / 2.0, p.y - CH / 2.0)
            .size(CW, CH)
            .radius(7.0)
            .rotate(p.rot)
            .scale(p.scale)
            .origin(0.5, 0.5)
            .instant_changes() // the view drives every frame: no layout spring lag
            .transition(Transition::none()) // no default 0.2 s fade on mount
            .child(card),
        p.lift,
        0.0,
    )
}

// ---------- seats ----------

const SEATS: [[f32; 2]; 4] = [[500.0, 560.0], [92.0, 320.0], [500.0, 78.0], [908.0, 320.0]]; // S W N E
const SLOTS: [[f32; 2]; 4] = [[500.0, 380.0], [405.0, 315.0], [500.0, 250.0], [595.0, 325.0]];
const SLOT_ROT: [f32; 4] = [0.03, -0.08, 0.05, 0.1];

fn seat_plate(i: usize, name: &str, active: bool) -> El {
    let [x, y] = SEATS[i];
    let (w, h) = (130.0, 40.0);
    el().key(format!("seat-{i}"))
        .absolute(x - w / 2.0, y - h / 2.0)
        .size(w, h)
        .radius(20.0)
        .fill(Color::BLACK.alpha(0.22))
        .border(1.0, if active { c(GOLD).alpha(0.9) } else { c("#ffffff").alpha(0.08) })
        .center()
        .child(label(name, 14.0, if active { "#ffe5a4" } else { "#d9e6cb" }))
}

// ---------- views ----------

fn view(s: &State, _cx: &mut Cx<State>) -> El {
    let root = el().key("root").w_full().h_full().fill(c("#0e2a20"));
    match s.tech {
        Tech::Flight => view_flight(s, root),
        Tech::Deal => view_deal(s, root),
        Tech::Flip => view_flip(s, root),
        Tech::Sweep | Tech::SweepCommits => view_sweep(s, root),
        Tech::Hover | Tech::HoverFx => view_hover(s, root),
        Tech::Score => view_score(s, root),
        Tech::Turn => view_turn(s, root),
        Tech::Burst => view_burst(s, root),
        Tech::Route | Tech::RouteBlur => view_route(s, root),
        Tech::Dialog => view_dialog(s, root),
        Tech::Felt => view_felt(s, root, 1),
        Tech::FeltNoise => view_felt(s, root, 2),
    }
}

fn view_flight(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1)).child(seat_plate(1, "West", false)).child(seat_plate(0, "You", false));
    for (k, f) in s.flights.iter().enumerate() {
        r = r.child(flight_card(s, k, f));
    }
    r
}

// ---------- 2. staggered deal ----------

/// Final rect for card `n` (0..5) at seat `seat`.
fn deal_rect(seat: usize, n: usize) -> ([f32; 4], f32) {
    let [x, y] = SEATS[seat];
    let off = n as f32 - 2.0;
    match seat {
        0 => ([x - CW / 2.0 + off * (CW + 10.0), y - 120.0 - CH / 2.0 + 40.0, CW, CH], 0.0),
        2 => ([x - BW / 2.0 + off * 22.0, y + 34.0, BW, BH], off * 0.06),
        1 => ([x - BW / 2.0 + 16.0 + off * 4.0, y - BH / 2.0 - 80.0 + n as f32 * 22.0, BW, BH], 0.0),
        _ => ([x - BW / 2.0 - 16.0 - off * 4.0, y - BH / 2.0 - 80.0 + n as f32 * 22.0, BW, BH], 0.0),
    }
}
const DECK: [f32; 2] = [820.0, 470.0]; // dealer (East) puts the deck down here
const DEAL_GAP: f32 = 0.055;
const DEAL_SPRING: Spring = Spring::new(0.42, 0.12);

fn view_deal(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for (i, n) in ["You", "West", "North", "East (D)"].iter().enumerate() {
        r = r.child(seat_plate(i, n, false));
    }
    if s.step == 0 {
        // the deck at rest before the deal
        r = r.child(back_px(s, el().key("deck").absolute(DECK[0] - BW / 2.0, DECK[1] - BH / 2.0).size(BW, BH).radius(4.0)));
        return r;
    }
    // deal order: left of dealer first, round robin, 5 rounds
    let order = [0usize, 1, 2, 3];
    let total = 20;
    for round in 0..5 {
        for (k, &seat) in order.iter().enumerate() {
            let i = round * 4 + k;
            let ([x, y, w, h], rot) = deal_rect(seat, round);
            // offset from the card's own rect so its centre starts on the deck
            let dx = DECK[0] - (x + w / 2.0);
            let dy = DECK[1] - (y + h / 2.0);
            let t = Transition::offset(dx, dy).springs(SpringPair::new(DEAL_SPRING, Spring::new(0.2, 0.0)));
            let card = el().key(format!("deal-{seat}-{round}")).absolute(x, y).size(w, h).radius(if seat == 0 { 7.0 } else { 4.0 }).rotate(rot);
            let card = if seat == 0 { card.fill(c(CARD_WHITE)).pixels_fit(img(s, round).unwrap().pixels(), SurfaceFit::Fit) } else { back_px(s, card) };
            r = r.child(
                card.box_shadow(Shadow { y: 3.0, blur: 8.0, color: Color::BLACK.alpha(0.3), ..Default::default() })
                    .transition(t)
                    .enter_delay(Stagger::new(DEAL_GAP).delay(i, total))
                    .paint_order((total - i) as i16), // first dealt paints on top of the waiting deck
            );
        }
    }
    r
}

// ---------- 3. flip ----------

const FLIP_D: f32 = 0.36;

fn view_flip(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    // A: poll-driven width squeeze, face swaps at the midpoint, lift scale + shadow
    let u = match s.flip_t0 {
        Some(t0) => Easing::EASE_IN_OUT.eval((((s.now - t0) as f32) / FLIP_D).clamp(0.0, 1.0)),
        None => 0.0,
    };
    let wide = (CW * (PI * u).cos().abs()).max(1.0);
    let lift = (PI * u).sin();
    let showing_face = u >= 0.5;
    let inner = el().key("flip-a-art").absolute((CW - wide) / 2.0, 0.0).size(wide, CH).radius(7.0);
    let inner = if showing_face {
        inner.fill(c(CARD_WHITE)).pixels(img(s, 0).unwrap().pixels())
    } else {
        back_px(s, inner)
    };
    let inner = card_shadows(inner, lift, 0.0);
    r = r.child(
        el().key("flip-a").absolute(300.0 - CW / 2.0, 320.0 - CH / 2.0).size(CW, CH).scale(1.0 + 0.1 * lift).instant_changes().child(inner),
    );
    // B: two commits, layout tween on width (no per-frame view runs)
    let wb = if s.flip_b_narrow { 1.0 } else { CW };
    let inner_b = el().key("flip-b-art").absolute((CW - wb) / 2.0, 0.0).size(wb, CH).radius(7.0);
    let inner_b = if s.flip_b_face { inner_b.fill(c(CARD_WHITE)).pixels(img(s, 1).unwrap().pixels()) } else { back_px(s, inner_b) };
    r = r.child(
        el().key("flip-b")
            .absolute(700.0 - CW / 2.0, 320.0 - CH / 2.0)
            .size(CW, CH)
            .child(inner_b.box_shadow(Shadow { y: 4.0, blur: 10.0, color: Color::BLACK.alpha(0.3), ..Default::default() })),
    );
    r.child(el().absolute(240.0, 420.0).child(label("A: poll squeeze", 13.0, "#d9e6cb")))
        .child(el().absolute(630.0, 420.0).child(label("B: two-commit tween", 13.0, "#d9e6cb")))
}

// ---------- 4. trick sweep ----------

const WINNER: usize = 2; // North
const GATHER: Spring = Spring::new(0.3, 0.12);
const SWEEP: Spring = Spring::new(0.5, 0.0);

const WIN_POP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.12).scale(1.09).easing(Easing::EASE_OUT),
    Beat::new(0.3).scale(1.0),
];
const RING_IN: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(1.18),
    Beat::new(0.22).opacity(1.0).scale(1.0).easing(Easing::EASE_OUT),
];

fn view_sweep(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for (i, n) in ["You", "West", "North (wins)", "East"].iter().enumerate() {
        r = r.child(seat_plate(i, n, i == WINNER && s.step >= 1));
    }
    let gathered = s.step >= 2;
    let commits = s.tech == Tech::SweepCommits;
    for i in 0..4 {
        let id = format!("trick-{i}");
        let sent = if commits { i < s.sweep_sent } else { s.step >= 3 };
        if sent {
            continue;
        }
        let [x, y] = if gathered { [500.0 + (i as f32 - 1.5) * 6.0, 300.0 - i as f32 * 2.0] } else { SLOTS[i] };
        let rot = if gathered { (i as f32 - 1.5) * 0.05 } else { SLOT_ROT[i] };
        // Transition::none(): the re-key below would otherwise cross-fade old and new art
        let mut card = card_shadows(face(s, i + 2), 0.0, 0.0).transition(Transition::none());
        if i == WINNER && s.step >= 1 {
            card = card.key(format!("{id}-pop")).sequence(Sequence::new(WIN_POP));
        }
        let mut e = el().key(id.clone())
            .absolute(x - CW / 2.0, y - CH / 2.0)
            .size(CW, CH)
            .rotate(rot)
            .origin(0.5, 0.5)
            .motion(GATHER)
            .matched(format!("m-{id}"))
            .paint_order(if i == WINNER { 10 } else { i as i16 })
            .child(card);
        if i == WINNER && s.step >= 1 && !gathered {
            e = e.child(
                el().key("win-ring").absolute(-6.0, -6.0).size(CW + 12.0, CH + 12.0).radius(11.0).border(3.0, c(GOLD)).sequence(Sequence::new(RING_IN)),
            );
        }
        r = r.child(e);
    }
    // the winner's pile: minis that the trick cards fly into, fading to 0
    let [wx, wy] = SEATS[WINNER];
    for i in 0..4 {
        let sent = if commits { i < s.sweep_sent } else { s.step >= 3 };
        if !sent {
            continue;
        }
        let mut mini = el().key(format!("mini-{i}"))
            .absolute(wx - 23.0 + i as f32 * 3.0, wy + 26.0)
            .size(46.0, 64.0)
            .radius(4.0)
            .rotate(0.0)
            .opacity(0.0)
            .matched(format!("m-trick-{i}"))
            .child(face(s, i + 2).size(46.0, 64.0).radius(4.0));
        if !commits {
            mini = mini.enter_delay(Stagger::new(0.06).delay(i, 4));
        }
        r = r.child(mini);
    }
    r
}

// ---------- 5. hover / press / shake ----------

const HOVER_SPRING: Spring = Spring::new(0.26, 0.32);

fn shake_x(t: f32) -> f32 {
    // damped sine: 3 swings in 0.36 s
    if !(0.0..0.36).contains(&t) {
        return 0.0;
    }
    11.0 * (-t / 0.11).exp() * (2.0 * PI * t / 0.12).sin()
}

fn view_hover(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    let n = 5;
    let illegal = 4;
    let fxglow = s.tech == Tech::HoverFx;
    let mut hand = row().key("rack").absolute(500.0 - (n as f32 * (CW + 14.0)) / 2.0, 430.0).gap(14.0).items_end().h(CH + 40.0);
    for i in 0..n {
        let hot = s.hovered == Some(i) && i != illegal;
        let lift = if hot { 1.0 } else { 0.0 };
        let shake = match s.shake {
            Some((k, t0)) if k == i => shake_x((s.now - t0) as f32),
            _ => 0.0,
        };
        // plate carries the shadows on the same spring as the lift (r3 1.2)
        let plate = card_shadows(el().key(format!("plate-{i}")).absolute(0.0, 0.0).size(CW, CH).radius(7.0).motion(HOVER_SPRING), lift, if fxglow { 0.0 } else { lift });
        let mut art = face(s, i + 6).absolute(0.0, 0.0);
        if i == illegal {
            art = art.opacity(0.55);
        }
        let mut card = el().key(format!("rack-{i}"))
            .size(CW, CH)
            .motion(HOVER_SPRING)
            .offset(shake, if hot { -16.0 } else { 0.0 })
            .scale(if hot { 1.05 } else { 1.0 })
            .rotate(shake * 0.004)
            .origin(0.5, 1.0)
            .press_scale(0.96)
            .on_hover_change(move |s: &mut State, on, _tx| {
                if on {
                    s.hovered = Some(i)
                } else if s.hovered == Some(i) {
                    s.hovered = None
                }
            })
            .on_press_tx(move |s: &mut State, tx| {
                if i == illegal {
                    s.shake = Some((i, tx.clock()));
                }
            });
        if fxglow && hot {
            card = card.child(
                effect(GLOW).key(format!("glow-{i}")).effect_bleed(64.0).absolute(0.0, 0.0).size(CW, CH).radius(7.0).transition(Transition::opacity()),
            );
        }
        card = card.child(plate).child(art);
        if s.shake.is_some_and(|(k, t0)| k == i && s.now - t0 < 0.36) {
            card = card.instant_changes(); // the view drives the shake per frame
        }
        hand = hand.child(card);
    }
    r.child(hand)
        .child(el().absolute(40.0, 40.0).child(label(if fxglow { "hover: rune-fx glow" } else { "hover: core shadow glow" }, 13.0, "#d9e6cb")))
}

// ---------- 6. score odometer + pulse ----------

const PULSE: &[Beat] = &[
    Beat::new(0.0).opacity(0.0).scale(0.9),
    Beat::new(0.12).opacity(0.9).scale(1.12).easing(Easing::EASE_OUT),
    Beat::new(0.5).opacity(0.0).scale(1.35).easing(Easing::EASE_IN_OUT),
];
const NUM_POP: &[Beat] = &[
    Beat::new(0.0).scale(1.0),
    Beat::new(0.1).scale(1.18).easing(Easing::EASE_OUT),
    Beat::new(0.32).scale(1.0).easing(Easing::OVERSHOOT),
];

fn view_score(s: &State, root: El) -> El {
    let k = Kit::neutral(false);
    let big = TextStyle::new(56.0, 600.0).features("'tnum' 1");
    let team = |key: &str, name: &str, v: u32, pulse: u32| -> El {
        let num = RollingText::new(format!("{key}-n"), v.to_string()).value(v as f64).font(big).color(c(INK)).spring(Spring::new(0.42, 0.18)).build(&k);
        let mut box_ = el().key(format!("{key}-box")).size(180.0, 150.0).radius(18.0).fill(c(CREAM)).center().box_shadow(Shadow {
            y: 6.0,
            blur: 18.0,
            color: Color::BLACK.alpha(0.3),
            ..Default::default()
        });
        if pulse > 0 {
            // keyed by the pulse count: a new key replays the sequence
            box_ = box_.child(
                el().key(format!("{key}-pulse-{pulse}")).absolute(-8.0, -8.0).size(196.0, 166.0).radius(24.0).border(3.0, c(GOLD)).sequence(Sequence::new(PULSE)),
            );
        }
        let num = if pulse > 0 { el().key(format!("{key}-pop-{pulse}")).sequence(Sequence::new(NUM_POP)).transition(Transition::none()).child(num) } else { el().key(format!("{key}-pop-0")).child(num) };
        box_.child(rune::col().center().gap(4.0).child(label(name, 14.0, "#58695b")).child(num))
    };
    let mut r = root.child(felt(s, 1)).child(
        row().key("scores").absolute(500.0 - 200.0, 220.0).gap(40.0).child(team("us", "Us", s.us, s.pulse)).child(team("them", "Them", s.them, 0)),
    );
    if s.pulse > 0 && s.step < 3 {
        r = r.child(
            el().key(format!("delta-{}", s.pulse))
                .absolute(390.0, 180.0)
                .padding(8.0)
                .radius(12.0)
                .fill(c(GOLD))
                .child(label("+2", 18.0, INK))
                .transition(Transition::asymmetric(
                    Transition::offset(0.0, 14.0).combined(Transition::opacity()),
                    Transition::offset(0.0, -18.0),
                )
                .springs(SpringPair::new(Spring::new(0.4, 0.3), Spring::new(0.3, 0.0)))),
        );
    }
    r
}

// ---------- 7. turn indicator ----------

static BREATHE: Cycle = Cycle { period: 1.6, phase: 0.0, stops: &[(0.0, 0.35), (0.5, 1.0), (1.0, 0.35)], still: 0.8, fps: 30.0 };
const PING: &[Beat] = &[
    Beat::new(0.0).opacity(0.7).scale(1.0),
    Beat::new(0.9).opacity(0.0).scale(1.45).easing(Easing::EASE_OUT),
];

fn view_turn(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for (i, n) in ["You", "West", "North", "East"].iter().enumerate() {
        r = r.child(seat_plate(i, n, i == s.turn));
    }
    let [x, y] = SEATS[s.turn];
    let (w, h) = (130.0, 40.0);
    // one keyed marker that moves between seats on a layout spring
    let marker = el()
        .key("turn-marker")
        .absolute(x - w / 2.0, y - h / 2.0)
        .size(w, h)
        .motion(Spring::new(0.5, 0.2))
        .child(
            el().key("halo").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(20.0).box_shadow(Shadow {
                blur: 30.0,
                spread: 4.0,
                color: c(GOLD).alpha(0.75),
                ..Default::default()
            })
            .cycle(BREATHE),
        )
        .child(
            el().key(format!("ping-{}", s.ping))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(20.0)
                .border(2.0, c(GOLD))
                .sequence(Sequence::new(PING)),
        );
    r.child(marker)
}

// ---------- 8. bursts ----------

fn sparks(t: f32, cx: f32, cy: f32) -> VShape {
    let mut sh = VShape::new();
    let n = 28;
    for i in 0..n {
        let a = i as f32 / n as f32 * 2.0 * PI + (i as f32 * 1.7).sin() * 0.12;
        let speed = 260.0 + 140.0 * ((i * 7919 % 13) as f32 / 13.0);
        let drag = (-4.0 * t).exp();
        let d = speed * (1.0 - drag) / 4.0;
        let g = 160.0 * t * t; // gravity
        let (dx, dy) = (a.cos(), a.sin());
        let head = [cx + dx * (40.0 + d), cy + dy * (56.0 + d) + g]; // start at the card edge
        let tail_len = 26.0 * drag + 2.0;
        let tail = [head[0] - dx * tail_len, head[1] - dy * tail_len];
        let life = (1.0 - t / 0.9).clamp(0.0, 1.0);
        if life <= 0.0 {
            continue;
        }
        let col = if i % 3 == 0 { c("#ffffff") } else { c("#ffd75e") };
        sh = sh.stroke(vec![tail, head], 3.0 * life + 0.5, col.alpha(life));
    }
    sh
}

fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

pub fn confetti(t: f64, w: f32, h: f32) -> VShape {
    let mut sh = VShape::new();
    let cols = [c("#e0c579"), c("#497d69"), c("#ba5e46"), c("#fff8df"), c("#f5d98e")];
    for i in 0..120u32 {
        let left = i % 2 == 0;
        let (ox, dir) = if left { (0.06 * w, 1.0) } else { (0.94 * w, -1.0) };
        let oy = h * 0.95;
        let life = 2.6 + hash(i * 7 + 1);
        let tt = (t as f32 - hash(i * 3 + 5) * 0.25).max(0.0);
        if tt > life {
            continue;
        }
        let speed = 760.0 + 420.0 * hash(i * 11 + 2);
        let ang = (60.0 + 22.0 * hash(i * 13 + 3)).to_radians();
        let (vx, vy) = (dir * speed * ang.cos(), -speed * ang.sin());
        let drag = (-1.6 * tt).exp();
        let x = ox + vx * (1.0 - drag) / 1.6 + 30.0 * (tt * 3.0 + i as f32).sin() * (1.0 - drag);
        let y = oy + vy * (1.0 - drag) / 1.6 + 420.0 * tt * tt * 0.5 * 0.55 + 90.0 * tt;
        let spin = tt * (4.0 + 6.0 * hash(i * 17 + 4)) + i as f32;
        let (sw, shh) = (9.0 * spin.cos().abs().max(0.15), 5.0);
        let (cs, sn) = (spin.sin(), spin.cos());
        let pts: Vec<[f32; 2]> = [[-sw, -shh], [sw, -shh], [sw, shh], [-sw, shh]].iter().map(|p| [x + p[0] * sn - p[1] * cs, y + p[0] * cs + p[1] * sn]).collect();
        let fade = ((life - tt) / 0.5).clamp(0.0, 1.0);
        sh = sh.fill(pts, cols[(i % 5) as usize].alpha(fade));
    }
    sh
}

fn view_burst(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    // left: core canvas sparks around the winning card; the canvas goes BEFORE the card so
    // the card paints over the streaks (tree order is paint order)
    if let Some(t0) = s.burst {
        let t = (s.now - t0) as f32;
        if t < 0.9 {
            r = r.child(canvas(sparks(t, 300.0, 300.0)).key("sparks").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).instant_changes());
        }
    }
    r = r.child(card_shadows(face(s, 0).key("win-l").absolute(300.0 - CW / 2.0, 300.0 - CH / 2.0), 0.0, 0.0));
    // right: rune-fx sparkles + ripple slot behind the card
    r = r.child(
        el().key("win-r").absolute(700.0 - 80.0, 300.0 - 60.0).size(160.0, 120.0).child(
            effect(BURST).key("burst-slot").effect_bleed(80.0).absolute(0.0, 0.0).size(160.0, 120.0).radius(60.0),
        ),
    );
    r = r.child(card_shadows(face(s, 1).key("win-r-card").absolute(700.0 - CW / 2.0, 300.0 - CH / 2.0), 0.0, 0.0));
    if let Some(t0) = s.won {
        if s.now - t0 < 3.7 {
            r = r.child(canvas(confetti(s.now - t0, W, H)).key("confetti").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).instant_changes());
        }
    }
    r
}

// ---------- 9. route change + shared card ----------

fn view_route(s: &State, root: El) -> El {
    let blur = s.tech == Tech::RouteBlur;
    let t = if blur {
        Transition::blur_replace()
    } else {
        // forward: new screen enters from the right, old leaves to the left; both fade
        Transition::asymmetric(Transition::offset(70.0, 0.0).combined(Transition::opacity()), Transition::offset(-70.0, 0.0))
            .springs(SpringPair::new(Spring::new(0.5, 0.0), Spring::new(0.4, 0.0)))
    };
    let screen = if s.route == 0 {
        el().key("screen-home").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(c(CREAM))
            .child(el().absolute(90.0, 180.0).child(text("Pull up a chair.").font(TextStyle::new(64.0, 500.0).family(SERIF)).color(c(INK))))
            .child(el().absolute(94.0, 280.0).child(label("Euchre for four. You and three good players.", 18.0, "#58695b")))
            .child(
                el().key("home-card").absolute(700.0, 200.0).size(CW * 1.6, CH * 1.6).radius(11.0).rotate(0.12).matched("hero-JS")
                    .fill(c(CARD_WHITE)).pixels_fit(img(s, 0).unwrap().pixels(), SurfaceFit::Fit)
                    .box_shadow(Shadow { y: 14.0, blur: 30.0, color: Color::BLACK.alpha(0.3), ..Default::default() }),
            )
    } else {
        el().key("screen-table").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).fill(c(TABLE_BG))
            .child(felt(s, 1))
            .child(seat_plate(0, "You", true))
            .child(
                el().key("table-card").absolute(500.0 - CW / 2.0, 300.0 - CH / 2.0).size(CW, CH).radius(7.0).rotate(0.03).matched("hero-JS")
                    .fill(c(CARD_WHITE)).pixels_fit(img(s, 0).unwrap().pixels(), SurfaceFit::Fit)
                    .box_shadow(Shadow { y: 4.0, blur: 10.0, color: Color::BLACK.alpha(0.3), ..Default::default() }),
            )
    };
    root.child(screen.transition(t))
}

// ---------- 10. dialog with backdrop blur ----------

fn view_dialog(s: &State, root: El) -> El {
    let mut r = root.child(felt(s, 1));
    for i in 0..4 {
        let [x, y] = SLOTS[i];
        r = r.child(card_shadows(face(s, i + 3).key(format!("t-{i}")).absolute(x - CW / 2.0, y - CH / 2.0).rotate(SLOT_ROT[i]), 0.0, 0.0));
    }
    if s.open {
        r = r.child(
            el().key("scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).backdrop_blur(10.0).fill(c("#0b1d16").alpha(0.45))
                .transition(Transition::opacity().springs(SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.22, 0.0)))),
        );
        let panel = el().key("panel")
            .absolute(500.0 - 210.0, 320.0 - 130.0)
            .size(420.0, 260.0)
            .radius(18.0)
            .fill(c(CREAM))
            .border(1.0, c("#ffffff").alpha(0.6))
            .box_shadow(Shadow { y: 24.0, blur: 60.0, color: Color::BLACK.alpha(0.45), ..Default::default() })
            .padding(28.0)
            .child(rune::col().gap(10.0).child(text("Hand 4").font(TextStyle::new(34.0, 500.0).family(SERIF)).color(c(INK))).child(label("You made it. +1 point.", 17.0, "#58695b")))
            .transition(
                Transition::asymmetric(
                    Transition::scale(0.9).combined(Transition::offset(0.0, 30.0)).combined(Transition::opacity()),
                    Transition::scale(0.96).combined(Transition::opacity()),
                )
                .springs(SpringPair::new(Spring::new(0.45, 0.3), Spring::new(0.2, 0.0))),
            );
        r = r.child(panel);
    }
    r
}

// ---------- 11. felt + shadows ----------

fn view_felt(s: &State, root: El, texture: u8) -> El {
    let mut r = root.child(felt(s, texture));
    for i in 0..3 {
        let lifted = s.step >= 1 && i == 1;
        let l = if lifted { 1.0 } else { 0.0 };
        let plate = card_shadows(el().key(format!("fplate-{i}")).absolute(0.0, 0.0).size(CW, CH).radius(7.0).motion(HOVER_SPRING), l, 0.0);
        r = r.child(
            el().key(format!("fcard-{i}"))
                .absolute(500.0 - 160.0 + i as f32 * 110.0 - CW / 2.0, 330.0 - CH / 2.0)
                .size(CW, CH)
                .motion(HOVER_SPRING)
                .offset(0.0, -22.0 * l)
                .scale(1.0 + 0.05 * l)
                .child(plate)
                .child(face(s, i + 3).absolute(0.0, 0.0)),
        );
    }
    r
}

// ---------- poll: the scripted timeline per technique ----------

fn at(s: &State, t: f64, step: u32) -> bool {
    s.now >= t && s.step < step
}

fn poll(s: &mut State, now: f64, tx: &mut Tx) -> bool {
    s.now = now;
    let mut changed = false;
    let mut next: Option<f64> = None;
    let mut due = |t: f64, next: &mut Option<f64>| {
        if now < t {
            *next = Some(next.map_or(t, |n: f64| n.min(t)));
        }
    };
    match s.tech {
        Tech::Flight => {
            if at(s, 0.4, 1) {
                s.step = 1;
                s.flights.push(Fl { from: SEATS[1], to: SLOTS[1], arc: 110.0, r0: -0.7, r1: SLOT_ROT[1], s0: 0.55, t0: now, face: 4 });
            }
            if at(s, 1.6, 2) {
                s.step = 2;
                s.flights.push(Fl { from: [500.0, 520.0], to: SLOTS[0], arc: 40.0, r0: 0.0, r1: SLOT_ROT[0], s0: 1.0, t0: now, face: 0 });
            }
            due(0.4, &mut next);
            due(1.6, &mut next);
            let moving = s.flights.iter().any(|f| ((now - f.t0) as f32) < FLY_END);
            if moving {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
        Tech::Deal => {
            if at(s, 0.4, 1) {
                s.step = 1;
                changed = true;
            }
            due(0.4, &mut next);
        }
        Tech::Flip => {
            if at(s, 0.5, 1) {
                s.step = 1;
                s.flip_t0 = Some(now);
                s.flip_b_narrow = true;
                tx.motion_curve(Curve::tween(FLIP_D / 2.0, Easing::EASE_IN));
                changed = true;
            }
            if at(s, 0.5 + (FLIP_D / 2.0) as f64, 2) {
                s.step = 2;
                s.flip_b_narrow = false;
                s.flip_b_face = true;
                tx.motion_curve(Curve::tween(FLIP_D / 2.0, Easing::EASE_OUT));
                changed = true;
            }
            due(0.5, &mut next);
            due(0.5 + (FLIP_D / 2.0) as f64, &mut next);
            if s.flip_t0.is_some_and(|t0| now - t0 < FLIP_D as f64 + 0.02) {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
        Tech::Sweep | Tech::SweepCommits => {
            for (t, st) in [(0.4, 1), (1.0, 2), (1.5, 3)] {
                if at(s, t, st) {
                    s.step = st;
                    changed = true;
                    if st == 3 {
                        tx.unclipped_flights();
                        tx.motion(SWEEP);
                        if s.tech == Tech::SweepCommits {
                            s.sweep_sent = 1;
                        }
                    }
                }
                due(t, &mut next);
            }
            if s.tech == Tech::SweepCommits && s.step >= 3 && s.sweep_sent < 4 {
                let t = 1.5 + 0.06 * s.sweep_sent as f64;
                if now >= t {
                    s.sweep_sent += 1;
                    tx.unclipped_flights();
                    tx.motion(SWEEP);
                    changed = true;
                }
                due(1.5 + 0.06 * s.sweep_sent as f64, &mut next);
            }
        }
        Tech::Hover | Tech::HoverFx => {
            if s.shake.is_some_and(|(_, t0)| now - t0 < 0.4) {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
        Tech::Score => {
            if at(s, 0.5, 1) {
                s.step = 1;
                s.us_target = 6;
                s.pulse = 1;
                changed = true;
            }
            // count up one point per 0.14 s
            if s.us < s.us_target {
                let t = 0.5 + 0.14 * (s.us - 4) as f64;
                if now >= t {
                    s.us += 1;
                    changed = true;
                }
                due(0.5 + 0.14 * (s.us - 4) as f64, &mut next);
            }
            if at(s, 1.6, 2) {
                s.step = 2;
                s.them = 8;
                changed = true;
            }
            if at(s, 2.2, 3) {
                s.step = 3;
                changed = true;
            }
            due(0.5, &mut next);
            due(1.6, &mut next);
            due(2.2, &mut next);
        }
        Tech::Turn => {
            // ping once per breath period while the seat waits
            let k = ((now - 0.2) / 1.6).floor().max(0.0) as u32 + 1;
            if now >= 0.2 && k != s.ping {
                s.ping = k;
                changed = true;
            }
            due(0.2 + 1.6 * s.ping as f64, &mut next);
            if at(s, 3.4, 1) {
                s.step = 1;
                s.turn = 2;
                changed = true;
            }
            due(3.4, &mut next);
        }
        Tech::Burst => {
            if at(s, 0.4, 1) {
                s.step = 1;
                s.burst = Some(now);
                changed = true;
            }
            if at(s, 1.6, 2) {
                s.step = 2;
                s.won = Some(now);
                changed = true;
            }
            due(0.4, &mut next);
            due(1.6, &mut next);
            let alive = s.burst.is_some_and(|t0| now - t0 < 0.95) || s.won.is_some_and(|t0| now - t0 < 3.75);
            if alive {
                tx.wake_after(Duration::ZERO);
                changed = true;
            }
        }
        Tech::Route | Tech::RouteBlur => {
            if at(s, 0.5, 1) {
                s.step = 1;
                s.route = 1;
                tx.motion(Spring::new(0.6, 0.12));
                tx.unclipped_flights();
                changed = true;
            }
            due(0.5, &mut next);
        }
        Tech::Dialog => {
            if at(s, 0.4, 1) {
                s.step = 1;
                s.open = true;
                changed = true;
            }
            if at(s, 1.8, 2) {
                s.step = 2;
                s.open = false;
                changed = true;
            }
            due(0.4, &mut next);
            due(1.8, &mut next);
        }
        Tech::Felt | Tech::FeltNoise => {
            if at(s, 0.4, 1) {
                s.step = 1;
                changed = true;
            }
            due(0.4, &mut next);
        }
    }
    if let Some(t) = next {
        tx.wake_at(t);
    }
    changed
}

// ---------- setup, scenes, main ----------

fn noise_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..w * h {
        let n = hash(i.wrapping_mul(2_654_435_761));
        let light = n > 0.5;
        let a = ((n - 0.5).abs() * 2.0).powf(2.2) * 30.0; // sparse, soft speckle
        let g = if light { 255 } else { 0 };
        v.extend_from_slice(&[g, g, g, a as u8]);
    }
    v
}

fn setup(s: &mut State, waker: Waker) {
    let cache = ImageCache::new(waker);
    s.faces = FACES.iter().map(|(_, b)| cache.load(*b)).collect();
    s.back = Some(cache.load(BACK));
    s.noise = Some(cache.rgba(1024, 640, noise_rgba(1024, 640)));
    for i in &s.faces {
        let _ = i.wait();
    }
    if let Some(b) = &s.back {
        let _ = b.wait();
    }
    if let Some(n) = &s.noise {
        let _ = n.wait();
    }
    s._cache = Some(cache);
}

type Scene = (&'static str, fn(&mut State));
const SCENES: &[Scene] = &[
    ("flight", |s| s.tech = Tech::Flight),
    ("deal", |s| s.tech = Tech::Deal),
    ("flip", |s| s.tech = Tech::Flip),
    ("sweep", |s| s.tech = Tech::Sweep),
    ("sweep-commits", |s| s.tech = Tech::SweepCommits),
    ("hover", |s| s.tech = Tech::Hover),
    ("hover-fx", |s| s.tech = Tech::HoverFx),
    ("score", |s| {
        s.tech = Tech::Score;
        s.us = 4;
        s.us_target = 4;
        s.them = 7;
    }),
    ("turn", |s| {
        s.tech = Tech::Turn;
        s.turn = 1;
    }),
    ("burst", |s| s.tech = Tech::Burst),
    ("route", |s| s.tech = Tech::Route),
    ("route-blur", |s| s.tech = Tech::RouteBlur),
    ("dialog", |s| s.tech = Tech::Dialog),
    ("felt", |s| s.tech = Tech::Felt),
    ("felt-noise", |s| s.tech = Tech::FeltNoise),
];

fn main() {
    let _ = FD;
    let spec = AppSpec {
        scenes: SCENES,
        fonts: FONTS,
        families: ("DM Sans", "IBM Plex Mono"),
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("motion spike", (W, H), State::default, view)
    };
    App::new(spec).compositor(fx::TableFx { fx: None, alive: false }).run();
}
```

