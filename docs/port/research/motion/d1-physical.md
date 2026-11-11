# D1 - Physical and cinematic motion proposal (felt under a warm lamp)

Stage: DESIGN d1-physical. Audience: the engineer who implements it. Read-only research; nothing in apps/desktop was edited.

## 0. Revs, citations, ground rules

- Pinned rev P = 85cd8c6, read at `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/`. HEAD H = 0ea75abe (scratchpad clone). The research notes (r1 section 0, r5 section 1) found no public motion API difference between P and H, so everything here is buildable at P with no pin bump. H differs only in internals (launch phases, `Mode::Due` wake fix).
- Citation form: `path:line @P` where I re-verified the line myself in this pass; otherwise I cite the note and section (`r2 s7.1`), and that note carries the source path:line. I did not invent line numbers.
  - Verified at P this pass: `crates/rune/src/el.rs:368-398` (the `live_*` bindings: opacity, offset, scale, rect, trim, fill; no `live_rotate`), `crates/rune/src/live.rs:95-200` (`Live`, `set`, `snap`, `smooth`), `crates/rune/src/tooling.rs:35` (`poll: fn(&mut S, f64, &mut Tx) -> bool`), `crates/rune/src/lib.rs:46` (`motion_gate`).
  - Everything else: r1 s2 (Spring, tokens, Curve/Anim), r1 s4 (presence and kit clamps), r1 s5 (stagger), r1 s6 (matched, unclipped_flights, launch/fly_text), r1 s7 (transforms, shadow, "Not in Rune"), r1 s8 (sequence/loop), r1 s9 (Live/Tween), r1 s11 (Tx, poll, time scale, reduce motion), r2 s2-s7 (Spring.sample, Stagger, app-owned driver, recipes), r3 s1-s5 (shadows, felt, rune-fx, glow, burst), r4 ranked top 15, r6 s1-s5 (audit).
- Repo facts used (read-only): `apps/desktop/src/theme/felt.rs`, `apps/desktop/src/theme.rs` (hexes at lines 105-133: TABLE_BG #17452f, FELT_CENTER #245a49, FELT_EDGE #10382c, FELT_RIM #dae6b038, RAIL_TOP #5a3a22, RAIL_BOTTOM #3b2415, BRASS #b4a57a, FOCUS_FELT #f5d98e, FELT_BUTTON #f5e8c3, FELT_BUTTON_BORDER #ead8aa, FELT_BUTTON_HOVER #fff0c7), `apps/desktop/src/screens/table/{present,stage,hand,header}.rs`.
- Contract (kept, not negotiated): 06-ui-spec copy, rules, a11y checklist (s20.4) untouched. Tempo from 02-protocol s9.1 and 03-table-actor s9: `TABLE_MOTION` = {play 700, read 1500, collect 780, stagger 45} ms; `TRICK_PRESENTATION_MS` 3300 (3800 when the trick seals a euchre), collect starts at +2200 and must end by about +3115; production `handScore` 2500, AI think floors cut 700 / bid1 and bid2 1900 / discard 1100 / play 1000 (+0-399 jitter); human turns wait indefinitely; fast tempo `trickResolve` 120, `handScore` 250, floors about 100. A cut yields `cut`, `dealt` (8 packets) and `upCardTurned` in one sync. `trick_resolve` to `trick_play` has `steps: []`, so "collected" and "next trick" come from view diffs. `steps: []` means snap (reconnect, review gate, 10 s resync). Stale `v` is ignored.
- Items I could not verify are in section 9 and are marked UNVERIFIED inline. Each has a Tier A fallback so nothing blocks the build.

## 1. Principles (what "physical" means here)

1. Weight: a card is a thin slab. Leaving a surface it rises (lift, soft shadow spreads and lightens). Landing it drops, squashes 3%, shadow tightens and darkens, and it settles with one small rebound. Heavier moves (sweep) use zero bounce; light moves (deal) use bounce 0.25-0.35.
2. Continuity: no teleporting. Every card visible on screen is the same element or a flight of it from its previous rect to its next rect. The only snap is a server snapshot (`steps: []`).
3. One lamp: a warm light centered at about (50%, 42%) of the felt. Shadows fall away from it; the active seat is lit by a pool of the same hue. Nothing glows in a second colour (brand: single accent, calm).
4. Motion serves state, never gates it: the view (server state) is always truth. Animation is a cosmetic overlay that finishes, accelerates or is dropped. Input is enabled the frame the view says it is.
5. Tempo-bounded: every flight total fits inside its `TABLE_MOTION` slot with margin (play total at most 640 ms of 700; sweep ends at about 2980 of 3115).
6. Quiet: no confetti rain, no "AI thinking" gags. Brand OS says calm and factual. Celebration is lamp-brightening, a brass shimmer and a restrained single-hue spark ring.

## 2. Mechanism toolbox (pick per moment)

**Tier A - matched flights, tuned (fallback and reduced-motion-adjacent).** `El::matched(id)` + `Tx::motion` / `.motion` spring (r1 s6; precedence Tx::motion > element `.motion` > REFLOW). Straight line, same curve both axes, carries rect/velocity/radius/fill/opacity but not rotation or scale; draws the new element's picture; needs `Tx::unclipped_flights()` to cross clips; fires only when an id leaves and another mounts in one commit; suppressed under reduce motion, `instant`, `settle` (r1 s6, cookbook 07 s15 items 17-19). Already used by the desktop (stage.rs springs PLAY/PICKUP/COLLECT).

**Tier B - Live-driven flight overlay (the cinematic mechanism; no view rerun per frame).** A pure function `pose(flight, now)`; `poll` (`AppSpec::poll`, `tooling.rs:35 @P`) evaluates it and writes `Live` values bound with `live_offset`, `live_scale`, `live_opacity`, `live_rect` (`el.rs:368-398 @P`). Request the next frame with `tx.wake_after(Duration::ZERO)` while any flight is active and stop when settled (r1 s11; r2 s1 "app-owned driver"). Along-track progress comes from the public closed-form `Spring::sample(x0, v0, target, t)` (r2 s2.1, s7.1 VERIFIED compile+samples), so duration and bounce are the same tokens as everywhere else. Arc:
```
s      = Spring::sample(0.0, v_in, 1.0, t)        // 0 -> 1, may overshoot with bounce
dir    = (to - from);  n = perp(dir).normalize()  // n points to screen-up side
pos    = from + dir * s + n * lift * sin(PI * clamp(s, 0, 1))
scale  = 1 + a * sin(PI * clamp(s, 0, 1))         // a = 0.08..0.12 (rise toward the lamp)
```
Rotation cannot be Live (no `live_rotate`, r1 s7 "Not in Rune"), so the tilt is a two-commit `rotate` -> `rotate_motion` spring on the overlay element (mount at start tilt, the next commit sets the end tilt; first frame never animates, r1 s1/s3). Ground shadow = a separate "plate" element (section 3.3 here, r3 s1.2 plate technique) with its own Live offset/scale/opacity, because shadow geometry only animates on `.motion`, not on `.offset_motion` (r1 s7, r3 s1.2). Flip = width squeeze via `live_rect` on a back child and a face child (r2 s7.2 VERIFIED compile), fallback crossfade + scale dip.

**Tier C - landing settle and emphasis (render-clock, zero commits).** `El::sequence` / `Sequence`+`Beat` (opacity and uniform scale only, `&'static` slice, `.delay(f64)`, lands final pose under reduce motion; r1 s8) for pops and pulses; `LoopSpec` opacity loop for lamp breath (r1 s8); `RollingText` odometer for numbers (r4 odometer); `fly_text` for "+2" flying to the header (text-sourced, correct use; r1 s6).

**Tier D - rune-fx (optional, last).** Needs a `Compositor`, wgpu 30 and the same rev as the app; glow, star sprites on a shape outline (max 64), ripple, grain; use one gold hue only (aura is a rainbow, do not use); gate `animating()`, pre-warm pipelines; skip rune-light (r3 s2, s8, s9). Every Tier D item below has a core-visuals fallback.

Z-order: flights above table, below banners and toasts; use `paint_order`/`raise` (r1 s7). Played cards stack by arrival order (later above earlier) so a trick reads like cards actually laid down.

## 3. Presentation Director (server-tempo safety)

The desktop already has `Presentation`/`poll`/`after_change` in `present.rs`. The Director replaces its animation planning, not its state handling.

### 3.1 Contract
- Source of truth is the view. A step produces a `Cue` (Flight, Pop, Banner, Count). A cue never writes game state, never defers a `view` update and never gates input: bid/play/discard controls are enabled the frame the view says so, even if a flight is mid-air.
- Visual ordering rule: a cue may hide the real element (opacity 0 via a `hide` bool in view, or `live_opacity` 0) only until its own landing time, and a hard watchdog `landed_at <= started + min(dur, 700 ms)` un-hides it regardless (so a stuck poll can never lose a card).
- On press of a hand card the card goes to a "pressed" pose (scale 0.98, lift stays) instantly and the action is sent. When the server `played` step arrives the flight starts from the card's current pose (retarget, no jump). If the server rejects (illegal), see section 4.14.

### 3.2 Coalescing and fast-forward
Backlog = queued cues not yet started + active flights remaining.
| Backlog | speed factor | Behaviour |
|---|---|---|
| 0-1 | 1.0 | normal choreography |
| 2-3 | 1.6 | durations /1.6, staggers /1.6, no landing rebound |
| 4-6 | 2.5 | durations /2.5, arcs flattened (lift x0.5), no flips (crossfade) |
| 7+ or `steps: []` or `v` jump > 8 | snap | all flights finish now: set Live to final, un-hide, clear overlays; the newest cue (if any) still animates normally |
Fast-forward of an in-flight cue never jumps: compute the current pose, then run a 140 ms settle flight (spring dur 0.14, bounce 0) from that pose to the final rect. Fast tempo (`trickResolve` 120 ms) naturally lands in the 2.5x/snap rows; that is intended, not a bug.
Effective `speed = tier_speed * user_speed` where `user_speed` is 1.0 default (optional setting, section 6.3). `Cx::time_scale` is separate and exists (clamped 0-8; r1 s11); do not multiply it in twice. UNVERIFIED: whether `poll`'s `now` is already scaled by `time_scale` (section 9).

### 3.3 Snapshot and reconnect
`steps: []` or a `v` jump larger than the number of steps delivered, or the first view after connect, is a snapshot: call `director.snap()` which (1) cancels all flights, (2) sets every Live to its final value via `Live::snap` (`live.rs:95-200 @P`), (3) clears hide flags, (4) leaves the view to render the final layout. Rune never animates on the first frame (r1 s1), so a freshly connected table simply appears settled. The resync every 10 s must not replay: it arrives `steps: []`, so it is a snap, and since positions already match the view, nothing visibly moves. Stale `v` is ignored (protocol), so the Director ignores cues with `v <=` last applied.

### 3.4 Time source
Flights use `poll` `now` (monotonic seconds) minus `t0`; never wall-clock. Under `--snap/--strip` headless runs `rt.presented` drives time (section 8).

## 4. Choreography per moment

Notation: dur = ms of the Tier B flight (time from release to touch), settle = rebound after touch; "sp(d, b)" = Spring duration d seconds, bounce b; stagger in ms; lift in px perpendicular to the path. All values at speed 1.0. Reduce-motion (RM) fallback is listed per item; shared rules are in section 7.

### 4.1 Deal (cut -> `dealt` 8 packets -> `upCardTurned`, one sync)
- What: dealer's deck (center-left of the stage by the dealer seat, stack of 4 backs offset (1.5, -1.5)) throws 8 packets in euchre order (2-3-2-3 / 3-2-3-2 per 02-protocol). Each card flies deck -> its seat slot (own hand: rack slot, face down until arrival; opponents/partner: seat fan top slot).
- Path/timing per card: Tier B arc, lift 28 px (rack) or 18 px (seat fans), dur 360 ms, along-track sp(0.36, 0.28), scale 0.62 at deck (card art at seat-back size) -> 1.0 at rack (or fan back size) with mid rise a=0.08, tilt from deck tilt (-6 deg) to slot tilt via rotate_motion sp(0.40, 0.30). Intra-packet stagger 55 ms, inter-packet stagger 110 ms; total about 7*110 + 2*55 + 360 = 1240 ms, fits inside bid1 floor 1900.
- Own cards flip at arrival: squeeze flip 150 ms (r2 s7.2), starting at 75% of the flight, face revealed at landing. Opponent cards stay backs.
- Z: each later card above earlier; deck pile below flights; plate under each card.
- Light/shadow: plate opacity 0.16 at deck -> 0.34 at landing (airborne = softer), scale 1.12 -> 1.0, offset (4, 12) -> (2, 4); landing squash scale 1.0 -> 0.97 -> 1.0 (sp(0.18, 0.4)).
- Rack re-layout: cards already in the rack reflow with the default REFLOW spring (no hand-tuned values). Rack slot rects: use deterministic rack layout math from hand.rs (cards = HAND_W 112 + HAND_GAP 18) or `Cx::window_layout(key)` (UNVERIFIED which is available at deal time; section 9).
- Input: if the human is the first bidder, the bid panel is live the frame the view says so; the remaining packets speed to 2.5x (director row 4-6).
- Reduce motion: no flights; own hand and fans appear in place with a 160 ms opacity fade (presence fade), staggered 40 ms by card; deck pile fades 160 ms.
- API: Tier B (`Spring::sample`, `live_offset/live_scale/live_opacity`, `rotate_motion`, `live_rect` flip), `Stagger::new(gap).from(..).delay(i, n)` (r1 s5 / r2 s3.1 VERIFIED) for scheduling start times in the Director (do not rely on `enter_delay` for matched arrivals, UNVERIFIED).

### 4.2 Up-card turn
- What: top card of the kitty lifts and turns face up beside the deck (06 s5.4.4 up-card, height cap UP_CARD_H 190).
- Path: rise 16 px and scale 1.0 -> 1.06 over 140 ms (sp(0.14, 0)), squeeze flip 240 ms (back width 1 -> 0 over first 120 ms ease-in, face width 0 -> 1 over next 120 ms ease-out, centered), settle scale 1.06 -> 1.0 with sp(0.22, 0.3). Delay after last dealt card: 90 ms. Total about 480 ms.
- Light: plate scales 1.0 -> 1.15 at apex -> 1.0; warm specular: a 1 px top edge highlight child `#ffffff40` fades 0 -> 1 during the face reveal (faint sheen), gone at rest.
- Input: none needed; bidding available per view. Do not hold the bid panel for this.
- RM: face swaps with a 160 ms crossfade, no rise.
- API: Tier B for rise/scale; `live_rect` flip (r2 s7.2); Sequence for the highlight (r1 s8).

### 4.3 Bidding panel and trump named
- Panel enter: bid panel (06 s5.8) rises from 12 px below with opacity 0 -> 1, `Transition` slide 12 + fade, spring sp(0.24, 0.2) (r1 s4 constructors; stay within kit clamps: scale 0.9-1.1, removal opacity forced 0). Buttons stagger 40 ms left to right via `enter_delay` + `Stagger` (r1 s5; enter_delay on plain presence is verified, only matched arrival is not).
- Press: scale 0.97 over 80 ms (`scale_motion` sp(0.08, 0)), release springs to 1.0 with bounce 0.3. Hover: fill to FELT_BUTTON_HOVER (#fff0c7), 120 ms colour animation (r1 s7 colour); the button arrow nudges +3 px via `offset_motion`.
- Pass: speech chip over the seat (existing copy) fades in 160 ms, holds, fades out; seat pool dims (4.15).
- Trump named: the trump suit glyph in the header/score area pops: Sequence Beats scale 1.0 -> 1.22 -> 1.0 over 260 ms (`Beat` uniform scale only), ink stays the 06 suit colour; the up-card gets a soft lamp halo (single gold `#f5d98e` shadow ring, alpha 0x40 -> 0, 400 ms, `box_shadow` anim ok up to 4 layers) when ordered up.
- RM: panel fades 160 ms; no pops; halo omitted.

### 4.4 Hover, select, press on a rack card (human, playable)
- Hover (pointer): lift -6 px, shadow HAND_CARD -> lift variant (section 5.4 `CARD_LIFT`), spring sp(0.16, 0.25) via the existing `offset_motion(LIFT)`; neighbours part by 5 px in x (their `offset_motion` too), so the fan opens under the pointer. Hover out: sp(0.18, 0).
- Select (existing `SELECTED_LIFT` -8): same spring, plus a 2 px FOCUS_FELT (#f5d98e) outline fade in 120 ms (never a second accent).
- Press-down: scale 0.98, 60 ms.
- Non-playable cards (must follow suit): no lift, dimmed per 06 (kept); pointer cursor unchanged.
- RM: lift snaps (spatial snaps), outline fades 150 ms; shadow swaps instantly.
- API: `offset_motion`, `scale_motion`, `box_shadow` anim (up to 4 layers; r1 s7, r3 s1.2 VERIFIED deepening shadow).

### 4.5 Own play (human card to the trick slot)
- What: the pressed card flies from its rack rect to slot 3 ("you", `t::SLOTS[3]` = (0.57, 0.68), tilt -3 deg, width `PLAYED_W` 0.22 of trick box, cap 145).
- Path: Tier B arc, start pose = current pose of the pressed card (so no jump after press), dur 440 ms, along-track sp(0.44, 0.22), lift 0.10*dist capped 36 px, scale from rack scale to slot scale with a=0.10 mid rise, rotate_motion sp(0.44, 0.30) from rack tilt to -3 deg (+ 4 deg overshoot from bounce). Total on screen 440 + settle 180 = 620 ms (< play 700).
- Landing: squash 1.0 -> 0.97 -> 1.0 sp(0.18, 0.4); plate tightens (opacity 0.30 -> 0.45, scale 1.10 -> 1.0, offset (2,10) -> (1,2)); a 120 ms ripple is NOT used (core only); contact shadow = `PLAYED` token.
- Rack: remaining cards close the gap with the existing REFLOW (default spring), starting at release.
- Z: above everything in the trick, below banners.
- Input: next state (partner/opponents) is server-driven; nothing waits on the landing.
- RM: card appears in slot 160 ms fade, rack closes snap.
- API: Tier B; REFLOW default; `PLAYED` shadow tokens (felt.rs `shadow::PLAYED`).

### 4.6 Opponent / partner play
- What: card leaves the seat fan's top back, becomes face-up, lands in that seat's slot (0: left (0.28, 0.50, -7 deg), 1: partner (0.43, 0.35, 3 deg), 2: right (0.72, 0.50, 7 deg)).
- Path: arc outward-then-in; lift 0.14*dist (max 44 px) on the side away from the stage center so the card visibly "comes from" the seat; scale 0.42 (fan back size 26x36 per section 5.5) -> slot scale; dur 500 ms, sp(0.50, 0.24); flip squeeze at 30%-60% of the flight (r2 s7.2) so the face is readable when over the slot; tilt from 0 to slot tilt via rotate_motion sp(0.5, 0.3).
- Fan: the fan's remaining backs close up with REFLOW starting at release; the card count badge (if any) uses RollingText -1 (r4).
- Total: 500 + settle 160 = 660 ms (< 700). Think-floor jitter is untouched: the flight starts when the step arrives.
- RM: face fades in at the slot 160 ms; fan snaps.

### 4.7 Landing and read window (700-ish to 2200 ms)
- Played card lands, plate settles, then idle: a barely perceptible float is NOT used (calm). A subtle lamp specular sweep passes once across the newest card's top edge over 400 ms (gradient child `#ffffff26` moving via `live_offset`) to mark "newest". Skip under backlog >= 2.
- Led suit marker: existing copy kept; marker fades in 120 ms at lead.

### 4.8 Winner emphasis (after landing only)
- Trigger: `trickResolve` step and the winner's card `landed_at` is set (never before the last card lands).
- Effect: WINNER_OUTLINE [3, 5] (theme `t::WINNER_OUTLINE`) outline in FOCUS_FELT #f5d98e, opacity 0 -> 1 over 160 ms, a pop via Sequence scale 1.0 -> 1.05 -> 1.0 over 260 ms, losers dim to opacity 0.78 over 200 ms. Hold through read (to +2200).
- Light: the winner's seat pool rises to 0.30 (4.15).
- RM: outline fades 150 ms, no scale pop, no dim animation (instant dim).
- Optional Tier D: single gold spark ring on the card outline, 18 sparkles, 600 ms (r3 s4 trick-win burst VERIFIED); gated `animating()`.

### 4.9 Trick sweep (collect)
- Window: starts +2200 after trick end (production). Must end by about +3115; budget: stagger 45 x 3 = 135, so the last card has at most 645 ms; use anticipation 80 ms + travel 540 ms = 620 ms. Last card ends at 2200 + 135 + 620 = 2955 (< 3115). Euchre-sealing trick: presentation 3800, collect start is later in the same ratio; keep the same durations.
- Order: led card first, then clockwise, 45 ms apart (`Stagger::new(0.045)` in the Director; r2 s3.1).
- Per card path: anticipation: 6 px away from the winner over 80 ms (ease-out); then Tier B flight to the winner's taken-stack, lift 0.06*dist (small arc), along-track sp(0.54, 0) (zero bounce = heavy), scale slot -> 0.46 (taken-stack card size), rotate_motion toward the stack's tilt sp(0.54, 0), flip to back via squeeze at 35%-60% (cards go to the stack face down), opacity stays 1 until 85% then fades under the stack top.
- Taken stack: receives a `+1` layer on landing: stack child scale 1.0 -> 1.06 -> 1.0 Sequence (180 ms); count numeral via RollingText (r4) if present in 06; copy unchanged.
- Z: sweep cards above the felt and below banners; winner's card travels last-in-front (raise it).
- Shadow plate: follows each card; opacity 0.34 -> 0.18 as the card shrinks and leaves the lamp.
- Fast-forward: at tempo `trickResolve` 120 ms the sweep sits in the 2.5x/snap rows; sweep still starts from current positions with the 140 ms settle flight.
- Reconnect during sweep: snapshot; trick area simply empty and stack count final.
- RM: cards fade out 160 ms where they lie (spatial snaps, opacity fades), stack count updates with a 160 ms crossfade, no flights.
- Tier A fallback: existing `matched` COLLECT flight per card (r6 s1), keep stagger 45.

### 4.10 Next trick lead
- The next-lead card (human) or AI card flies as 4.5/4.6 normally; the turn pool crossfades (4.15). The empty trick area has no placeholder animation.

### 4.11 Up-card pickup (order up) and dealer discard
- Pickup (AI dealer): the up-card flies from the up-card position to the dealer seat fan: dur 420 ms, sp(0.42, 0.1), lift 18 px, scale 1.0 -> 0.42 (fan back size), flip to back via squeeze at 30%-55% (hand hides it), plate fades out, fan's top slot receives a pop (Sequence 1.0 -> 1.12 -> 1.0, 180 ms). Then the "discard" step: one back hops from the fan to the kitty/taken pile: dur 300 ms, sp(0.30, 0.2), lift 14 px, scale 0.42 -> 0.34, opacity ends 1 under the pile.
- Pickup (human dealer): the up-card flies into the rack slot assigned by hand sort (REFLOW opens a gap in the rack first, 160 ms, then the card arrives): dur 400 ms, sp(0.40, 0.25), scale 1.0 -> rack scale, tilt to rack tilt; face stays face up (own cards). Discard prompt (existing copy) appears when the card lands. Discard: pressed card flies to the pile: dur 340 ms, lift 20 px, scale 1.0 -> 0.34 with squeeze flip to back at 40%-65%, rest as above.
- Timing check: discard AI floor is 1100 ms; the pickup + discard cues are 420 + 300 + 160 stagger = about 880 ms, within the floor. If the human dealer is not yet to discard, nothing is gated.
- Backlog: if the discard step arrives before the pickup finishes (fast tempo), the Director's snap/fast-forward rules apply (3.2); the pickup flight retargets with the 140 ms settle.
- RM: up-card fades 160 ms and a back fades in on the dealer's fan; human: card appears in rack with fade, discarded card fades out.
- Tier A fallback: existing PICKUP matched flight (r6 s1).

### 4.12 Hand score (result card)
- Problem from the audit (r6 s4 "Hand scored"): the screen is empty. Add a result card (copy from 06 s9 unchanged; only the presentation changes): cream plate (FELT_BUTTON #f5e8c3, border #ead8aa, radius 10), rises 24 px and fades over 280 ms, sp(0.28, 0.15) (kit clamp respected: slide, not scale). Rows (team scores) stagger 60 ms.
- Count: points chips for the hand ("+1", "+2", "+4") appear with a pop (Sequence scale 1.0 -> 1.15 -> 1.0, 200 ms), then `fly_text` carries the numeral from the chip to the header score (text-sourced launch; r1 s6; reverse onto the numeral) at about +900 ms, and the header numeral uses `RollingText` (existing `Transition` in header.rs; r4 odometer) to roll 600 ms.
- Window: production `handScore` is 2500 ms: enter 0-280, hold to 2200, exit 2200-2500 (opacity 160 ms). Fast tempo 250 ms: snap to the final card with a 120 ms fade, no fly, no roll (director snap row).
- RM: card crossfades 160 ms; numerals crossfade 160 ms; no flight, no roll.
- API: Transition slide+fade (r1 s4), Sequence (r1 s8), `Tx::fly_text` (flight.rs:16 @P per r1 s6), RollingText (r4).

### 4.13 Euchre, march, lone hand, loner banners
- Banner (existing `Banner` in present.rs; stage.rs `shadow::BANNER`): enters from 14 px above with fade, sp(0.30, 0.25), holds, exits 160 ms. Euchre: also a 6 px horizontal shake on the euchred team's seat plate (poll damped sine 8*exp(-t/90)*sin(t/18) over 280 ms, section 4.14) is NOT used for the winner side; only on the euchred side. March: banner plus a gold brass-line shimmer sweeping once across the rail's brass line (border_gradient position via `live_offset` on a thin 40 px highlight child, 700 ms linear). Lone: seat plate of the lone player gains a 2 px gold outline pulse (LoopSpec opacity 0.6 <-> 1.0 over 1.6 s while lone is active, r1 s8; RM: static outline).
- Optional Tier D: banner outline sparkles, 24 stars, one hue (#f5d98e), 700 ms (r3 s4/s5).
- RM: banner fades 160 ms; no shake, no shimmer.

### 4.14 Illegal-play nudge
- Server rejects (or client-side legality shows "must follow suit"): the pressed card returns to its rack pose from its current pose (retarget, never a jump) with sp(0.22, 0.35); a horizontal shake x = 8*exp(-t/90)*sin(t/18) over 280 ms driven by poll into `live_offset`. Existing copy/toast unchanged.
- RM: 150 ms FOCUS_FELT outline flash (opacity), card returns in place with no motion.

### 4.15 Turn light (replaces the invisible seat tint) and "thinking"
- Each seat has a warm light pool under it: radial gradient `#f5d98e` alpha 0x2e at center to 0x00 at edge, size 1.5x seat block (SEAT_W 180), opacity 0 when inactive, 1 when active. Crossfade between seats with `opacity_motion` sp(0.36, 0). The human's seat pool is the rack glow (below the hand).
- "Thinking" (AI turn) is the pool alone: opacity 0.14 <-> 0.20 over 2.4 s `LoopSpec` opacity loop, calm; no dots, no text. Step arrives -> loop stops, flight begins.
- Human's turn: pool at 0.26 plus the rack's playable cards gain the lamp (no extra glow), per 06 a11y which already states turn indication (kept; the light is additive).
- RM: pool snaps to active seat with a 160 ms fade, no loop.

### 4.16 Game won and game lost (06 s9 outcome moments)
- Panel (existing copy kept): panel rises 32 px with fade, sp(0.40, 0.2), 20 px above-seat shadow `BANNER`; final score numerals roll in 700 ms via RollingText.
- Won: lamp brightens (pool 0.14 -> 0.28 over 800 ms, felt center +8% brightness via an overlay child `#fff3c8` alpha 0x10 -> 0x1c), brass line shimmer once (700 ms), optional Tier D sparkle ring on the panel outline (40 stars, #f5d98e, 1.2 s, glow `animating()` gated; r3 s5 VERIFIED). Buttons stagger 60 ms.
- Lost: no sparkles, panel sp bounce 0, lamp dims 0.14 -> 0.08 and a cool overlay `#0a2a20` alpha 0x38 fades over 600 ms (calm, not alarming; no red).
- RM: panel fades 160 ms, numerals crossfade, lamp states swap with a 160 ms fade, no shimmer, no sparkles.

### 4.17 Screen transitions (home <-> new game <-> table <-> game over; loading/reconnect)
- Home -> table: route content crossfade 220 ms; on arrival the lamp "switches on": light pool opacity 0 -> 1 over 600 ms (slow ease-out), vignette 0.6 -> 1.0 over 600 ms. Deal starts at 300 ms (the Director sequences it) but gameplay input is never delayed (deal fast-forwards if the view says it is the human's turn).
- Table -> game over: panel animation 4.16; table dims under: backdrop overlay `#04140e` alpha 0x66 over 300 ms.
- Loading (06 s14): existing copy; skeleton fades 160 ms; no animation of cards.
- Reconnecting: banner 06 s14 copy kept; table dims 0.6 opacity overlay `#04140e` alpha 0x4d fade 200 ms; when synced the overlay fades 160 ms and the Director has snapped (3.3).
- RM: crossfades only; lamp states swap.

## 5. Visual tokens (colours, radii, shadows, sizes)

Items marked (spec) are existing 06/theme tokens, retained. Items marked (new) are proposed and need owner sign-off if they deviate from a numeric 06 value; none change copy.

### 5.1 Felt stack (bottom to top; one gradient slot per element, so each layer is a pinned child)
1. base `FELT_EDGE` #10382c (spec).
2. radial `FELT_CENTER` #245a49 -> `FELT_EDGE`, stops `FELT_STOPS` [0.15, 0.72] (spec, theme `t::FELT_STOPS`).
3. lamp pool (new): radial, center (0.50, 0.42), radius 0.55 of longer side, stops `#f5d98e2e` at 0 -> `#f5d98e12` at 0.45 -> `#f5d98e00` at 1.0.
4. vignette (new): radial, transparent to 0.55, `#04140e8c` at 1.0, fully covers felt.
5. grain (new): one pre-baked 1024x640 noise PNG, stretched, opacity 0.035 (`cache()` the static layers; tiling is UNVERIFIED so use the stretched PNG, r3 s1.3).
6. rim stroke `FELT_RIM` #dae6b038 inset `size::FELT_RIM_INSET` 12 (spec).
Letterbox fix (new, owner check vs 06 s5 / s16.1): extend felt + rail to the window and keep the 06 s5.4 stage box inside; audit images (r6 s4) show the felt boxed with empty margin.

### 5.2 Rail and brass
- Rail width 14 (spec `size::RAIL`), gradient `RAIL_TOP` #5a3a22 -> `RAIL_BOTTOM` #3b2415 (spec).
- Add (new): 1 px top specular `#ffffff1f`, inner shadow on the felt side (inset) `0 2 6 #00000066`, felt inner shadow from rail `inset 0 6 18 #0000008c`. Existing `shadow::RAIL` kept: (0,3,0,0,#153c2b) + (0,18,36,0,#0000004d).
- Brass line 1 px #b4a57a (spec) -> 2 px via `border_gradient` `#d8c894` -> `#8d7c52` (new, one gradient slot).

### 5.3 Radii
10 px base (`--radius`, spec); cards 10; buttons per spec; plates 999 for pills; seat plates 12 (new, +2 over base for glass plates).

### 5.4 Shadows and light direction
Lamp at (0.50, 0.42). Shadow x-offset = clamp((cx - 0.5*W)/W * 12, -8, 8) px, y offset positive; computed once per slot (4 trick slots, 5 rack slots) at layout time, not per frame.
- `HAND_CARD` (spec, 06 s1.4): (0,1,0,0,#fff), (0,3,0,0,#c4c7b9), (0,7,13,0,#16382c24) - kept at rest on the cream card edge.
- `CARD_REST` on felt (new, trick/taken): (0,1,0,0,#ffffff33), (0,2,2,0,#0000005c), (0,8,16,0,#00000047).
- `CARD_LIFT` (new, hover/select): (0,2,0,0,#00000040), (0,14,24,0,#0000004d).
- `PLATE` (new, airborne): rect, radius 10, fill `#00000000`, shadow (0,18,22,-2,#00000052); plate Live: opacity 0.16-0.45, scale 1.12-1.0; one constant shadow count so it never needs `.motion` to change geometry (UNVERIFIED render of a blurred-shadow-only plate, section 9; fallback `fill #00000040` blurred element).
- `shadow::PLAYED` and `shadow::BANNER` (spec, felt.rs) kept.

### 5.5 Sizes
- Rack card: `card::HAND_W` 112 (spec, min) -> clamp(112, 0.085*W, 148) (new); HAND_GAP 18 (spec).
- Seat fans (audit: tiny): back 14x20 (current) -> 26x36 (new), fan arc +/-12 deg, 5 backs; the deck pile uses the same artwork.
- Unified card back: one 512x716 back art for deck, fans and taken stacks (new; audit shows mixed backs).
- Played card: `PLAYED_W` (0.22, cap 145) (spec) -> cap 168 on windows >= 1440 (new).
- Taken stack: up to 4 layered backs offset (1.5, -1.5), plus count (RollingText).
- Up-card: `UP_CARD_H` 190 (spec).
- Seat plate: SEAT_W 180 (spec), fill `#0f2f2459`, 1 px border `#dae6b02e`, radius 12 (new); `backdrop_blur` optional (max 4 instances; exactly the four seats) only if the perf budget allows (r3 s8).

### 5.6 Buttons
Felt button: `FELT_BUTTON` #f5e8c3, border `FELT_BUTTON_BORDER` #ead8aa, hover `FELT_BUTTON_HOVER` #fff0c7, min width 230, label/arrow gap 18 (spec). Add press scale 0.97 (80 ms) and arrow nudge +3 px on hover (new). Primary blue fills are for the web shell only; on felt keep the cream button (spec).

### 5.7 Hand-score and game-over panels
Result card: fill #f5e8c3, border #ead8aa, radius 10, shadow `BANNER` (spec) + 1 px inner highlight `#ffffffaa` at top (new); heading font per brand (DM Sans 600), body IBM Plex Serif; no emoji.

## 6. Reduce motion and settings

### 6.1 Rules (r2 s5; r1 s11)
Under `cx.is_reduced_motion()`: spatial, scale and rotation snap; opacity, colour and presence get a 0.15-0.18 s fade; matched/launch are suppressed by the engine. App-owned values (Tier B flights, poll-driven shakes, loops) are not covered by the engine, so the Director must branch: if RM then `director.snap()` for all spatial cues and run only opacity cues at 160 ms. No loops (lamp breath) under RM. No flip squeeze under RM (face swaps with a crossfade).

### 6.2 Per-moment RM summary
Deal: place + 160 ms fade, 40 ms stagger. Up-card turn: crossfade. Hover/select: snap lift, outline fade. Own/opponent play: fade-in at the slot. Winner emphasis: outline fade. Sweep: fade out in place. Pickup/discard: fades. Hand score: crossfade. Banners: fades. Illegal: outline flash. Turn light: 160 ms fade. Game won/lost: fades. Screen transitions: crossfade.

### 6.3 Optional speed setting (proposal; copy needs owner approval, not in 06)
Animation: Full / Quick / Off. `user_speed` 1.0 / 1.6 / snap(=RM path). Do not invent final copy without approval; r4 (settings speed knob) shows the pattern. Server tempo is never changed by this setting.

## 7. Gameplay-moment cost and risk table
| Moment | Mechanism | GPU/CPU notes | Fallback |
|---|---|---|---|
| Deal | Tier B x up to 20 cards | Live per frame, no view rerun; wake only while active | Tier A matched |
| Flip | `live_rect` squeeze | cheap | crossfade + scale dip |
| Plate shadows | 4-8 elements | blur cost small (r3 s8) | fixed shadow, no plate |
| Light pool/vignette | gradient children, `cache()` | static | drop pool |
| Sparkles/glow | rune-fx | pipeline pre-warm; gated | omit |

## 8. Test plan
- Unit-test the Director with no GPU: feed cue queues, assert backlog tier speeds and that `snap()` leaves every Live at its final value and clears hide flags; assert watchdog un-hides a card after 700 ms with a frozen clock.
- Pure-function tests for `pose()`: p=0 equals `from`, p=1 equals `to` (arc term sin(pi) = 0), with bounce overshoot is bounded by 0.6x of lift.
- Timing contract tests: sweep last-card end <= 3115 ms from trick end using `TABLE_MOTION`; own play total <= 700; opponent total <= 700; deal total <= 1900.
- Headless goldens: `--snap` at t = 0, 120, 240, 360, 480 ms of a play; `--strip` for deal, sweep, pickup; use `rt.presented` frame counter for deterministic time (r1 s14); `--trace` to confirm no view rerun per frame during Tier B flights (only start/retire commits); `--tree` to check z-order and `hide` flags.
- `motion_gate` (`lib.rs:46 @P`) to assert RM snaps spatial values and keeps opacity fades.
- Reconnect test: deliver `steps: []` mid-sweep and assert zero active flights next frame; replay a 10 s resync and assert no pose changes.
- Matched fallback: r1 s14 recipe for asserting a matched flight with stagger delay (the enter_delay item).
- Manual: run at production and fast tempo; check each Tier B flight is continuous in the strip (no frame where a card disappears and reappears elsewhere).

## 9. UNVERIFIED (resolve before depending on them)
1. `enter_delay` on a matched arrival (use Director-scheduled start times instead).
2. Whether rotation or ancestor offsets ride a matched flight (r1 s6 says rect/velocity/radius/fill/opacity only).
3. Whether `poll`'s `now` is scaled by `time_scale` (test with `Cx::time_scale` = 2).
4. Whether a blurred-shadow-only plate renders as intended (fallback: blurred fill element).
5. How to get rack rects at deal and at press time: `Cx::window_layout(key)` vs deterministic layout math.
6. Whether elements in an enter transition are hit-testable (matters for pressed cards mid-pickup).
7. rune-fx under the app's wgpu surface (wgpu 30, same rev); all Tier D items are optional.
8. Image tiling for grain (use one pre-baked stretched PNG).
9. Two-commit rotation arming for overlays: confirm the first-frame no-animate rule does not swallow the second commit when it occurs in the same frame as the mount.
10. A user-facing "Animation" setting is not in 06; copy and placement need owner approval.

## 10. Build order (visible payoff per effort; each step ships alone)
1. Director skeleton + snapshot snap + RM branch (no visuals; makes everything below safe).
2. Felt stack, rail, brass, lamp pool, vignette, grain (section 5.1-5.2), bigger rack cards and seat fans, unified back (visual-only, biggest look win; r6 s5).
3. Hover/select/press and shadow tokens (4.4, 5.4).
4. Own play arc + opponent play arc + landing settle with plate (4.5-4.7).
5. Trick sweep with anticipation, flip to back, taken-stack pop (4.8-4.9); winner emphasis after landing.
6. Deal with packets and flip (4.1) + up-card turn (4.2).
7. Pickup and discard (4.11); bidding panel and trump pop (4.3).
8. Hand-score card, count flight, game won/lost (4.12, 4.16); banners (4.13).
9. Turn-light pools and thinking breath (4.15); screen transitions (4.17); illegal nudge (4.14).
10. Tier D rune-fx (glow, sparkles) only if the compositor integration spike (r3 s2) passes; pin bump to H is not required (r5 s6 may be adopted independently).
