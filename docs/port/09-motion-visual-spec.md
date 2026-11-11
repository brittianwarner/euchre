# 09 — Motion and visual spec (desktop)

This is the binding spec for the motion and visual wave in `apps/desktop`. It judges the three proposals (`research/motion/d1-physical.md`, `d2-game-feel.md` and `d3-craft.md`) against the spike (`research/motion/spike.md`). It keeps what the spike verified and marks the rest **SPIKE-FIRST**. A SPIKE-FIRST item must be proven in a throwaway crate, or behind a test, before it ships. If the proof fails, the item falls back to the listed fallback.

**Revs read**

- **Rune.** Pin P = `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb`, source at `~/.cargo/git/checkouts/rune-c9b805e9ed8a3730/85cd8c6/crates/`. Rune citations are `crate/src/file.rs:line` at P.
- **Rune HEAD.** H = `0ea75abef0a6caa014067a88f89518b9a04cce2b`. Only §1 cites it.
- **Desktop.** euchre-rune working tree at HEAD `38143da`. Another agent is editing `apps/desktop` right now, so desktop line numbers will drift. Re-grep the named symbol before you edit.

**Status words**

- **VERIFIED**: rendered or tested in the spike at P (`research/motion/spike.md`, technique S1-S11), or in cookbook 07 §5.
- **SOURCE**: read in the Rune source at P, but not run.
- **SPIKE-FIRST**: not proven. It needs a spike, and it has a fallback.

---

## 1. Rune pin decision: stay on 85cd8c6 for this wave

**Decision: stay.** Bump in a separate, later change.

Reasons:

1. **The spike verified every technique in this spec at P.** HEAD adds no motion API that the spec needs. The public motion surface is identical at P and H (r1 §0). The following already exist at P:
   - `Tx::launch` (`rune/src/flight.rs:28`)
   - `Tx::fly_text` (`:16`)
   - `Tx::unclipped_flights` (`:36`)
   - `rune::motion_gate` (`rune/src/lib.rs:46`)
2. **What H fixes does not block us:**
   - flights LaunchPhase refactor (H `rune/src/flights.rs:21,123,146`)
   - the `Mode::Due` wake fix, dde00138 (H `rune-motion/src/store/tracks.rs:84-85`)
   - finished-Sequence loops that stop requesting frames, 047f6f1d
   - the Picture min/max layout fix, d8f1ffed
   - the window draw-backoff refactor, 627c2b3e
   - sticky `unseen_presence` (#572)
3. **The riskiest H change is 627c2b3e (window repaint and backoff).** It touches the native draw loop that headless snapshots never exercise. Landing it in the middle of a motion wave would confuse every frame-time regression.
4. **Another agent is changing `apps/desktop` now.** A lockfile bump on top of that is avoidable churn.

**Exact changes for the later bump** (its own change, after this wave is merged):

- `Cargo.toml:44-45`: set both `rune` and `rune-kit` to `rev = "0ea75abef0a6caa014067a88f89518b9a04cce2b"`, or to a newer HEAD that is at least two weeks old. They must move together, because rune-kit depends on rune by path inside the same git rev.
- No `rune-motion` or `rune-fx` line is needed. Every motion type used here is re-exported from `rune` (`rune/src/lib.rs:259-286`: Beat, Curve, Easing, Sequence, Spring, SpringPair, Stagger, Transition and the rest; `Cycle` at `rune/src/el.rs:68`). If rune-fx is ever adopted (§2.9), it gets its own line at the same rev.
- Run `cargo update -p rune -p rune-kit`. Expect lock drift. Cookbook 07 §15 #31 covers `taffy`.
- Re-run cookbook 07 §3, §5 (including 5.5 reduce motion), §9, §13 and §14, every scene snapshot in §6 of this spec, and `bun run --cwd apps/desktop test`.
- Manual check 1: minimise the window, restore it, cover it with another window, uncover it. The table must repaint with no stale frame (627c2b3e).
- Manual check 2: the rack and trick card sizes at 720x520 and at 1440x900 (d8f1ffed changes Picture min/max).
- Manual check 3: the stats line `rune: animating frames … cpu p50/p95` for `table-trick-win` must be within 10% of the pre-bump number.
- After H, a launched flight replaces an existing flight on the same node (H flights.rs:123). Nothing in this spec launches twice on one node, so no code change is needed.

---

## 2. Motion architecture

### 2.1 Where state lives

| State | Owner | File |
|---|---|---|
| Game truth (view, steps, optimistic) | `TableStore` (`crates/euchre-protocol/src/store.rs`; `last_steps` :517, `display_view` :533) | unchanged |
| How the table is being shown: slots, gather, banners, deal-in | `TableScreen.present: Presentation` (`apps/desktop/src/app.rs:77-107`, `src/screens/table/present.rs:251`) | present.rs |
| **NEW**: the choreography queue and the running shots | `Presentation.director: choreo::Director` | **new** `src/screens/table/choreo.rs` |
| The poll clock | `AppState.clock` (app.rs, set in `poll`) | unchanged |
| Effective reduce motion (D15) | `AppState.motion_reduced: Cell<bool>`, set in `app::view` from `state.prefs.reduce_motion \|\| cx.system_reduced_motion()` (app.rs:916-918; `rune/src/runtime/cx.rs:238,256,260`) | unchanged |

Rules:

- The store never learns about motion. The view stays pure, `fn(&State, &mut Cx) -> El`.
- Every pose is a pure function of `(shot, state.clock)`.
- Every deadline is stamped lazily in `Presentation::poll` on the frame clock. This is the existing pattern; see the module docs at present.rs:1-6.
- A handler or bridge reply never reads a clock. That is what makes headless snapshots deterministic (§2.6).

### 2.2 From SyncEvent steps to a choreography queue

Today:

- `on_conn_event` (app.rs, the `TableEvent::Sync` arm) applies the sync, calls `tx.instant()` for a stepless sync that moves the table on, then calls `after_store()`.
- `Presentation::after_change` (present.rs) dedupes `store.last_steps()` by `"{hand}:{step:?}"` (SEEN_STEPS = 80) and calls `on_step`. Today that only cues sound and banners.

New: `on_step` also calls `self.director.enqueue(Shot::from_step(view, step))`. Mapping (`Step` variants at `crates/euchre-engine/src/types.rs:882-967`):

| Step | Shot | Waits for |
|---|---|---|
| `Dealt` (or the rack deal-in rule at present.rs `after_change`) | `Deal { hand_no, dealer }` | anything older of the previous hand: fast-forwarded (§2.3) |
| `UpCardTurned` | `Flip { hand_no, card }` | `Deal` end − 0.10 s |
| `Bid` | `Bubble { seat, hand_no }` (opacity and offset only) | none; runs alongside |
| `TurnedDown` | `Flip { …, down: true }` | previous shot |
| `TrumpSet` | `Trump { suit, maker, alone }` | previous shot |
| `DealerDiscarded` / pickup | `Pickup { hand_no }` (matched; §4.6) | `Trump` |
| `CardPlayed` | `Play { hand_no, index, seat, card, mine }` | the previous `Play` landing (`t0 + PLAY_TRAVEL`) |
| `TrickWon` | `Win { hand_no, index, seat }` | its trick's 4th `Play` landing |
| `HandScored` | `Score { result, … }`, plus `Stamp` for Euchre, March or LoneMarch | the trick sweep's end (`Collect::done`) |
| `GameWon` | `MatchEnd { you_won }` | `Score` end |
| `Cut`, `ThrowIn` | none (copy only) | |

`Director` (new file, Lane A):

```rust
// src/screens/table/choreo.rs
pub struct Director {
    queue: VecDeque<Shot>,      // not started
    live: Vec<Running>,         // started; removed at end (+ FX tail)
    landed: VecDeque<ShotKey>,  // capped 32; views ask "has it landed?"
    pub feel: Feel,
}
pub struct Running { pub shot: Shot, pub t0: Option<f64>, pub dur: f64, ff: Option<(f64, f32)> }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feel { Full, Brisk, Skim, Snap }

impl Running {
    /// 0..=1. Fast-forward remaps the rest of the shot into FF_SECS, continuously.
    pub fn progress(&self, now: f64) -> f32 {
        let Some(t0) = self.t0 else { return 0.0 };
        let raw = (((now - t0) / self.dur) as f32).clamp(0.0, 1.0);
        match self.ff {
            None => raw,
            Some((at, p)) => (p + (1.0 - p) * (((now - at) / FF_SECS) as f32).clamp(0.0, 1.0)).min(1.0),
        }
    }
}
```

`Director::poll(now, reduced) -> PollOut` is called from `Presentation::poll`, which is called from `app::poll`:

1. Pick a `Feel` (§2.3).
2. Start every queued shot whose wait is met, stamping `t0 = now`.
3. Retire finished shots into `landed`.
4. Set `changed` while anything is live.
5. Wake with `tx.wake_after(Duration::ZERO)` (`rune/src/el/tx.rs:226`) while a poll-driven pose is live, else `tx.wake_at(next_start)` (`tx.rs:242`).
6. Stop waking when nothing is live or queued. The app then idles. This is the spike's core pattern, VERIFIED S1/S3/S5.

`PollOut` (present.rs) gains two fields. `app::poll` acts on them:

- `frame: bool`, which leads to `tx.wake_after(Duration::ZERO)`
- `unclipped: bool`, which leads to `tx.unclipped_flights()` on the commit that sends sweep cards (§4.5)

### 2.3 Coalescing, tiers and fast-forward

Take d2's four tiers, plus d3's "animate only the last card" rule. Snap on `steps: []`. Drop d3's `Cx::time_scale` retime: S9 did not verify it. Tiers pick different tokens instead.

| Feel | When (checked on every enqueue and poll) | What plays |
|---|---|---|
| Full | queue ≤ 1 shot and the oldest waits < 0.5 s | everything in §4 |
| Brisk | queue 2-3, or the oldest waits 0.5-1.5 s | durations ×0.6 (`*_BRISK` tokens); plays use matched flights (cookbook §5.1/5.2, VERIFIED), not arcs; no flips (faces snap); no sparks |
| Skim | queue ≥ 4, or the oldest waits > 1.5 s | only the **last** `Play` of each trick flies (matched); older ones mount in place with no `.matched` and `.transition(Transition::none())`; the deal is skipped (cards mount at rest); banners keep their hold |
| Snap | a stepless sync with `should_snap()` (app.rs:199), a resync, a reconnect, or reduce motion | `director.clear()`, and app.rs keeps `tx.instant()` (`tx.rs:513`) |

Fast-forward (`Director::fast_forward(now)`) is a 140 ms catch-up, `FF_SECS = 0.14`:

- Every live shot gets `ff = Some((now, progress(now)))`, so its pose runs to the end inside 0.14 s with no jump.
- Every queued shot is dropped into `landed`.

It fires when:

1. **The player commits a move.** Hooks: `AppState::play_card`, the bid buttons and Continue. The table must show the truth the move was made against.
2. **A `Deal` shot is enqueued while shots of an older hand remain.**
3. **The window becomes visible after being hidden** (`app::on_visibility`). A covered window does not play a backlog.

Never-delay rules (from d2):

- No shot ever gates input. `controls_enabled()` stays store-owned.
- No shot extends a server hold. The trick choreography fits inside `TRICK_PRESENTATION_MS = 3300` (06 §6.1). §4.4 has the budget.
- The illegal message, banners, the 2.2 s read hold and Continue behave exactly as today (present.rs `poll`).

### 2.4 Reconnect

Behaviour today, which stays: `ConnEvent::Status(false)` sets `reconnected`, and the next stepless sync snaps (`should_snap`, app.rs:199-206).

Add: `should_snap() && stepless` also calls `present.director.clear()` and `present.collect`'s existing behaviour stays. Cards that are present at the first render after a reconnect never fly, matching 06 §6.2 "pre-seen keys". `seen_steps` still dedupes replayed steps.

Test: §6, `A-reconnect-snaps`.

### 2.5 Reduce motion (D15)

Effective flag: `prefs.reduce_motion || cx.system_reduced_motion()` (app.rs:916). It already flows to views as `Ctx.reduced` (via `cx.is_reduced_motion()`, `table/mod.rs:36`) and to polls as `motion_reduced`. Rune itself disables matched flights and lands layout springs under it (cookbook 07 §5.5, VERIFIED).

| Piece | Reduced behaviour |
|---|---|
| Play, deal, flip, pickup, sweep, shake, route slide, panel scale, confetti, sparks | none: the state lands at once. `Director::poll(reduced = true)` starts and finishes each shot at `now`, with `feel = Snap` |
| Read hold 2.2 s, Continue, banners 4.5 s, illegal 3.2 s | unchanged (06 §6.5: comprehension time) |
| Winner ring, trump badge, turn marker | shown at once, final state; no `Sequence` element is built (Sequence under reduce motion is unverified, so do not rely on it) |
| Turn halo `Cycle` | not built; a static halo at alpha 0.8 instead (`Cycle.still` exists, `rune-motion/src/cycle.rs:12`, SOURCE, so do not depend on it) |
| Score | plain text at the final value; no `RollingText` roll |
| Fades | allowed: ≤ 0.2 s opacity only (Rune's own enter/exit under reduce motion) |

### 2.6 Deterministic headless snapshots at a given time

- **One clock.** All motion time is `AppState.clock`, which is set from `poll(t)`. Headless runs drive `t` from the virtual clock, so `--at T` is exact. Never read `Instant::now`, `SystemTime` or a random number generator in motion code. Confetti and sparks use the spike's integer `hash(n)` (S8) seeded by banner id and piece index.
- **Lazy stamps.** A scene sets state only (`scenes.rs`, `Scene = (&str, fn(&mut AppState))`). Its shots sit in the queue with `t0 = None` until the first poll at t ≈ 0. So `--scene table-play-opp --at 0.25` is mid-flight every time.
- **Scenes start animated.** `TableScreen::showing` (app.rs:125) shows a first view, which never animates (the rule at present.rs `after_change`). Lane A adds `TableScreen::showing_then(prev: SyncEvent, next: SyncEvent)`. It shows `prev` as the first view, then applies `next` (with its steps) through the normal path. Each animated scene in §6 is `showing_then` over `scenes::drive` (scenes.rs:82).
- **Reduce-motion variants.** A scene named `…-reduced` sets `state.prefs.reduce_motion = true` before `showing_then`.
- **Strips.** `--strip out.png --crop x,y,w,h --start S --frames N --every K --cols C --scale 1` (07 §14.1, VERIFIED) is the review artefact for every moving piece.

### 2.7 Frame budget

The figures are the stats line printed by every headless run (07 §14). They are CPU-only, debug profile (deps at opt-level 2, root `Cargo.toml:97-105`), size 1280x800, scale 1. The spike's measurements are the reference: flight 0.12/0.27 ms; one-commit sweep 0.33/0.75; staggered sweep 0.63/1.19; deal 0.91/6.1, where the p95 was the first image upload.

| Limit | Value |
|---|---|
| Animating frame CPU p95, any table scene | ≤ 4.0 ms |
| Animating frame CPU p95, the first deal of a run | ≤ 8.0 ms (image upload; see the Lane D warm-up) |
| Animating frame CPU p50 | ≤ 1.5 ms |
| Live poll-driven flyers | ≤ 4 at once |
| Canvas | ≤ 28 spark strokes + 120 confetti quads per frame |
| `backdrop_blur` | ≤ 2 per frame (Rune max is 4, `rune/src/el/backdrop.rs:34`) |
| `Cycle` | ≤ 1 (turn halo, 30 fps) |
| Idle | after the last shot + 0.5 s, only keys in `IDLE_OK = {turn-halo, seat-*-thinking*}` may change between frames (checked with `--trace`) |

Never assume 120 Hz: every pose is time-based, never frame-counted (r5).

### 2.8 Two motion mechanisms, and when to use each

| Mechanism | Use for | Proof |
|---|---|---|
| **A. Poll-driven pose.** The view computes x/y/rotate/scale from `state.clock`. The element gets `.instant_changes()` (`rune/src/el/motion.rs:181`) and `.transition(Transition::none())` (`rune-motion/src/transition.rs:194`). It lives in the fx layer (§2.10) | the Full-tier play arc, the flip, the illegal shake, sparks and confetti (canvas) | VERIFIED S1, S3-A, S5, S8 |
| **B. Matched flight.** The same `.matched(id)` (`motion.rs:215`) leaves and mounts in one commit; the spring is `Tx::motion` > `El::motion` > REFLOW (H flights.rs:190, same order at P) | sweep (staggered commits), pickup, Brisk/Skim plays, Home→Table hero | VERIFIED 07 §5.1-5.3, S4, S9 |
| C. Presence transitions | deal (`Transition::offset` + `enter_delay`), panels, route, banners, bubbles | VERIFIED S2, S9, S10 |
| D. `Sequence` / `Beat` (opacity and uniform scale only) | pops, rings, stamps, pulses, ping | VERIFIED S4, S6, S7 |
| E. `Cycle` | the turn halo only | VERIFIED S7 |
| F. `Live` flights (`live_offset`/`live_rect`, d1 tier B) | **not used.** SPIKE-FIRST; there is no `live_rotate` | |

Verified rules that bind both A and B:

- **`enter_delay` does not stagger matched flights.** To stagger, send one card per commit with `wake_at` (S4). The existing collect already does this at 45 ms (present.rs `poll`).
- **A one-commit multi-flight ignores the source `paint_order`.** Staggered commits fix it (S4).
- **A re-keyed element needs `.transition(Transition::none())`,** or it cross-fades and ghosts (S4, S7).
- **A matched flight carries rect, velocity, radius, fill and opacity, not rotation** (07 §15 #17, S9). The rotation snaps. Give the source and destination equal rotations, or use mechanism A.
- **Matched flights are disabled under reduce motion, `tx.instant()` and `tx.settle()`** (07 §5.5, `tx.rs:513,553`).
- **Shadow geometry animates on `.motion`, not on `.offset_motion`, and only while the shadow count stays constant.** Every animating card always emits exactly 3 shadows (S5, S11; `card_shadows` in §3.6).

### 2.9 rune-fx: not in this wave

Reasons:

- Grain, glow and aura need a compositor (`App::compositor`, `rune/src/app.rs:539`) plus GPU pipelines that warm up lazily in 20-150 ms (r3).
- Every effect here has a core equivalent:
  - grain: a noise `Pixels` image
  - glow: the gold `box_shadow`
  - sparkle: canvas sparks

Revisit only after the bump.

### 2.10 The fx layer

One full-window layer, the last child of the `table` column before dialogs (`table/mod.rs` `view`):

```rust
el().key("fx-layer").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).raise().a11y_hidden()
    .child(fx::layer(&ctx))   // Lane B: sparks (first), flyers, stamps, then confetti (last)
```

- **Coordinates are window points.** Endpoints come from `cx.layout(key)` (`rune/src/runtime/layout_read.rs:239`) on persistent keys:
  - `seat-{i}-fan` (stage.rs `fan`)
  - `hole-{card}` (hand.rs `rack`)
  - `trick` (stage.rs `trick`)
  - `stack-{i}` (stage.rs `stack`)
  - `deck-{hand}` (stage.rs `bid_area`)
- **SPIKE-FIRST: does `cx.layout` give window coordinates for nested keys on the first frame after a change?** Proof: test `A-layout-rects`. Fallback: when `cx.layout` returns `None` or the rect is stale, the shot plays as Brisk (matched).
- **SPIKE-FIRST: does a layer with no handlers pass clicks through?** Proof: test `A-fx-layer-passes-clicks`. Fallback: mount the layer only while a flyer or burst is live.

---

## 3. Motion tokens

All of these live in **new** `src/theme/motion.rs`, created in wave 0, with this content verbatim. It replaces the inline `pub mod motion` at `src/theme.rs:689-699`. `FLIGHT`, `PLAY`, `UI` and `TOAST_SECS` keep their values.

`theme::felt::timing` (`src/theme/felt.rs:371-397`) stays frozen. Those constants are the 06 §6 contract and present.rs uses them.

### 3.1 Durations (seconds)

| Token | Value | Use | Source |
|---|---|---|---|
| `PLAY_TRAVEL` | 0.55 | arc travel; 0.46 read rushed | S1 |
| `PLAY_SETTLE` | 0.40 | rotation wobble and squash after touchdown | S1 |
| `PLAY_END` | 0.95 | = travel + settle; the flyer hands off here | S1 |
| `SQUASH_T` | 0.16 | touchdown squash window | S1 |
| `DEAL_IN_PACKET` | 0.055 | gap between cards in one packet | S2, d1 |
| `DEAL_PACKET_GAP` | 0.110 | gap between packets (3-2 / 2-3) | d1 (gap value only; the mechanism is S2) |
| `FLIP` | 0.36 | width squeeze turn | S3-A |
| `FLIP_BRISK` | 0.18+0.18 | the two-commit tween variant | S3-B |
| `RING_DELAY` | 0.05 | after the 4th card lands | d2 |
| `GATHER_LEAD` | 0.30 | gather starts this long before the sweep | d3 |
| `SWEEP_SECS` | 0.60 | one sweep card's flight at `SWEEP` (used by `Collect::finish_at`) | S4 |
| `SWEEP_STAGGER` | 0.045 | = `timing::COLLECT_STAGGER`, one card per commit | 06 §6.4, S4 |
| `SHAKE` | 0.36 | illegal shake | S5 |
| `SCORE_STEP` | 0.14 | one point per step in a roll | S6 |
| `STAMP_HOLD` | 1.10 | euchre/march stamp on screen | d2 |
| `SPARKS` | 0.90 | canvas burst life | S8 |
| `CONFETTI_LIFE` | 2.6-3.6 | per quad, fade over the last 0.5 | S8 |
| `FF_SECS` | 0.14 | fast-forward | d1 |
| `ROUTE_SETTLE` | 0.5 | max route transition | S9 |

### 3.2 Easings

`Easing::cubic` (`rune-motion/src/easing.rs:41`); constants at `:29-36`.

| Token | Value |
|---|---|
| `PLAY_EASE` | `Easing::cubic(0.22, 0.7, 0.25, 1.0)` (06 §6.2 curve; S1) |
| `FLIP_EASE` | `Easing::EASE_IN_OUT` |
| `POP_EASE` | `Easing::EASE_OUT` |
| `SETTLE_EASE` | `Easing::OVERSHOOT` = cubic(.34, 1.45, .64, 1) |

### 3.3 Springs

`Spring::new(period_s, bounce)` (`rune-motion/src/spring.rs:30`). Settling takes 1.2-1.5× the period. A bounce under 0.3 barely reads (r2: the POP token overshoots 0.6%).

| Token | Value | Use | Source |
|---|---|---|---|
| `ROT_SETTLE` | (0.42, 0.30) | `.sample(0, 0, 1, t)` (`spring.rs:103`) for play rotation | S1 |
| `DEAL_CARD` | (0.42, 0.12) enter / (0.20, 0) exit | deal presence offset | S2 |
| `HOVER` | (0.26, 0.32) | rack plate lift, scale and shadow (`.motion`) | S5 |
| `PLAY_BRISK` | (0.42, 0.10) | Brisk/Skim matched play | 07 §5.1 range |
| `PICKUP` | (0.55, 0.15) | up card → dealer (was 0.85/0.05) | d2 |
| `GATHER` | (0.30, 0.12) | trick cards converge | S4 |
| `SWEEP` | (0.45, 0.0) | matched into the minis (was `COLLECT` 0.78/0; 0.5 VERIFIED, 0.45 inside the hold) | S4 |
| `MARKER` | (0.50, 0.20) | turn marker between seats | S7 |
| `CHIP_IN` / `CHIP_OUT` | (0.40, 0.30) / (0.30, 0) | "+2" chip | S6 |
| `ROLL` | (0.42, 0.18) | `RollingText::spring` (`rune-kit/src/rolling_text.rs:131`) | S6 |
| `ROUTE_IN` / `ROUTE_OUT` | (0.50, 0) / (0.40, 0) | screens | S9 |
| `SCRIM_IN` / `SCRIM_OUT` | (0.30, 0) / (0.22, 0) | dialog scrim opacity | S10 |
| `PANEL_IN` / `PANEL_OUT` | (0.45, 0.30) / (0.20, 0) | dialog panel | S10 |
| `HERO` | (0.60, 0.12) | Home→Table matched hero | S9 |
| `RAIL` | (0.32, 0) | settings rail slide (06 §6.5: 320 ms) | 06 |

### 3.4 Beats (`&'static [Beat]`; `rune-motion/src/sequence.rs:6-39`)

```rust
pub const WIN_POP: &[Beat] = &[Beat::new(0.0).scale(1.0), Beat::new(0.12).scale(1.09).easing(Easing::EASE_OUT), Beat::new(0.30).scale(1.0)];
pub const RING_IN: &[Beat] = &[Beat::new(0.0).opacity(0.0).scale(1.18), Beat::new(0.22).opacity(1.0).scale(1.0).easing(Easing::EASE_OUT)];
pub const PULSE:   &[Beat] = &[Beat::new(0.0).opacity(0.0).scale(0.9), Beat::new(0.12).opacity(0.9).scale(1.12).easing(Easing::EASE_OUT), Beat::new(0.5).opacity(0.0).scale(1.35)];
pub const NUM_POP: &[Beat] = &[Beat::new(0.0).scale(1.0), Beat::new(0.10).scale(1.18).easing(Easing::EASE_OUT), Beat::new(0.32).scale(1.0).easing(Easing::OVERSHOOT)];
pub const PING:    &[Beat] = &[Beat::new(0.0).opacity(0.7).scale(1.0), Beat::new(0.9).opacity(0.0).scale(1.45).easing(Easing::EASE_OUT)];
pub const THUMP:   &[Beat] = &[Beat::new(0.0).scale(1.0), Beat::new(0.08).scale(1.10).easing(Easing::EASE_OUT), Beat::new(0.22).scale(1.0)];
pub const TRUMP_IN:&[Beat] = &[Beat::new(0.0).opacity(0.0).scale(0.6), Beat::new(0.18).opacity(1.0).scale(1.3).easing(Easing::EASE_OUT), Beat::new(0.42).scale(1.0).easing(Easing::OVERSHOOT)];
pub const STAMP:   &[Beat] = &[Beat::new(0.0).opacity(0.0).scale(1.1), Beat::new(0.16).opacity(1.0).scale(0.97).easing(Easing::EASE_OUT), Beat::new(0.34).scale(1.0).easing(Easing::OVERSHOOT)];
```

Notes:

- `STAMP` stays inside the Transition scale clamp of [0.9, 1.1] (`kit_rules`, `rune-motion/src/transition.rs:148`). Beats are not clamped.
- `THUMP`, `TRUMP_IN` and `STAMP` are new beat lists. They are the same mechanism, so they count as VERIFIED.
- `TRUMP_IN` peaks at scale 1.3 for a 0.6 → 1.3 → 1.0 read (d2).

### 3.5 Staggers and the cycle

```rust
pub const SWEEP_ORDER: [usize; 4] = [0, 1, 2, 3];   // slot order left, partner, right, you (present.rs Collect::new)
pub static TURN_HALO: Cycle = Cycle { period: 1.6, phase: 0.0, stops: &[(0.0, 0.35), (0.5, 1.0), (1.0, 0.35)], still: 0.8, fps: 30.0 };
pub fn deal_delay(j: usize) -> f32 { /* sum of DEAL_IN_PACKET within a packet, DEAL_PACKET_GAP between; packets [3,2,3,2,2,3,2,3] */ }
```

`Cycle` is at `rune-motion/src/cycle.rs:8`; `El::cycle` is at `rune/src/el/visuals.rs:556`.

### 3.6 Elevation (shadow ladder; the lamp is upper-left, so shadows fall right and down)

`Shadow` is at `rune/src/el/visuals.rs:569` and `box_shadow` at `:148`. Every card emits exactly three shadows (edge, drop, glow), so geometry springs (S5).

```rust
pub fn card_shadows(e: El, lift: f32, glow: f32) -> El {          // S5/S11 VERIFIED, x added (d3)
    let l = lift.clamp(0.0, 1.0);
    e.box_shadow(Shadow { y: 1.0, blur: 0.0, color: hex(EDGE).alpha(1.0 - l), ..Default::default() })
     .box_shadow(Shadow { x: 1.0 + 3.0 * l, y: 4.0 + 16.0 * l, blur: 10.0 + 26.0 * l, color: Color::BLACK.alpha(0.28 + 0.2 * l), ..Default::default() })
     .box_shadow(Shadow { y: 0.0, blur: 28.0, spread: 2.0, color: hex(GOLD).alpha(0.75 * glow), ..Default::default() })
}
```

| Level | `lift` | Where |
|---|---|---|
| E0 | 0.0 | trick card at rest, stack minis |
| E1 | 0.15 | rack card at rest |
| E2 | 0.45 | rack hover |
| E3 | 0.65 | rack selected |
| E4 | `sin(π·u)` (peak 1.0) | flyer mid-flight |
| E5 | separate: `y 24 blur 60 black a0.45` | dialogs, banners, stamps (S10) |

`EDGE = "#c4c7b9"`, which is the existing `SHADOW_CARD_EDGE` (theme.rs:177).

### 3.7 Z-layers (tree order, then `.raise()`; `paint_order` only among siblings, `motion.rs:142`)

| Z | Layer | Key |
|---|---|---|
| 0 | table background | `table-layout` fill |
| 1 | rail | `felt` parent (stage.rs `felt`) |
| 2 | felt surface, vignette, grain, lamp pool | children of the felt, before content |
| 3 | seat plates, fans, stacks, deck | stage `content` |
| 4 | trick cards at rest | `trick` |
| 5 | rack | `rack` |
| 6 | banners | `banner-layer` (already raised, `table/mod.rs` `banner`) |
| 7 | fx layer: sparks → flyers → stamps → confetti | `fx-layer`, raised |
| 8 | settings rail, scrims, dialogs | existing `settings-layer`, `rules`, `hand-review` |
| 9 | toasts, reconnect notice | root, app.rs `view` |

Matched flights draw above the whole window while flying (07 §5.1). The sweep therefore also needs `tx.unclipped_flights()` to cross the `trick` and seat clips (S4).

### 3.8 Colours (new or changed; one accent: gold)

| Token | Value | File | Note |
|---|---|---|---|
| `GOLD` | `#f4d27a` | theme/motion.rs | the only accent: winner ring, turn halo, glow, selected outline (d2) |
| `GOLD_DEEP` | `#c9a24a` | theme/motion.rs | ring border on light surfaces, stamp text edge |
| `FELT_HI` | `#2d6b56` | felt.rs | radial centre highlight (S11) |
| `FELT_CENTER` / `FELT_EDGE` | `#245a49` / `#10382c` | felt.rs | unchanged |
| `RAIL_HI` | `#7a5232` | felt.rs | rail gradient top stop (S11) |
| `VIGNETTE` | black a0.42 | felt.rs | outer stop (S11) |
| `LAMP` | `#f5d98e2e` | felt.rs | lamp pool at (0.5, 0.42) (d1) |
| `SEAT_PLATE` | `#0f31238c` | felt.rs | replaces the invisible `SEAT_TINT #ffffff0a` (d3) |
| `SEAT_PLATE_ON` | `#1a4a37d9` | felt.rs | active seat (d3) |
| `SCRIM` | `#0b1d16` a0.45 | felt.rs | dialog scrim (S10) |
| `SPARK` | `#ffd75e`, `#ffffff` | felt.rs | sparks (S8) |
| `CONFETTI` | `#e0c579 #497d69 #ba5e46 #fff8df #f5d98e` | felt.rs | extends the existing 3 (felt.rs:96) |
| `WINNER` | `#eed591` → use `GOLD` | felt.rs | one gold |

---

## 4. Every gameplay moment and screen transition

Each moment lists a timeline (t = shot start), the mechanism letter (§2.8), a verified sketch, its proof, and its reduced and Brisk/Skim behaviour.

### 4.1 Deal

**Timeline:**

- 0: the deck tile pops with `THUMP`.
- Then 20 cards leave the deck in packets, in deal order from the dealer's left. Each card mounts with an offset transition from the deck centre.
- Rack cards (yours) fly face-up (S2).
- Each fan back lands rotated to its fan tilt. Presence carries no rotation, so the back mounts already rotated.
- The last card lands at about `deal_delay(19) + 0.5 ≈ 1.95 s`.

**Mechanism C** (S2 VERIFIED: 20 cards in one commit, 0.91/6.1 ms).

```rust
// hand.rs rack (Lane C) and stage.rs fan (Lane B): j = global deal index of this card
let [dx, dy] = deck_centre_minus_own_centre;              // from cx.layout("deck-{hand}") and the cell geometry
el.transition(Transition::offset(dx, dy).springs(SpringPair::new(motion::DEAL_CARD, Spring::new(0.2, 0.0))))
  .enter_delay(motion::deal_delay(j))                     // rune/src/el/enter_delay.rs:36
  .paint_order(20 - j as i16)                              // first dealt over the waiting deck
```

- A re-deal must use new keys (`rack-{hand}`, `back-{hand}-{k}`), or the cards will not re-enter (S2).
- Rack cards compute their own centre from `cx.layout("rack")` plus the slot index × `cell_w`. Today's opacity + scale 0.92 deal-in (hand.rs `rack`) becomes this.
- Brisk: delays ×0.5, no deck pop. Skim/Snap: no transition. Reduced: none (already gated by `deal = … && !cx.reduced`).

### 4.2 Up-card turn (and turned down)

**Timeline:** the Flip shot starts at Deal end − 0.10 and runs 0.36 s. The width squeezes, the face swaps at u = 0.5, and the card rises to scale 1.1 at mid-turn. At the end it gets a one-shot gold glint: `glow` 0 → 0.6 → 0 over 0.4 s via `card_shadows(…, glow)`. "Turned down" plays the same flip back to the back face, then the up card drifts into the deck (matched, existing up-id logic, stage.rs `up_id`).

**Mechanism A** (S3-A VERIFIED):

```rust
// fx.rs or stage.rs bid_area (Lane B); u from director.progress
let u = motion::FLIP_EASE.eval(p);
let wide = (w * (PI * u).cos().abs()).max(1.0);           // floor at 1 pt
let lift = (PI * u).sin();
let art = el().key("up-art").absolute((w - wide) / 2.0, 0.0).size(wide, h).radius(r);
let art = if u >= 0.5 { art.fill(PAL.card_white).pixels(face_px) } else { back(art) };   // .pixels stretches (visuals.rs:215)
el().key(format!("up-{hand}")).size(w, h).scale(1.0 + 0.1 * lift).instant_changes()
    .child(motion::card_shadows(art, lift, 0.0))          // fixed outer element; the art squeezes
```

Brisk: two-commit tween variant B, `tx.motion_curve(Curve::tween(0.18, EASE_IN))` (`tx.rs:536`, `rune-motion/src/curve.rs:23`). Skim/Snap/reduced: the face shows at once.

### 4.3 Card play (the hero moment)

**Timeline (Full):**

- 0: the flyer appears at the source.
  - Opponent: the top back of the `seat-{i}-fan`; `s0` = back width / trick width ≈ 0.4; `arc` 110.
  - You: the `hole-{card}` rect; `s0` = rack width / trick width; `arc` 40.
- It travels on a quadratic Bézier for 0.55 s with `PLAY_EASE`. The rotation springs from the source tilt (opponent: the fan tilt − 0.10; you: 0) to the slot turn (`t::SLOTS[..].2`) via `ROT_SETTLE`. The lift is `sin(π·u)` (shadow E4, scale +14%).
- 0.55: touchdown. Squash `0.035·sin(π·land)` over 0.16 s. The `Cue::Place` sound fires here, not at enqueue. Move it from `on_step` to the shot landing.
- 0.95: hand-off. The flyer unmounts and the static trick card mounts with `.transition(Transition::none())` in the same commit.

The trick slot is **not rendered** while its Play shot is queued or live (like `has_moved`, stage.rs `trick`). The fan renders `hand_counts[seat] + director.unlaunched_from(seat)` backs, so a back leaves only when its flyer launches.

**Mechanism A** (S1 VERIFIED, 0.12/0.27 ms):

```rust
// choreo.rs (Lane A): pure pose, copied from the spike's pose() (scratchpad/motion-spike/src/main.rs:230-248)
pub fn play_pose(f: &PlayFlight, t: f32) -> Pose {
    let u = PLAY_EASE.eval((t / PLAY_TRAVEL).min(1.0));
    let (cx, cy) = ((f.from[0] + f.to[0]) * 0.5, (f.from[1] + f.to[1]) * 0.5 - f.arc);
    let a = 1.0 - u;
    let x = a * a * f.from[0] + 2.0 * a * u * cx + u * u * f.to[0];
    let y = a * a * f.from[1] + 2.0 * a * u * cy + u * u * f.to[1];
    let lift = (PI * u).sin();
    let (rp, _) = ROT_SETTLE.sample(0.0, 0.0, 1.0, t);
    let land = ((t - PLAY_TRAVEL) / SQUASH_T).clamp(0.0, 1.0);
    let squash = if t > PLAY_TRAVEL { 0.035 * (PI * land).sin() } else { 0.0 };
    Pose { x, y, rot: f.r0 + (f.r1 - f.r0) * rp, scale: (f.s0 + (1.0 - f.s0) * u) * (1.0 + 0.14 * lift) * (1.0 - squash), lift }
}
// fx.rs (Lane B): the flyer element
el().key(format!("fly-{}", shot.key)).absolute(p.x - w / 2.0, p.y - h / 2.0).size(w, h).radius(r)
    .rotate(p.rot).scale(p.scale).origin(0.5, 0.5)
    .instant_changes().transition(Transition::none())
    .child(motion::card_shadows(stage::face(state, card, w), p.lift, 0.0))
```

- The **opponent's card shows its face from launch.** That matches today's matched flight, which draws the new picture (07 §15 #17).
- d1/d2 "flip mid-flight" is SPIKE-FIRST: S3-A in a flyer is untested. If tried, swap at u = 0.35 with a width squeeze over 0.12 s.
- **Brisk/Skim:** render the trick slot at once with `.matched(play_id)` and `.motion(PLAY_BRISK)`. This is today's path (stage.rs `trick`, `fan`; 07 §5.1/5.2 VERIFIED).
- **Reduced/Snap:** the slot mounts in place.

### 4.4 Trick won (ring, gather, sweep, thump) inside the 3.3 s hold

**Budget** (t = the 4th card's Play start, about when its sync arrives; the server hold is 3.30 s from about the same moment, 06 §6.1):

| t (s) | Beat | Mechanism |
|---|---|---|
| 0.55 | 4th card touches down | A |
| 0.60 | the winner card pops with `WIN_POP`. The ring enters with `RING_IN` (3 pt `GOLD` border, inset −6). The winner's seat plate → `SEAT_PLATE_ON`. The result line changes (§4.8) | D |
| 0.95 | flyer hand-off (the ring is a child of the static card, so it mounts with it; before 0.95 the ring rides the flyer) | |
| 1.90 | gather: the four cards move to the trick centre, offset 18 pt toward the winner, rotations to (i − 1.5)·0.05, `.motion(GATHER)` | layout spring, S4 |
| 2.20 | sweep: `timing::COLLECT_AFTER` from the first poll after the winner shows (unchanged present.rs logic). One card per commit at 45 ms → into 24x34 minis at opacity 0 (`stack`, stage.rs), `.motion(SWEEP)`, `tx.unclipped_flights()` | B, S4 |
| 2.335 | 4th card sent | |
| ≈ 2.95 | last card lands, about 0.6 s at SWEEP 0.45. `Cue::TrickTake`. The stack glyph `THUMP`s, re-keyed `stack-glyph-{moved}` + `Transition::none()` | D |
| 3.30 | server hold ends | |

**Bug fix:** today the winner outline shows as soon as `winner_seat` is known, before the 4th card lands (r6). Gate it with `present.winner_shown(view)`, which is true once the `Win` shot has started. `Win` waits for its trick's 4th Play landing (§2.2).

`Collect::finish_at` (present.rs) changes `timing::COLLECT` to `motion::SWEEP_SECS = 0.60` for Continue. In review mode (`c.review`), the gather and sweep start on Continue exactly as today.

```rust
// stage.rs trick (Lane B), winner slot after its Win shot started
let card = motion::card_shadows(face, 0.0, 0.0).transition(Transition::none());
let card = if won && pop_live { card.key(format!("{key}-pop")).sequence(Sequence::new(motion::WIN_POP)) } else { card };
let mut slot = slot.motion(motion::GATHER).child(card);
if won && ring_shown {                                     // ring_shown = present.winner_shown(view)
    slot = slot.child(el().key("win-ring").absolute(-6.0, -6.0).size(w + 12.0, h + 12.0)
        .radius(r + 4.0).border(3.0, hex(motion::GOLD)).sequence(Sequence::new(motion::RING_IN)));
}
// stage.rs stack: unchanged except .motion(SWEEP); app::poll: if out.unclipped { tx.unclipped_flights() }
```

- **Reduced:** the ring is shown static. No gather. The sweep is instant (existing `reduced` branch in present.rs `poll`).
- **Brisk:** no gather, sweep unchanged.
- **Skim:** sweep in one commit (`moved = plays.len()`) with `tx.instant()`.
- **d1's "sweep anticipation 80 ms" wind-up is dropped:** unverified, and it eats into the hold margin.

### 4.5 Turn indicator

- One keyed `turn-marker` per table: a 2 pt gold ring plus a halo. It moves between seat plates with a layout `.motion(MARKER)` (S7).
- The halo is a gold `box_shadow` (blur 30, spread 4, alpha 0.75) with `.cycle(TURN_HALO)`. It shows **only on your turn**, so AI thinking stays idle-cheap. The existing thinking dots (`LoopSpec`, stage.rs `seat`) stay.
- Each turn change re-keys `ping-{turn_seq}` with `Sequence(PING)` and `Transition::none()`.
- Reduced: static halo at alpha 0.8, no ping, no marker spring (Rune lands it).

### 4.6 Bid bubbles, trump called, pickup

- **Bid bubble.** A chip under the seat name with the bid word. It enters with `Transition::offset(0, 6).combined(Transition::opacity())`, springs `CHIP_IN/OUT`, and lasts 1.6 s or until the next bid. Mechanism C.
- **Trump called.** The trump badge in the status row mounts keyed `trump-{hand}` with `Sequence(TRUMP_IN)`. The maker chip mounts with `THUMP`. Mechanism D.
  - The felt wash is SPIKE-FIRST (d3): a radial overlay in the suit's tint at alpha 0.08, fading in 0.6 s (`radial_gradient` opacity animation; only one gradient per element, `visuals.rs:164/190`). Fallback: none.
- **Pickup** (dealer takes the up card). This is the existing matched `up_id` → dealer fan or your rack (stage.rs `bid_area`, `fan`), with `PICKUP` retuned to (0.55, 0.15). The 06 §6.3 arc is not expressible as a matched path. Accept the straight line (07 §6 row 6.3). The rotation snaps, so give the source and destination equal tilt.
- Reduced: all of these appear in place.

### 4.7 Illegal card

- The rack card shakes by `x = 11·exp(−t/0.11)·sin(2πt/0.12)` for 0.36 s, with rotation `x·0.004` (S5). It is poll-driven (`present.illegal` gains `shake_t0: Option<f64>`, stamped lazily).
- The card carries `.instant_changes()` **only while it shakes**. Otherwise the hover spring would be lost.
- The message is unchanged (3.2 s).
- **Legal dim.** On your play turn, a card you may not play renders its art at opacity 0.55 and gets no hover lift (S5, d2).
- Reduced: no shake; the message only.

### 4.8 Result line and hand scored

- **Result line.** Today it hard-swaps (r6). It becomes a keyed `result-{hand}-{trick}-{phase}` text with `Transition::offset(0, 8).combined(Transition::opacity())` enter and opacity exit, springs (0.3, 0)/(0.2, 0). Mechanism C.
- **Hand scored.** This replaces the empty `table-hand-scored` (r6). A hand-result card sits over the trick area, keyed `hand-result-{hand}`:
  - entry: `PANEL_IN/OUT` (asymmetric scale 0.9 + offset(0, 30) + opacity; S10)
  - a title, using the strings from 06 §10/§9
  - the makers' tricks as five pips (filled = won)
  - the points chip "+N", via `CHIP_IN/OUT` with asymmetric offset(0, 14) in and offset(0, −18) out (S6)
- **Score roll.** The header score number rolls with `RollingText::new(key, n).spring(ROLL).value(n as f64)` (`rune-kit/src/rolling_text.rs:110,131,145,159`) and `TextStyle::features("'tnum' 1")`. Clip the cell, because digits spill below the line mid-roll (S6). It rolls one point per `SCORE_STEP` (the value steps in poll). At the end, a `PULSE` ring and a `NUM_POP` fire on the number.
  - Launching the "+N" chip into the score (`Tx::launch` → RollingText) is SPIKE-FIRST. Fallback: the chip fades in place.
- **Stamps** for Euchre, March and Lone march (d2). A centred stamp over the trick, keyed `stamp-{hand}`:
  - serif 44/600, `GOLD_DEEP` 2 pt border, cream fill, E5
  - `Sequence(STAMP)`, hold `STAMP_HOLD`, exit by opacity
  - canvas sparks (§4.9) mount **before** the stamp, at its centre
- Reduced: card and stamp static; the score as plain text.

### 4.9 Bursts

- **Sparks** (S8 VERIFIED): `canvas(sparks(t, cx, cy))` (`rune/src/el.rs:359`, `Shape::stroke` `shape.rs:62`), 28 streaks, 0.9 s, drag `exp(−4t)`, gravity 160. Mount it before the card or stamp it surrounds, and with `.instant_changes()`. Fires on a stamp, and on your trick win that seals a march.
- **Confetti** (S8 VERIFIED): 120 quads from two bottom cannons, `confetti(t, w, h)` from the spike (`main.rs:668-700`), mounted **last** in the fx layer. Fires on `GameWon` when your team wins, and on March. It replaces the 24-piece `stage::confetti` (stage.rs:986). `Banner.confetti_until` becomes `t0 + 3.7`.
- Reduced: neither is built.

### 4.10 Match end

- **Win.** The banner becomes a centred match-end panel (`PANEL_IN`), placed at 38% height. That fixes the low game-over banner (r6). It has a `SCRIM` without blur, confetti, and a final score roll.
- **Loss.** The same panel with no confetti. The felt cools: the felt's `FELT_HI` goes to `FELT_CENTER` over 0.8 s (d3). Gradient stops do not spring, so this is SPIKE-FIRST. Fallback: an overlay of `#0b1d16` a0.18 fading in by opacity (VERIFIED mechanism).

### 4.11 Rack hover, select and press (plate-child pattern, S5/S11 VERIFIED)

```rust
// hand.rs rack (Lane C): the outer plate springs; matched stays on the art (Brisk/Skim and pickup)
let l = if selected { 0.65 } else if hovered { 0.45 } else { 0.15 };
let plate = el().key(format!("plate-{c}")).size(w, h).radius(r).origin(0.5, 1.0)
    .motion(motion::HOVER)                               // shadows spring on .motion only
    .offset(0.0, if selected { -18.0 } else if hovered { -10.0 } else { 0.0 })
    .scale(if selected { 1.06 } else if hovered { 1.04 } else { 1.0 })
    .press_scale(0.96);                                  // visuals.rs:350
motion::card_shadows(plate, l, if selected { 0.6 } else { 0.0 })
    .child(face(state, c, w).key(format!("card-{c}")).matched(format!("hcard-{}-{c}", hand)).motion(motion::PLAY_BRISK))
```

- Selected = `GOLD` glow 0.6 plus the existing 3 pt `focus_felt` outline.
- Hover comes from `on_hover_change` (`rune/src/el/layout.rs:676`; hand.rs already wires it).
- 06 §6.5's −4/−8 px lifts are superseded by −10/−18. Owner sign-off item S-1.
- Reduced: no lift, no scale (Rune lands them anyway). The glow still shows on select.

### 4.12 Screen transitions

- **Route.** Use keyed screens (app.rs `view` already keys `screen-{route}`) with a direction-aware transition:
  - forward: `Transition::asymmetric(Transition::offset(70, 0).combined(Transition::opacity()), Transition::offset(-70, 0)).springs(SpringPair::new(ROUTE_IN, ROUTE_OUT))`
  - back: the mirror
  - `transition.rs:250,254,261`, S9 VERIFIED
  - Exit ghosting lasts about 0.3 s: acceptable.
  - Direction: `AppState.route_dir: RouteDir`, set in `navigate`/`back` (app.rs:324-337).
  - Lane D provides `screens::route_transition(dir, reduced) -> Transition`. Reduced → `Transition::opacity()`.
- **Home → Table hero** (S9 VERIFIED with caveats). The home art's Jack of spades carries `.matched("hero-JS")`. The loading table shows one card with the same id; `HERO` spring plus `tx.unclipped_flights()`. The rotation snaps, so both get `rotate(0.0)`. P2.
- **Dialogs** (rules, hand review, settings narrow overlay), S10 VERIFIED:
  - the scrim gets `.backdrop_blur(10.0).fill(hex(SCRIM).alpha(0.45))` with opacity transition `SCRIM_IN/OUT`. The scrim's opacity also fades the blur.
  - the panel gets `PANEL_IN/OUT`
  - this replaces `panels::scrim` and `panels::sheet` bodies (panels.rs:601,619)
  - Reduced: opacity only, no blur.
- **Settings rail.** `Transition::offset(RAIL_W, 0).combined(Transition::opacity())` with `RAIL`, using the S9 mechanism (offset presence).
- **Table build-in** (first table view after the route, d3): the seats enter with `Stagger::new(0.04)` opacity + offset(0, 8), the rack after 0.2. Mechanism C. P2.

---

## 5. Visual redesign per screen

Tokens: §3 for motion and colours; existing `theme.rs`/`felt.rs` for the rest. Sizes are in points at the default window.

### 5.1 Table: felt and rail (Lane B, `stage.rs::felt`, `felt.rs`)

S11 VERIFIED except where marked. Layer stack inside `felt`:

1. **Rail.** `pin 10, radius 32, fill RAIL_BOTTOM, linear_gradient(PI, [(0, RAIL_HI), (0.12, RAIL_TOP), (1, RAIL_BOTTOM)])`, shadow y12 b34 black a0.5. Brass line 1 pt (`BRASS`) at inset 3.
2. **Felt.** `pin 14 inside the rail, radius 20, radial_gradient(centre [0.5, 0.46], radius [560, 340], [FELT_HI, FELT_CENTER, FELT_EDGE])`, border 1 pt `BRASS` a0.55.
3. **Lamp pool.** A child element `radial_gradient([0.5, 0.42], [420, 260], [LAMP, transparent])` (d1). It is a separate child, because one gradient slot per element (`visuals.rs:164/190`).
4. **Vignette.** A child `radial_gradient([0.5, 0.5], [620, 400], [(0, transparent), (0.62, transparent), (1, VIGNETTE)])`.
5. **Grain.** A 256×256 noise `Pixels` (alpha ≈ 18/255, the spike's `noise_rgba`, `main.rs:1013`), tiled by stretching over the felt with `.pixels` (`visuals.rs:215`). Generated once in `setup`.
6. **Watermark.** Use the unused `FELT_WATERMARK` (theme.rs:119): the club glyph at 220 pt, centred, under the trick.
7. **Inset rim shadow** (y3 b22 spread 2 a0.55): SPIKE-FIRST (an inset `Shadow` exists, `visuals.rs:576`, but the spike did not confirm it). Fallback: skip.
8. **d3's U-shaped rail with negative pin insets:** SPIKE-FIRST. Fallback: the rounded rail above.

At rest, the felt draws nothing per frame. It is static, and every layer is cached by Rune.

### 5.2 Table: seats (Lane B, `stage.rs::seat`, `fan`)

- **Seat plates.** Rest `SEAT_PLATE`, radius 14, padding 8/12. Active `SEAT_PLATE_ON` with a 1 pt `GOLD` a0.5 border.
  - d3's `border_gradient` gold: SOURCE-only, skip.
  - The plate fill change springs via `.motion(Spring::new(0.3, 0))` (fill animates with motion; r1). This retires `SEAT_TINT` (felt.rs:46).
- **Fans.** Backs 30×42 (d3; was 14×20, r6), step 9, tilt 0.08/card about the bottom centre (existing). **One back image everywhere** (r6: mixed backs). `stage::back` uses `assets` `back.png` only.
- **Names.** `NAME_MAX_W` 130 → 150; two lines allowed at `SEAT_TIGHT_W` (fixes "Gran…" truncation, r6).
- **Captions** (seat detail words) show on hover of the plate only (d3), using `on_hover_change`.
- **Stack.** Glyph 24×16 unchanged; `THUMP` on landing (§4.4).

### 5.3 Table: trick area (Lane B, `stage.rs::trick`)

- **Trick card size.** Unchanged rule (`PLAYED_H_SHARE`, `PLAYED_CAP`). Shadows are `card_shadows(…, 0, 0)` (E0), replacing `fshadow::PLAYED`.
- **Winner ring.** `GOLD` 3 pt at inset −6 (§4.4), replacing `outline(WINNER_OUTLINE)`.
- **Led-suit pill** (d2). A small pill `LED_PILL` (felt.rs:44) under the first card: "Led ♠". Opacity transition. Mechanism C.
- **Currently winning** (d2). A 1.5 pt `GOLD` a0.45 outline on the currently-winning card while the trick is open. It moves by re-render, with no motion. The engine already exposes the trick order: compute it with the engine's trick-winner function. Lane B must confirm the function name in `crates/euchre-engine` before building this; if there is no such function, drop the piece.

### 5.4 Table: rack and controls (Lane C, `hand.rs`, `panels.rs`)

- **Rack card width.** `clamp(112, 0.092·W, 140)` (d3; was a fixed max `HAND_W = 112`). The rack must not be smaller than the trick cards (r6: 98×137 rack vs larger trick cards).
- **Holes.** Become card-sized invisible cells, `.size(cell_w, w / ASPECT)` keyed `hole-{c}`. Today they are 1 pt tall (hand.rs `rack`). This is the flyer origin (§2.10).
- **Captions.** The card name shows only on hover or select (d3). The sub line ("Must follow suit", "Selected ✓") stays.
- **Trump pip.** A 12 pt suit glyph at the card's top-right on trump cards (d2), `TRUMP_GLYPH` (felt.rs:26).
- **Play button.** Hidden until a card is selected. It enters with offset(0, 8) + opacity (d3).
- **Felt buttons** get key-button physics (d3): `press_scale(0.96)`, `.motion(Spring::new(0.2, 0.3))` on hover offset −1.
- **Bid copy overflow** (r6: 137.5 pt text in a 123.6 box). The bid panel copy wraps with `max_lines(3)` and the box grows to its content. Keep the existing test `the_bid_copy_stays_in_its_box_beside_your_seat` (tests/table.rs:654).

### 5.5 Header (Lane C, `header.rs`)

- Score numbers become `RollingText` (§4.8), with the `PULSE` ring behind.
- The trump badge in the header mirrors the status row (`TRUMP_IN`).
- Spacing is unchanged. `HEADER_H` 64/52.

### 5.6 Dialogs and the settings rail (Lane C, `panels.rs`)

- Scrim and panel as in §4.12. Panel radius 18, `CREAM` fill, 1 pt white a0.6 border, E5.
- **Settings clipped at 720 wide** (r6). Below `RAIL_INLINE_MIN_W`, the rail uses `w(min(RAIL_W, width − 32))`. The overlay path (table/mod.rs) already exists; fix the width math in panels.rs.

### 5.7 Home, New game, Games, Settings, Match detail (Lane D)

- **Home** (`screens/home.rs`):
  - The hero card art keeps its layout.
  - Hero cards get E2 shadows and a slow parallax on pointer move. The parallax is SPIKE-FIRST (no pointer-move API was verified). Fallback: static.
  - Buttons use the key-button physics (§5.4).
  - The hero Jack of spades carries `matched("hero-JS")` (§4.12).
- **New game** (`screens/new_game.rs`):
  - Option cards with a selected state: a `GOLD_DEEP` 2 pt border on cream; springs `Spring::new(0.25, 0.2)`.
  - The start button `press_scale(0.96)`.
- **Games, Settings, Match detail** (`screens/other.rs`):
  - List rows enter with `Stagger::new(0.03).delay(i, n)` (r4 kit-gallery pattern; `rune-motion/src/choreo.rs:26,52`), opacity + offset(0, 6).
  - Reduce motion: a toggle row already exists (other.rs:280).
- **Ace of spades has a baked QR code and URL** in its art (r6). This is an asset fix, not code. Owner decision S-2: replace the PNG in `apps/web/static/art/cards`, or overdraw a cream corner rect in `stage::face` for `AS` only.
- **Image warm-up** (`src/assets.rs`): in `setup`, after decode, draw every face and back once in a hidden 1×1 strip on the first frame. SPIKE-FIRST: whether Rune culls an opacity-0 element before upload. Proof: first-deal p95 ≤ 8 ms (§6). Fallback: accept the first-deal spike.

---

## 6. Implementation plan

### 6.0 Wave 0: contract (Lane A, one serial change before the lanes fork; start after the current desktop fix lands)

1. **Tokens.** Create `src/theme/motion.rs` with §3 verbatim. In `src/theme.rs`, replace the inline `pub mod motion { … }` (theme.rs:689-699) with `pub mod motion;`.
2. **Director API.** Create `src/screens/table/choreo.rs` with the public API below, stubbed so that `feel()` returns `Snap`. Behaviour stays identical to today until Lane A fills it in.

   ```rust
   pub fn flyers(&self, now: f64) -> impl Iterator<Item = (&ShotKey, &PlayFlightSpec, f32 /*t*/)>;
   pub fn hides_slot(&self, hand: u32, index: u8, seat: Seat) -> bool;
   pub fn unlaunched_from(&self, seat: Seat) -> u8;
   pub fn started(&self, key: &ShotKey) -> Option<f64>;          // ring/pop/stamp t0
   pub fn progress(&self, key: &ShotKey, now: f64) -> Option<f32>;
   pub fn feel(&self) -> Feel;
   pub fn play_pose(f: &PlayFlight, t: f32) -> Pose;              // pure
   ```
3. **Presentation.** `Presentation.director: Director`. `PollOut.frame` and `PollOut.unclipped`, wired in `app::poll`.
4. **Layout helpers.** `stage::slot_rect(view, seat, trick_rect, h) -> ([f32; 4], f32)` is extracted from `stage::trick` as a pure move, done by Lane A in wave 0 only. Also `hand::hole_key(c)`.
5. **fx module.** `screens/table/fx.rs` with `pub fn layer(ctx: &stage::Ctx, cx: &Cx<AppState>) -> El` returning an empty layer. It is mounted in `table/mod.rs`.
6. **Scenes.** All §6.5 scenes in `scenes.rs` via `TableScreen::showing_then`.
7. **Test harness.** `tests/motion.rs` with `presented_at(rt, key, t)`, `trace_idle(rt, secs)` and `stats(args)` helpers.

Gate: `bun run --cwd apps/desktop check`, `bun run --cwd apps/desktop test` and `bun run --cwd apps/desktop lint` all green. Every existing scene snapshot is pixel-identical to pre-wave-0 (no behaviour change).

### 6.1 File ownership after wave 0 (disjoint)

| Lane | Owns (edit only these) | Reads |
|---|---|---|
| **A: Choreography and harness** | `src/screens/table/choreo.rs`, `src/screens/table/present.rs`, `src/screens/table/mod.rs`, `src/app.rs`, `src/scenes.rs`, `src/theme/motion.rs`, `tests/motion.rs`, `tests/table.rs`, new `scripts/*.json` | everything |
| **B: Felt, trick and fx** | `src/screens/table/stage.rs`, `src/screens/table/fx.rs`, `src/theme/felt.rs`, new `tests/felt.rs` | motion.rs, choreo.rs API |
| **C: Rack, header and panels** | `src/screens/table/hand.rs`, `src/screens/table/header.rs`, `src/screens/table/panels.rs`, `src/screens/table/copy.rs`, new `tests/rack.rs` | motion.rs, choreo.rs API |
| **D: Shell and screens** | `src/theme.rs`, `src/screens/mod.rs`, `src/screens/home.rs`, `src/screens/new_game.rs`, `src/screens/other.rs`, `src/assets.rs`, `src/lib.rs`, `tests/shell.rs` | motion.rs |

Cross-lane rules:

- **API changes and wiring.** A change to the choreo API, `motion.rs` or `slot_rect` is a request to Lane A. Lane A also does every one-line wiring in `app.rs` or `table/mod.rs` that another lane needs: `route_transition`, `fx::layer`.
- **Frozen constants.** `theme::felt::timing` values are frozen for every lane.
- **Shared test names.** Every lane runs `bun run --cwd apps/desktop test` before handing over. Only Lane A edits `tests/table.rs`; another lane that breaks a test there asks Lane A.

### 6.2 Lane A: choreography (Director, tiers, fast-forward, reconnect, reduced, play flyer data)

Order: Director queue/poll → `Play` + `Win` gating → sweep `unclipped` + `SWEEP` + `finish_at` → tiers → fast-forward → reconnect clear → reduced → route wiring.

Runtime tests (`tests/motion.rs`; `rt.presented` `rune/src/runtime/elements.rs:402`, `rt.frame` `frame.rs:305`, `set_system_reduced_motion` `system_settings.rs:65`):

- `A-layout-rects`: frame 1 after a `CardPlayed` sync. `fly-…` is presented within 2 pt of the `seat-{i}-fan` centre. Proves §2.10.
- `A-play-arcs`: at t = 0.275 the flyer centre is ≥ 0.3·arc above the chord from→to. At t = 1.0, no `fly-*` key exists and `trick-{h}-{i}-{seat}` is presented at its rest rect, opacity 1.
- `A-ring-waits`: `win-ring` is absent at t = 0.3 and present at t = 0.7, for a sync that carries the 4th `CardPlayed` + `TrickWon`.
- `A-sweep-inside-hold`: the last mini reaches its rest rect at ≤ 3.05 s, and Continue's `finish_at` is ≤ started + 0.74.
- `A-skim-last-only`: four `CardPlayed` syncs in one frame → exactly one `fly-*` or matched flight. The others are presented at rest within 1 frame.
- `A-fast-forward`: press a legal card while a flyer is at u = 0.3. By +0.15 s, no `fly-*` key exists.
- `A-reconnect-snaps`: Status(false) → stepless sync. `!rt.animating` after one frame (excluding `IDLE_OK`).
- `A-reduced-none`: `rt.set_system_reduced_motion(true)`. No `fly-*`, `win-ring-pop`, `sparks` or `confetti` key in any frame of a whole trick. The gather still starts at 2.2 s (keep the existing `reduce_motion_quiets_the_table`, tests/table.rs:411). The same holds with only `prefs.reduce_motion = true` (D15).
- `A-idle`: 1.5 s after the last shot, `Presentation::poll` returns `wake = None`, apart from the banner and illegal deadlines.
- `A-fx-layer-passes-clicks`: with a live burst, a click on `hand-{c}` selects the card.

Snapshots and frame time: `table-play-opp`, `table-play-you`, `table-trick-win`, `table-reconnect` from §6.5. `bun run --cwd apps/desktop test` green.

### 6.3 Lane B: felt, seats, trick visuals, fx layer

Order: felt layers → seat plates and fans → `card_shadows` in the trick → flyer render in `fx.rs` → ring/pop/gather → stack thump → up-card flip → sparks and confetti → stamps.

Runtime tests (`tests/felt.rs`):

- `B-felt-static`: `table-play` at rest, 30 frames. Trace changes only in `IDLE_OK`.
- `B-gather`: `table-trick-win` at 2.05 s. All 4 trick cards are presented within 40 pt of the trick centre.
- `B-flip-squeeze`: `table-upcard-flip` at the flip mid (0.18 s after its start). `up-art` width ≤ 4 pt. At the end, width = card width.
- `B-fans-size`: every top back is presented 30×42 ± 0.5.
- `B-confetti-bounded`: `game-over-win` from 0 to 4 s. The canvas exists only while t < 3.7. Frame p95 ≤ 4 ms.
- `B-shadow-count`: every `fly-*` and `trick-*` element has exactly 3 shadows (tree JSON).

### 6.4 Lane C: rack, header, panels

Order: card-sized holes (needed first by Lane A's origin) → plate-child hover/select → legal dim → shake → deal-in from deck → rack width rule → captions on hover → trump pip → Play button presence → result line → hand-result card → RollingText score → dialogs and scrim → settings width fix.

Runtime tests (`tests/rack.rs`):

- `C-hover-lift`: hover `hand-{c}`. At +0.5 s, `plate-{c}` is presented offset −10 ± 1 and scale 1.04. Unhover returns to rest by +0.6.
- `C-shake`: an illegal press. `card-{c}` x deviates ≥ 4 pt at +0.06 and is back within 0.5 pt at +0.4. It does not shake when reduced.
- `C-deal-from-deck`: `table-deal-anim` at 0.10. Rack card 0 is presented within 60 pt of the `deck-{hand}` centre. At 2.0 it is at rest.
- `C-score-rolls`: `table-hand-result`. The score cell shows a mid-roll offset at 0.3 s and the final digit at 1.2 s. Reduced shows the final digit at 0.
- `C-rack-not-smaller`: at 1280×800, the rack card width ≥ the trick card width.
- `C-settings-720`: at `--size 720x520`, `settings-rail` is inside the window (extends `the_narrow_settings_rail_lies_over_the_table`).
- `C-dialog-scrim`: `table-rules-open` at 0.12. The scrim opacity is between 0 and 1, and the panel scale is < 1.

### 6.5 Lane D: shell

Order: route transition fn → `route_dir` wiring request to Lane A → home hero matched id → list staggers → new-game option physics → image warm-up → Ace of spades decision.

Runtime tests (`tests/shell.rs`):

- `D-route-forward`: Home → New. At +0.15 s, `screen-new` is presented x > 0 and `screen-home` x < 0. Both are gone or at rest by +0.6.
- `D-route-back` mirrors it.
- `D-route-reduced`: no x offset at any frame.
- `D-hero-flies`: Home → Table. `hero-JS` is presented between the two rects at +0.2. SPIKE-FIRST for the loading table; skip the test if the hero is dropped.
- `D-first-deal-p95`: the stats line for `--scene table-deal-anim --frames 150` gives p95 ≤ 8 ms.

### 6.6 Named scenes (Lane A adds them in wave 0) and checks

Each animated scene is `showing_then(prev, next)`. Commands:

- Run `cargo run -p euchre-desktop --bin euchre -- …`, or `bun run --cwd apps/desktop dev -- …`, which wraps it.
- Every snap uses `--size 1280x800 --scale 1`.

| Scene | `--at` snapshots (s) | Strip | Who checks what |
|---|---|---|---|
| `table-deal-anim` | 0.10, 0.80, 2.00 (rest) | `--start 0 --frames 120 --every 6 --cols 5` | C rack, B fans |
| `table-upcard-flip` | flip start + 0.09, + 0.18 (edge), + 0.40 | `--frames 30 --every 2` | B |
| `table-play-opp` | 0.15, 0.30, 0.62 (squash), 1.00 (rest) | `--crop` trick rect, `--frames 60 --every 3` | A data, B render |
| `table-play-you` | 0.20, 0.45, 1.00 | same | A, C holes |
| `table-trick-win` | 0.40 (no ring yet), 0.75 (ring in), 2.05 (gathered), 2.30 (sweep mid), 3.10 (settled) | `--start 1.8 --frames 80 --every 4` | A timing, B visuals |
| `table-trick-win-reduced` | 0.0, 0.75, 2.30 | none | A: no motion, ring at 0.0, sweep instant at 2.2 |
| `table-pickup` | 0.25, 0.80 | yes | B |
| `table-trump-call` | 0.10, 0.20, 0.50 | no | B badge, C header |
| `table-illegal-shake` | 0.06, 0.20, 0.40 | `--frames 25 --every 1 --crop rack` | C |
| `table-hover` (via `--script scripts/hover.json`) | `mark:hover+0.1`, `mark:hover+0.6` | no | C |
| `table-hand-result` | 0.20, 0.60, 1.40 | yes | C card and roll |
| `table-euchre-stamp` | 0.10, 0.34, 0.80, 1.30 | yes | B stamp and sparks |
| `game-over-win` | 0.40, 1.50, 4.00 | `--frames 120 --every 6` | B confetti, C panel |
| `game-over-loss` | 0.40, 1.50 | no | B cool, C panel |
| `table-rules-open` | 0.12, 0.60 | no | C |
| `route-home-new` (script click) | `mark:go+0.15`, `mark:go+0.6` | yes | D |
| `table-reconnect` | 0.00 and 1.00 must be identical (diff 0 px) | no | A |
| `table-play` (existing, rest) | default | no | everyone: no regression |

Frame-time check, for every scene above:

```
euchre --scene X --frames 240 --fps 60 --out <dir> --size 1280x800 --scale 1
```

Read the stderr `rune: animating frames N (cpu p50 a p95 b ms)` line. Limits per §2.7. Record the numbers in the lane's PR description.

Idle check:

```
--trace out.jsonl --start 4.0 --frames 30
```

Only `IDLE_OK` ids may change.

Reduce-motion check, per lane: every `…-reduced` scene and `A-reduced-none` pass. Plus one manual run with macOS System Settings → Accessibility → Display → Reduce motion on, and the in-app toggle off. It must look identical to toggle-on/OS-off (D15 OR).

### 6.7 Priority (when time runs short, cut from the bottom)

- **P0:** Wave 0, card-sized holes, play arc (§4.3), ring gating (§4.4), sweep inside the hold, tiers + fast-forward + reconnect + reduced, felt layers, card_shadows.
- **P1:** deal from deck, up-card flip, hover plate, shake + legal dim, result line, hand-result card + score roll, dialogs + scrim, route transitions, seat plates + fans + one back.
- **P2:** stamps + sparks, confetti rewrite, turn marker + halo + ping, trump badge, bid bubbles, hero flight, table build-in, list staggers, led pill and currently-winning outline.
- **SPIKE-FIRST** (only with a passing proof): mid-flight face flip, launch into RollingText, inset rim shadow, U-rail with negative insets, felt cool by gradient, trump felt wash, home parallax, image warm-up strip, Live flights, rune-fx, `Cx::time_scale` pace, `motion_gate` with `MotionIntent`.

### 6.8 Owner sign-off items

- **S-1:** rack lifts −10 hover / −18 selected, replacing 06 §6.5's −4/−8.
- **S-2:** the Ace of spades QR/URL art fix (asset swap or corner overdraw).
- **S-3:** `PICKUP` 0.85 → 0.55 s and `SWEEP` 0.78 → 0.45 s spring. Both stay inside the 06 tempo contract holds; 06 §6.3/6.4 durations become "about".
- **S-4:** confetti also on March (not only on a won match).
