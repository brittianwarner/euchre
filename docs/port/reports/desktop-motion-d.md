# Report: DESKTOP MOTION / LANE D (shell, screens, typed profile data)

Crate: `apps/desktop` (`euchre-desktop`), Rune pinned at 85cd8c6. Spec: `docs/port/09-motion-visual-spec.md`
§5.7 and §6.5 (Lane D rows), `docs/port/06-ui-spec.md` §3, §4, §17, §18.

Lane D changed only its own files: `src/theme.rs`, `src/screens/{mod,home,new_game,other}.rs`,
`src/assets.rs` (+ `assets/cards/AS.png`), `src/net/{mod,bridge,remote,sim}.rs` and
`tests/shell.rs`. Outside the crate: this report and `docs/port/requests/motion-D-to-A.md`.
No git writes were made.

## Gates (target `.cargo-targets/motion-d`)

- `check` (clippy `-D warnings`, all targets): green.
- `lint` (`fmt --check`): green.
- `test`: every Lane D test passes (lib 71, `tests/shell.rs` 13, none ignored). The one failure in
  the crate is `tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` at 720×820: the
  bid copy (83.5 pt tall) outgrows the trick box (65.6 pt). That is felt geometry and bid copy,
  not a Lane D file. Lanes A and C already filed it to B (`motion-A-to-B.md`,
  `motion-C-to-B.md` item 4).

## What is done

### 1. Route transitions (09 §4.11, S9)

`screens::route_transition(dir, reduced) -> Option<Transition>`. Forward: the new screen comes in
from the right (offset 70 pt + opacity, `motion::ROUTE_IN`) and the old one leaves to the left
(`motion::ROUTE_OUT`). Back is the mirror. Reduce motion is a plain fade with no travel. Lane A
wired it in `app.rs::view`.

- D-route-forward: `route_forward_slides_the_new_screen_in_from_the_right`. Passes.
- D-route-back: `route_back_mirrors_the_slide`. Passes, both halves: Home comes back from the
  left and New game leaves to the right. Lane A set `child_exit` on the screen's parent (request
  item 1), so a removed screen leaves the way the change goes, not the way it came in.
- D-route-reduced: `route_change_under_reduce_motion_only_fades`. Passes.

### 3. List staggers (09 §5.7)

Games match cards, match-detail hand rows and the Settings cards enter with
`Transition::offset(0, 6) + opacity`, delayed by `Stagger::new(0.03).delay(i, n)`. Rune skips the
delay on the first frame (snapshots stay put) and makes it a fade under Reduce motion.

### 4. New game option physics (09 §5.7)

- Each house rule is an option card on cream: a 2 pt ring, `GOLD_DEEP` when picked and a faint
  `disabled_border` (alpha 0.45) when not. It changes on `Spring::new(0.25, 0.2)`. The ring draws
  over the fill and takes no layout space, so "the note never moves the button" still holds.
- The option card presses to 0.98. "Deal me in" presses to 0.96 (`green_button` itself stays at
  0.98, because Lane A's table uses it too).
- Home ("Let's play", the footer link) and the dark scrapbook buttons get the key-button physics
  that do not need state: `press_scale(0.96)` and a hover that springs in and out on
  `motion::HOVER`.

### 6. S-2: a clean Ace of spades

`assets/cards/AS.png` is a desktop copy of the CC0 face. The big spade, the QR code and the URL
are cleared, and a plain centre pip is drawn from the deck's own spade path at the size the
other aces use. The script was calibrated against the real AC face's club pip. `assets.rs` embeds
the copy. `the_ace_of_spades_is_the_clean_desktop_copy` checks that AS uses it.

### 7. Typed profile seam and the real scrapbook screens (06 §17, §18)

`serde_json::Value` is gone from the shell's profile path.

- `net::Backend` has typed methods: `list_games -> MatchListPage`,
  `match_detail -> ClientMatchReplay`, `stats -> ProfileStats`, `get_settings -> ProfileSettings`,
  `patch_settings(SettingsPatch) -> ProfileSettings`. Remote stubs return `NotWired`.
- `bridge::Reply::MatchDetail` carries `ClientMatchReplay`. `Reply::Settings` carries
  `SettingsReply { settings: Box<ProfileSettings>, saved }`; it is boxed because it is the
  largest reply.
- `Intent::PatchSettings(SettingsPatch)`, `Intent::GetStats` and `Reply::Stats` are added.
  The Games screen reads `state.games.stats` (Lane A's field) and falls back to the copy that
  rides with the first page until `app.rs` reads `Reply::Stats`. The aliases in
  `screens/other.rs` (`MatchDetailData = ClientMatchReplay`, `SettingsEdit = SettingsPatch`,
  `SettingsData`) keep `app.rs` compiling unchanged.
- **Your games**:
  - A lifetime stats grid (06 §17.1, `role=group`, "Lifetime stats"; only when there is at
    least one match): matches, win rate, record (with "+N left unfinished" in its label),
    euchres sent–taken, loners made, hands played.
  - Match cards: short date, an outcome chip (gold fill for a win, outline otherwise), "You a – b
    them", "N hands", "At the table: …".
  - A pager: "← Newer games" / "Older games →".
  - The 06 empty state.
- **Match detail**:
  - Back link, long date ("Sunday, November 8, 2026 at 3:56 PM"), "You won" / "They won" / "Left
    unfinished", the score, the opponents.
  - A totals grid: hands, euchres, loners, marches, thrown in, tricks won.
  - An ordered hand list: "Hand n", "Dealer: …", "♦ Diamonds called by East · alone",
    "a–b tricks", and the 06 result line with "(+n us)" / "(+n them)" / "(no score)". Hands we
    scored on get a gold left edge.
  - A new plain failure line: "Couldn't open this game just now. Go back to your games and try
    it again."
- **Settings**:
  - The 06 §18 disclaimer, verbatim, with the middle sentence bold.
  - "The table": pace (Brisk / Normal / Relaxed as a radio group), banter, reasoning, plus this
    device's Sound / Reduce motion / Hints switches. The device switches show even when the
    profile fails to load.
  - The house prompt, with a counter.
  - Three persona cards (West, North · your partner, East): name and blurb with counters, the
    three dials, difficulty with its 06 explainer, the seven preset chips, and the character
    prompt.
  - "Save changes", "Saved. Your next match will use these." (`role=status`), the 06 save-failure
    line, and About.
- **Settings fields and dials**:
  - Fields keep to their 06 limits as `maxlength` does: 32 / 120 / 2000 / 1000 characters,
    clamped on every change, paste included. A counter turns gold past 90 %.
  - Multi-line fields take Return as a newline. A name or blurb field saves on Return.
  - Dials: drag or click the track, arrow keys step 5 %, Home / End jump to the ends. Each is
    a `role=slider` with `range_value` and a percent `a11y_value`.
  - A preset fills the blurb, prompt, difficulty and dials but never the name, and nothing saves
    until "Save changes".
  - Save sends the whole form as one `SettingsPatch` (all three seats). Sound and Reduce motion
    are taken from this device's switches, so the profile matches the device.
- **Identity gate**: none is needed on the desktop. The device enrollment says who is playing
  (D5), so the web's "Playing as" gate and line have no part here.

### 8. The sim serves real typed data

- `SimBackend` seeds `SimOptions.history` (default `SIM_HISTORY = 12`) past matches the first
  time the history is read (`sim::seeded_history`).
  - The matches are played out by the engine with the heuristic on every seat, in virtual
    time, over varied house rules.
  - The opponents are the default profile personas (Marge, Dutch, Verna).
  - The gaps between matches are realistic (2–5 h for the newest, then 20–70 h).
  - One match is left unfinished after three hands.
- Each match records its `MatchSummary` and `ClientMatchReplay`, folded the way the profile does
  (`accumulateStats`):
  - A throw-in replays as `ThrowIn` with a 0–0 delta.
  - The count of hands played leaves throw-ins out, and the replay lists them.
  - Hand deltas add up to the final score.
- Live sim tables are recorded into the same history when they end, and an unfinished one reads
  "Left unfinished".
- `history_page` pages 10 rows with an index cursor. `history_stats` is the profile's
  `compute_stats`.
- Settings live in the sim: they start as `default_settings`, and a patch goes through the
  profile's own `apply_settings_patch`, so a bad patch is refused with its message. New tables
  seat the saved personas.
- The app's sim plays its computer seats with `euchre_ai`: `sim::AiDecider` (`top_move` with
  per-seat `HeuristicOptions`, neutral by default). `SimOptions::default()` still uses
  `HeuristicDecider`, so Lane A's seeded scene and test fixtures do not shift.
- Set `history: 0` for an empty scrapbook (the shell tests do).

### Scenes (render checks)

Lane A wired `games`, `settings` and `match-detail` to `screens::other::fixture_games`,
`fixture_settings` and `fixture_match_detail` (request item 3, done). The fixtures are pure:
the seeded history at `sim::FIXTURE_NOW_MS` (Tue 10 Nov 2026 19:20 UTC), with no network.

These were rendered at 1280×800 and looked at:

- `home`
- `new`, plus a script that picks "Stick the dealer" (the gold ring springs in)
- `games`, and the same at 1280×2400 (pager at the bottom)
- `settings`, and the same at 1280×4000 (every card)
- `match-detail`
- `route-home-new` at `mark:go+0.15` and `+0.6`
- `match-detail` → back to games at `+0.12` and settled

Two layout bugs were caught and fixed this way: the record tile overflowed its card, and the
disclaimer ran off the page. The disclaimer was a `row`, so its text never wrapped.

### Tests added

- `tests/shell.rs`:
  - `route_back_mirrors_the_slide` now asserts the leaving half too. The separate ignored test
    is gone.
  - `games_show_lifetime_stats_page_through_and_open_a_match` (live sim: stats grid, Older →
    page 2, Newer → page 1, open a match, its hands and totals, back).
  - `settings_save_round_trips_through_the_sim` (pace, preset, a dial by click and arrow key,
    a pasted name clamped to 32, save, "Saved." shows and clears on the next change).
  - The live-sim tests start with `tour_seen: true` (request `motion-E-to-D.md` item 1, done).
- `net::sim`:
  - `seeded_history_reads_like_a_real_scrapbook`: order, ids, one unfinished, both outcomes,
    replay/summary agreement, deltas sum to the score, deterministic, under 5 s.
  - `history_pages_and_stats_follow_the_rows`.
  - `settings_patch_round_trips_and_refuses_bad_values`.
- `net::bridge`: `the_first_page_fills_the_shell_stats_and_the_back_stack_resets` (also covers
  `GetStats` → `Reply::Stats`, and get and patch settings).
- `screens::other`:
  - Dates (weekday from the civil date, 12 h clock).
  - The 06 result and delta lines.
  - The form round-trips into one patch.
  - A preset keeps the name.
  - Field clamps by character, not byte.
  - Dial rounding.

## What was cut (09 §6.7 order) and why

1. **Home → Table hero flight (`matched("hero-JS")`, S9, P2). Dropped.** Home never goes
   straight to the table: "Let's play" opens New game (06 §4). A matched flight pairs a node
   removed in one build with one inserted in the same build, so a Jack on Home cannot meet a card
   on the loading table two route changes later. The hero art is also one flattened picture with
   no separate Jack. D-hero-flies is skipped.
2. **Image warm-up (SPIKE-FIRST). Dropped after the spike.** The goal it serves, first-deal
   p95 ≤ 8 ms, is met without it (numbers below). The only slow frame is frame 0, which holds
   setup, and a warm-up strip would add to it rather than remove it.
3. **Home hero parallax and E2 shadows (P3). Cut.**
   - Rune has `on_pointer_move_local`. But the pointer position has to live in `AppState`
     (`app.rs`, Lane A) for the view to redraw from it.
   - The hero is one picture under a vignette mask, so a parallax would slide the whole image,
     mask and all.
   - The cards in it carry their own baked shadows, so E2 has nothing separate to apply to.
4. **Hover lift (−1 pt on hover) for Home and scrapbook buttons. Cut.** Rune has no hover
   offset without state, and per-button hover state would also live in `AppState`. The press
   dip and the sprung hover tint are in.

## Frame times (debug build, 1280×800, scale 1)

| Scene | Animating p95 | Notes |
|---|---|---|
| `table-deal-anim --frames 150` (D-first-deal-p95) | 1.38 / 1.46 / 2.43 / 1.93 ms (4 runs) | frame-cpu p95 1.44–2.63 ms. Gate ≤ 8 ms met. Frame 0 is 31–58 ms (setup and art upload). |
| `route-home-new --frames 60` | 0.66 ms | Was 0.58 ms before the option rings. |
| `match-detail` → Games, staggered list (`--frames 90`) | 2.30 ms | 67 animating frames. |
| `games`, `settings`, `match-detail` at rest | 0.39–0.79 ms forced | Layout nodes 130 / 266 / 122. |

`the_first_deal_stays_inside_the_frame_budget` in `tests/shell.rs` runs the binary and asserts
the 8 ms p95.

Seeding 12 past matches takes a fraction of a second in a test build. The sim test bounds it at
5 s. The live Games test (seed, page, open) finishes in about 0.6 s.

## Open requests

`docs/port/requests/motion-D-to-A.md`:

1. **`child_exit` for the leaving screen.** Done by Lane A. The test now checks both halves.
2. **Lifetime stats.** Lane A added `GamesState.stats` / `newer` and sends `GetStats` on entering
   Games. Lane D added `Reply::Stats`. Open: the one-line `Reply::Stats` arm in
   `AppState::receive` (it lands in the catch-all for now). Until then the stats copy that
   rides with the first page stands in.
3. **Fixture scenes.** Done by Lane A.
4. **The pager's back-stack (new, small).** `GamesState` has no cursor for the page on screen,
   and `enter(Route::Games)` keeps `newer` from the last visit. So the stack stays in
   `Bridge::shell()` for now. Ask: `GamesState.shown` and a `newer.clear()` on entering Games.
   Then `ShellSlots` goes away.

Done for Lane E: `motion-E-to-D.md` item 1 (the live-sim tests start with the tour seen).

## Notes for the next lane

- `SettingsReply.settings` is boxed. Read it with `*reply.settings` or through deref.
- The Games pager reads `state.bridge.shell()` (stats: `state.games.stats` first). Never hold that guard across
  `Bridge::send`, because `send` locks it too.
- `screens::other::fixture_*` are the deterministic scene fixtures. `sim::FIXTURE_NOW_MS` pins
  the dates. The displayed times are local, so a snapshot differs by time zone.
