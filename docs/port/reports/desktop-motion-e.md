# Report: DESKTOP MOTION / LANE E (coach hints, sound, announcer, first-run tour)

Crate: `apps/desktop` (`euchre-desktop`), Rune pinned at 85cd8c6. Spec: `docs/port/06-ui-spec.md`
§10 (narration), §13 (hints), §14.3 (tour), §15 (sound), with the motion tokens of
`docs/port/09-motion-visual-spec.md` §3.

Lane E filled the four wave 0 stubs and added `tests/coach.rs`. Nothing outside Lane E's files
changed, apart from this report and three request files (`docs/port/requests/motion-E-to-A.md`,
`motion-E-to-C.md`, `motion-E-to-D.md`). No git writes were made.

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/motion-e`, at
01:25 on 2026-11-11 (final pass), while Lanes A to D were still editing.

| Gate | Result |
| --- | --- |
| `check` (clippy `-D warnings`, all targets) | Fails only in Lane B's `tests/felt.rs:183` (`Option::unwrap_or`). An earlier pass, with the other lanes' in-progress lints allowed, showed every target clippy-clean, `tests/coach.rs` included. |
| `test` | lib, `coach` (15/15), `motion`, `rack` and `shell` all pass. The failures are in other lanes' tests: `felt.rs` (Lane B, 2) and `table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` (bid copy layout). Both scenes run with hints off and the tour seen, so neither touches Lane E's code. Under a load average of 13 to 22, `shell.rs::the_first_deal_stays_inside_the_frame_budget` failed once, then passed on its own and on the final run. |
| `lint` (`fmt --check`) | Fails only in Lane B's `tests/felt.rs`. Lane E's five files are rustfmt-clean. |

Lane E's files were formatted with `rustfmt` on those files only, so no other lane's file was
touched. No test processes were left under the target dir (`pgrep` is empty).

## What is in each file

### `src/screens/table/hint.rs`: the coach (06 §13)

- `for_view(view, prefs)` is the suggestion, or `None`. It shows only when the Hints pref is on,
  the match is active, it is your turn, more than one move is legal, the view has a situation
  class (`cut`, `bid1`, `bid2`, `discard`, `lead`, `follow`) and that class is not retired. The
  move is `euchre_ai::rank_moves(RankParams::new(view))[0]`, read from the player's own
  `PublicGameView`, so it can use nothing the player cannot see.
- `record_move(view, move_id, prefs)` is the auto-retire streak. It returns the new
  `prefs.hint_streaks` map, or `None` when nothing changes. A move that matches the AI's top move
  adds one; any other resets the class to 0, which brings a retired hint back. After
  `RETIRE_AFTER` (3) matches in a row, the class retires. app.rs (`play_card`, `submit_move`)
  already calls it and saves the result.
- `line(hint)` is `Try: {label} — {gloss(why)}`, with the label's byte range for the bold span.
  The reason is dropped when it is empty or repeats the label, ignoring case. `gloss`
  capitalises, ends with a period, and the first time per session a jargon term appears, adds
  ` — meaning {meaning}.` The five terms and meanings are 06 verbatim.
- `badge(card, hint)` is the "Try" pill on the suggested rack card (hand.rs already mounts it).
  `move_badge(move_id, hint)` is the same pill for a bid button (hand.rs `bid_panel` mounts it). The pill is
  pinned across the card's top edge, out of flow, so the rack never shifts when a suggestion
  comes or goes. It is gold on gold-deep (`motion::GOLD`, `GOLD_DEEP`), carries the coach line as
  its tooltip, and enters with scale 0.7 plus opacity on `CHIP_IN` / `CHIP_OUT`.
- `panel(hint)` is the suggestion box in the settings rail (panels.rs mounts it under the Hints
  toggle): padding 12, `#e6eadc` on the hairline, ink text, the move in bold green,
  `role=status`.

**Deviation.** A forced move (one legal choice) does not count toward a streak. In the web, a
forced play always matches the AI and would retire hints on plays that teach nothing. 06 says
"after each real human decision", which this reads literally.

### `src/sound.rs`: the cues (06 §15)

- `recipe(cue)` lists each cue's voices, verbatim from the 06 §15.2 table: tone (sine or
  triangle, attack, exponential decay to 0.0001, optional octave overtone at 35 % gain ending at
  70 % of the duration) and noise (12 ms attack, RBJ band-pass, low-pass or high-pass, with an
  optional exponential sweep). `render(voices)` mixes them into one mono 44.1 kHz buffer,
  clamped to ±1, deterministic (fixed-seed noise). Both are pure, so tests check the mapping
  without a device.
- `place(shift)` and `place_shift(card)` are the web's per-card pitch (FNV-1a over the card id,
  ±3 semitones). They are ready for `Cue::Place(Card)` (request to Lane A).
- `Sound` is the player. It is off by default (D14). `prefs.sound` turns it on through
  `set_enabled`. The first enabled cue spawns one `euchre-sound` thread that opens the default
  rodio output and renders each cue once, then replays the cached buffer. If there is no
  device, the thread logs one `warn` line and ends, and the player goes silent for the run. No
  output opens in a test binary or a headless run (`sound::live_app()`), so `--snap`,
  `--frames` and `cargo test` never touch audio.

### `src/screens/table/announcer.rs`: narration (06 §10)

- `sentence(step, view)` is every 06 §10 sentence, verbatim, with the seat names `You`,
  `Left opponent`, `Your partner`, `Right opponent`. Scores read from your team's side. The barn
  line follows `is_in_the_barn` and is left out once the game is over. A bid is silent.
- `narrate(steps, view)` joins one sync's sentences with spaces and collapses repeated deal
  packets to one "Cards dealt" line. `take(hooks, view)` drains `hooks.announce` (app.rs `poll`
  calls it and hands the line to `Presentation::announce_line`).
- `layer(said)` is the live region. Rune at 85cd8c6 has no "speak this string" call. A screen
  reader hears a node marked `LiveRegion::Polite` (`aria-live=polite`) appear with a name, and
  AccessKit's adapters do the speaking (VoiceOver on macOS, UI Automation on Windows). So the
  native port has the 06 region itself, not a fallback. The region is 1×1, pinned, at full
  opacity (Rune drops nodes at opacity ≤ 0.01 from the tree), and its child is keyed by the
  bumped id with no fade, so the same words twice are a fresh node and are spoken again.

**Gap.** While the tour (a modal) is open, Rune leaves the table behind it out of the tree, so
steps that land then are not spoken. The tour is the player's focus at that moment, and a fresh
hand's steps have usually landed before the tour opens.

### `src/screens/table/tour.rs`: the first-run tour (06 §14.3)

- Six slides, verbatim from 06, with the eyebrow `Quick tour · {n} of 6`, the dots, and `Skip`,
  `Back` (from the second slide) and `Next` / `Deal me in`. The sheet is `role=dialog`,
  `Quick tour`, and traps focus with `modal_tx`; Next takes focus when the tour opens.
- Skip, Escape, a click on the backdrop and Deal me in each call `AppState::tour_dismiss`,
  which clears the step and sets `prefs.tour_seen`.
- `wanted(view, prefs)`: never seen, and a real hand is on the felt (cards in your rack, or a
  phase past lobby, cutting and deal). `should_auto_open` adds `sound::live_app()`, so only the
  app window opens it by itself; a headless run shows it only in the `table-tour` scene.
- Motion, all from `theme::motion`: the scrim fades on `SCRIM_IN` / `SCRIM_OUT`; the sheet
  enters with scale 0.96 plus a 10 pt rise on `PANEL_IN` and leaves on `PANEL_OUT`; each new
  slide rises 6 pt and fades in on `UI` (the old one leaves at once, so text never overlaps).
  Under reduce motion (the pref or the OS), the sheet and slides only fade.

## Tests (`tests/coach.rs`)

Fifteen tests, all passing, plus ten unit tests inside the four source files.

**Hints**
- `the_hint_is_the_ai_top_move`: four kinds of decision from the driven match (bid round 1,
  bid round 2, a lead, a follow). For each, the hint is `rank_moves(view)[0]`: the same id and
  reason, the legal move's label, the view's turn id, and a suggested card from your own hand.
  The badge, line and panel are built from it.
- `no_hint_when_hints_are_off_or_it_is_not_your_turn`: none with hints off, on another seat's
  turn, with one legal move, or after the match.
- `a_hint_streak_retires_and_a_miss_brings_it_back`: three matches retire the class. A miss
  resets it to 0 and the hint returns. A second miss and a forced move change nothing.
- `the_rack_shows_the_badge_and_a_play_keeps_the_streak`: in the `table-hint` runtime, exactly
  one rack card wears the badge and the settings rail shows the box. Playing the suggested
  card adds one to `prefs.hint_streaks`.

**Sound**
- `every_cue_follows_its_06_recipe`: each cue's voices against the 06 §15.2 table: pitches,
  durations, gains, attacks, start times and overtones, the four trump pitches, and the place
  thud through `sound::place`.
- `every_cue_renders_a_bounded_deterministic_buffer`: every cue, four card-shifted place thuds
  and the lift render to the right length, peak between 0.005 and 1, decay to silence, and
  render the same twice. `place_shift` matches the web (JS -2.028, AH -2.592) and stays within
  ±3.
- `a_test_never_opens_a_speaker`: the player starts muted; enabled, it plays every cue and opens
  nothing.

**Tour**
- `the_tour_is_owed_once_a_real_hand_is_dealt`: owed over a dealt hand, not over the cut, not
  once seen, and never opened by itself in a test.
- `the_tour_steps_through_and_finishing_marks_it_seen`: Next through all six slides (each
  title checked), Back once, and Deal me in closes it and sets `tour_seen`.
- `skip_escape_and_the_backdrop_each_dismiss_it_for_good`: each of the three dismisses from
  slide 2 and sets `tour_seen`, and `wanted` is false afterwards.
- `a_new_slide_replaces_the_old_one_at_once`: one frame after Next, only the new slide is
  visible.
- `the_sheet_rises_in_and_reduce_motion_only_fades_it`: opened again from the rules panel, the
  sheet rises in and fades. With `reduce_motion` it only fades (no travel, no scale).
- `the_tour_eyebrow_and_slides_are_verbatim`.

**Announcer**
- `the_announcer_says_06_verbatim`: 25 steps against their exact sentences, covering cuts,
  up-card, turned down, trump (yours, your partner's alone, yours alone), discards, plays, all
  three lead lines and the last trick, every hand result with and without the barn line, the
  throw-in, both game ends and a silent bid. A deal with two packets is said once, followed by
  the up-card in the same line. An empty hand's deal is silent.
- `a_played_card_reaches_the_live_region`: in `table-play-opp`, the opponent's play is announced
  and `accessibility_json()` holds a `live-polite` node with that name. The same words again are
  a fresh node.

## Scenes looked at

Rendered at 1280×800, scale 1, and opened with Read. The files are in `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/motion-e`.

- `hint.png` (`table-hint`, 1.5 s): the "Try" pill sits centred on the top edge of the suggested
  card (the jack of hearts, the left bower), half over it, gold on gold-deep. The rack did not
  shift.
- `hint-panel.png` (`table-hint` with Settings opened by script): the box under Hints reads
  "Try: **Jack of hearts** — Take the third and be done." on the light green fill.
- `tour.png` (`table-tour`, 1.5 s): the sheet is centred over the darkened table, with the
  eyebrow `QUICK TOUR · 1 OF 6`, title, body, dots, Skip and the lime Next.
- `next-mid.png` and `tour-next/` (Next at 0.8 s, Skip at 1.6 s): the first render showed the
  old slide fading out over the new one, so the words overlapped for about 0.2 s. The cause:
  `Transition::asymmetric` takes its springs from the insertion side, so a `none` removal still
  faded on `PANEL_OUT`. **Fixed** by giving the slide an exit spring of `Spring::NONE` (an
  instant curve, even under reduce motion). The re-render shows the old slide gone in the frame
  the new one mounts, and the new words rising 6 pt into place. Skip: the sheet and scrim are
  gone by about 0.25 s.
- `reopen-sheet.png` and `tour-reopen/` (the rules panel's "Take the quick tour again"): the
  sheet scales up from 96 % and rises as the scrim fades in, settled by about 0.35 s.

## Frame times

From `--frames 240 --fps 60` headless runs, two passes each. `p50` is steady; `p95` is
machine noise. The load average was 13 to 22 from the other lanes' builds, and the at-rest
frames, which animate nothing, show the same spikes (forced p95 up to 72 ms).

| Run | Animating frames | cpu p50 | cpu p95 |
| --- | --- | --- | --- |
| `table-hint` at rest | 0 (no idle motion) | 0.84 to 1.09 ms (forced) | noise |
| `table-tour` at rest | 0 (no idle motion) | 1.02 ms (forced) | noise |
| tour: Next, then Skip | 69 | 1.63 to 2.44 ms | 28.6 to 55.4 ms |
| tour opened from the rules panel | 200 | 1.27 to 1.43 ms | 19.7 to 66.5 ms |
| Settings opened with the coach box | 50 | 1.09 to 1.26 ms | 6.2 to 15.1 ms |

## Requests

- **Lane A** (`motion-E-to-A.md`):
  - Item 1 is done by Lane A: turning sound on plays the hearts trump cue.
  - Item 3 is done by Lane A: `sim_table` starts with the tour seen.
  - **Open:** item 2, `Cue::Place(Card)`, so each landed card gets its own pitch. Lane E's
    side is ready. `sound.rs` matches `Cue::Place { .. }`, and `tests/coach.rs` builds no
    `Cue::Place`, so Lane A can change the variant alone. The follow-up is one line in
    `sound.rs`: `Cue::Place(card) => place(place_shift(card))`.
- **Lane C** (`motion-E-to-C.md`): done by Lane C. The bid buttons wear `hint::move_badge`.
- **Lane D** (`motion-E-to-D.md`): done by Lane D. The live-sim shell tests start with the tour
  seen.

Lane E keeps the `sound::live_app()` gate on the auto-open. The sound player needs the same
check, because a cargo test is not headless.

## Cut

Lane E's motion is the tour's dialog and scrim, which is P1 under 09 §6.7, and it shipped in
full. Two things were left out:

- **The per-card place pitch.** It waits on Lane A's item 2. Until then, every place thud plays
  at the centre pitch.
- **The scrim dip on the rules panel's "Take the quick tour again".** The rules dialog's scrim
  fades out while the tour's fades in, so the table brightens for about 0.1 s. This happens only
  on that reopen path, never on the first-run open. Fixing it means the two dialogs share one
  scrim, which needs changes in Lane C's rules dialog and in the mount order in Lane A's
  `table/mod.rs`. It is not worth that cross-lane change at P1.

**Known gap:** steps that land while the tour is open are not spoken. Rune makes the table
behind a modal inert.
