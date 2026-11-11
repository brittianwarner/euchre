# r3 - Rune visual effects and materials (implementer notes)

Scope: crates/rune-fx, crates/rune-light, demo/fx, themes, glass/frosted examples, plus the core `El` visual API they sit on. Extends `docs/port/07-rune-cookbook.md` (sec 5, 6, 9, 13, 14, 15 are NOT repeated; sec 9 felt/rail is the baseline this builds on).

Revs: all citations are the app's pinned rev **85cd8c6** (`~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/`, paths below are relative to `crates/` unless noted). I diffed against HEAD 0ea75abe in the scratch clone: `rune-fx`, `rune-light`, `rune/src/el/`, `demo/fx`, `themes` are byte-identical. Only rune-motion (looping/store/tracks) and some window/render marks differ. Nothing in this note changes on upgrade.

Labels: VERIFIED = compiled and rendered headless by me (spike crate, scratchpad `spike-fx/`, snapshots in `spike-fx/out/*.png`, debug build, CPU/visual only, NO GPU timing measured by me). FROM SOURCE = read, not run.

## 0. Verdict (what to use)

1. Core only (no new dep, zero risk): felt radial vignette, layered `box_shadow`, `outline`, `border_gradient`, `canvas` confetti. VERIFIED.
2. rune-fx via a `Compositor`: glow on playable cards, sparkle burst on trick win, grain on felt, aura on game won. VERIFIED. Costs one extra crate and taking over the frame draw call (sec 2).
3. rune-light: skip. 2D SDF lighting for cards on felt is not worth 1-3 ms and a split glyph target (sec 9).
4. `system_material`, `backdrop_blur`, rune-fx Distort/Fire/Aurora: not needed for a card table (sec 8).

## 1. Core visuals (no rune-fx) - the 80% answer

### 1.1 Shadows
- `El::box_shadow(Shadow)` el/visuals.rs:148; `Shadow { x, y, blur, spread, color, inset }` el/visuals.rs:569-576. `blur` is CSS blur = 2 sigma (doc el/visuals.rs:565-567); `Shadow::core_animation(x,y,radius,color)` el/visuals.rs:643 passes `blur = 2*radius`. Analytic erf shader (shader.wgsl:105-152, FROM SOURCE) so any blur is one quad, no offscreen pass. Multiple shadows stack in declaration order; first four have animatable lanes (runtime/motion/props.rs:42,52,67).
- CSS strings: `rune_kit::css::box_shadows(src, color_fn)` rune-kit/src/css.rs:7 parses `"0 7px 13px rgba(0,0,0,.25), 0 3px 0 #c4c7b9"`. Useful to paste Figma/CSS values from 06-ui-spec.
- **Shadow spring gotcha (VERIFIED, the important one).** Shadow geometry and colour animate on the node's `.motion(spring)` or `tx.motion`, NOT on `.offset_motion`. Same shadow count and order in both states or the change lands instantly. The desktop hand card carries `.motion(PLAY 0.7 s)` for shared-element flight AND `.offset_motion(LIFT)`, so a shadow placed on that element deepens on the 0.7 s spring while the card rises on the 0.18 s one: the card floats over a hairline shadow. Spike A vs B at 0.06 s after lift (`spike-fx/out/s_0.36.png`): card B (plate) shadow already spread and dark; card A (shadow on element) still a 3 px hairline. Fix = plate child (sec 1.2).

### 1.2 Card shadow that deepens when the card lifts (VERIFIED, card B)
```rust
const LIFT: Spring = Spring::new(0.18, 0.0);
fn card(face: El, lifted: bool) -> El {
    let (y, blur, a) = if lifted { (22.0, 36.0, 0.50) } else { (7.0, 13.0, 0.25) };
    // Plate: same size, behind the face, carries ONLY the shadows, on the lift spring.
    let plate = el().key("plate").absolute(0.0, 0.0).size(100.0, 140.0).radius(10.0)
        .motion(LIFT)
        .box_shadow(Shadow { y: 3.0, color: hex("#c4c7b9").unwrap(), ..Default::default() }) // contact edge
        .box_shadow(Shadow { y, blur, color: Color::BLACK.alpha(a), ..Default::default() });
    el().key("card").size(100.0, 140.0)
        .motion(LIFT)                                   // lift on the same spring
        .offset(0.0, if lifted { -18.0 } else { 0.0 })
        .child(plate)
        .child(face.absolute(0.0, 0.0))                 // face has NO shadow
}
```
Rules: keep both shadows present in both states (only numbers change); keep contact shadow constant, animate the soft one; for a flight (PLAY 0.7 s) put the flight spring on the OUTER element and the LIFT spring on the plate and inner card. Cost: shadows are shader-analytic, one extra quad per card; 5-8 cards is nothing.

### 1.3 Felt with vignette and subtle texture (VERIFIED, `rest.png`)
```rust
col().key("felt").w_full().h_full().radius(22.0).fill(hex("#163626").unwrap())
    .radial_gradient([0.5, 0.45], [520.0, 380.0],       // centre as fraction, ellipse radii in pt
        [(0.0, hex("#3a7a5a").unwrap()), (0.55, hex("#285641").unwrap()), (1.0, hex("#0f2619").unwrap())])
    .child(effect(GRAIN).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(22.0)) // texture, needs rune-fx
```
- `radial_gradient` el/visuals.rs:164 and `linear_gradient` el/visuals.rs:190 share ONE gradient slot per element (last call wins); `gradient(top,bottom)` el/visuals.rs:153 is the same slot. To stack (e.g. lit centre + edge darkening) use a child el with `.pin(0,0,0,0)` and a second gradient.
- Core has NO dithering: a dark 3-stop ramp at 8 bit can band on large flat felt. Mitigation is the grain slot below (static, amount 0.03-0.04 hides banding), or a tiled noise PNG child with `.layer_opacity(0.05)` if you refuse rune-fx.
- Texture without rune-fx: a pre-baked 256 px tileable noise PNG as `image` child, low opacity. Not tested.
- Grain: `Fx::grain(GrainParams { amount: 0.04, ..Default::default() })` (rune-fx/src/params.rs:709-720). Defaults: amount 0.03, `fps: 0.0` = STATIC (no shimmer; keep it 0 for felt, shimmer reads as TV noise), feather 0. GPU ~0.15 ms at 960k px (demo/fx/timings.md "grain"). Honours `reduced`.
- Existing app felt: apps/desktop/src/screens/table/stage.rs:101-151 already does a rail + felt; adding grain is one more child, gradient tweak is stops only.

### 1.4 Other core pieces
| API | cite | Use |
|---|---|---|
| `outline(width, offset, color)` | el/visuals.rs:231 | selection / legal-card ring without layout change |
| `border_gradient(w, top, bottom)` | el/visuals.rs:105 | gold rim on trump chip, winner highlight |
| `blur(sigma)` layer blur | el/visuals.rs:248; MAX_SIGMA 64 render/layer_blur.rs:5 | defocus a loser card; costs an isolated layer pass |
| `backdrop_blur(sigma)` | el/backdrop.rs:34; MAX_BACKDROP_BLURS=4 render/backdrop_blur.rs:10 | frosted modal over table (7-mip dual filter); max 4 per frame |
| `layer_opacity`, `cache()` | el/visuals.rs:283, :304 | fade a group once; cache a static composite |
| `clip_rounded/soft/fade`, `cutout_circle` | el/visuals.rs:453,438,483,477 | feathered edges, spotlight cutout on dim overlay |
| `canvas(Shape)` | shape.rs:62,94 (`stroke`, `fill`) | vector polygons for confetti, suit pips, burst rays |
| rotate/scale | el/ | run in an isolated layer (rune-fx slot draws upright inside the turned layer, el/effect.rs:8-13) |
- Dim overlay for "Euchre!" / modal: `el().fill(BLACK.alpha(0.45))` + optional `.backdrop_blur(8.0)` on the panel only (glass-panel example pattern, examples/glass-panel, FROM SOURCE).
- `system_material` (lib.rs:26,314; MAX_NATIVE_MATERIALS 4) is macOS-only and needs a transparent window: do not use.
- `srgb_blend`: Rune defaults to blending in linear light (compose.rs:109 `Frame::srgb_blend()`); shadows and glows look softer/lighter than CSS. Pass the same flag to `FxRenderer::set_srgb_blend` (rune-fx/src/renderer.rs:372).

## 2. rune-fx integration (VERIFIED end to end)

rune-fx is a library; `rune` does not depend on it. You attach it with a `Compositor`:

1. Dep (`apps/desktop/Cargo.toml`, path/git at the SAME rev as rune): `rune-fx = { git = ".../rune", rev = "85cd8c6..." }`. Needs wgpu 30 (rune-fx/Cargo.toml:11; same as rune). I built against a path dep on the pinned checkout with wgpu 30, `cargo build --offline`, 1m47s cold debug. Compositors keep Rune's native Metal backend as long as they do not request unsupported wgpu Features (compose.rs doc; native backend rejects depth/stencil/MSAA, FROM SOURCE gpu/backend.rs:85-125).
2. Effect slots in the tree: `effect(MARK_ID)` (el/effect.rs:43) makes an element a hole the compositor paints into. `.effect_bleed(pt)` (el/effect.rs:52) lets it paint pt outside the rect (glow reach, sparkle margin). `.effect_data([f32;4])` (el/effect.rs:69) carries per-slot data bit-for-bit (we use t0 bits). `.effect_backdrop(true)` (el/effect.rs:60) only for Distort/frost (needs a copy; none of ours do).
3. `App::new(spec).compositor(MyFx { fx: None }).run()` (app.rs:539). Trait `Compositor<S>` compose.rs:34: `render(&mut self, f: &mut Frame<S>)`, `animating(&self) -> bool` (compose.rs:51).
4. In `render`: collect slots `f.rt.instances.iter().enumerate().filter(|(_, i)| i.params[1] == KIND_EFFECT)`, mark id = `f.rt.marks[n]`, rect px = `i.rect / i.params[3]` (scale), radius = max `i.radii`/scale. Build `FxLayer`s: `Fx::glow(GlowParams{..}).on(Shape::rounded(x,y,w,h,r)).at(t).reduced(reduced)` (rune-fx/src/lib.rs:277 enum Fx, :394-396 `FxSlot::scale/clip`). Then:
```rust
let fx = self.fx.get_or_insert_with(|| FxRenderer::new(device, f.format, FxOptions::default()));
fx.set_srgb_blend(f.srgb_blend());
fx.add_format(device, wgpu::TextureFormat::Rgba16Float);                    // renderer.rs:348
fx.prepare_with_slots(device, queue, &FxFrame{ viewport:[w,h], scale:1.0, layers:&layers, backdrop:None, sources:&[] }, &placements); // renderer_prepare.rs:13
f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, size);
f.ui.draw_with_backdrop(f.encoder, f.target, clear, &mut |phase, site| {    // render.rs:835, EffectPhase render.rs:189
    if let EffectPhase::Draw(pass) = phase { /* for each layer owned by this site: */ fx.draw_layer_in(pass, site.format, site.origin, li); } // renderer.rs:429
});
```
   Placements need `FxSlot::default().scale(sc).clip(i.clip, i.fade)` so scissor/clip/fades follow the tree (cards in a clipped fan clip their glow).
5. Time: `f.rt.t` (f32 seconds, same clock as loops); reduced motion `f.rt.reduced_motion` (VERIFIED passthrough to `.reduced()`); every effect has a calm fallback under `reduced`.
6. Redraw: return `animating()` true ONLY while a glow/burst/win is alive. My spike returned `true` always, which defeats the idle floor; gate it on state (`state.burst`, `state.won`, any `glow_slots > 0`).
7. The compositor replaces the default draw: you must still call `f.ui.prepare` + `draw_with_backdrop` every frame (above) even with zero layers. Per-frame vs per-slot data: I read t0 from `f.rt.state` at render time; reading `EffectSite::data` inside the hook is possible but layers are prepared before the pass, so state is simpler.
8. First-use cost: pipelines build lazily per effect kind; cold Metal shader cache 20-150 ms per pipeline (rune-fx/README.md:172). Pre-warm: draw each used `Fx` kind once on a 1x1 offscreen shape at startup (not tested) or accept a one-frame hitch at the first trick win. My spike first frame was 138 ms total incl. gpu init (debug build, glow+grain+nothing else).
9. Instance-compat: effect slot elements still participate in layout, `.radius()`, `.key()`, transitions, `.absolute()/.pin()`; removed slot keeps its last `effect_data` through exit transition (el/effect.rs:35-39).

## 3. Glowing playable-card highlight (VERIFIED, `rest.png` card G)
Tree: glow slot is a SIBLING behind the face, same rect, with bleed.
```rust
const GLOW: u32 = 2;
fn playable(face: El) -> El {
    el().key("g").size(100.0, 140.0)
        .child(effect(GLOW).effect_bleed(40.0).absolute(0.0, 0.0).size(100.0, 140.0).radius(10.0))
        .child(face.absolute(0.0, 0.0))
}
// compositor, mark GLOW:
Fx::glow(GlowParams { style: GlowStyle::Soft, color: rgb(0xffd75e), core: rgb(0xfff2c2),
    radius: 18.0, intensity: 1.2, breathe: 0.35, breathe_period: 1.6, occlusion: 0.0,
    ..Default::default() }).on(shape).at(t).reduced(reduced)
```
- Params rune-fx/src/params.rs:33-50: style Soft/Neon/Aura/Mask (:21-29), radius 1..64 (:77), intensity, inner, `occlusion` (default 0.15; set 0 so the card face is not darkened under blend; occlusion only applies in Normal blend), `breathe`+`breathe_period` gentle pulse, `heat` 1 lit..0 cool, `reach`.
- Use `Heat` (rune-fx/src/lib.rs:145-164, attack/cool seconds) to fade the glow in/out when legality changes instead of snapping.
- Neon style = hairline white core + coloured bloom, reads better on light card edges than Soft; Soft reads as a halo (what I rendered). Gold on green felt: strong; match to theme accent token.
- Cost: glow soft 0.229 ms, neon 0.205, aura 0.398 per 960k px covered, 10 draws/pass (demo/fx/timings.md). Glow covers rect+bleed only, so 4-5 glowing cards at ~140x200 px+bleed are far below that: budget ~0.05 ms each, FROM SOURCE not measured.
- No-rune-fx fallback (core): `outline(2.0, 2.0, gold)` + `box_shadow(Shadow{blur:24, color: gold.alpha(0.55), ..})`. Works, static (animate alpha via `.motion`).
- Reduced motion: drop `breathe` (steady glow); do not remove the highlight (it is information).

## 4. Trick-win burst (VERIFIED, `s_0.62.png`, `s_0.9.png`)
```rust
// slot el centred on the winner's trick position, bleed wide:
effect(BURST).effect_data([t0_secs, 0.0, 0.0, 0.0]).effect_bleed(60.0).absolute(0.0, 0.0).size(160.0, 120.0).radius(60.0)
// compositor:
let sp = SparkleParams { count: 28, size: 11.0, life: 1.3, margin: 30.0, burst: Some(t0), color: rgb(0xffe6a8), ..Default::default() };
if t - t0 < sp.burst_life() {                                   // params.rs:678 burst_life = STAGGER + life*BURST_LIFE*1.3
    add(Fx::sparkles(sp).on(Shape::circle(cx, cy, 30.0)).at(t).reduced(reduced));
    add(Fx::pulse(PulseParams { style: PulseStyle::Ripple { at: [w/2.0, h/2.0] }, color: rgb(0xffd75e),
        t0, duration: 0.8, reach: 90.0, ..Default::default() }).on(shape).at(t).reduced(reduced));
}
```
- What it actually is (FROM SOURCE rune-fx/src/cpu.rs:247-249; VERIFIED visually): four-point STAR sprites spawned on the OUTLINE of the shape, pushed outward by `margin`, staggered, each pops, drifts slightly, twinkles out. Count is capped at 64 (lib.rs:478 `count.min(64)`). It is NOT a gravity particle system: no velocity/gravity/trails. For a "pop" feel, use a small circle shape (r 30) + big `margin` so the stars form a ring around the winning card; pair with a ripple.
- Trigger once: store `t0 = rt.t` in state when the trick resolves; slot is created only while `now - t0 < burst_life()`; `OneShots<T>` (lib.rs:204, FADE 0.16 s, cap, oldest retires) is the library helper if several fire at once (2 trick wins can overlap).
- Cost sparkles x9 0.270 ms at 960k px; ripple 0.086 ms (timings.md). Sparkles layer covers only slot+bleed.
- Reduced motion: `.reduced(true)` is accepted on every layer, but I did not verify what sparkles do under it (cpu.rs reduced handling is read only for electric/distort, cpu.rs:179-276; distort.wgsl:61). Gate the slot yourself (skip it when `rt.reduced_motion`) and show a static ripple/outline instead.
- Core-only alternative (FROM SOURCE, NOT run): ring + sparks with loops. Outer `el` with `.looping(LoopSpec::slide_eased(from,to,period,EASE_OUT))` along `angle` (looping.rs:16,41-47; fractions of own size; wraps), inner `.sequence(Sequence)` fading/shrinking (el/motion.rs:259; finite, holds last pose, new key replays). `Easing::cubic(x1,y1,x2,y2)` accepts y outside 0..1 for overshoot arcs (rune-motion/src/easing.rs:41). Slide wraps, so remove the element after period (state timer), or the spark will loop.

## 5. Game-won celebration (VERIFIED, `s_1.4.png`)
Layers: (a) felt `canvas` confetti (core), (b) full-felt sparkle burst along the felt edge, (c) a border glow.
```rust
// (b)+(c): one full-felt slot, bleed 0
effect(WIN).effect_data([t0,0.,0.,0.]).effect_bleed(0.0).pin(Some(0.0),Some(0.0),Some(0.0),Some(0.0)).radius(22.0)
add(Fx::sparkles(SparkleParams{ count: 48, size: 12.0, life: 1.6, margin: 60.0, burst: Some(t0), color: rgb(0xfff2c2), ..Default::default() }).on(felt_shape));
add(Fx::glow(GlowParams{ style: GlowStyle::Aura, color: rgb(0xffd75e), third: rgb(0x4cc9f0), core: rgb(0xff5d73), radius: 26.0, inner: 0.5, ..Default::default() }).on(felt_shape));
// (a) confetti: core canvas, rebuilt each frame from t = now - t0 (deterministic hash per piece)
canvas(confetti(now - t0, w, h)).pin(Some(20.),Some(20.),Some(20.),Some(20.)).raise()
```
- Confetti (VERIFIED): 120 quads, two cannons (left/right bottom), exponential drag `exp(-1.6 t)`, gravity, flutter `sin`, spin with `|cos|` width squash for the tumbling look, fade out over last 0.5 s. `Shape::fill(points, color)` per piece (shape.rs:94). One `canvas` with 120 polys rebuilt each frame is cheap on CPU in my debug build (frame cpu p50 0.58 ms with everything on, debug, 22 instances; no worse than the in-repo confetti at apps/desktop/src/screens/table/stage.rs:957-982). If the existing stage confetti already looks right, keep it and add only (b)+(c).
- Aura rendered as a rainbow rim (red/yellow/cyan): LOUD. Violates the Five Laws "calm"; use a single hue: set `color`, `third` and `core` to three tints of gold (or the winning team colour) and lower `intensity`. Aura default speed circulates the palette (`aura_speed`, `rim_width` params.rs:33-50).
- Duration: tie the slot lifetime to `sp.burst_life()` (about 2-3 s) and stop returning `animating() == true` afterwards.
- Reduced motion: skip confetti and sparkles, show a static gold rim glow (`heat` fixed, `breathe 0`) plus the banner text.
- Cost: confetti = 0 GPU extra; sparkles 0.27 ms + aura glow 0.40 ms at 960k px. Both within 16.6 ms on any Apple GPU.

## 6. Gradients, noise, specular, lighting
- Card gloss/sheen without rune-fx: child el `.linear_gradient(angle, [(0.0, WHITE.alpha(0.0)), (0.5, WHITE.alpha(0.35)), (1.0, WHITE.alpha(0.0))])` clipped by the card radius, animate its `.offset` by loop `Slide` for a one-shot shimmer on lift. FROM SOURCE not run.
- rune-fx Shimmer (shape) 0.148 ms (timings.md): travelling highlight band, useful for "your turn" on a card. Not needed.
- Fills (liquid metal gold/chrome `LiquidMetalParams::gold()` README.md:113-115, plastic glossy 0.150, mesh gradient 0.382, grain gradient 0.186, fluted glass 0.129/0.249 backdrop): decorative; a gold liquid-metal trump badge costs 0.95 ms animated / 0.217 still (timings.md). Use "still" for static badges.
- Specular on cards: only rune-light (sec 9). Fake it with the sheen above.
- Banding: see 1.3.

## 7. Blur, glass, frosted (examples read)
- glass-card / glass-panel / frosted-bar / background (examples/*): all use `backdrop_blur` + semi-transparent fill + `border_gradient` hairline + soft shadow. Pattern: `el().backdrop_blur(10.0).fill(WHITE.alpha(0.12)).border_gradient(1.0, WHITE.alpha(0.4), WHITE.alpha(0.05)).radius(16.0).box_shadow(...)`. Backdrop slot cost: one blur pyramid per layer (README rune-fx backdrop frost), max 4 per frame (backdrop_blur.rs:10).
- For Euchre: use for the pause/settings sheet and score pill over the table only; never per card.
- rune-fx frosted glass fill (fluted glass + frost 0.249 ms) needs `effect_backdrop(true)` and the Backdrop phase (render.rs:189); only worth it if you want the fluted look.
- Platform: macOS Metal is the only tested native backend for this stack; Vulkan/DX12 are listed in rune-fx/Cargo.toml:15 but I did not run them. Backdrop blur and fx need `COPY_SRC` on the target (el/effect.rs:30: without it the slot draws with `backdrop == None`).

## 8. GPU cost and limits (demo/fx/timings.md; README.md:297 says fills were measured on an Apple M4 Pro, headless Metal; numbers are per draw, median, at the stated covered px; FROM SOURCE, not re-measured)
| effect | ms | covered px |
|---|---:|---:|
| glow soft / neon / aura | 0.229 / 0.205 / 0.398 | 960k |
| pulse ripple / radar / breathe | 0.086 / 0.276 / 0.134 | 544k / 960k / 960k |
| sparkles x9 | 0.270 | 960k |
| grain | 0.150 | 960k |
| shimmer | 0.148 | 960k |
| lens + frost (backdrop) | 0.650 | 960k |
| fire / swirl / voronoi | 1.2-1.8 / 1.06 / 1.18 | 544-960k |
Rules of thumb: one-shot, covered area small -> budget under 0.5 ms each. Avoid Fire/Swirl/Voronoi/MotionBlur (~1 ms+) on a card table. Limits: sparkles count 64 (lib.rs:478), glow radius 1..64 (params.rs:77), layer blur sigma 64 (layer_blur.rs:5), backdrop blurs 4/frame, system materials 4. Pipelines are built lazily (README.md:172). Effect slots work under scale/offset/opacity/rotate via isolated layers (el/effect.rs:8-13): a card that is rotating AND has a glow slot pays for an isolated layer.

## 9. rune-light verdict (FROM SOURCE)
Real-time 2D lighting: place a light, cards cast soft SDF shadows, bevel catch-light, god rays, 2D radiance-cascade GI, glass slabs (rune-light/README.md:3,87-93). Costs per README: 1-4 lights about 1 ms, GI 2.3-3.4 ms. Integration is manual (README.md:78: split glyph instances into their own target, call `render`; see examples/lab), unlike rune-fx. Lets you do physically-sized contact shadows and a lamp over the table, but a nice shadow already comes from sec 1.2 for free. Recommend skipping; revisit only for a "lamp over the table" theme.

## 10. Recipes cheat sheet (priority order for the port)
1. Replace flat felt gradient with radial vignette + static grain slot. (sec 1.3, ~0.15 ms)
2. Shadow plates on cards, deepen on lift/hover. (sec 1.2)
3. Playable-card glow via Heat fade-in. (sec 3)
4. Trick win: sparkle ring + ripple at winner position. (sec 4)
5. Game won: keep confetti, add a single-hue aura rim + sparkle burst. (sec 5)
6. Compositor gating: animating() only while an effect is alive; pre-warm pipelines once. (sec 2.6, 2.8)

## 11. Gotchas (all VERIFIED or cited)
- Shadow springs follow `.motion`, not `.offset_motion`; keep shadow count/order constant (sec 1.1).
- `radial_gradient`/`linear_gradient`/`gradient` share one slot per element (el/visuals.rs:153-190).
- Glow slot must have `effect_bleed`; otherwise the halo is clipped at the card rect (confirmed by `rest.png` showing the glow only because bleed 40).
- `occlusion` default 0.15 darkens beneath the glow in Normal blend; set 0 for a card face over the glow.
- A compositor must run `ui.prepare` + `draw_with_backdrop` itself, even with no effects.
- Sparkles are stars on a shape outline, not particles; a circle shape + margin makes a ring, a rect shape makes a rectangle of stars (visible in `s_1.4.png` along the felt edge).
- `animating()` true forever keeps the GPU spinning; gate it.
- Do not edit apps/desktop to try this: the spike lives in the scratchpad and uses path deps on the pinned checkout; the desktop integration is the 6 steps of sec 2.

## 12. Spike artefacts
`/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/spike-fx/` (`src/main.rs`, scripted via `lift.json`; `out/rest.png`, `s_0.36.png` (shadow timing A vs B), `s_0.62.png`/`s_0.9.png` (burst), `s_1.4.png` (win)). Run: `spike-target/debug/spike-fx --snap out/x.png --at T --script out/lift.json --scale 1`.
