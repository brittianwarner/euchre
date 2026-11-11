# Report: DESKTOP MOTION / WAVE 0 (Lane A)

Crate: `apps/desktop` (`euchre-desktop`), Rune pinned at 85cd8c6. Spec: `docs/port/09-motion-visual-spec.md` §6.0, plus the lane seams the wave 0 brief asked for. Lanes B, C, D and E fork from here.

Wave 0 adds the contract and changes no behaviour. The Director runs at `Feel::Snap`: every shot lands as it is enqueued, and nothing reads the new state to draw. All 13 existing scenes are pixel-identical to before wave 0.

Nothing outside `apps/desktop` changed, apart from this report and `Cargo.lock`, which cargo updated. No git writes were made.

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/motion-a`.

| Gate | Result |
| --- | --- |
| `bun run --cwd apps/desktop check` (clippy `-D warnings`, all targets) | clean |
| `bun run --cwd apps/desktop test` | lib 44, motion 8, shell 7, table 13: all pass |
| `bun run --cwd apps/desktop lint` (`fmt --check`) | clean |

No test processes were left under the target dir (`pgrep` is empty).

## What was added (09 §6.0 items 1-7)

1. **Tokens.** `src/theme/motion.rs` holds the §3 tokens verbatim. It replaces the inline `pub mod motion` in `theme.rs`, and `FLIGHT`, `PLAY`, `UI` and `TOAST_SECS` are kept. The hex colours go through `theme::color`. Three unit tests cover it: the colours parse, `deal_delay` follows `DEAL_PACKETS`, and the durations hold together.
2. **Director.** `src/screens/table/choreo.rs` has the full public API. `feel()` is `Snap`, and `enqueue` lands at once.
3. **Presentation.** These fields are new:
   - `director`, `shake`, `dialog_since`, `score: ScoreRoll`, `deck_at`, `announced`
   - `Hooks.rules_query` and `Hooks.rules_flipped`
   - `PollOut.frame` and `PollOut.unclipped`

   `app::poll` turns `frame` into `tx.wake_after(ZERO)` and `unclipped` into `tx.unclipped_flights()`.
4. **Layout helpers.** `stage::slot_rect` is a pure move out of `stage::trick`; `trick` now calls it. `stage::deck_key(hand)` and `hand::hole_key(card)` are new; the holes and the deck use them.
5. **fx layer.** `src/screens/table/fx.rs` has `layer(ctx, cx)`, which returns an empty `el()`. It is mounted in `table/mod.rs` as specified (`fx-layer`, pinned to the window, raised, a11y-hidden), after the body and before the settings layer and dialogs. It is always mounted: the click test passes, so the fallback was not needed.
6. **Scenes.** Every §6.6 scene is added through `TableScreen::showing_then(prev, Then)`, plus Lane E's three (list below).
7. **Harness.** `tests/motion.rs` has `presented_at(rt, key, t)`, `trace_idle(rt, secs)` (with an `IDLE_OK` filter, empty for now) and `stats(args)`. `stats` runs `CARGO_BIN_EXE_euchre` and parses `rune: animating frames N (cpu p50 a p95 b ms)`. Every helper is used by a test.

### Tests in `tests/motion.rs`

- `every_motion_scene_reaches_its_moment`: each motion scene finds its moment, and its sync or action lands by 0.1 s. The sound and announcer queues drain.
- `the_fx_layer_covers_the_table_and_passes_clicks` (A-fx-layer-passes-clicks): the layer is `[0, 0, W, H]`, and a click on a rack card under it selects the card.
- `the_reconnect_scene_lands_in_place`: no keyed element moves from 0 to 1 s.
- `a_played_card_comes_to_rest`: `table-play-opp` is idle from 2.0 s.
- `the_stats_line_reads_a_scene`
- `the_tour_steps_and_escape_dismisses_it`
- `the_rules_messages_reach_the_hooks`
- `the_sound_toggle_turns_the_player_on_and_off`

## Stub APIs by owning lane

### Lane A: choreography (`choreo.rs`, `present.rs`, `app.rs`, `scenes.rs`)

**Types**

- `enum Feel { Full, Brisk, Skim, Snap }`
- `struct ShotKey`, with constructors `deal`, `flip`, `bubble`, `trump`, `pickup`, `play`, `win`, `score`, `stamp`, `match_end`, plus `as_str()`
- `struct PlayFlightSpec { hand_no, index, seat, card, mine }`
- `struct PlayFlight { from, to, arc, r0, r1, s0 }`
- `struct Pose { x, y, rot, scale, lift }`
- `enum Shot { Deal, Flip, Bubble, Trump, Pickup, Play, Win, Score, Stamp, MatchEnd }`, with `Shot::key()` and `Shot::from_step(view, step) -> Vec<Shot>`
- `struct Running { shot, key, t0, dur }`, with `progress(now) -> f32`

**`Director` methods**

- `enqueue(Shot)`, `clear()`, `fast_forward(now)`
- `poll(now, reduced) -> PollOut`
- `flyers(now) -> impl Iterator<Item = (&ShotKey, &PlayFlightSpec, f32)>`
- `hides_slot(hand, index, seat) -> bool`, `unlaunched_from(seat) -> u8`
- `started(&ShotKey) -> Option<f64>`, `progress(&ShotKey, now) -> Option<f32>`, `landed(&ShotKey) -> bool`
- `feel() -> Feel`, `busy() -> bool`

**Free functions**

- `play_pose(&PlayFlight, t) -> Pose`
- `shake_x(t) -> f32`

**`Presentation` methods**

- `shake_card(Card)`, `shake_t(now) -> Option<(Card, f32)>`
- `dialog_progress(now, secs) -> Option<f32>`
- `announce_line(String)`

**`app.rs`**

- `TableScreen::showing_then(SyncEvent, Then) -> TableScreen`, `mark_reconnected()`, and `on_sync(SyncEvent, Option<&Bridge>, &mut Tx)`, the factored Sync arm. It also clears the Director on a snap.
- `enum Then { Sync(Box<SyncEvent>), Do(fn(&mut AppState)) }`
- Fast-forward runs on a played card, a submitted move, Continue, and the window coming back into view.

**`scenes.rs`**

- `drive_pair(seed, review, until) -> Result<(before, found)>`
- `search_pair(seeds, review, until)`
- `SEARCH_SEEDS`

### Lane B: felt, trick and fx (`stage.rs`, `fx.rs`)

- `fx::layer(ctx: &stage::Ctx<'_>, cx: &Cx<AppState>) -> El`: empty. The doc comment lists the persistent keys for `cx.layout`.
- `stage::slot_rect(view, seat, trick_rect: [f32; 4], height) -> ([f32; 4], f32)`
- `stage::deck_key(hand_no) -> String`
- `stage::banter_bubble(cx, seat, line: Option<&str>) -> Option<El>`: returns `None`. It is called last in `seat()`, and `copy::seat_banter(store, seat) -> Option<&str>` supplies the line.
- `AppState::deck_laid_out(rect: [f32; 4])`: for `deck-{hand}` `on_layout`. It sets `Presentation.deck_at`.

### Lane C: rack, header and panels (`hand.rs`, `header.rs`, `panels.rs`, `copy.rs`)

- `hand::hole_key(card) -> String`
- `Presentation.shake` and `shake_t(now)`. `AppState::play_card` and `press_card` call `shake_card` on an illegal card. Under reduce motion `shake_t` returns `None`.
- `Presentation.score: ScoreRoll { shown, target, next_step, landed_at }`. In wave 0, `shown` always equals `target`.
- `Presentation.dialog_since` and `dialog_progress(now, secs)`: stamped while the rules, the hand review or the tour is open.
- `enum RulesMsg { Query(String), Toggle(usize) }` and `AppState::rules(RulesMsg)`. These write `hooks.rules_query` and `hooks.rules_flipped` (one bit per section, 8 sections).
- `AppState::toggle_sound()`: both Sound toggles call it now.

### Lane D: shell (`screens/mod.rs`, `other.rs`, `home.rs`, `new_game.rs`)

- `screens::route_transition(dir: RouteDir, reduced: bool) -> Option<Transition>`: returns `None`. See "Deviations" for why this is an `Option`.
- `enum RouteDir { Forward, Back }`, kept in `AppState.route_dir`. `navigate` sets `Forward` and `back` sets `Back`. App `view` applies the transition to `screen-{route}`.
- Typed profile seam in `other.rs`. Lane D can switch to the `euchre_profile_model` types without touching `app.rs`:
  - `type MatchDetailData`, `type SettingsData`, `type SettingsEdit`, all `serde_json::Value` for now
  - `parse_match_detail(Value) -> Result<MatchDetailData, String>`
  - `parse_settings(Value) -> Result<SettingsData, String>`
  - `settings_patch_value(&SettingsEdit) -> Value`

  `app.rs` stores the parsed types (`SettingsState.value`, `match_detail`), and `AppState::patch_settings(SettingsEdit)` sends `Intent::PatchSettings`.

### Lane E: sound, announcer, tour and hints

- **`sound.rs`:** `struct Sound`, with `new()`, `set_enabled(bool)`, `enabled()` and `play(&Cue)`, all no-ops.
  - `AppState.sound` is built muted and set from `prefs.sound`. Nothing opens an audio device at startup.
  - `poll` drains `hooks.sound` every frame. It plays only if `prefs.sound` is on, the window is visible and the run is not headless (D14).
  - `rodio` is a dependency (workspace, `playback` only).
- **`announcer.rs`:**
  - `take(&mut Hooks, &PublicGameView) -> Option<String>` drains `hooks.announce` and returns `None`. `poll` hands any line to `announce_line`.
  - `layer(said: Option<&(u64, String)>) -> El` is an empty zero-size pinned element, mounted last in the table.
  - The doc comment records what Rune 85cd8c6 offers for accessibility (see "Rune accessibility findings").
- **`tour.rs`:**
  - `const SLIDES: u8 = 6`, `should_auto_open(view, prefs) -> bool` (returns `false`) and `layer(step, size, reduced) -> El` (empty).
  - The layer mounts after the dialogs, only while `hooks.tour_step` is `Some`. `poll` checks the auto-open every frame.
  - Handlers: `AppState::tour_open`, `tour_advance` (closes after the last slide), `tour_back`, and `tour_dismiss` (sets `prefs.tour_seen`).
  - Escape closes the tour first.
- **`hint.rs`:**
  - `enum Situation`, with `of(view)` and `key()`
  - `struct HintSpec`
  - `for_view(view, prefs) -> Option<HintSpec>`: returns `None`
  - `badge(card, &HintSpec) -> Option<El>`: called per rack button in `hand.rs`
  - `panel(&HintSpec) -> Option<El>`: the coach slot under the Hints toggle in `panels.rs`
  - `record_move(view, move_id, prefs) -> Option<BTreeMap<String, u32>>`: returns `None`. `play_card` and `submit_move` call it before sending and save the streaks it returns.
- **Dependencies.** `euchre-ai` and `euchre-profile-model` are dependencies now. They are read only; D28 keeps them frozen.

## Scenes

The motion scenes start at `prev`, at rest, at t = 0. The moment lands one frame later (1/60 s), so `--at` times read about 17 ms of motion less than the moment's own clock.

| Scene | Moment |
| --- | --- |
| `table-deal-anim` | `Dealt`, second hand |
| `table-upcard-flip` | `UpCardTurned`, second hand |
| `table-play-opp` | an opponent's `CardPlayed`, from trick 2 on, with no `TrickWon` |
| `table-play-you` | your `CardPlayed` |
| `table-trick-win` | `TrickWon`, index ≥ 1 |
| `table-trick-win-reduced` | the same, with `prefs.reduce_motion` on |
| `table-pickup` | `TrumpSet` via order-up |
| `table-trump-call` | `TrumpSet` in round 2 |
| `table-illegal-shake` | `Then::Do`: press the first card you may not play |
| `table-hover` | `table-play`, with `scripts/hover.json` (move to `hand-JS`, mark `hover` at 0.3) |
| `table-hand-result` | `HandScored` |
| `table-euchre-stamp` | `HandScored` Euchre (seed search) |
| `game-over-win` | `GameWon`, your team (seed search) |
| `game-over-loss` | `GameWon`, the other team (seed search) |
| `table-rules-open` | `Then::Do`: open the rules |
| `route-home-new` | Home, with `scripts/route-home-new.json` (click `home-nav-play`, mark `go` at 0.3) |
| `table-reconnect` | `mark_reconnected`, then the same view step-less with `v + 1` |
| `table-hint` | Lane E: hints on, your turn with a choice |
| `table-tour` | Lane E: `table-deal` with `tour_step = Some(0)` and `tour_seen` off |
| `table-banter` | Lane E: `table-play` plus a West banter line for the current hand |

Every table scene sets `prefs.tour_seen` except `table-tour`.

## Diff results

- **Baseline.** All 13 baseline scenes were re-rendered into `scratchpad/motion/w0/` at 1280x800, scale 1, and diffed with PIL against `baseline/`: **0 px in all 13**. No deliberate visual changes were made.
- **New scenes.** Every new scene was rendered at its §6.6 `--at` times into `scratchpad/motion/w0-new/`: 54 PNGs, with stats in `index.txt`. All runs succeeded.
- **Reconnect.** `table-reconnect` at 0.00 and 1.00 differs by 0 px.
- **Determinism.** Re-renders of `table-trick-win@2.30` and `game-over-win@4.00` differ by 0 px.
- **Frame time (debug build, CPU).** p95 stays at or under 3.5 ms everywhere. The highest is `route-home-new@mark:go+0.15` at 3.46 ms. `game-over-win@4.00` reads 240 animating frames at p95 1.44 ms.

## Deviations, and what is not pre-wired

- **`route_transition` returns `Option<Transition>`, not `Transition`.** With no `.transition()`, Rune applies its default opacity cross-fade. Setting `Transition::opacity()` explicitly is not the same thing: `commit/apply.rs` `quiet_row` checks `transition.is_none_or(..)`. Returning `None` keeps today's behaviour exactly. Lane D returns `Some(..)` for both directions, with `Some(Transition::opacity())` when reduced.
- **The shake lives in `Presentation.shake`, not `present.illegal.shake_t0`.** It is stamped lazily in `poll`, and `None` under reduce motion.
- **A-play-arcs conflicts with the spec's own tokens.** With `PLAY_EASE = cubic(0.22, 0.7, 0.25, 1.0)`, at t = 0.275 u ≈ 0.92, so the verbatim `play_pose` puts the flyer only about 0.15·arc above the chord. A-play-arcs asks for ≥ 0.3·arc. The peak, 0.5·arc at u = 0.5, comes at about t = 0.1.
  - The wave 0 unit test checks the geometry instead: the flyer stays above the chord and peaks at 0.5·arc.
  - Lane A must re-time A-play-arcs, sampling near t ≈ 0.1, or the owner must soften `PLAY_EASE`. Wave 0 changed no tokens.
- **The rest point for `a_played_card_comes_to_rest` is 2.0 s, not 1.5 s.** The existing trick-card spring settles at about 1.6 s. Its rect then jumps to the rotated bounding box, which is Rune behaviour.
- **Escape with the rules dialog open closes the rules, not the tour.** The dialog handles Escape itself, so `close_table_panel` never sees it. Lane E's tour layer needs its own Escape (modal) handling, because the tour can open from the rules panel.
- **`rules_query` and `rules_flipped` are not reset when the rules close.** Lane C should say whether they should be. That reset would be a one-line Lane A change in `set_rules_open`.
- **No preview cue on turning sound on.** Wave 0 does not play a cue when sound is switched on, since 06 §15 does not ask for one. If Lane E wants one, it goes in `toggle_sound`.
- **Lane E's hint panel.** For the hint panel to show `why`, `euchre_ai::rank_moves` has to be read inside `hint::for_view`. Nothing in wave 0 calls `euchre_ai`; the dependency is only declared.

## Rune accessibility findings (85cd8c6, recorded in the `announcer.rs` doc comment)

- There is no imperative "announce" call. A live region announces changes to its text or label:
  - `El::live_region(LiveRegion::{Off, Polite, Assertive})`
  - role `status` and `log` are polite by default, and `alert` is assertive
- AccessKit adapters do the speaking. Rune only reports nodes added, removed or moved (`a11y/tree.rs` `Changes`).
- Nodes that are `a11y_hidden`, at opacity ≤ 0.01 or secure are dropped from the content prints (`a11y/node.rs`). The announcer must therefore stay at full opacity and be visually empty.
- Repeating an identical line needs a new key (`announcer-{id}`).
