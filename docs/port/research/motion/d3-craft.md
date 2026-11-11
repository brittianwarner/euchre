# d3 - Craft proposal: materials, light, screens, transitions and gameplay choreography

Stage: DESIGN. Angle: visual craft and screens. Reads: r1 (core motion), r2 (rune-motion), r3 (fx and materials), r4 (examples), r5 (docs at HEAD), r6 (audit and snapshots), 06-ui-spec.md, 02-protocol.md (section 3.1 lines 280-288, section 10 lines 547-575, section 9.1/9.2 lines 514-545), 03-table-actor.md section 9 (lines 444-468), and the app source read-only (`apps/desktop/src/{theme.rs,theme/felt.rs,screens/table/{present,stage,hand,header}.rs,app.rs,scenes.rs}`).

Nothing was built or run for this note. Every API below is taken from r1-r6 or from a line I read at the pinned rev. Items I could not prove are in section 7 as spikes (S1..S14) and are tagged `[spike Sn]` where they appear.

## 0. Revs, citation form, tiers

- **Rev.** All `path:line` cites are the app's pinned rune rev `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb`, `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/`. r1 section 0 shows every motion file I use is byte-identical at GitHub HEAD `0ea75abe`. **Nothing here needs a pin bump.** `[r1 s6]` means "r1 section 6", which carries its own `path:line`. Lines I checked myself in this pass: `rune/src/el/motion.rs:5-76` (`MotionIntent`, `El::motion_intent` at `:74`), `rune/src/el/visuals.rs:105,283,295,438,477` (`border_gradient`, `layer_opacity`, `dissolve`, `clip_soft`, `cutout_circle`), `rune/src/el/backdrop.rs:34` (`backdrop_blur`), `rune/src/el.rs:368-390` (`live_opacity/live_scale/live_trim`), `rune-kit/src/rolling_text.rs:110-154` (`RollingText::new/font/color/spring/value/trailing`), `rune-motion/src/transition.rs:231` (`blur_replace` = sigma 1.5, scale 0.96, fade).
- **Tiers.** `[CORE]` = rune + rune-kit only, no new dependency. `[DRV]` = needs `rune-motion` as a direct dependency and an app-owned `Animated` driven from `AppSpec::poll` (r2 section 1, 6). `[FX]` = needs `rune-fx` and an `App::compositor` (r3 section 2). Every `[FX]` item has a `[CORE]` fallback. **Ship order: all `[CORE]` first.**
- **Keep from the spec (not negotiable).** All strings in 06 (sections 3-5, 9, 10, 12, 14), all a11y labels (06 20.4), 44 pt targets, the 3 px focus ring, tempo contract (06 6.1), slot retention (06 5.6), Reduce Motion behaviour (06 6.5 last paragraph). New visible text is limited to strings that already exist in the spec (reused in a new place) and is listed in section 8 for sign-off.
- Palette hexes below are sRGB; wrap with `theme::color("#..")` (the app's hex-to-linear helper, `theme.rs`; cookbook section 9: `Color` is linear sRGB).

## 1. What the notes and the snapshots say, in one page

1. Today's table reads as a UI panel, not a table (r6 s4): two different greens (flat page `#17452f` under a 388 pt letterbox felt), flat brown rail with two fighting hairlines, no grain although `FELT_WATERMARK` exists, opponents as 14x20 pt backs, invisible active-seat tint (`#ffffff0a`), no primary button on the felt, header out-weighs the score, a baked-in `www.me.uk/cards/` QR on the ace of spades, truncated names ("Gran..."), hero "D" blob.
2. Motion is at parity with the web (r6 s2), but every moment is a straight slide on one spring. The gap to "badass" is choreography: deal, up-card reveal, trump named, trick spotlight, hand score, match end, route changes.
3. Hard Rune limits that shape the design:
   - No 3D, no `scale_x`, no skew (r1 s7). Flips are width squeezes `[DRV]` or crossfade+dip `[CORE]`.
   - `matched` flights are straight, one curve on both axes, carry rect/velocity/radius/fill/opacity but not rotation (r1 s6, r2 s4).
   - Presence transitions clamp scale to [0.9, 1.1] (0.5 with `.allow_small_scale()`), removal opacity is forced to 0, blur drops when the transition travels (`Effect::kit_rules`, `rune-motion/src/transition.rs:148-157`, r1 s4).
   - A newly mounted element snaps its offset/scale/rotate (`rest_quiet`, `runtime/commit/apply.rs:~527`, r1 s3): entrance is the transition's or a `Sequence`'s job, not a plain prop.
   - `Sequence` = opacity + uniform scale only, `&'static [Beat]`, new key replays (r1 s8). `LoopSpec` takes one spec per node; slides wrap (r1 s8).
   - `Choreography`, `Keyframes`, completion tokens are unreachable from `rune` (r1 s5). Sequence game logic off state + `wake_after`, never off animation completion.
   - `.radius()` does not clip children (cookbook gotcha 2); `clip_rounded` hides the element's own shadow (gotcha 1).
4. Server tempo (02 s9.2, 03 s9) is the budget. Between the events the client animates: AI play floor 1000 ms (+0-399 jitter, fast 100+0-39), bid floor 1900 (fast 120), discard 1100 (fast 120), trick hold 3300 (fast 120; 3800 when `sealsEuchre`), hand-score hold 2500 (fast 250). `trick_resolve -> trick_play` emits `steps: []` (02 s10), so "collected / next trick" must come from view diffs (it does: `present.rs after_change`).

## 2. Visual system

### 2.1 Concept: "the club table at night"

One warm lamp, one dark green cloth, a wooden bumper, cream card stock, a brass line. Calm and legible: depth comes from elevation and light, not from decoration. One accent on the felt (gold). Lime stays on the forest/cream shell only (home, nav pill).

Key light: upper-left, warm (`#fff1c9`, the spec's own point light, 06 s16: position (-0.8, 1.2, 2)). Therefore **every shadow below carries a +x offset of about 0.15 x its y**, and highlights sit on top and left edges.

### 2.2 Elevation ladder (shadows, layer order bottom to top)

All via `El::box_shadow(Shadow{x,y,blur,spread,color,inset})` (`el/visuals.rs:148`, struct `:569-576`; `blur` = CSS blur = 2 sigma; up to 4 shadows, geometry and colour animate on the node's `.motion`, not `.offset_motion`, same count/order in both states: r3 s1.1). `INK` = `#05160e` (shadow base, near-black green; reads warmer than `#000` on felt).

| Level | Used for | Shadows (x, y, blur, spread, colour) |
|---|---|---|
| E0 | felt, rail top | rail: keep `theme::shadow::RAIL` (`theme.rs:667`) |
| E1 card at rest in rack | hand cards | `(0,2,0,0,#abb3a0)` edge; `(1,6,12,0,#05160e55)` drop |
| E2 card played | trick cards | existing `felt::PLAYED` (`felt.rs:265`: `0 2 0 #d9d8ca, 0 4 0 #abb3a0, 0 9 15 #082b2350`) plus `(2,12,22,0,#05160e33)` |
| E3 hover | rack card | `(0,3,0,0,#abb3a0)`; `(3,16,26,0,#05160e66)` |
| E4 selected / carried | rack card, flying card | `(0,4,0,0,#bac2b1)`; `(4,24,36,0,#05160e80)` |
| E5 overlay | dialogs, banner, hand-result card | `(0,2,0,0,#ffffff40 inset)`; `(0,12,40,0,#00000070)` (spec dialog: `0 12px 40px rgba(0,0,0,.55)`, 06 s5.10) |

Card shadows go on a **plate** child (same size, behind the face, carries only the shadows, on its own `.motion(LIFT_PLATE)`), never on the flying element: shadow geometry follows `.motion`, and the card carries `.motion(PLAY)` for flights (r3 s1.2 VERIFIED spike A vs B). Plate = `el().key("plate").absolute(0,0).size(w,h).radius(7)` + shadows; face has none.

### 2.3 Felt, light, rail (the table)

Restructure so the table is **one object** (06 s5.1 already says the felt is one continuous surface under stage and hand area):

```
root (screen bg #17452f, unchanged)
 +- tabletop  .w_full().grow(1).radius(30)  clip_rounded(30)   // clip so the light pool cannot bleed onto the rail
     +- felt-base    fill #10382c + radial_gradient([0.5,0.42], [0.62w,0.7h], [(0.0,#2d6a54),(0.55,#245a49),(1.0,#10382c)])   // 06 s16.1 centre #2f6a55-ish -> #10382c
     +- felt-grain   Pixels buffer, pinned 0,0,0,0, layer_opacity(1.0) (alpha is in the pixels)
     +- felt-vignette child with inset shadow (0, 8, 28, 0, #05160e99, inset=true) pinned 0,0,0,0, radius 30   [spike S3]
     +- lamp-pool    big radial gradient el, springs between seats (2.5)
     +- content      stage + hand area (existing)
 +- rail          U-shaped bumper (2.4), drawn ABOVE tabletop edge only (pointer-events none, a11y_hidden)
```

- **Cache the static layers.** `felt-base`, `felt-grain`, `felt-vignette`, rail: wrap in `.cache()` so only moving cards redraw (r1 s12; `el/visuals.rs:296-308`; draws live while any spring inside runs, so keep moving things out of the cached subtree).
- **Grain** `[CORE]`: generate once at startup a 1024x640 RGBA `Pixels` buffer, the 06 s16 speckle (deterministic `grain(x,y)=frac(sin(x*12.9898+y*78.233)*43758.5453)`; dot density 900/512^2 = 0.0034 per px -> about 2250 dots; each 1 px, white if `grain(i,5.2)>.5` else black, alpha 0.05 = 0x0d). Stretch with `SurfaceFit::Fill` over the felt (<= 1.9x: reads as cloth, not as blotches). Same image path as the card art (`Pixels`, cookbook gotcha 5/s4; image creation lives in `setup`). `[spike S8]` check size/cost; if tiling is available use a 512 tile as 06 s16.1 says. `[FX]` upgrade: `Fx::grain(GrainParams{amount:0.04, ..})` static (r3 s1.3, 0.15 ms), also hides 8-bit banding.
- **Watermark**: `FELT_WATERMARK` already exists and is unused (`theme.rs:119`, r6 s4): draw `♣` (serif 150, `#dfeac529` = 16%) at felt (0.5, 0.52), and the dotted ring `#dae6b038` r = 0.455 S. Behind the trick, under the lamp pool.
- **Light pool** (the "lamp"): `el().absolute(..).size(0.9*fw, 0.8*fh).radial_gradient([0.5,0.5], [0.45*fw, 0.4*fh], [(0.0,#fff1c91f),(0.6,#fff1c90a),(1.0,#fff1c900)])`. Alpha 0.12 centre. Default home = trick area centre. It moves with the turn (section 4.8). Gradient shares one slot per element (`el/visuals.rs:153-190`): the pool is its own el.
- **Top-left sheen**: one `linear_gradient(angle -> toward bottom-right, [(0.0,#ffffff12),(0.4,#ffffff00)])` el, static, over felt-base. Gives the key-light direction.
- **Edge**: `felt-rim` stays (`#dae6b038`, inset 12) but add a second rim 4 px outside it at `#00000040` so the felt reads as stitched/set into the wood.

### 2.4 Rail: a U-shaped bumper that opens toward the player

You sit at the bottom edge, so the rail runs along the top and both sides and **leaves the bottom open**; your rack lies on cloth. Implementation: rail el radius 30, pinned `top 0, left 0, right 0, bottom -40` so its bottom corners are off-window (`pin` with a negative inset `[spike S11]`; fallback: an extra 40 pt row below, clipped by the window). Fill: `linear_gradient(PI, [(0.0,#6a4428),(0.45,#5a3a22),(1.0,#3b2415)])`; border `(1, #2b190c)`; **bevel**: child `border_gradient(1.0, #ffffff2e, #ffffff00)` (`el/visuals.rs:105`) inside edge = top-left catch-light; inner **brass line** keep `#b4a57a` 1 pt (`theme.rs:125`, `space::BRASS_LINE`) plus a 1 pt `#e3d7a680` highlight on its top edge. Width: 14 pt top, 14 pt sides (existing `space::RAIL`). Optional wood grain `[CORE]`: 24 faint horizontal gradient strips (alpha 0.06, deterministic hash), baked once into a 1024x48 `Pixels` strip; skip if time.
Rail owns the dark under-shadow `RAIL` token. Pointer-events off, `a11y_hidden`.

### 2.5 Seat plates (replaces floating text islands)

Each seat = plate + fan + name block (06 5.4.2 content unchanged, same strings, same data attributes/anchors).

- Plate: `fill #0f31238c`, radius 14, hairline `border(1,#dae6b01f)`, shadow E1-soft `(0,3,10,0,#05160e40)`; width 168 (side seats) / 200 (partner); padding 10x14.
- **Active seat** (turnSeat): plate fill `#1a4a37d9`, `border_gradient(1.5, #e6d08ce6, #e6d08c26)`, name `#ffe5a4` (existing `SEAT_NAME_ACTIVE`), soft outer glow `box_shadow(0,0,22,0,#eed59140)`. All as property changes on one element, so it springs on `.motion(Spring::new(0.30,0.0))` (colour and shadow animate: r1 s3). This replaces the invisible `SEAT_TINT`.
- Avatar disc: 36 pt (was 28), `INITIAL` colours kept (`felt.rs hexes`), ring 2 pt `#ffffff1f` -> gold when active.
- Name: width never truncates under 150 pt: dealer chip, maker chip, lead arrow wrap to a second row under the name instead of squeezing it (fixes "Gran..." r6 s4).
- **Fan** (the card count): real burgundy back art (`back.png`, 06 s19) 30x42 pt, up to 5, fanned about the bottom centre: `rotate((i - (n-1)/2) * 0.10)` with `origin(0.5,1.0)` (cookbook 5.2 VERIFIED), overlap 12 pt, `E1` shadow on a plate each. Cards that leave the fan (played) stay out. 30x42 is big enough to carry a flight origin (r6 s4 said 14x20 could not). A count badge `5` (12/700, `#f4ead0` on `#0f3123`) at the fan's right when `handCounts` > 0; text "N cards" in the detail row stays (06 5.4.2).
- Partner sitting out (loner): plate `opacity .45`, fan empty, "Sitting out · partner alone" text unchanged.
- Dealer chip, maker chip: enamel look: `fill #f4ead0`, `box_shadow(0,2,0,0,#b4a57a)` + `(0,4,6,0,#05160e40)`, text per spec; 22 pt (was 18).

### 2.6 Cards: stock, backs, sizes

- **One back everywhere**: `back.png` for fans, deck, taken stacks, hero (r6 s4: four inconsistent backs). The spec's green deck tile (06 5.4.4 Case B) is a placeholder; replace it with 3 stacked back images at offsets (-0,-0), (-2,-3), (-4,-6), E2 shadow on the lowest, sized 137x192 (spec size kept), **brass `♣` medallion** 36 pt (`#b4a57a` ring, `#f2e8cd` glyph) centred on the top card.
- **Taken stack** per seat (06 5.4.2 target of the gather, `data-stack`): 28x40 back image, 2 offset edges `#c6c6b5`/`#e7e5d8` (existing `STACK_EDGE_*`), count badge (see 4.7). Replace the diagonal-stripe monitor glyph.
- **Rack cards**: art width `clamp(112, 0.092*window_w, 140)`, so 132 x 185 at 1280 (was 98x137 per r6; `HAND_W` is 112, `theme.rs:541`); radius 7; the rack is allowed to grow into the 80-100 pt of dead green (r6). To buy the height: heading + instruction share one row (heading left, instruction right-of-it at 17 pt), and **Play selected card is hidden until a card is selected** (it was a permanent dead grey button; 06 5.5 requires it to be enabled only with a legal selection, hiding keeps that rule). Double-click and Enter keep working.
- **Captions** (06 5.6 label block: card name 15/600, sub-line 13): in the a11y tree always (`.label(..)` exists); visible only on hover, selection or keyboard focus (fade 0.15 s) and for the `Must follow suit` / `Selected ✓` sub-lines when they apply. Rest state shows card art only.
- **Ace of spades**: re-export without the `www.me.uk/cards/` text and QR (asset fix; CC0 so attribution is optional, keep it in `LICENSE`/About). Until then mask with a bottom clip. This is the single cheapest-looking pixel on screen (r6 s4).
- **Dim/legal marking** (06 5.6 `unavailable` state): illegal cards get a veil child `fill #10382c`, alpha 0.38, radius 7 (opacity springs 0.2 s), plus `Must follow suit` sub-line; legal cards when a card choice is open get a 2 px `#eed59199` underline-glow: `outline(2,2,#eed59199)` + `box_shadow(0,0,24,0,#eed59140)` `[CORE]`; `[FX]` glow slot `GlowStyle::Soft`, `breathe 0.35`, `occlusion 0` (r3 s3, Heat fade-in). Information, so never removed under Reduce Motion (static glow).

### 2.7 Typography (families unchanged: DM Sans + Source Serif 4, 06 s1.3)

New styles (add to `theme::felt::ty`): 
| Token | Spec | Use |
|---|---|---|
| `HAND_RESULT_LABEL` | serif 600 28/32, tracking -0.02em | "March!" etc. |
| `HAND_RESULT_NUM` | serif 600 72/72, `'tnum' 1` | "+2" |
| `STAMP` | serif 700 44/44, tracking 0.01em | "Euchre!" stamp, rotates -6 deg |
| `END_TITLE` | serif 500 44/48, tracking -0.03em | match-end panel |
| `END_SCORE` | sans 600 80/80, `'tnum' 1` | final score |
| `SAY` | sans 600 15/18 | optional call pill |
| `SEAT_NAME` | sans 600 17 (unchanged) | |
Rules: serif for moments (trump, result, titles), sans for controls and numbers; do not mix within one block. Score numerals keep sans 600 36 tnum (06 s1.3) but go through `RollingText` (4.7). Home contrast: `HERO_NOTE` `#a8bcb0` -> `#c3d3c8` (7.5:1 on `#153f36`), strip captions `#9db3a4` -> `#b4c7ba` (6.5:1), header `First to 10` `#596952` -> `#4b5a45` on cream.

### 2.8 Buttons (one primary per decision)

- **Key button** (primary on felt: Continue, Play {card}, bid suit buttons, Play again): `linear_gradient(PI, [(0,#f9efcd),(1,#efdca4)])`, `border(1,#ead8aa)`, text `#233e32`, radius 8 (spec), **physical key**: shadows `(0,3,0,0,#b9a46b)` + `(0,10,18,0,#05160e55)`; top highlight child 1 pt `#ffffffa6`. Hover: lift `offset(0,-2)` + drop deepens to `(0,14,22,0,#05160e66)`, 0.18 s. **Press**: `press_scale(0.985)` + shadows collapse to `(0,1,0,0,#b9a46b)`, `offset(0,2)`. Motion: `.motion(Spring::new(0.18,0.0))` (shadow geometry follows `.motion`: r3 s1.1) with `.press_scale` from `el/visuals.rs:350`.
- **Ghost button** (Pass, secondary, header tools): transparent, `border(1.5,#f5e8c333)`, text cream on felt (on cream shell: border `#cad3c1`, no fill, hover `#e9eddf`). Pass keeps the spec colours `#fffdf6`/`#85957b` as a *light* ghost on felt for contrast; demoted by size (not by hiding).
- Suit buttons (`Spades/Hearts/Diamonds/Clubs`): key button with an enamel suit disc (22 pt, `#fffdf6` disc, glyph `#b44835` for hearts/diamonds, `#243d31` for spades/clubs) left of the label. Labels unchanged (06 5.8).
- Disabled: opacity .55 (spec), shadows collapse; no lift on hover.

### 2.9 Home and New game (cream/forest shell)

- **Home hero**: keep layout (06 s3.1). Hero bg: `fill #153f36` + `radial_gradient([0.72,0.5],[0.52w,0.75h],[(0,#1f5a47),(0.6,#164a3d),(1,#0f2f28)])` so the disc behind the cards becomes light falling on a table, with a darker rim (r6: "smudge"). `border_gradient(1, #d4ed9b26, #d4ed9b00)` on the panel. The art's flat grey **"D" blob** is baked into `euchre-hero.webp`: fix the asset (cream/lime enamel chip with the dealer shadow). Fallback in Rune: an overlay chip at the same art fraction, `fill #f4ead0`, shadows `(0,2,0,0,#b4a57a),(2,8,12,0,#05160e66)`, 52 pt, text `D` `#35482f` `[measure fractions with --tree]`.
- **Compact hero under 640 pt tall** (r6: no CTA visible at 720x520): hide art, drop eyebrow margin, headline to `clamp(40,..)`, CTA at `y <= 0.62 h`. Layout only, no copy change.
- **CTA** "Let's play": lime `#d4ed9b` (spec), add key-button physics: `(0,3,0,0,#a9c46d)` edge + `(0,10,20,0,#05160e40)`; hover `offset(0,-2)` 0.2 s (spec: translateY(-2px), 200 ms, 06 s3.1), arrow `↗` slides `offset(3,-3)`; press `press_scale(0.98)`.
- **Bower stamp** (lime disc 100 pt, 14 deg): add `box_shadow(0,4,0,0,#0f2f28),(2,10,18,0,#05160e66)` and ring `0 0 0 6 #153f36` as before.
- **New game**: two option **cards** instead of bare checkboxes (r6): each card = radius 12, fill `#fffdf6`, hairline `#d9ddcf`, padding 20, checkbox 28 pt with `#285641` accent (outline contrast raised: `#bdc9b2` -> `#8fa384`), hover: lift -2 + E1-soft shadow; checked: fill `#f1f6e4`, border `#285641` 1.5, tick draws on. Right of the column (>=1000 wide) a **table thumbnail**: 260x170 felt miniature (same tokens, `a11y_hidden`, no new copy): four seat dots, dealer chip, trick of 4 mini backs; stick-the-dealer ON puts a small `D` lock tick on the dealer chip; natural-trump ON adds a gold ring on the up-card. Button "Deal me in →" full width, green `#285641`, now with key physics (edge `#1a4431`, drop). Keep column 640 wide but centre the group vertically in the window; nav pill stays top-right (8 pt in).

## 3. Motion system

### 3.1 Tokens (add `theme::fx::motion`; all `Spring::new(duration_s, bounce)`, `rune-motion/src/spring.rs:30`; duration is the period, tail is 1.2-1.5x: r2 s2.1)

| Name | Value | Used for |
|---|---|---|
| `HOVER` | `Spring::new(0.20, 0.0)` | rack hover, button lift |
| `SELECT` | `Spring::new(0.26, 0.28)` | rack select (about 4.6% overshoot: r2 s2.1 table) |
| `PLATE` | `Spring::new(0.18, 0.0)` | shadow plates (r3 `LIFT`) |
| `THROW_YOU` | `Spring::new(0.46, 0.25)` | your play flight (peak about 1.03, settle within 0.01 by about 0.55 s) |
| `THROW_OPP` | `Spring::new(0.50, 0.20)` | opponent play flight |
| `DROP` | `Spring::new(0.20, 0.30)` | elevation release at landing (4.4) |
| `DEAL_CARD` | `Spring::new(0.38, 0.12)` | deal-in per card |
| `PICKUP` | `Spring::new(0.60, 0.05)` | up-card to dealer (spec 850 ms is the cap; see 4.6) |
| `SWEEP` | `Spring::new(0.55, 0.0)` | trick collect (zero bounce: `motion_curve` flights only take zero-bounce springs, r1 s11) |
| `STAMP` | tween `Curve::tween(0.34, Easing::OVERSHOOT)` | stamps, medals (exact-duration overshoot: r1 s2 `easing.rs:26-38`) |
| `PANEL` | `Spring::new(0.34, 0.12)` | dialogs, hand-result card (`SpringPair::new(enter, exit)` exit `Spring::new(0.22,0.0)`) |
| `ROUTE` | `Spring::new(0.40, 0.0)` | screen enter |
| `DRIFT_LAMP` | `tok::motion::DRIFT` 0.85/0 | lamp pool between seats (`tokens.rs:33`) |
| `SHAKE` | `Spring::new(0.12, 0.30)` | illegal-card refusal |
Easings (tweens): flights authored as springs; CSS parity items from 06 (`cubic-bezier(.22,.7,.25,1)` play, `(.4,0,.2,1)` collect, `(.22,1,.36,1)` panel) as `Easing::cubic(..)` (`easing.rs:41`) for non-flight props via `offset_motion/scale_motion/rotate_motion(Curve::tween(d, Easing::cubic(..)))` (`el/motion.rs:122`, `el/visuals.rs:369,389`).
Every spring/curve that deliberately overshoots gets `.motion_intent(MotionIntent::OVERSHOOT)` (`el/motion.rs:74`), shakes `JIGGLE`, wind-ups `RECOIL`, so `rune::motion_gate` accepts them (r5 s2.7).

### 3.2 The pace contract (animation never delays state or input)

Principle: **the view always renders the final game state at once; motion is the diff.** Input follows `legal`, never an animation.

1. **State first.** Rack, bid buttons, Continue, Play are live at the sync, exactly as 06 s7 (disabled only by `submitting`/disconnect). Decorative gating (e.g. winner ring after landing, result line cross-fade) uses `enter_delay` or a `landed_at` stamp in `Presentation::poll`, never a state hold longer than 0.5 s, and never on `legal`, `Continue`, announcer text or `bids[]`.
2. **Flights are never queued.** A newer commit retargets a running matched flight from its drawn pose and velocity (r1 s6 "retarget_drawn"; two springs in a row merge, r1 s3). A card that arrives while another still flies just flies too (cap 6 concurrent).
3. **Coalesce.** Several `cardPlayed` steps in one sync (a client that lagged) animate **only the last**; earlier ones are marked seen and appear settled (reuses `seen_steps`, `present.rs after_change`). A sync with `steps: []` (connect, review set/ready, trick_resolve -> trick_play, 02 s3.1) is a **snap**: apply with `tx.instant()` (which also suppresses flights and launches, r1 s11), clear every overlay, drop banners.
4. **Fast-forward knob = `Cx::time_scale`** (`runtime/cx.rs:266`, clamp [0,8], r1 s11; the `settings` example uses exactly this for a user speed, r4). `scale = user_speed * catchup`, `catchup = min(3.0, 1.0 + 0.6 * (busy - 1))`, `busy` = number of presentation items in `Presentation` whose `ends_at > now` (flights, overlays, the collect). Prod tempo: `busy` is 1-2, `catchup` about 1.0-1.6. `EUCHRE_FAST_TEMPO=1` (100-250 ms gaps): `busy` climbs, `catchup` -> 3.0, a 0.5 s flight lands in about 0.17 s. **Spike S9:** confirm `Cx::time_scale` retimes springs already in flight (it is documented as the window clock scale); if not, pick the flight spring per commit instead: `tx.motion(Spring::new(nominal * k, bounce))`, `k = clamp(gap_since_previous_sync / (1.25 * nominal), 0.35, 1.0)`.
5. **Drop order under pressure** (first to go): confetti and sparkles, hand-score extras (pips, stamp), trump wash, impact rings, winner breathing, score roll (jumps), then flights shorten to a 0.2 s floor; text and state never.
6. **Overlay budget**: at most 2 live one-shot overlays (a `VecDeque` like `OneShots`, r3 s4: oldest retires with a 0.16 s fade); at most 1 `backdrop_blur` panel + 1 scrim at a time (max 4 per frame, `render/backdrop_blur.rs:10`, r3 s1.4).
7. **Reconnect**: view arrives via snapshot (`snap.v >= view.v`) or `sync` with `steps: []` -> snap path (3). Pre-seed all keys (existing `dealtKeys` equivalent) so no deal, no flight, no confetti replays. If the server advances out of `trick_resolve` before our gather finished (reconnect mid-hold, fast tempo): the trick cards vanish with `tx.instant()` and stacks update (existing `c.moved = c.plays.len()` path, `present.rs:~320`), not a flight into a state that no longer exists.
8. **First second still** (r5 s2.5; launch rule `README.md:733-737` at HEAD): nothing animates the first second after launch; Home's entrance flourishes start at `t >= 1.0 s` or on back-navigation. Table enter is unaffected (a route change, not launch).
9. **Keep the 2.2 s read hold** (06 s6.5): the gather starts `COLLECT_AFTER` 2.2 s after the winner shows, in state logic, never in an animation delay. Hand-result card must be gone by 2.4 s of a 2.5 s hold.

### 3.3 Reduce Motion matrix (one table, applies to every item in s4)

Rune does this for free: matched/launch off, `press_scale` off, `scale/rotate/offset` land at once, `Sequence` lands its final pose, opacity/colour/presence get the 0.15-0.18 s fade (`rune-motion/src/props.rs:44-75`, r1 s11, r2 s5). Not free (you must branch on `cx.is_reduced_motion()`):

| Thing | Reduced behaviour |
|---|---|
| anything owned by `Live`/`Tween`/`Animated` (`[DRV]` flips, timers) | `set()` the end value; Tween bars may stay (timed value, informative) |
| `LoopSpec`, `Cycle` ambient (breathing, thinking dots) | static: use `Cycle::still` (r2 s7.5) or opacity 1; thinking dot stays static (spec) |
| lamp pool travel, felt wash, impact rings, sparkles, confetti, stamp shake | removed; static gold ring where it carries information (winner, active seat, legal glow) |
| `time_scale`/catch-up | ignored (1.0) |
| `Transition::none()` for elements that must not fade | keep |
| Score roll (`RollingText`) | numeral swaps with the 0.15 s fade |
| Hand-result card, match-end panel | appear by opacity only (no scale, no offset), same dwell |
| Deal | cards appear by fade with `enter_delay` stagger kept (kept under Reduce Motion: r1 s5) so deal order is still readable |

## 4. Gameplay moments (choreography, build, reduced, pace, look)

Notation: `t` in ms from the sync's arrival; `P` = pace factor from 3.2 (durations multiply by `1/scale`). "Commit B" = a second view commit scheduled with `tx.wake_after(Duration)` from `AppSpec::poll` (`tooling.rs:35`, `el/tx.rs:226`).

### 4.1 Route transitions (Home, New, Table, Games, Settings)

Facts: `screen.key("screen-{route}")` swaps (`app.rs:915-936`); root `fill(screen_bg)` is a property that springs (colour animates in OKLab, r1 s3). An exiting node plays the transition it carried when it was last drawn, so **exit cannot depend on the direction of the navigation that removes it**; only enter can. Therefore direction-agnostic pair + direction-aware enter.

| Item | Spec |
|---|---|
| Pair | `Transition::asymmetric(Transition::offset(0.0, enter_dy).combined(Transition::opacity()), Transition::opacity().combined(Transition::scale(0.985)))`, `.springs(SpringPair::new(ROUTE, Spring::new(0.24,0.0)))` (`transition.rs:210,250,254,261`). `enter_dy`: forward `+18`, back `-12`. Reduced: opacity only (automatic). |
| Root bg | cream -> `#17452f`: `tx.color_motion(Spring::new(0.55, 0.0))` in `navigate` (`el/tx.rs:558`, a theme change uses REFLOW-class timing). The old screen fades over it. |
| Home -> New shared CTA | Home "Let's play" (`key "cta"`) `.matched("cta-primary").matched_crop()` -> New "Deal me in" (same id). Lime button grows into the full-width green key, fill carried then colour-springs; `matched_crop` keeps its label at own size instead of cover-scaling (`el/motion.rs:224`) `[spike S4]`. Spring `Spring::new(0.5, 0.0)` via `tx.motion` on the navigate commit. |
| New -> Loading -> Table | "Deal me in" `.matched("cta-orb")` -> the loading screen's 96 pt green disc with `♣` (radius 48, fill `#285641`): the button becomes the orb. Orb then fades as the table builds (4.2); **no** chained flight into the deck (the deck is not mounted in every first view: Case A, 06 5.4.4). `unclipped_flights` on this commit. |
| Nav pill active highlight | active link bg `.matched("nav-pill")`: lime pill glides between links (`Spring::new(0.30,0.12)`), r4 technique #14. The pill itself enters from Home with `offset(0,-16)+opacity`. |
| Back / Esc | same pair, `enter_dy -12`. |
| Settings rail, dialogs | 4.12. |

### 4.2 Loading ("A seat at the table.", "Your hand is on its way.") 

- Layout: orb 96 pt centred (the matched target), headline serif 500 40, one line p 19 (strings 06 14.1). Under the orb **four seat dots** (10 pt, `#d4ed9b`) lighting in sequence: `LoopSpec::opacity(1.6, i*0.2, 0.25, 1.0)` each (`looping.rs:33`; same period stays in phase, delay offsets the order) = seats filling. The `♣` inside the orb breathes `Cycle{period:2.4, stops:[(0,0.7),(0.5,1.0),(1,0.7)], still:0.9, fps:30}` (`cycle.rs:8`, wakes at 30 fps not display rate).
- Headline swap "A seat..." -> "Your hand is on its way." `Transition::blur_replace()` on the keyed heading (`transition.rs:231`); reduced = fade.
- Reconnect button/link/error: `notice` row `offset(0,8)+opacity`.

### 4.3 Table build-in (first view of a table, and returning to the table)

`t` offsets with `enter_delay(Stagger::new(0.06).delay(i, n))` (`el/enter_delay.rs:36`, `choreo.rs:35`; mounts only; ignored on first frame and on instant):
| i | element | transition |
|---|---|---|
| 0 | header | `offset(0,-14)+opacity` (exists, `header.rs:113`), `PANEL` |
| 1 | tabletop + rail | `Transition::scale(0.97).combined(opacity)`, `Spring::new(0.50,0.0)`; rail follows with `+0.04` delay |
| 2 | seat plates, partner, left, right (stagger 0.06) | `offset(0,10)+opacity`, `PANEL` |
| 3 | trump readout + tricks readout | `offset(0,8)+opacity` |
| 4 | hand area / rack | `offset(0,24)+opacity` |
Lamp pool fades `0 -> 0.12` alpha over 0.8 s (opacity prop on the pool el, `.opacity_motion(Curve::ease_in_out(0.8))`). Total 0.9 s. Reconnect and Back-to-table with a hand in progress: `tx.instant()` (no build-in). Reduced: the existing opacity fade (stagger kept).

### 4.4 Your card to the trick: the throw (06 6.2)

Today: one straight spring (`PLAY` 0.7/0.1), nothing else. Target feel: lifted, thrown, landing with weight.

| t (ms, P=1) | What |
|---|---|
| 0 | Press / double-click commits `submitMove`; optimistic view moves the card (existing). Rack slot keeps its hole (06 5.6). The card **mounts in the trick carrying `matched("hcard-{h}-{card}")`** (exists, `stage.rs:732`) with `.motion(THROW_YOU)`. It starts **elevated**: the trick card el has `scale(1.10)` and its plate shadow is E4 (`(4,24,36,0,#05160e80)`), because new-mount offset/scale snap to the target (`rest_quiet`) and so carry the elevation into the flight `[spike S1]`. |
| 0-280 | straight matched path rack -> slot (about 250-320 pt), `THROW_YOU`; draws above everything; `unclipped_flights()` on this commit (`flight.rs:36`). Card art keeps cover-scaling (single `Pixels` image: leave `matched_crop` off, r1 s6). |
| 280 | **Commit B** (`wake_after`, `landed_at` stamp): trick card `scale 1.0`, plate shadow to E2; spring `DROP` (bounce 0.30) = the drop. `motion_intent(OVERSHOOT|RECOIL)`. |
| 300-640 | landing **slam**: card el `.sequence(Sequence::new(&SLAM).delay(0.26))` with `static SLAM = [Beat 0.00 scale 1.0, Beat 0.07 scale 1.045 EASE_OUT, Beat 0.20 scale 0.99, Beat 0.34 scale 1.0]` (`sequence.rs:14-39`; key includes the play key so it replays once per card). `[spike S2: Sequence.delay holds the first pose; interplay with matched + enter_delay]` |
| 300-900 | **ring** under the card: `el().key("ring-{play}").absolute(centre).size(w,h).radius(10).border(2,#eed5917f)` `.paint_order(-1)` `.sequence(RING.delay(0.30))` with `RING = [Beat 0.0 scale 0.7 opacity 0.0, Beat 0.06 scale 0.8 opacity 0.6, Beat 0.55 scale 1.45 opacity 0.0]`; removed by state at 0.95 s (sequence holds last pose, invisible; remove to avoid dead nodes). `[FX]` replacement: `Fx::pulse(PulseStyle::Ripple)` 0.8 s, 90 pt reach, gold (`r3 s4`, 0.086 ms). |
| 300-640 | plate shadow settles `E4 -> E2` on `PLATE`; the trick card gets its tilt (existing `-7/3/7/-3` deg, 06 5.4.3). Tilt is applied to the destination, carried-flight rotation is not (r1 s6/gotcha 17): set `rotate(slot_tilt)` on the destination from the start so it lands tilted (no animated tilt-in). |
| z-order | flights draw above the window; settled trick cards use `paint_order(play_index)` so later plays overlap earlier ones (`el/motion.rs:142`); winner raise uses +10 (4.5). |
Reduced: no match; card appears with the REDUCED fade; no ring, no slam; plate shadow sets at once. Pace: if `busy>1` the slam/ring are skipped first (drop order 3.2.5).

**Fallback if S1 fails** (elevation not carried): plain `matched` + `Spring::new(0.46,0.25)` and the slam/ring only (still a big gain over today's slide). **Upgrade** `[DRV]` for one hero card (optional): `Flight` arc driver (r2 s7.1 VERIFIED compile): `p` spring `Spring::new(0.46,0.25)` with `PropClass::Opacity` (r2 s2.3), lift `sin(pi*clamp(p,0,1))*54`, spin `0.30*(1-p)` rad, real destination hidden (`opacity 0`) until `!moving`; start/end from `cx.window_layout("hcard-..")`/`("trick-you")` (`layout_read.rs:281`); `poll` returns false when settled (r2 gotcha 8). Use only if S1 reads as a slide.

### 4.5 Opponent card to the trick

- Origin: top back of the seat's **30x42 fan** (2.5, was 14x20) carries `matched("play-{h}-{trick}-{seat}")` (exists, `stage.rs:558`, cookbook gotcha 15 monotonic ids). During the flight it draws the **face** (r1 gotcha 17): grows 30x42 -> 125x163 in `THROW_OPP`, `unclipped_flights()`.
- Start tilt -6 deg (06 6.2) cannot ride the flight; fan back already carries its fan rotation, flight starts there, the face lands at its slot tilt. Accept.
- Seat plate "release": on the commit the card leaves, the plate's `offset` nudges 3 pt toward the trick and returns (`Spring::new(0.2,0.3)`, two commits `wake_after(0.12)`); skip when `busy>1`. Count badge decrements with `RollingText` size-8 numerals.
- Same landing sequence as 4.4 (SLAM delay 0.30, ring, shadow settle). Seat plate loses its active edge-light at the same commit (turn passes, 4.8).
- Thinking: three dots on the active AI plate, `LoopSpec::opacity(1.2, i*0.2, 0.25, 1.0)` (extends the spec dot, `stage.rs:491`; static under reduced). Extended think (`ThinkingEvent.extended`, 02 s3.2) adds a 4th dot only if the server ever emits it.

### 4.6 Trick won: spotlight, wind-up, sweep (06 6.4; server hold 3300)

Master clock for a normal trick (P=1; matches 02 s11):
| t (ms) | Beat |
|---|---|
| 0-500 | 4th card throw/slam (4.4/4.5); `trickWon` + `cardPlayed` arrive in one sync (02 s10). |
| 420 | result line **cross-fades in**: `Transition::blur_replace()` on the result text (key = winner seat+text) with `enter_delay(0.42)`; strong lead-in `{Seat} wins the trick.` colour `#ffecb7` (exists). Announcer fires at 0 (unchanged). |
| 560 | **winner ring** (after landing, 06 6.4 wanted it after the flight; r6 s1 shows it at frame 1 today): child ring `border(3,#eed591)`, radius 10, offset 5, `Transition::scale(1.06).combined(opacity)`, `PANEL`; `.sequence(PING)` `[0:scale 1.0, 0.14: 1.04 EASE_OUT, 0.40: 1.0]`. Gold soft glow plate behind the winner card `(0,0,28,2,#eed59180)` that **breathes**: `looping(LoopSpec::opacity(1.8, 0.0, 0.7, 1.0))` (stops under reduced). |
| 600 | winner card: `offset(0,-8)`, `scale 1.03`, `paint_order(+10)` on `HOVER`; the other three get the veil (`#10382c`, alpha 0.22). Winner's seat plate goes active-gold (2.5). Lamp pool slides to the winner (`DRIFT_LAMP`). Tricks counter ticks: `RollingText("tricks-{team}", n)` `.spring(Spring::new(0.45,0.10))`. |
| 600-2040 | read: static + breathing glow. |
| 2040 | **wind-up** (anticipation, `RECOIL`): the four cards `offset` 10 pt *toward* the *opposite* of the winner and `scale 1.03` for 0.16 s on `Spring::new(0.16,0.0)` (commit via `wake_after`, like the cursed-settings flush: r4). |
| 2200 | **sweep**: existing gather (`present.rs poll`: one mini per 45 ms into the winner's stack, `COLLECT_STAGGER 0.045`, `COLLECT_AFTER 2.2`) with `SWEEP` and `tx.unclipped_flights()`; destination minis are the 28x40 stack backs (2.6) so cards shrink to 0.18 and the *face turns into a back* during flight (flight draws the destination picture, r1 gotcha 17: a visual flip for free). |
| each landing (2200+45i+~330) | stack **thump**: stack el keyed `stack-{seat}-{n}` `.sequence(THUMP)` `[0:1.0, 0.09: 1.18 EASE_OUT, 0.26: 1.0]` + count badge `RollingText`. |
| 2900 | gather done; veil/ring/glow removed (instantly: they were children of the trick cards that left). Lamp pool returns to centre. Server advances at 3300 (margin 400). |
`sealsEuchre` (server hold 3800, 02 s9.2): at 2900 add a **crack** beat: felt rim goes `#c0554a` (spec euchre ring colour, 06 s16) `border` colour spring 0.3 s, holds 0.5 s, returns. Optional shake: tabletop `offset` +-2 pt x2 via commits (`JIGGLE`); off under reduced.
Review mode (Continue): no `COLLECT_AFTER` timer; wind-up + sweep start on press, button label `Collecting… →` (exists). Fast tempo (trick hold 120 ms): winner ring etc. never appear; the gather snaps (3.2.7). `[FX]` sparkles ring at winner's stack: `SparkleParams{count 28,size 11,life 1.3,margin 30,burst:t0}` + ripple (r3 s4), 0.27 ms; core version = the ring above.

### 4.7 Scores: header, pips, numerals

- Header numerals via **`RollingText`** (digit roll, `rolling_text.rs:110-154`; keep `key` stable, `.value(f64)` really changes, `.spring(Spring::new(0.5,0.12))`) replacing the offset+opacity swap in `header.rs:111-126` (r5 s5: supported "number ticks" effect). Fallback `Transition::blur_replace()`.
- **ScoreCards pips** (06 5.3: pip i lit when `i < min(score,6)` / `i < max(0,score-6)`, unlit opacity 0.08): lighting pips get `.sequence(PIP)` `[0:opacity 0 scale 0.4, 0.14: scale 1.35 EASE_OUT, 0.30: scale 1.0]` keyed `pip-{team}-{i}-{lit}`, `Sequence::delay(0.09 * rank)` so a +2 fills pip by pip; lit pip colour holds, a 0.8 s gold halo (`box_shadow(0,0,10,0,#eed59180)` sprung to 0). Unlit stays 0.08.
- Leader underline: 3 pt gold bar under the leading score numeral, width springs (`Spring::new(0.4,0.1)`), hidden at a tie.
- Trick counters (`Your team 1 · Other team 2`, 06 5.4.1): `RollingText` 25 pt.

### 4.7a Hand scored (hold 2500; 06 5.5/9/12; 02 s10)

Today the weakest screen (r6 s4). Sequence, `t` from the `hand_score` sync; card must be gone by 2400:
| t | Beat |
|---|---|
| 0-250 | previous collect may still be running; hand-area heading/instruction dim to alpha 0.55 (their strings unchanged: "Make yourself at home." / "Hand complete. Adding the points…", 06 5.5/12). Trick area veil `#0b1f168c` fades in over the felt content (0.3 s). |
| 250 | **Hand-result card** mounts (gated by `Presentation.poll` after 200 ms in `hand_score`, so it never flashes at fast tempo): centred over the felt, 360 x 200, `fill #fff8df` (banner colour 06 s9), radius 16, E5 shadows, text `#253c2d`. `Transition::scale_anchor(0.94, [0.5,0.5]).combined(opacity).combined(offset(0,14))`, `PANEL`. Content: label (`HAND_RESULT_LABEL`) = the announcer's own result words `Point.` `Lone point.` `March!` `Lone march!` `Euchre!` `Thrown in.` (06 s10); numeral `+{n}` (`HAND_RESULT_NUM`, `RollingText` 0 -> n from t=500, 0.45 s); line `You score {n}.` / `They score {n}.` / `Nobody scores.` (06 s10); line `Tricks: your team {a} · other team {b}.` (06 s11 string). |
| 600 | header score numerals roll; pips fill (4.7); gold halo. |
| result flavours | `point`/`lone_point`: plain. `march`: add ring burst (4.4 ring, bigger) behind the numeral. `lone_march`: + confetti (existing banner path). `euchre`: **stamp** child `Euchre!` `STAMP` colour `#c0554a`, `rotate(-0.10)`, scale `1.3 -> 1.0` via `Sequence` `[0: scale 1.35 opacity 0, 0.10: 1.0 EASE_OUT opacity 1, 0.20: 0.97, 0.32: 1.0]` + the 4.6 crack and a 2 px shake; card border `#c0554a`. `throw_in`: card grey `#8a8a8a` (spec ring colours: point/march/lone `#e8c27a`, euchre `#c0554a`, throw_in `#8a8a8a`, 06 s16), no numeral. |
| 2200 | card exits (`PANEL` exit 0.22) before the server advance at 2500; veil off. Next hand's deal begins on the server tempo (4.9). |
Reduced: card by opacity, numeral swaps, no stamp rotation (static), same dwell. Fast tempo (250 ms hold): card never mounts; header numerals jump.
`[FX]` sparkle ring for march/lone, ripple for others (0.27 ms).

### 4.8 Turn flow: seat plates and the lamp

- Active seat plate (2.5) springs `0.30` colour/glow; previous plate relaxes at the same commit.
- **Lamp pool follows the turn**: `lamp.offset(dx, dy)` to the active seat's pull point (35 percent of the way from centre to the seat anchor) on `DRIFT_LAMP` (0.85/0), `.offset_motion(DRIFT_LAMP)` (`el/motion.rs:122`); alpha rises to 0.15 on the human's own turn. Reduced: stays centred, plate light carries the information.
- Human's turn: heading "Your turn." gets a 6 pt gold dot to its left that pulses `LoopSpec::opacity(1.4,0,0.45,1)` (static under reduced). No timer: human turns never time out (02 s6.5).

### 4.9 Deal (hand start; `cut`, `dealt` x8 packets, `upCardTurned` in one sync, then AI bid floor 1900)

Budget <= 1.6 s; bid buttons live from t=0 if the human is eldest (state-first). Pattern 3-2/2-3 packets (02 s12.3): card i of seat s arrives at `stagger(packet_index) + intra`.
| t | Beat |
|---|---|
| 0 | dealer **chip hops** to the new dealer: `matched("dealer-chip")` between the old and new seat chips (`Spring::new(0.45,0.12)`); hand number eyebrow rolls (`blur_replace`). |
| 50-830 | 8 packets, `Stagger::new(0.095).from(First).delay(packet, 8)` + 0.03 per card: backs mount in the three fans and your rack with `Transition::offset(dx,dy).combined(Transition::opacity()).springs(SpringPair::new(DEAL_CARD, Spring::new(0.2,0.0)))` where `(dx,dy)` = dealer anchor minus the card's final position (from `cx.window_layout("seat-anchor-{dealer}")`, `layout_read.rs:281`; layouts exist from the previous frame; fallback constants per seat). Transition `offset` is not clamped (`kit_rules` clamps scale/blur/opacity only, r1 s4). Cards land with `E1` shadows. |
| 830-1250 | **your five flip** back -> face, 60 ms apart, 300 ms each: `[DRV]` width squeeze (`flip(p, w, h, back, face)`, r2 s7.2; `p` tween EASE_IN_OUT 0.30 s, bounce 0; lift `offset(0,-6*sin(pi p))`). Only 5 concurrent `Animated`, per-frame rebuild of the rack only (r2 s6: fine at 5). `[CORE]` fallback: face-up from the first frame, entrance only. |
| 850-1580 | **up card**: leaves the deck stack (`matched("deck-top")` -> up-card slot) with `Spring::new(0.40,0.15)`, lifts `scale 1.08` then flips (same `flip`, 0.32 s) at 1250; on reveal a **glint** sweeps the face: child `linear_gradient` band (white 0 -> 0.35 -> 0) clipped by the card radius (`radius()` on the parent + a `clip_rounded` wrapper), `looping(LoopSpec::slide(0.7,[-1.3,0],[1.3,0]).easing(EASE_IN_OUT))` for one pass, removed by state at 0.8 s (slides wrap, r1 s8). |
| 1250 | trump-ready light: lamp pool brightens to 0.16 for the first bidder. |
Reduced: backs and face appear by opacity with the stagger kept; flips are crossfades (`El::dissolve`, `el/visuals.rs:295`); no glint. Pace: at `busy>=2` skip flips and glint (backs appear face-up directly). First hand after table build: build-in (4.3) then deal after 0.9 s (still inside the 1.9 s floor because the server's first think is for the first bidder).
Discard-phase deal never replays (existing rule: skipped during `dealer_discard`).

### 4.10 Bidding

- **Bid area** (06 5.4.4): deck stack + text block left; up card with E2 shadow and a slow **float** (`offset(0,-3)`, no loop available for ping-pong: use `Cycle` opacity on its shadow plate only, `(0.0,0.7),(0.5,1.0),(1.0,0.7)` period 3.0, so the shadow breathes under a still card) `[CORE]`.
- **Bid buttons** row (human's turn): `Transition::offset(0,14).combined(opacity)`, `PANEL`, `enter_delay(Stagger::new(0.045))`; Pass ghost first, suit keys after, `Go alone` toggle last (checkbox: tick `Transition::draw()` on a canvas path, box colour spring; `el/transition draw()` `:242`). Pressing a suit: `press_scale(0.985)`, key collapses; the chosen button stays 0.15 s then row exits `offset(0,8)+opacity` (exit 0.2).
- **Seat state words** ("Passed", "Thinking…", 06 5.4.2) swap with `Transition::blur_replace()` (not hard cut); "Passed" plate dims to alpha 0.8.
- **Call pills** (optional, 06 s8 says call lines are *not* bubbled; needs founder OK, section 8): speech pill under the seat plate with the `bids[].say` string (`Pass.` `Order it up.` `I assist.` `Crossing the creek.`), `SAY` 15/600, fill `#f4ead0`, radius 999, `Transition::scale_anchor(0.9,[0.5,0.0]).combined(opacity)`, `Spring POP`-class `Spring::new(0.25,0.15)`, auto-remove 1.6 s; stacks at most 1 per seat.
- **Bid-area overflow bug** (r6: `bid-copy` 137 tall in a 123 box): layout fix in `stage.rs`, not motion; text `max_h` + `clip`-free wrapping; drop the duplicate "You - Your turn" detail when heading says it (strings stay, hidden from view only; a11y labels remain).

### 4.11 Trump named

On `trumpSet` (suit, makerSeat, aloneSeat, viaOrderUp; 02 s1.4):
| t | Beat |
|---|---|
| 0 | trump readout: text swap "Choosing trump" -> "{Suit} are trump" `blur_replace`; **suit disc** (30 pt enamel `#f4ead0`, glyph suit-coloured) left of it `Transition::scale(0.5).allow_small_scale().combined(opacity)`, `.springs(SpringPair::new(Spring::new(0.30,0.20), Spring::new(0.2,0.0)))`, keyed by suit so it replays; plus `.sequence(POP [0:1.0, 0.12:1.25 OVERSHOOT, 0.30:1.0])`. |
| 0-900 | **felt wash**: full-tabletop overlay, `fill` suit tint (hearts/diamonds `#c0493a`, spades/clubs `#cfe8d8`), `.sequence(WASH [0:opacity 0, 0.18:opacity 0.14, 0.90:opacity 0])` (opacity-only beats; all other props 1.0). One-shot, key = `wash-{hand}`; 1 overlay slot. |
| 100 | maker chip pops on the maker seat: same Transition as the disc; `matched` not needed. If `viaOrderUp`: the up-card then flies (4.12). |
| alone | `★ {seat} alone` chip `offset(0,-8)+opacity`; felt rim gold pulse `#f2d887` 0.4 s; **sitting partner**: plate `opacity .45` (property spring), fan cards fade; banner `{seat} {is/are} going alone!` (existing). |
Reduced: swaps by fade, disc static, no wash. Pace: wash dropped at `busy>=2`. `[FX]` single-hue Aura rim (r3 s5: rainbow default is loud, set all three colours to gold tints) only for lone calls.

### 4.12 Up-card pickup and dealer discard (06 6.3: 850 ms, shrink 0.28, rotate 8 deg, arc 24)

- **Pickup** (viaOrderUp, dealer is not you): existing `matched("pickup-{h}")` up-card -> dealer fan back (`stage.rs:776`) with `PICKUP` `Spring::new(0.60,0.05)`, `unclipped_flights()`. Add the arc + lift cheaply: up-card el `scale 1.08` at mount (S1 elevation carry) and plate shadow E4, released on commit B at 0.26 s; the fan back receives it with a `THUMP` sequence + count badge roll. 850 ms was the web's cap, 0.60 s period lands inside it. Reduced: card fades, fan count updates.
- Dealer is you: `matched("hcard-{h}-{up}")` up-card -> your rack's 6th slot (exists, `stage.rs:67`): same lift; rack slot appends (06 5.6 slot retention) with a gold underline that fades after 1.2 s so the new card is noticed.
- **Discard** (`dealerDiscarded`, no card field ever, 02 s1.4): the selected card **turns face down and shrinks into the deck/kitty stack**: matched id `kitty` on the stack top; because flights draw the destination picture (r1 gotcha 17) the card visibly becomes a back. 0.45 s `Spring::new(0.45,0.0)`; kitty count not shown (never reveal buried cards, 06 s11). AI dealers: a back drops from their fan to the kitty by the same matched id.
- Reduced: fade.

### 4.13 Your hand: hover, select, press, illegal, keyboard

All on the plate pattern (2.2):
| State | Card props | Plate shadow | Spring |
|---|---|---|---|
| rest | `offset 0`, `scale 1.0` | E1 | |
| hover (`hover: hover` pointers only, 06 5.6) | `offset(0,-10)`, `scale 1.04`, `origin(0.5,1.0)` | E3 | `HOVER` |
| selected | `offset(0,-18)`, `scale 1.06`, `outline(3,3,#f5d98e)` | E4 | `SELECT` + `motion_intent(OVERSHOOT)` |
| keyboard focus | as hover + focus ring 3 pt | E3 | |
| press | `press_scale(0.97)` (`el/visuals.rs:350`) | E2 | built-in |
Neighbour parting: cards within 1 slot of the hovered card `offset(+-8,0)` on `HOVER` (reorder rows slide aside via offset springs, r1 s10); selection also fans the row open 4 pt. Caption fades in (2.6). **Illegal press**: `refuse` shake: offsets `+6, -5, +3, 0` over 4 commits 55 ms apart (`wake_after`), `SHAKE`, `motion_intent(JIGGLE)`; the instruction line becomes the reason message (exists, 3.2 s) with `Transition::blur_replace()`; no sound change. Reduced: no shake; the veil pulses opacity once (fade) and the message shows. Selection `Selected ✓` stays text. Hit targets never move (slots keep holes; the visual offset does not change hit rects except by `offset`, which hit-tests follow: keep hover lift <= 10 pt so the pointer does not drop off the card at the lower edge: use `hover` on the plate-free outer el of fixed size `[spike S6]`).
`Double-click` plays at once; the throw (4.4) starts from the card's drawn (lifted) rect, so the card never jumps.

### 4.14 Match end and loss (`gameWon`; 06 s9)

- Hand-score card (4.7a) runs first (hold 2500, then `gameWon` on the next release, 02 s10). On `gameWon`:
  - Tabletop veil `#0b1f168c` + `backdrop_blur(6)` on the veil el (one blur, `el/backdrop.rs:34`) over 0.6 s; seat plates dim to 0.5.
  - **End panel** (centre, 440 x 300, cream `#fff8df`, E5): `END_TITLE` = the banner words verbatim: "Your team wins!" or "A good game. Well played." (06 s9); `END_SCORE` `10 – 7` via `RollingText` from the previous score (0.7 s); line "Your team wins the match!"/"The other team wins the match." (06 s12); key button `Play again →` (primary), `Back to home` ghost. `Transition::scale_anchor(0.94,[0.5,0.5]).combined(opacity).combined(offset(0,18))`, `PANEL`.
  - **Win**: confetti 40 pieces, 3.2 s, two-level loops for a ballistic path: wrapper `LoopSpec::slide(T,[0,0],[dx,0]).easing(EASE_OUT)` (decelerating sideways) + child `LoopSpec::slide(T,[0,-1],[0,fall]).easing(EASE_IN)` (accelerating fall) + a third static `rotate(angle_i)`; colours `#e0c579 #497d69 #ba5e46 #eed591`; piece 8x14, delay `(i%8)*0.06`; the layer removed by state at `T+0.5` (slides wrap: r3 s4, `confetti_until` exists). `[spike S12]` for nested slides. Spin: `LoopSpec::rotation` is documented for icon quads only (r1 s8): do not use. `[FX]`: sparkle burst along the felt edge, single-hue aura (r3 s5).
  - **Loss**: no confetti (spec); felt colours drift cooler over 0.8 s (`felt-base` stops `#2d6a54 -> #285a4d`, colour spring `REFLOW`), end panel in plain tone, `Play again →` still primary. This gives losses a tone without celebration.
  - Replaces the "banner under three repeats" layout (r6 s4): the hand heading/instruction/Play-again row stay in the tree for a11y and keyboard order but are visually dimmed under the veil.
Reduced: no confetti, veil by opacity, numerals swap, panel by fade.

### 4.15 Dialogs, rails, toasts, banners

- **Scrim**: `el().fill(#06050 + alpha .68)` (rules; `#10251dcc` review, spec 1.4/5.10) with `backdrop_blur(8)` on a *single* scrim el, fade 0.2 s (spec 200 ms). Never more than one.
- **Dialog sheet** (Rules, Review, Tour; spec radii 16/0.9rem): E5 shadows, `border_gradient(1, #e8c27a59, #e8c27a14)` (spec border `rgba(232,194,122,.35)`), `Transition::opacity().combined(Transition::scale(0.96)).combined(Transition::blur(3.0))` (kit-legal: scale in [0.9,1.1], blur <= 4, nothing travels; example `glass-panel/src/main.rs:143`, r4), `.springs(SpringPair::new(PANEL, Spring::new(0.22,0.0)))`. Esc/backdrop close unchanged.
- **Tour**: dots become a pill row: active dot width 7 -> 20 pt springs (`Spring::new(0.30,0.12)`); slide content `Transition::blur_replace()` keyed per slide; primary `Deal me in` key (`#d4ee86` on `#12362b`, spec).
- **Settings rail**: width spring 0 -> 340 (existing, `panels.rs:549`), `Spring::new(0.38,0.0)` (320 ms `(.22,1,.36,1)` tween in the web); panel sections enter with `enter_delay(Stagger::new(0.04))` `offset(24,0)+opacity`; compact mode overlay shadow `-16px 0 48px #10251d30` (spec) + backdrop 200 ms; chevron on the Settings button rotates 180 deg (`rotate_motion(Spring::new(0.22,0))`). `game-content` shift `translateX(-24)` stays.
- **Toasts / notices**: `offset(0,16)+opacity`, `PANEL`, E5 token (`theme::shadow::TOAST`); a drain hairline `Tween::<[f32;2]>::linear([0,1]->[1,1], 4s)` on a canvas line `live_trim` (`live.rs:16`, `el.rs:390`); timed, informative, so not reduced. Banner (06 s9): `reveal(Edge::Leading)` wipe in (24 pt feather, `transition.rs:246`) + star `★` pops (`POP` sequence); 4.5 s; position at top 104 (spec) not inside the hand area (r6 s4 bug).
- **Reconnecting notice** (06 14.2): `offset(0,-12)+opacity`; amber dot `Cycle` pulse; hides instantly on reconnect (`tx.instant()` for that commit).

### 4.16 Games, Match detail, Settings (dark scrapbook, 06 s17-18)

Minimal pass, all `[CORE]`: match rows hover lift -2 + E1; stat tiles `RollingText` count-up on load (once, not on reconnect); outcome chips unchanged; persona cards slide dial thumbs with `live_offset(knob.smooth(Spring::new(0.16,0.0)))` (r4, live-values); presets chip press `press_scale(0.97)`; save success note `offset(0,8)+opacity`. Pagination: list swaps with `blur_replace` on the list container (no per-row transitions).

### 4.17 Home motion

Launch: static (first-second rule). At `t >= 1.0 s` (state flag from `poll`): bower stamp mounts with `Transition::scale(0.6).allow_small_scale().combined(opacity)` + `STAMP` tween + `.sequence([0:1.0,0.10:1.12,0.24:1.0])`, rotating to its 14 deg rest (set `rotate(14°)` at mount; no tilt-in). On back-navigation to Home: hero text lines enter `offset(0,16)+opacity` with `Stagger::new(0.07)` (eyebrow, h1 line 1, line 2, lede, CTAs); art `scale(0.97)+opacity` +0.15. Rule cards: hover lift -3 + E1, `+` in the disclosure rotates 45 deg (`rotate_motion(Spring::new(0.25,0.2))`, spec). Scroll reveals deferred (needs `on_scroll_offset` -> `Live`, r1 s13).

## 5. Perf and cost (from docs, not measured here)

- Static: the cached tabletop (2.3) means a resting table draws nothing. Window sleeps when idle (r5 s2.5): every `looping`/`Cycle` must be removed or paused when its subject ends (idle rule, `README.md:752` at HEAD); breathing loops live only while the state they decorate (active seat, winner) lasts.
- Concurrent: flights <= 6, one-shot overlays <= 2, `backdrop_blur` <= 2, rotated fans: 15 cards (3 seats x 5) each draw into a layer while turned (r1 s12): `cache()` each fan and rotate once at mount; do not animate fan rotation.
- `RollingText`: 6 live (2 team scores, 2 trick counts, 2 stacks).
- `[DRV]` flips: 6 concurrent `Animated` max (5 rack + up card), each one view rebuild per frame for ~0.4 s (r2 s6).
- `[FX]`: lazily built pipelines cost 20-150 ms the first time (r3 s2.8): pre-draw each used kind once at `on_ready`; `animating()` true only while an effect is alive (r3 s2.6). Total GPU per active effect about 0.1-0.4 ms on M-class (r3 s8, not re-measured).

## 6. Build plan

| Phase | Content | Deps |
|---|---|---|
| P0 look, no motion | 2.2-2.9 and 2.9 home/new: felt/light/rail/plates/fans/cards/buttons/type, assets fixes (ace of spades, hero D, back art), layout fixes (bid overflow, header clip at 720, names) | none |
| P1 `[CORE]` motion | 3.1 tokens, 3.2 pace, 4.1-4.8, 4.9 (core), 4.10-4.15 | none |
| P2 `[DRV]` | flip + optional arc (`rune-motion` dep, `workspace.dependencies` same git+rev, r2 s1) | rune-motion |
| P3 `[FX]` | glow, sparkles, ripple, grain via one `Compositor` (r3 s2) | rune-fx, wgpu 30 |
New scenes needed in `scenes.rs` (r6: no scene captures the deal): `table-deal-in`, `table-trump-called`, `table-hand-scored` (point/euchre/march), `table-game-won`, `table-game-lost`, `table-loading`. Scripts for `--strip` per moment (`--script ... --strip --start --frames --every`, cookbook 14).

Test plan (extends cookbook 13/14, r1 s14): (1) per moment a `rune::Runtime` test: `rt.frame(0.0)`, `rt.update(..)`, step 1/120 s; assert `rt.presented(key)` rect/opacity at t, `moving` toggles, and `!rt.frame(..)` idleness after settle (`runtime/elements.rs:402`, `tests/enter_delay.rs:68-72`); (2) every moment again with `rt.set_system_reduced_motion(true)` asserting no flight and final pose within 0.2 s; (3) `motion_gate` cells (r5 s2.7) with `MotionIntent` marks instead of allowlists; (4) pace tests: feed 8 syncs at 100 ms gaps, assert final state equals no-animation state and `busy` drains; (5) reconnect test: `steps: []` snapshot mid-trick -> no flight, no overlay.

## 7. Spikes before building (prove, then commit; headless unless noted)

| # | Question | How | Blocks |
|---|---|---|---|
| S1 | Does elevation (`scale 1.10`, plate shadow) set on a just-mounted matched card ride the flight, and does a second commit release it smoothly? Is it hidden by `rest_quiet`? | `rt.presented_pose(key)` (`elements.rs:493`) at 0/0.1/0.28/0.35 s, `--strip` | 4.4, 4.12 |
| S2 | `Sequence::delay` holds the first pose during the delay; `enter_delay` x `matched` (r1 s5 unverified item): does the delay hold the flight start? | r1 s14 recipe | 4.4, 4.6, 4.9 |
| S3 | `Shadow{inset:true}` renders; +x offset shadows look right; shadow count/order stable | snapshot | 2.3 |
| S4 | Route-level `matched` between two whole-screen roots; `matched_crop` for a lime -> green button | `--script` navigate + strip | 4.1 |
| S5 | Exit uses the transition the old tree carried; `tx.color_motion` on a root fill during a route swap | trace + strip | 4.1 |
| S6 | Hover lift vs hit rect: use outer fixed el; `hover` + `on_press` + `focusable` interplay; neighbour parting | tree + press script | 4.13 |
| S7 | `RollingText` in header and the result card (key stability, widths) | `rt` test | 4.7 |
| S8 | `Pixels` grain buffer creation and stretch (cost, banding), tiling availability | snapshot at `--scale 1`/`2` | 2.3 |
| S9 | `Cx::time_scale` retimes running springs; else per-commit `tx.motion` scaling | headless frames with scale changed mid-flight | 3.2.4 |
| S10 | rune-fx compositor on the pin inside the app; prewarm | build + timing | P3 |
| S11 | Negative `pin` insets for the bottomless rail; `clip_rounded` + lamp pool bleed | snapshot | 2.4 |
| S12 | Nested `LoopSpec::slide` x+y parabola; removal after one period | strip | 4.14 |
| S13 | `motion_gate` accepts marked overshoots/shakes | `cargo test -p euchre-desktop motion_gate` | all |
| S14 | `Tx::unclipped_flights` needed for rack -> trick/stack (is anything clipping?) | strip at 720x520 | 4.4-4.12 |

## 8. Spec deviations and decisions to sign off

Visual-only deviations from 06 (copy, a11y, tempo unchanged):
1. 5.1/16.1 rail: U-shaped bumper open at the bottom, felt continuous under rack (was a closed rail around the letterbox).
2. 5.4.2 seat plates + 30x42 fans (was text islands, 14x20 backs); active-seat treatment stronger than `#ffffff0a`.
3. 5.4.4 Case B deck tile: real burgundy back stack with brass club (was a green striped placeholder).
4. 5.6 rack captions visible on hover/selection/focus only (a11y labels always present); rack art 112-140 pt (was <= 150 / 98 on the audit build); **Play button hidden until a legal card is selected** (still "disabled without a legal selection": same rule).
5. 5.3 header buttons ghost-styled; leader underline added.
6. 5.8 suit buttons carry an enamel suit disc; primary key physics; Pass demoted.
7. Hand-result card, end panel, trump wash, winner glow: new visual elements that reuse existing strings only (06 s10 result sentences, s9 banner words, s11 tricks line, s12 match lines). Needs owner OK that the SR result sentences may be shown on screen.
Decisions owed: (a) call pills (4.10) are optional and contradict 06 s8; (b) animation-speed setting (settings example, r4) would add a label to Settings: not in the spec; (c) `[DRV]` dependency (flip); (d) `[FX]` dependency; (e) re-export the ace of spades without the QR; re-export `euchre-hero.webp` without the baked "D"; (f) rail U-shape needs the extra 40 pt overflow row if S11 fails.
