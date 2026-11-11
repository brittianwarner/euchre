# Report: DESKTOP MOTION / LANE A (choreography)

Crate: `apps/desktop` (`euchre-desktop`), Rune pinned at 85cd8c6. Spec: `docs/port/09-motion-visual-spec.md`
§2, §4.3, §4.4, §4.11 and §6.2 (Lane A rows).

Lane A changed only its own files: `src/screens/table/{choreo,present,mod}.rs`, `src/app.rs`,
`src/scenes.rs`, `src/theme/motion.rs`, `tests/motion.rs` and `tests/table.rs`. Outside the crate it
changed only this report and the request files under `docs/port/requests/` (DONE notes, and new
items for Lanes B, C and D). There were no git writes, and `crates/**` was not touched.

## Gates (target `.cargo-targets/motion-a`)

Last run at 01:23, after the `Reply::Stats` arm. Lane A's own files are green.

- `check` (clippy `--all-targets -D warnings`): green.
- `lint` (`cargo fmt --check`): green.
- `test`: every target is green except one failure:
  - lib: 71 passed
  - coach: 15 passed
  - felt: 6 passed
  - motion: 18 passed
  - rack: 8 passed
  - shell: 13 passed
  - table: 12 of 13 passed. The failure is
    `the_bid_copy_stays_in_its_box_beside_your_seat` (tests/table.rs:673).

The table failure:

```
table-bid at (720.0, 820.0): bid-copy [254.51563, 215.04688, 267.5625, 83.5] inside [181.92188, 224.0, 356.15625, 65.59375]
```

- The test file is Lane A's, but the geometry is Lane B's. At 720x820 the felt and seat changes in
  stage.rs and felt.rs leave a trick box 65.6 pt tall, and the 83.5 pt bid copy overflows it by
  about 9 pt above and below.
- It has failed on every run since Lane B's felt and seat work landed, so it is not flaky.
- It is written up in `motion-A-to-B.md` items 7 and 8.
- Lane A did not loosen the expectation, because the overflow is a real layout fault.

Earlier failures in other lanes all cleared on retry: `tests/felt.rs` clippy
`map_or_identity`, `b_gather` at 41.9 pt, and `the_first_deal_stays_inside_the_frame_budget`
under machine load.

## What is done (09 §6.2 order)

### 1. Director queue and poll (§2.2, §2.3)

`choreo::Director` owns every choreographed moment as a `Shot`: Deal, Flip, Play, Win, Score and
Stamp. Each shot is keyed by `ShotKey`, so a shot already queued, live or landed is never queued
twice.

`Director::poll(now, reduced)` does four things:

- picks the feel
- starts every shot whose wait is met
- retires finished shots
- returns `PollOut { changed, frame, wake }`

When the table has nothing left to do, `wake` is `None` and `frame` is false.

`Presentation::poll` runs it every frame, before the handlers (Rune's frame order is poll, then
handlers, then commit). The view reads only these:

- `flyers`
- `play_slot`
- `unlaunched_from` / `unlaunched_play`
- `started`, `progress`, `elapsed`, `feel_of`
- `winner_shown`, `gathering`, `confetti_t`

### 2. Play and Win gating (§4.3, §4.4)

- A play waits for the play before it to touch down. Your own play starts Full in the same commit
  as the click when nothing is queued ahead of it.
- The `Win` shot waits for its trick's last touchdown plus `RING_DELAY` (0.05 s).
- `present.winner_shown(view)` is true only once `Win` has started. Lane B gates the ring, pop and
  seat plate on it.
- The flyer lives `PLAY_END` (0.95 s). Then the slot takes over with `Transition::none()`, so the
  hand-off has no fade and no jump.

### 3. Sweep (§4.4)

- The sweep is unclipped and uses `motion::SWEEP` (0.45/0). `app::poll` calls
  `tx.unclipped_flights()` on every commit that sends a card.
- A Skim win sends all four cards in one `tx.instant()` commit.
- `Collect` gained `gather` (true from `GATHER_LEAD` before the sweep, Full wins only) and `took`
  (the last card has landed; `Cue::TrickTake` fires then).
- `Collect::finish_at` uses `motion::SWEEP_SECS` (0.60) for Continue.

### 4. Tiers (§2.3)

A backlog of 4 or more shots skims: every queued play lands Still except the last, which flies
Matched (Skim). A backlog of 2 or 3 is Brisk, at `BRISK` time.

Two things count toward the backlog:

- Win and Deal shots do not.
- The deal's own tier is kept, so a deal that landed as Snap does not deal in.
  `present.deal_in` clears then.

### 5. Fast-forward (§2.4)

`play_card` calls `director.fast_forward(clock)`. A card in the air finishes over the
fast-forward window (≤ 0.15 s), and every queued shot lands in place. A new hand's Deal also
fast-forwards anything older than it.

### 6. Reconnect clear (§2.4)

Both `on_sync`'s snap path and `Reply::Snapshot` clear the Director with `present.snap()` and
commit with `tx.settle()`, not `tx.instant()`. The change was needed because `instant()` covers
only this commit, so springs already running on the seat backs and seat detail rows kept moving
for half a second after a reconnect. `settle()` lands those as well.

### 7. Reduced motion (§2.5)

Under reduce motion every shot lands at once:

- no flyer, ring pop, sparks or confetti
- the gather still starts at `COLLECT_AFTER`, and the sweep is instant

The Director checks reduce motion on every poll, and a shot that is live when reduce motion turns
on lands in that poll. The OS setting and `prefs.reduce_motion` both reach it through
`cx.is_reduced_motion()`.

### 8. Route wiring (§4.11)

- `app.rs::view` sets `screen.transition(t)` and `root.child_exit(t)` from Lane D's
  `route_transition`, so the leaving screen exits in the direction of the route change.
- `navigate`/`back` no longer call `tx.motion(UI)`. That call overrode every matched spring in the
  change.
- Navigating to the table commits with `tx.unclipped_flights()`.

## Tests (`tests/motion.rs`, 18 pass)

| Spec | Test |
|---|---|
| A-layout-rects | `a_layout_rects_the_flyer_leaves_from_its_fan` (flyer ≤ 2 pt from the `seat-{i}-fan` centre on frame 1) |
| A-play-arcs | `a_play_arcs_above_the_chord_and_lands_in_its_slot` (≥ 0.3·arc above the chord at t0 + 0.275; no `fly-*` at t0 + 1.0; slot at rest, opacity 1) |
| A-ring-waits | `a_ring_waits_for_the_last_touchdown` (`win-ring-{h}-{i}` absent at 0.3 s, present at 0.7 s) |
| A-sweep-inside-hold | `a_sweep_lands_inside_the_hold` (last mini at its rect by ≤ 3.05 s; Continue `finish_at` ≤ started + 0.74) |
| A-skim-last-only | `a_backlog_skims_to_the_last_card` (feels Snap, Snap, Snap, Skim; no `fly-*`; only the last slot moves) |
| A-fast-forward | `a_press_fast_forwards_the_card_in_the_air` (press at u = 0.3; no `fly-*` by +0.15 s) |
| A-reconnect-snaps | `a_reconnect_lands_everything_at_once` (Status(false) → Status(true) → stepless sync; nothing moves 1 frame later) |
| A-reduced-none | `reduce_motion_draws_no_flight_and_no_burst` (OS and pref; gather at `COLLECT_AFTER` ± 2 frames) |
| A-idle | `the_table_sleeps_after_the_last_shot` (`wake = None`, `frame = false` 1.5 s after the last shot) |
| A-fx-layer-passes-clicks | `a_click_reaches_the_rack_under_a_flyer`, `the_fx_layer_covers_the_table_and_passes_clicks` |

The 17 Director/presentation unit tests in `choreo.rs`/`present.rs` also pass. `tests/table.rs` has
13 tests. 12 pass, and one fails in Lane B's layout (see Open requests).

## Scenes looked at (1280x800, scale 1)

- **table-play-opp** (strip, every 5 frames).
  - Grandpa's card leaves his fan at about 0.4 scale and lifts clear of the chord, peaking about
    55 pt up near mid-travel.
  - It grows and turns as it comes, then slows and drops into its slot with the squash.
  - At the 0.95 s hand-off the static card takes its place with no jump and no fade.
  - It reads as a tossed card with weight, not a slide.
- **table-play-you** (strip, plus a snap at 0.12 s).
  - K♦ lifts out of the rack, which leaves a faded hole for a moment, then flies up and left and
    lands beside A♣ by about frame 25.
  - There is never a second copy of the card.
- **table-trick-win** (strip 0 to 3.2 s, plus a snap at 0.7 s).
  - Ned's card is in the air at 0.2 s and lands about 0.45 s. There is no ring before it lands.
  - The gold ring and pop are on K♦ at 0.6 to 0.7 s.
  - The cards gather at about 1.9 s and the sweep runs from 2.2 s. The trick area is empty by
    about 2.8 s, inside the hold.
- **table-trick-win-reduced.**
  - The card is in its slot from the first frame. The ring is static, with no pop and no gather.
  - At 2.2 s all four cards leave at once.
  - Only 20 frames animate, all of them opacity fades.
- **table-reconnect** (`--at 0` and `--at 1`).
  - The two PNGs are byte-identical, and 0 frames animate over 240 frames.
  - Everything lands at once.

## Frame time (`--frames 240 --fps 60 --size 1280x800 --scale 1`)

The binary is a debug build of `euchre-desktop` with dependencies at opt-level 2. There was no
release build, because the disk had about 5 GB free.

| Scene | All frames: mean / p50 / p95 / max (ms) | Animating: frames, p50 / p95 (ms) | At rest: frames, forced p50 / p95 (ms) |
|---|---|---|---|
| table-play-opp | 0.915 / 0.912 / 1.514 / 31.6 | 57, 1.045 / 1.435 | 182, 0.734 / 1.470 |
| table-play-you | 0.847 / 0.772 / 1.329 / 30.2 | 239, 0.769 / 1.313 | 0 |
| table-trick-win | 0.848 / 0.750 / 1.509 / 30.7 | 145, 0.788 / 1.509 | 94, 0.726 / 1.298 |
| table-trick-win-reduced | 0.838 / 0.740 / 1.552 / 30.8 | 20, 0.752 / 1.585 | 219, 0.736 / 1.500 |
| table-reconnect | 0.775 / 0.799 / 1.596 / 30.0 | 0 | 239, 0.798 / 1.553 |

- In every scene, the max is the first frame (gpu and text warm-up, about 30 ms).
- `table-play-you` never rests in 240 frames. A trace shows one 6x6 element on a `loop`: the
  `seat-{i}-thinking` dot for Rita, who plays next. That loop is meant to run until her play
  arrives, and the scene never sends that play. It is not an idle leak in the Director.

## Deviations from 09

- **`play_pose` is a toss, not the §4.3 sketch's eased Bezier.** The sketch's easing put the peak
  at t = 0.1 and failed A-play-arcs.
  - The pose now drags along the ground track, with a time parabola for height that peaks at
    `arc/2` at mid-travel.
  - It ends with a skid and the same squash.
  - The inputs and the `Pose` output are unchanged.
- **A-fast-forward** asserts on the opponent's flyer that was in the air. The pressed card's own
  flight starts Full after the fast-forward, which is the point of the press.
- **Win and Deal shots do not count toward the backlog.** Only plays decide Brisk or Skim, so a
  trick's Win cannot turn its own last card Brisk.
- **Snaps commit with `tx.settle()`, not `tx.instant()`** (see item 6).
- **`confetti_until` is the banner's first poll + `CONFETTI_SECS` (3.7 s).** The presentation asks
  for a frame on every tick while the burst runs, so Lane B's canvas can pose the burst from the
  clock.
- **A-sweep-inside-hold** checks the "rest rect" as the mini's presented rect within 0.5 pt of its
  target. With `!moving`, the SWEEP spring's sub-pixel tail outlasts 3.05 s.

## Cuts (09 §6.7)

None from P0 or P1. Lane E's request 2 (`Cue::Place(Card)`, 06 §15.2) was held back while it would
have broken the exhaustive match in Lane E's file. Lane E then made its arm `Cue::Place { .. }`,
and the change landed:

- `Director::take_touchdowns` returns the landed cards in order.
- `present.poll` pushes one `Cue::Place(card)` per poll that saw a landing.

## Requests

Handled (DONE notes appended in each file):

- **motion-B (A→B):** items 1 to 5 are in Lane B's code, and every A-* test passes on it.
  - Lane B removed `stage::confetti`; `table/mod.rs` no longer mounts it.
- **motion-C-to-A:** items 1 to 4 are done.
  - Item 1: `header_with` with the score roll.
  - Item 2: the overlay rail width.
  - Item 3: `AppState.rules_search` and `rules_dialog_with`.
  - Item 4: Snap clears `deal_in`.
- **motion-D-to-A:** items 1 to 4 are done.
  - Item 1: `child_exit` for routes.
  - Item 2: `GamesState.stats`/`newer`, `Intent::GetStats` on entering Games, and the
    `Reply::Stats` arms. The catch-all arm is gone.
  - Item 3: the `games`, `settings` and `match-detail` scenes.
  - Item 4: `GamesState.shown`, and entering Games clears `newer`.
- **motion-E-to-A:** items 1 to 3 are done.
  - Item 1: the test cue on sound on.
  - Item 2: `Cue::Place(Card)`.
  - Item 3: `tour_seen: true` in `tests/table.rs::sim_table`.

Open:

- **Lane B** (`motion-A-to-B.md` items 7 and 8):
  1. `tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` fails at 720x820. The bid copy
     is 83.5 pt tall in a trick box 65.6 pt tall, after the felt and seat changes in
     stage.rs/felt.rs.
  2. The result line ("You win the trick.") and the "Tricks this hand" tally change when the sync
     lands, while the last card is still in the air. 09 §4.4 puts that change on the 0.60 beat
     with the ring. Lane B should gate both on `winner_shown`.
- **Lane D:** the pager should set `s.games.shown` and push it onto `s.games.newer`. Then
  `ShellSlots` can go.
- **Lane E:** one line in sound.rs: `Cue::Place(card) => place(place_shift(card))`.
- **Lane C:** read the deal's own tier with `director.feel_of(&ShotKey::deal(h))`, not `feel()`.
  This is a suggestion, not a failure.
