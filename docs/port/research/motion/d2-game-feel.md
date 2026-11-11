# D2 - Game feel and clarity (motion + visual proposal)

Angle: every game fact readable at a glance, pacing that never makes the player wait, nothing that outlives a reconnect sync.
Inputs: r1-r6 in this folder, `docs/port/06-ui-spec.md`, `02-protocol.md` (1.4, 9, 10, 11), `03-table-actor.md` 9, and the current `apps/desktop` (read-only; another agent is editing it, nothing here was applied).

## 0. Citation rules

- P = pinned rev `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb`, root `/Users/brittianwarner/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/`. H = HEAD `0ea75abe`. Every motion API used below is byte-identical at H (r1 line 13), so cites are P only. Nothing here needs a pin bump.
- Cite form: `path:line (P)`, path relative to the crates root. Lines marked `~` are the research notes' own approximations (apply.rs drifts a few lines).
- `[rN §x]` points at the research note where a recipe was compiled or run. VERIFIED = compiled/run in r2/r3 scratch crates. SOURCE = read only. Section 11 lists everything not verified.
- App paths are relative to `/Users/brittianwarner/goods/euchre-rune/apps/desktop/src`.

## 1. Principles (the rules everything below obeys)

1. State first, motion second. Input, turn state and legality come from the view and never wait on an animation. A flight is cosmetic: the destination element exists and is correct from the commit; the flight only decorates the move.
2. The server tempo is the budget. Choreography for a trick fits inside the 3300 ms hold (3800 ms when `sealsEuchre`), hand score inside 2500 ms, AI actions inside their think floor (cut 700, bid 1900, discard 1100, play 1000, plus 0-399 ms jitter). If a beat does not fit it is cut, never the clock stretched.
3. Derive from the view, stamp from the clock. Steps (cardPlayed, trickWon, handScored...) start cosmetics. What is shown after a landing (winner ring, collect, next lead) comes from the view diff plus a `landed_at` the Presentation stamps. `trick_resolve -> trick_play` emits no step, so the client already must derive it from view diffs (02-protocol 10).
4. Backlog compresses (section 2). Resync snaps (`steps: []` -> `tx.instant()`, existing at `app.rs` sync handler ~815-930).
5. One accent. Gold (`#f4d27a` light, `#c9a24a` deep) carries every "this matters now" signal; suit red is only for suits. Blue is not used. Never two accents on one surface.
6. Information is never carried by motion alone. Every cue below has a static end state (a ring, a chip, a glyph) that is what Reduce Motion shows.
7. Copy strings are untouched: 06-ui-spec 5.4.5, 5.7, 9, 12 stay verbatim. New elements are numerals, glyphs and suit marks only, plus text that already exists.

## 2. Tempo model: Full, Brisk, Skim, Snap

Presentation owns one `Feel { tier, k, fx, arcs }` (`present.rs` `Presentation`). Stage/hand builders read it instead of consts, so `Spring::new(timing::X * feel.k, bounce)` (const fn at `rune-motion/src/spring.rs:30` (P); clamp duration >= 0.08 or it panics).

| Tier | Entered when | k | Arcs (poll-driven) | rune-fx / sparkles | Stagger | Hold before collect |
|---|---|---|---|---|---|---|
| Full | queue depth <= 1, normal tempo | 1.0 | yes (max 2 live) | yes | table values | 2200 ms (read 1500 after landing) |
| Brisk | depth 2-3, or fast tempo (120/180/250 ms) | 0.6 | no, straight matched | no | 45 -> 20 ms | 1200 ms |
| Skim | depth >= 4, or presentation is > 1500 ms behind the newest step | 0.3 | no | no | 0 | none: collect when the view shows the next trick |
| Snap | `sync.steps.is_empty()` (connect, resync, review) | 0 | no | no | 0 | none |

- Depth = steps received but not yet started by `Presentation::on_step`. Dedupe stays by hash over the 80-entry window (existing).
- Snap = `tx.instant()` (existing, `app.rs` ~815-930). Also: set every owned `Animated` to its end value, clear `landed_at`, clear pending `wake_at`, reset `cx.time_scale(1.0)`. Rune keeps already-running flights through an instant commit (r1 line 25), so also drop the old matched ids by key change in the same commit.
- Human input during any cosmetic beat: bump to Brisk for the rest of that beat via `Cx::time_scale(2.5)` (window clock, clamped [0, 8], `rune/src/runtime/cx.rs:266` (P)). `Tween::linear` ignores it (r1 §9), which is fine: no Tween is on a gameplay path.
- Whole-subtree speed: `El::time_scale(k)` (`rune/src/el/visuals.rs:419` (P)) on the felt root is the alternative if the window clock must stay 1.0 for menus. Scroll, growth and Tween ignore it.
- Latency floor: every cosmetic delay is a `Tx::wake_after`/`wake_at` (`rune/src/el/tx.rs:226`, `:242` (P)) from `Presentation::poll`, so an idle table requests no frames. Test with `assert!(!rt.frame(..))` (r1 §12).

## 3. Shared motion tokens (new, in `theme/felt.rs` `timing`)

Existing tokens stay; these are additions. `Spring::new(duration, bounce)`; settle takes 1.2-1.5x duration (r2 line 56); visible bounce needs bounce >= 0.3 on a 0..1 progress, but on a 250 px travel bounce 0.25 gives about 2.8% = 7 px overshoot, which reads (r2 line 55).

| Token | Value | Use |
|---|---|---|
| `SLAM` | `Spring::new(0.38, 0.25)` | any card landing in the trick; `Tx::motion` or arriving `.motion` (spring order `Tx::motion` > `El::motion` > REFLOW, `rune/src/runtime/motion/flights.rs:190` (P)) |
| `THUD` | `Curve::tween(0.18, Easing::OVERSHOOT)` | impact scale on the landed card via `scale_motion` (`rune/src/el/visuals.rs:369` (P); `Easing::OVERSHOOT` = cubic(0.34,1.45,0.64,1.0), `rune-motion/src/curve.rs` Easing consts, r1 line 86) |
| `LIFT2` | `Spring::new(0.22, 0.25)` | winner lift, legal-card lift (`offset_motion`, `rune/src/el/motion.rs:122` (P)) |
| `PLATE` | `Spring::new(0.3, 0.0)` (= SETTLE) | shadow plate depth, seat glow (shadows animate on `.motion`, not `.offset_motion`: r3 line 21) |
| `FADE_SNAP` | `Spring::new(0.25, 0.0)` (= SNAP, `rune-motion/src/tokens.rs:12-36` (P)) | dimming, ring swaps (`opacity_motion`, `rune/src/el/motion.rs:153` (P)) |
| `FLIP` | tween 0.32 s `Easing::EASE_IN_OUT`, bounce 0 | up-card turn-down (section 5.5) |
| `STAMP` | Beat list, section 5.9 | EUCHRE / MARCH / ALONE stamps |

## 4. Visual tokens (what the table looks like)

The audit snapshots (r6 section 0) show a flat felt, an invisible 4% seat tint, and a banner that sits low on game over. Changes, all core Rune (no new dependency), each with a static end state.

### 4.1 Felt, vignette, rail
- Felt keeps `#17452f` base, radial `#245a49` centre to `#10382c` edge (06-ui-spec 5).
- Vignette: a separate non-interactive overlay El above the felt, `radial_gradient` (`rune/src/el/visuals.rs:164` (P); arg order to confirm at implementation) from transparent at 55% radius to `#000000` alpha 0.30 at the corner. One gradient slot per element, so the felt and the vignette are two Els.
- Rail: a 2 px `border_gradient` (`rune/src/el/visuals.rs:105` (P)) `#3a7a62` top to `#0c2a21` bottom on the felt rim, radius 16. It frames the table without adding a second accent.
- Optional (gated, section 10): `Fx::grain` static felt grain at alpha 0.05 via the rune-fx compositor (r3 section 2). Without it the felt is just cleaner, not grainier. Skip if the compositor is not adopted.

### 4.2 Shadow plates (key light from above, 2 shadows always)
Use the plate child pattern from r3 lines 29-40: the card art is the inner element, a sibling `plate` El with the same rect and radius carries the two shadows. The flight spring (`.motion(SLAM)`) sits on the OUTER element and `PLATE`/`LIFT2` on the plate and inner card. Shadow count and order never change; only numbers do, or the change lands instantly.

| State | contact (constant) | soft (animated) |
|---|---|---|
| Rack rest | y 1, blur 0, `#000` a0.35 | y 3, blur 8, a0.28 |
| Hover / legal lift | same | y 10, blur 18, a0.30 |
| Selected | same | y 12, blur 20, a0.34 (keeps `outline(3,3,focus)`) |
| In flight peak (arc only) | same | y 18, blur 32, a0.22 |
| Landing (first 120 ms) | same | y 2, blur 6, a0.42 |
| Played, resting in trick | same | y 6, blur 14, a0.32 |
| Winner | same | y 10, blur 20, a0.36 plus gold glow (4.3) |

Shadow lanes: only the first four shadows animate (`rune/src/runtime/motion/props.rs:42,52,67` via r3 line 19), so keep each plate at 2 plus at most 2 glow shadows. `Shadow { x, y, blur, spread, color, inset }` is `rune/src/el/visuals.rs:569-576` (P); `blur` is CSS blur (2 sigma). Existing `shadow::{PLAYED, UP, SELECTED, STACK, DEALER, BANNER}` keep their names and take these numbers.

### 4.3 Seat plates and the turn glow (answers "whose turn")
Today the active seat is a 4% white tint (`SEAT_TINT #ffffff0a`) that cannot be seen at 1x. Replace with a plate El per seat, radius 14 (same as banner):
- Rest: fill `#0c2a21` alpha 0.55, `border_gradient` 1 px white a0.14 to a0.04, glow shadow alpha 0 (the glow slot exists, so the count is constant).
- Active: fill `#c9a24a` alpha 0.10 over the dark, `border_gradient` 2 px `#f4d27a` to `#c9a24a`, glow shadow `0 0 24px #f4d27a a0.35`. Transition on `.motion(PLATE)`: 300 ms. Name text colour goes to `#fff8df`.
- Breathe: a child glow El with `looping(LoopSpec::opacity(1.6, 0.0, 0.45, 1.0))` (`rune-motion/src/looping.rs:33` (P), r1 line 289). Idle floor: a looping opacity keeps requesting frames, so only the active seat loops and only while the table is awaiting that seat. The existing thinking dot (`stage.rs` `LoopSpec::opacity(1.2, 0.0, 0.25, 1.0)`) stays and sits inside the active plate.
- You (human): same plate, plus the rack's top edge gets a 3 px gold bar that fades in (`opacity_motion(FADE_SNAP)`). This is the single strongest "your turn" cue because it is where the eye already is.
- Static end state (Reduce Motion): gold border and filled plate, no breathe.

### 4.4 Cards and backs
- One back art at every size (rack never shows it; seat fans 14x20, deck, up card down, collect minis 24x34). Today the minis, fans and deck are drawn separately, which is why they look unrelated. Same asset, scaled.
- Stack glyph: each seat's won-tricks stack shows a round 18 px plate `#c9a24a`, numeral 12 px bold `#17452f`, at the stack's top-right, with a `Transition::scale(0.9)` + opacity on the numeral (`rune-motion/src/transition.rs:202` (P)) keyed by count so it ticks. This is the "trick count at a glance" for the human who today must count minis.
- Card faces keep the 06-ui-spec sizing; do not change `size::*`. Rack gets lift and scale only: hover `scale_motion` 1.04, selected 1.06.

### 4.5 Buttons
Primary button = Layerr-independent; euchre palette: fill `#c9a24a`, text `#17452f` (serif 600), radius 12, contact shadow `0 3px 0 #8a6d2a` plus soft `0 8px 16px #000 a0.25`. Hover: `.offset(0,-2)` + `offset_motion(LIFT)`, matching the web's `translateY(-2px)` (r6 section 1). Press: `press_scale(0.97)` (`rune/src/el/visuals.rs:350` (P)), contact shadow y 3 -> 1 on press. Disabled: opacity 0.45, no shadow change.

## 5. Gameplay choreography

Notation: `T` is the instant the step reaches the client. `L` is `landed_at`, stamped at T + flight time. Z-order: use `El::paint_order` (r1 line 25) with these bands: felt 0, plates 10, trick cards 20 (+1 per play order), fx rings 30, banner 40, matched flights draw above the window by construction (r1 section 6), toasts 50.

### 5.1 Cut and deal (steps `cut`, `dealt`, `upCardTurned`)

Cut (AI floor 700 ms): the deck tile hops: `offset(0,-10)` then back 120 ms later (two commits, `wake_after`), `offset_motion(Spring::new(0.16, 0.2))`. Total 280 ms. Soft shadow follows via the plate.

Deal (`dealt`, 8 redacted packets): each fan/rack mounts with a `Transition::offset(dx,dy).combined(Transition::opacity())` where (dx,dy) points from the seat toward the dealer's deck (computed from `cx.window_layout("deck")` and the seat rect, `rune/src/runtime/layout_read.rs:281` (P)). `.springs(SpringPair{enter: Spring::new(0.36, 0.12)})` and `.enter_delay(Stagger::new(0.075).delay(packet, 8))` plus 30 ms per card inside a packet (`rune/src/el/enter_delay.rs:36`, `rune-motion/src/choreo.rs:35` (P)). `kit_rules` clamps scale to [0.9, 1.1], so no scale growth: the cue is travel + fade (`rune-motion/src/transition.rs:148-157` (P)). Packet sizes (2 or 3) come from the step payload; verify the field name in 02-protocol 1.4 before use.
- Total at Full: 8 x 75 + 360 x 1.3 = about 1070 ms. Brisk (k 0.6, gap 20): about 420 ms. Skim/Snap: mounted.
- Your five keep the existing deal-in (`hand.rs` offset(0,-24), scale 0.92, 75 ms stagger), but only when `deal` is set (never first view, never discard); keep that gate.
- Each landed card plate shadow goes from "flight peak" to "rest" automatically because the state change is one commit.
- Reduce Motion: `enter_delay` is kept (becomes a fade at the same timing, r1 line 191), so shorten the Stagger gap to 20 ms under reduced motion; fade 0.15-0.18 s (`rune-motion/src/tokens.rs:48` (P)).

Up card turn (`upCardTurned`, AI floor 1900 for the first bid): the deck's top card flips face-up with the width-squeeze (r2 section 7.2, VERIFIED compile): back at `scale-x` 1 -> 0 over 160 ms, swap the face at p = 0.5, face 0 -> 1 over 160 ms, lifted 6 px at the midpoint, `FLIP` tween, bounce 0 (a bouncy `p` would swap twice). Poll-driven `Animated<f32>` with `.with_class(PropClass::Opacity)` or the 1e-4 settle keeps waking frames 0.2 s too long (r2 lines 67-68). Needs `rune-motion` as a direct dep at the same rev (r2 line 26). Plus an opacity-only fallback for Brisk: a `Transition::blur_replace()` (`rune-motion/src/transition.rs:231` (P)) swap of back for face, 150 ms.
- Landing: the up card settles at the table centre with the existing `UP` shadow, then a 1 px gold ring keyed `upcard-ring-{hand}` runs the ring Sequence (5.9) once.
- Reduce Motion: face-up at once, ring static for 600 ms then removed.

### 5.2 Whose turn (all phases)

Section 4.3 is the whole cue. Timing: when the actor changes, the old plate glow fades 300 ms (PLATE) while the new one rises 300 ms (cross-fade, no gap). AI `thinking` events set the active seat before the action arrives, so the glow can precede the play by up to 1 s; this is the intended "they are about to move" read. Do not animate between seats with a travelling token: it would need a path and blocks nothing but reads as lag.

### 5.3 Bid and pass (steps `bid`, `turnedDown`, `trumpSet`)

- Bubble per seat: a small pill anchored to the seat plate. `Pass`: neutral `#0c2a21` a0.8, text `#fff8df` a0.8, enter `Transition::offset(0, 8).combined(Transition::opacity())` 200 ms (SETTLE), dims to opacity 0.6 after the next seat acts (`opacity_motion(FADE_SNAP)`). Order up / Call: gold fill `#f4d27a`, text `#17452f`, Sequence pop `0.7 -> 1.15 -> 1.0` over 240 ms (Beat list 5.9) and a ring.
- Order up: the up card flies to the dealer (existing `matched("pickup-{hand}")`, `PICKUP`). Tighten `PICKUP` to `Spring::new(0.55, 0.15)` (it is 0.85 s today and ahead of nothing) and add `tx.unclipped_flights()` if the fan sits in a clipped container (`rune/src/flight.rs:36` (P)). The dealer's fan gets one extra back with the same enter transition; the discard step (`dealerDiscarded`) removes one with an exit fade, so the fan count stays visibly at 5 after.
- Turned down (round 1 all pass): up card flips back to face-down (5.1 FLIP reversed, 240 ms) then settles into the deck (existing matched id handoff). Bubbles from round 1 clear with a 150 ms fade as round 2 begins so the new round reads clean.
- Bid buttons: the legal set does not change layout. Disabled = opacity 0.45 (`opacity_motion(FADE_SNAP)`); the focused default gets the gold primary look (4.5). The audit shows bid copy overlapping the "You" box in round 2; give the bid area its own 16 px gap inside the seat plate geometry (06-ui-spec 5.4.2).
- Reduce Motion: bubbles and rings appear with a 150 ms fade, no pop; the up-card flight becomes a crossfade (matched is disabled under reduced motion, r1 line 377).

### 5.4 Trump named (step `trumpSet`) - and where trump lives afterwards

- Trump badge in the header: a 40 px round plate (`#17452f`, 2 px gold `border_gradient`) with the suit glyph. On `trumpSet` the badge element is keyed by suit, so it replays: Sequence `scale 0.6 -> 1.3 -> 1.0` with `Easing::OVERSHOOT` over 360 ms and a ring ping (5.9). The ring is 1.0 -> 1.5 scale, a0.6 -> 0, 400 ms.
- Maker chip: a 20 px gold chip with a small "M" glyph already in the spec on the maker's seat; make it enter with `Transition::scale(0.9).combined(opacity)`.
- Trump cards in the rack get a 5 px gold pip in the top-left corner (static). This costs no motion and is the highest-value "trump" cue once the hand is under way.
- Felt tint: none. Tinting the felt by suit was considered and rejected: it is a second accent (principle 5).
- Reduce Motion: static badge swap by fade; no ring.

### 5.5 Led suit and legal cards

- Led suit: under the lead card, a pill with the suit glyph, gold `#f4d27a` a0.9, enter `Transition::offset(0,6).combined(opacity)` 180 ms, exit opacity 120 ms. It lives while the trick is open.
- Your turn, a suit led: legal cards lift 6 px (`LIFT2`) and get `outline(2, 2, #f4d27a)` (`rune/src/el/visuals.rs:231` (P)); illegal cards dim to opacity 0.55 on `FADE_SNAP`. If you lead, nothing dims. If it is not your turn, the rack is at rest (no dimming), so the glow means "you can act".
- Hover keeps its own `-4/-8` lift on top; lift is additive in the offset, not a competing spring.

### 5.6 Card play and the live winner (step `cardPlayed`)

Opponent card (arrives from a seat fan; today a 14x20 back grows to a 125x163 face along a straight line with the face shown throughout; cookbook 15 #17):
- Keep `matched` (straight flight is free and correct under backlog). Change the spring: `.motion(SLAM)` on the arriving trick card, flight 380 ms at Full, 230 ms at Brisk, `SNAP` at Skim. Rotation does not carry through a matched flight (r1 line 473, unverified), so after landing apply a 4 degree tilt (`rotate`) by seat direction on a second commit with `rotate_motion(Spring::new(0.4, 0.2))` (rest_quiet snaps first-mount rotate: `rune/src/runtime/commit/apply.rs:~527` (P)); one `wake_after(ZERO)` between. UNVERIFIED, P3.
- The sibling plate shadow follows 4.2: flight peak while moving, landing, then played.

Your card (the one the human taps; the largest moment, so it gets an arc):
- Full tier only: poll-driven arc (r2 section 7.1, VERIFIED compile and samples): progress `p` as `Animated<f32>` `Spring::new(0.45, 0.25)` `.with_class(PropClass::Opacity)`; position = lerp(hand slot, trick slot, p) + perpendicular lift `sin(pi * clamp(p,0,1)) * 48` pt (toward the felt centre-up); rotation eases from the held tilt to the seat tilt with p. Endpoints from `cx.window_layout("hand-{card}")`, `cx.window_layout("trick-you")` (`layout_read.rs:281` (P)). Hide the real destination card (opacity 0) until `p` settles, so there is no duplicate (the destination owns the final matched shadow).
- The poll uses `tx.wake_after(Duration::ZERO)` while moving and returns `false` otherwise, so the window idles (r2 line 121).
- Max two arcs alive. At Brisk and below use `matched` + `SLAM`, which already lands correctly.
- Reduce Motion (poll arcs are NOT reduced for you, r2 line 109): branch on `cx.is_reduced_motion()` and `set(1.0)`.
- Input: the rack's played slot closes the instant the tap commits (existing `RackSlots`, no slide to centre). The next phase's controls do not wait for the flight.

Landing impact at `L` (all tiers except Snap):
1. `scale_motion(THUD)` on the card: 1.0 -> 1.06 -> 1.0 (the overshoot easing exceeds 1.0).
2. Ring ping keyed `land-{hand}-{trick}-{seat}`: a 1 px white a0.5 ring, scale 1.0 -> 1.35, a0.5 -> 0, 280 ms (5.9 Beat list RING).
3. Plate shadow goes Landing -> Played (4.2).
4. Z: the card takes paint-order band 20 + play order; later plays always cover earlier ones.

Live "currently winning" glow (answers "who is winning"): after each play the current trick leader gets `outline(2, 2, #f4d27a)` and a glow shadow `0 0 16 #f4d27a a0.4` on its plate. Key the outline Els per card and animate `opacity_motion(FADE_SNAP)` so the gold hands over from card to card in 250 ms when a higher card lands. Needs the current winner: use the view's if it carries one, else derive it with the same trick-winner logic already used for `trickWon` (check; flagged in section 11). Not shown when there is one card.

Illegal attempt (existing 3.2 s message `ILLEGAL` stays): the card shakes in its rack slot and flashes red-amber.
- Shake: offset +/- 6, +/- 4, +/- 2, 0 across four commits, 45 ms apart (`wake_after(45ms)` chain, `offset_motion(Spring::new(0.08, 0.3))`). Total 180 ms.
- Flash: outline `#e0653a` 2 px for 400 ms via `opacity_motion`.
- Sound/haptics: none (not Rune's concern here).
- Reduce Motion: skip the shake; keep the outline flash for 1.2 s (information).

### 5.7 Trick won (view diff plus `trickWon`) and collect

Timeline at Full, T = the fourth `cardPlayed`, L = T + 380 ms (all inside the 3300 ms hold):

| t | Event | Spring / curve |
|---|---|---|
| T + 0..380 | fourth card flight | SLAM 0.38 / 0.25 |
| L | impact: THUD, ring, shadow compress | 180 ms, 280 ms |
| L + 60..300 | winner reveal: winner card `offset(0,-8)` `LIFT2`, gold outline + glow, the other three cards to opacity 0.8 | 220 ms |
| L + 60 | trick result line swap | `Transition::blur_replace()` 180 ms (today a hard text swap; r6 section 1) |
| L + 360..2200 | read window (>= 1500 ms: TABLE_MOTION read) | no motion; the glow does not breathe |
| T + 2200 | collect starts; turn glow moves to the winner (next leader) | |
| T + 2200 + 45 i | card i flies to the winner's stack mini, 3 stagger steps after the first | `Spring::new(0.66, 0.0)` (was 0.78, trimmed for margin) |
| about T + 3100 | last mini lands; stack count ticks | within 3300 |

- Winner ring after landing, not with the fourth card: today the ring appears with the 4th card while it is still in the rack (r6 section 1). Gate it on `landed_at`. `Presentation::poll` already schedules via `tx.wake_at`; add `landed_at = now + 0.38 * k`.
- Collect: keep the existing per-card `matched(play_id)` and 45 ms stagger in `Presentation::poll` (`present.rs` ~395-415) and `tx.unclipped_flights()` (the stack minis are clipped by seat plates).
- Euchre seal (`sealsEuchre`, hold 3800 ms): stamp (5.9) at L + 80; collect starts at T + 2700 (not 2200), last mini lands about T + 3600, inside 3800. Worked arithmetic: 2700 + 3 x 45 + 660 x 1.2 = 3627.
- Brisk: reveal 130 ms, read 600 ms floor, collect at T + 1200, stagger 20, spring 0.4. Skim: collect when the view shows trick N+1, `SNAP`.
- The matched flights are disabled under Reduce Motion and become a crossfade at 0.15-0.18 s (`rune-motion/src/tokens.rs:48` (P); r2 line 109); the landing card, winner outline and stack count tick are static states and still show. Collect under reduced motion: cards fade out of the trick and the stack numeral changes; no wait beyond the read window.

### 5.8 Loner (maker goes alone) - dramatic but quiet

Trigger: `bid` with alone, then `trumpSet`. All cues are held for the entire hand.
- Partner of the lone maker: seat plate `layer_opacity(0.35)` (`rune/src/el/visuals.rs:283` (P)) and its fan fades out (`Transition::opacity`, 300 ms). A sitting-out glyph (existing copy only) sits on the plate.
- Maker seat: permanent glow plate (4.3 active style) but slower breathe `LoopSpec::opacity(2.4, 0.0, 0.5, 1.0)` and a 3 px gold border; the stack of tricks keeps normal visuals.
- Stamp "ALONE" (5.9) at the top of the felt, 420 ms, holds 1500 ms, then settles into a 28 px chip next to the trump badge (a 300 ms `FADE_SNAP`). The stamp text uses the existing banner string (06-ui-spec 9); no new copy.
- Vignette to alpha 0.40 over 300 ms (overlay fill, `Transition::opacity`) and back at handScored: a focusing dim of the room.
- Reduce Motion: no breathe, no vignette change; the border, dimmed partner and chip carry it.
- Loner march (4 points): see 5.9 aura option.

### 5.9 Stamps and the one-shot toolkit

All one-shot cues use a keyed element with a static `Sequence` (`Beat` data must be `&'static`, delay is runtime: `rune-motion/src/sequence.rs:14-17` (P); a new key replays it, r1 line 313).

```
STAMP: Beat::new(0.00).opacity(0.0).scale(1.6)
       Beat::new(0.14).opacity(1.0).scale(0.94).easing(Easing::OVERSHOOT)
       Beat::new(0.22).scale(1.0)
       Beat::new(1.60).opacity(1.0)
       Beat::new(1.90).opacity(0.0)
RING:  Beat::new(0.00).opacity(0.5).scale(1.0)
       Beat::new(0.28).opacity(0.0).scale(1.35)
POP:   Beat::new(0.00).opacity(0.0).scale(0.7)
       Beat::new(0.12).opacity(1.0).scale(1.15)
       Beat::new(0.24).scale(1.0)
```

- Euchre ("euchred"): stamp EUCHRE, ink `#fff8df` on `#a2321f` plate (radius 14, 2 px gold border), at the felt centre above the trick; ring ping on the maker's seat plate in red-amber `#e0653a` (a single warm hit, replaces gold for failure, principle 5 exception: failure state); a felt shake: the trick group offsets +/- 5, 4, 2, 0 over four 40 ms commits (`offset_motion(Spring::new(0.08, 0.3))`). Defenders' glow plate takes the 4.3 active style for 1.2 s. Brisk: no shake. Skim: stamp only, 600 ms.
- March (all five by the maker side): stamp MARCH, gold plate `#f4d27a`, ink `#17452f`; sparkle ring around the stamp (rune-fx `Fx::sparkles`, count 24, margin 40; core fallback: 12 `canvas` dots, 0.6 s) and the existing confetti canvas for 1.2 s (`CONFETTI` 2.4 s -> 1.2 s so it ends inside the score hold). Loner march: same plus `GlowStyle::Aura` with a single gold hue (a multi-hue aura breaks principle 5) if fx is adopted; core fallback is a 28 px gold glow shadow animated 0.2 -> 0.6 -> 0.2 over 1.2 s.
- Sparkles are STAR sprites on a shape outline, max 64, staggered, no gravity (r3 line 128); use a small circle shape + large margin for a ring burst. They are not velocity-based, so do not promise "explosions".
- Reduce Motion: no stamp slam, no shake, no sparkles/confetti; the plate fades in at 0.18 s and holds 1500 ms; ring is static.

### 5.10 Hand score (step `handScored`, hold 2500 ms)

The audit snapshot (`table-hand-scored.png`) is an empty felt with a mislabelled heading. Rebuild as a centre panel (radius 14, `#0c2a21` a0.85, 1 px white a0.14) over the cleared trick area:
- Row 1: five trick pips, 14 px, filled in the winner's colour (gold = us, `#fff8df` outline = them), each entering with POP and `enter_delay(Stagger::new(0.06).delay(i, 5))`: 5 x 60 + 240 = 540 ms.
- Row 2: delta "+N" in 28 px serif gold. After 400 ms it launches to the header score numeral with `Tx::launch("hand-delta", "score-us")` (text-sourced flight, `rune/src/flight.rs:28` (P); r1 section 6: launch and `fly_text` are text-sourced only). UNVERIFIED that the destination can be a `RollingText`; fall back to a keyed opacity.
- Header score: `RollingText::new("score-us", ..).spring(SETTLE).value(n)` (`rune-kit/src/rolling_text.rs:93,110,131,145` (P); odometer is the one built-in number tick). If the kit-level `RollingText` is not wanted, keep the existing keyed numeral with `Transition::offset(0,10)` + opacity (`header.rs:111-126`).
- Score pips (`ScoreCards`): pip opacity 0.08 -> 1 today; add `Transition::scale(0.9)` per newly lit pip and a one-shot RING in gold on the newest pip.
- Timeline: panel in 0-200, pips 200-740, delta 740-1140, launch 1140-1600, hold to 2300, panel out 2300-2480, next deal starts. If the game is won the panel is replaced by 5.11 at the same timings.
- Brisk: panel appears with all pips lit, delta launches at 300 ms, out at 1200. Skim/Snap: header only.
- Reduce Motion: flights disabled so the delta fades out and the header numeral changes; keep the panel static for 1500 ms.

### 5.11 Game won (step `gameWon`)

- Scrim: `el().fill(BLACK.alpha(0.45))` with `Transition::opacity()` 250 ms (r3 line 66). Result panel centre with `backdrop_blur(8.0)` only on the panel (`rune/src/el/backdrop.rs:34`, max 4 per frame `rune/src/render/backdrop_blur.rs:10` (P)); the panel enters `Transition::scale(0.92).combined(opacity)` `Spring::new(0.45, 0.2)`.
- Banner: today it sits low on game over (r6 section 1); the spec's `top 104`, `#fff8df` fill, radius 14, 24 px serif stays, but on game over it is a header of the panel, not a floating element.
- Final score: two `RollingText` numerals counting from the last hand score, 500 ms.
- Winner side: gold glow plate on its seats; confetti canvas (existing 24 pieces) with a true fall: `LoopSpec::slide` plus per-piece rotation (`LoopSpec::rotation` conflicts with slide: only one loop kind per element, r1 line 285-291). Use 24 pieces for the slide and put a second El per piece with `LoopSpec::rotation(period)`: UNVERIFIED (`LoopKind::Rotation` on non-icon elements). Without it keep the fixed tilt.
- Optional rune-fx: `Fx::sparkles` count 48 along the felt edge and `GlowStyle::Aura` on the winner panel, tied to `sp.burst_life()` (about 2-3 s), then `animating() == false` (r3 lines 135-147).
- Reduce Motion: no confetti; static gold rim glow and the banner.

### 5.12 Screens and chrome

- Route changes (Home -> New -> Table are hard cuts today, `app.rs:324-337`): the outgoing screen is replaced by `Transition::asymmetric(in, out)` with in = `offset(0,12).combined(opacity)` at SETTLE 0.3, out = opacity only at 0.12 (`rune-motion/src/transition.rs:250`, `:254` (P)). Table mounts with header at 0 ms, seats at 40/80/120/160 ms, rack at 160 ms via `enter_delay`.
- Dialogs/rails keep the existing `offset(0,12)`/`offset(RAIL_W,0)`; add the 0.45 scrim.
- Reduce Motion: fades only (0.15 s), no stagger.

## 6. Reconnect and resync (the safety case)

- A `sync` with `steps: []` -> `tx.instant()` and the Presentation clears: `landed_at`, owned `Animated` set to end, `Feel` = Snap, pending timers dropped, fx slots dead, `cx.time_scale(1.0)`. Everything shown after a resync must be reproducible from the view alone: turn glow (actor), led pill (trick lead), live winner glow (trick cards), trump badge and pips (trump), loner dimming (alone flag), stack numerals (tricks won). These are all view-derived by design (principle 3). The one-shot cues (stamps, rings, pops) are the only step-derived items and are skipped.
- A resync mid-trick: the trick shows its cards at rest with the winning glow; mid-collect: the trick is gone and the stacks are right.
- Dedupe by step hash over 80 entries (existing) means a replayed batch does not re-fire a stamp. Key one-shot Els by hand/trick/seat so a replay after a crash would at worst replay one ring.
- Chat `call` lines precede their sync; do not use them to start cosmetics.
- Test: feed `steps: []` after a partial choreography and assert (a) no flight in flight (`motion_gate` `lands-unseen`), (b) idle (`!rt.frame`), (c) all of the view-derived cues present.

## 7. Never-delay rules (testable)

1. Input enabled the same commit the view allows it. No `enabled = !animating`.
2. Total cosmetic time per step <= the server floor for the next step at Full; <= 60% at Brisk.
3. No step queues behind a cosmetic: `Presentation::on_step` pushes, `poll` consumes, tier recomputed per poll.
4. A new human action cancels remaining non-essential cosmetics by time-scaling (section 2), not by cutting them mid-frame.
5. Total wakes: zero while the table awaits a human with no loop running (only the turn glow loops).

## 8. Reduce Motion matrix

| Cue | Normal | Reduce Motion |
|---|---|---|
| Card flights (matched) | SLAM / arc | crossfade 0.15-0.18 s (matched disabled, `tokens.rs:48` (P)) |
| Poll arcs | arc | `set(1.0)` (branch on `cx.is_reduced_motion()`) |
| Landing impact, ring, shake | THUD + ring | none; static outline |
| Winner reveal | lift + glow | static gold outline |
| Turn glow | gold plate + breathe | gold plate only |
| Stamps (EUCHRE / MARCH / ALONE) | slam 1.6 -> 1 | fade 0.18 s, holds 1500 ms |
| Deal | staggered travel | staggered fade, gap 20 ms (`enter_delay` kept) |
| Score numerals | RollingText / launch | numeral swap |
| Confetti / sparkles | on | off; static gold rim |
| Presence transitions | offset + opacity | fade 0.15 s (auto, `Transition::none()` excepted) |
| Looped glow | breathe | steady glow |

Reduce Motion never removes a highlight; it removes the way the highlight arrives (r3 line 114).

## 9. Rune API map

| Need | API (P) |
|---|---|
| Spring | `Spring::new` `rune-motion/src/spring.rs:30` |
| Tokens | `rune-motion/src/tokens.rs:12-36`, pairs `:66-73` |
| Per-property curve | `offset_motion` `rune/src/el/motion.rs:122`; `opacity_motion` `:153`; `scale_motion` `rune/src/el/visuals.rs:369`; `rotate_motion` `:389` |
| Presence | `transition` `rune/src/el/motion.rs:196`; constructors `rune-motion/src/transition.rs:186-275`; `kit_rules` `:148-157` |
| Stagger / delay | `enter_delay` `rune/src/el/enter_delay.rs:36`; `Stagger` `rune-motion/src/choreo.rs:35` |
| Matched flight | `.matched`; order `runtime/motion/flights.rs:190`; `Tx::unclipped_flights` `rune/src/flight.rs:36`; `Tx::launch` `:28` |
| Sequence | `Beat` `rune-motion/src/sequence.rs:14-17` |
| Loops | `LoopSpec` `rune-motion/src/looping.rs:20-50`; `Cycle` `cycle.rs:8` |
| Live / Tween | `Live` `rune/src/live.rs:142-193`; bindings `rune/src/el.rs:368-390` |
| Wake | `Tx::wake_after` `rune/src/el/tx.rs:226`; `wake_at` `:242` |
| Poll hook | `AppSpec::poll` `rune/src/tooling.rs:35` |
| Odometer | `RollingText` `rune-kit/src/rolling_text.rs:93,110,131,145` |
| Visuals | `box_shadow` `rune/src/el/visuals.rs:148`; `Shadow` `:569-576`; `border_gradient` `:105`; `radial_gradient` `:164`; `outline` `:231`; `layer_opacity` `:283`; `press_scale` `:350`; `time_scale` `:419` |
| Layout read | `cx.window_layout` `rune/src/runtime/layout_read.rs:281` |
| Reduce Motion | `Cx::reduced_motion` / `is_reduced_motion`; flights `tests` per r1 line 377 |
| Backdrop | `rune/src/el/backdrop.rs:34`; max 4 `rune/src/render/backdrop_blur.rs:10` |
| FX slots (optional) | `rune/src/el/effect.rs:43,52,69`; `Compositor` `rune/src/compose.rs:34,51`; `App::compositor` `rune/src/app.rs:539`; rune-fx `lib.rs:277,478`, `cpu.rs:247-249` |
| Motion audit gate | `rune::motion_gate` `rune/src/lib.rs:46` |

## 10. Dependencies and costs

- Core only (no new dep): everything in sections 3-8 except the poll arcs and flip. Risk: none.
- `rune-motion` as a direct dep at the same git rev (r2 line 26): arcs and flip. `Cargo.lock` already holds it at that rev. Without it, the straight `matched` + `SLAM` and the `blur_replace` swap are the fallback.
- rune-fx: optional, gated behind a `fx` cargo feature. Needs `wgpu 30` (same as rune), a custom `Compositor`, `animating()` true only while fx are alive, and first-use shader compile of 20-150 ms per kind (r3 line 92). Skip rune-light. Aura uses one hue.
- Per-frame cost: shadows are shader-analytic, one quad each; <= 12 plates is nothing. `backdrop_blur` <= 1 live (game over). Glow/sparkle costs are under 0.4 ms per 960k px covered (r3 line 112-130, from source).

## 11. Unverified, and tests that must pass before shipping each stage

Unverified (flagged where they appear):
1. `enter_delay` on a matched arrival (r1 line 196): matters for the staggered collect; the fallback is already in the app (`Presentation::poll` moves one card per 45 ms).
2. Scale/rotation carry through a matched flight (r1 line 473): the post-landing tilt (5.6) depends on the two-commit rotate working.
3. `LoopKind::Rotation` on a non-icon element (5.11 confetti).
4. `Tx::launch` to a `RollingText` destination (5.10).
5. rune-fx under the project's wgpu 30 + Metal native backend (r3 section 2 built, not run in-app).
6. The view carrying the current trick winner (5.6). If absent, derive it.
7. `radial_gradient` argument order (4.1) and that the overlay does not intercept hits.
8. Packet-size field in the `dealt` step (5.1).

Gate plan (`rune::motion_gate`, `lib.rs:46` (P); `scripts/motion-audit/README.md:66-116` H; r5 line 87):
- Cells: `deal`, `bid-round`, `trump-set`, `play-opp`, `play-you`, `trick-won`, `trick-won-euchre`, `march`, `hand-scored`, `game-won`, `illegal-shake`, `resync-mid-trick`.
- Fail on any of `pop-in pop-out cut-exit jump offset-jump appear-then-jump jerk overshoot jiggle fade-snap flicker never-settles wakes-after-end lands-unseen remount unpinned twice`. Mark authored motion with `El::motion_intent` (`BOUNCE KICK STEP CUT PULSE`) rather than allow-listing. The SLAM overshoot and the shake are authored BOUNCE/KICK.
- Headless virtual clock: `rt.frame(dt)` (`rune/src/runtime/frame.rs:305` (P)) with a scripted `steps` list; `assert!(!rt.frame(..))` proves idle after each beat. Run each cell with `rt.set_system_reduced_motion(true)` (`system_settings.rs:65` (P)) and assert flights = 0, no poll arc left, outlines present.
- Budget tests: replay the 02-protocol 11 sequence with the fast tempo (120/180/250) and a queue of 8 steps in one commit; assert the tier hits Skim, total settle <= 600 ms, and no per-step choreography exceeds its server floor (trick 3300/3800, hand 2500).
- Strips: `--scene table-play --script scripts/play.json --strip ...` (r6 section 0) for the landing, winner reveal and collect; compare against `table-play.png`/`table-hand-scored.png`.
- Visual: 720x520 minimum window must keep the seat plates, bubbles and stamps clear of the rack (audit saw overlap in round 2).

## 12. Priority order

| P | Change | Why | Cost |
|---|---|---|---|
| P0 | Seat plate + turn glow, rack gold bar (4.3) | the single largest clarity gap | small, core |
| P0 | Winner ring gated on `landed_at` + live winner outline (5.6, 5.7) | fixes a wrong reveal and answers "who is winning" | small |
| P0 | SLAM spring, landing THUD + ring, shadow plates (4.2, 5.6) | the "feel" of every play | small |
| P0 | Tempo tiers + Snap cleanup (2, 6) | the safety net for everything else | medium |
| P1 | Trump badge/pips, led pill, legal lift and dim (5.4, 5.5) | glance-readability | small |
| P1 | Bid bubbles + pickup retune (5.3) | bids at a glance | small |
| P1 | Hand-score panel + `launch` delta + RollingText (5.10) | the empty screen in the audit | medium |
| P1 | Stamps: EUCHRE, MARCH, ALONE (5.8, 5.9) | the three emotional peaks | small |
| P2 | Deal from the dealer, packet stagger, up-card flip (5.1) | the opening moment | medium |
| P2 | Screen transitions, button lift (5.12, 4.5) | polish | small |
| P2 | Game-won panel with blur scrim (5.11) | the end moment | small |
| P3 | Poll-driven arc for your card; rune-fx; post-landing tilt; felt grain | bold but each has a clear fallback | medium, optional |

Out of scope: sound, haptics, 3D flips (no `rotateY`, no `scale_x`, `rotate` isolates a layer: r2 line 118), per-axis curves, Keyframes/Choreography (unreachable from the `rune` crate: r1 line 194), apps/desktop edits.
