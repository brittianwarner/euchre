# Desktop motion wave: integration report

**Back to:** the lane reports `desktop-motion-w0.md`, `desktop-motion-a.md` to `desktop-motion-e.md`,
and the spec `docs/port/09-motion-visual-spec.md`.

The five lanes (A choreography, B felt and fx, C rack, header and panels, D shell, E coach, sound,
narration and tour) are merged in `apps/desktop`. This pass:

- closed every open cross-lane request
- ran the three gates
- rendered every scene and every 09 §6.6 frame and strip, and fixed what looked wrong
- measured frame times and idle
- checked reduce motion both ways

The build is the debug profile with dependencies at opt-level 2 (09 §2.7). Rune stays at rev
85cd8c6. The target dir is `.cargo-targets/motion-a`.

## Gates

| Gate | Command | Result |
|---|---|---|
| check | `bun run --cwd apps/desktop check` (clippy, all targets, `-D warnings`) | pass |
| test | `bun run --cwd apps/desktop test` | pass, 148 tests, 0 failed |
| lint | `bun run --cwd apps/desktop lint` (`cargo fmt --check`) | pass |

## What this pass changed

| Change | Files | Why |
|---|---|---|
| The match-end panel (09 §4.10). The match's banner is now a panel centred at 38% of the window's height. It arrives on `PANEL_IN` over a scrim with no blur. Before, it was the low banner over the hand area (r6). In-game banners (going alone, Lone march) still sit over the hand area. The panel and scrim take no press, so Play again works while they show. | `table/mod.rs`, `table/panels.rs` (`plain_scrim`), `theme/felt.rs` (`MATCH_END_AT`) | Seen in `game-over-win` and `game-over-loss`: the banner sat low and doubled the hand area's heading. |
| The stamp sits on the hand-result card's title. `fx::stamp_centre` puts the plate's lower edge 5 pt above `hand-result-{h}-line`. The title hides under the stamp while it shows, and fades back as the stamp leaves. When no card shows, the trick-area centre is still the fallback. | `table/fx.rs`, `table/hand.rs` | Seen in `table-euchre-stamp` at 3.3 to 3.8 s: the stamp covered both "Euchre!" and "You score 2." on the card. |
| The hand area names the trick's winner on the ring's beat, not while the last card is in the air. | `table/hand.rs`, `table/stage.rs` (`ring_waits`) | B→C item 2. |
| The place thud is pitched by its card (06 §15.2). | `sound.rs`, `tests/coach.rs` | E→A item 2, the sound.rs half. |
| The Your games pager keeps its back-stack in `GamesState` (`newer`, `shown`). `ShellSlots`, `Intent::AttachShell` and `Bridge::shell()` are removed. | `net/bridge.rs`, `screens/other.rs`, `tests/shell.rs` | D→A item 4, Lane D's half. |
| New tests: `b_stamp_clears_the_hand_result_line`, `b_match_end_panel` (felt.rs), `the_system_setting_matches_the_preference` (motion.rs), `play_again_works_under_the_match_end_panel` (table.rs), `your_games_pages_and_reads_the_lifetime_stats` (bridge.rs). | tests | Each one pins a fix from this pass. |

## What shipped, by moment (09 §4)

| § | Moment | Shipped | Not shipped |
|---|---|---|---|
| 4.1 | Deal | Rack cards fly from the deck tile in the real 3-2 / 2-3 packet order. Each card stays hidden until its own delay. Brisk halves the delays. Skim, Snap and reduce motion skip the deal. | The deck's `THUMP` pop. The opponents' fans do not deal in: they show their backs from the first frame. |
| 4.2 | Up-card turn | The width squeezes, the face swaps at the halfway point, the card lifts to scale 1.1, then a 0.4 s gold glint. The card's name shows only once the face does. | The turn back to the back face when the card is turned down (the deck takes its spot without a turn). The Brisk tween (Brisk shows the face at once). |
| 4.3 | Card play | Flyers posed by the Director every frame, for both opponents and you. A toss path with a skid and squash. Card-sized holes in the rack. The fans keep their unlaunched backs. The place thud fires at touchdown, pitched by the card. Brisk and Skim use the matched path. | The mid-flight face flip (spike-first). |
| 4.4 | Trick won | Ring, pop, winner outline and the lit seat plate all wait for the Win shot. Gather 18 pt toward the winner, then a sweep into the stack minis, then a stack thump. Under reduce motion the ring shows at once and the sweep is instant. | The 80 ms sweep wind-up (dropped by 09). |
| 4.5 | Turn indicator | The lit seat plate marks the seat to act. The AI's thinking dots stay. | The gold turn marker, its halo and the ping (P2). |
| 4.6 | Bids, trump, pickup | The trump pip fades in on every trump card. The pickup flies on the matched path into the dealer's fan or your rack. | Bid bubbles (P2). The trump badge's `TRUMP_IN` in the felt title (P2). The trump felt wash (spike-first). |
| 4.7 | Illegal card | A poll-driven shake on the plate, the legal dim at 0.55, and "Must follow suit" under the card. Under reduce motion only the message shows. | None. |
| 4.8 | Result line and hand scored | The result line is keyed by its moment and enters and leaves. A hand-result card shows the title, the line, five trick pips and a `+N` chip. The header score rolls one point per step, with a pop and a pulse ring. The Euchre, March and Lone march stamps land on the card's title. | Launching the `+N` chip into the score (spike-first; the chip pops in place). |
| 4.9 | Bursts | Stamp sparks: 28 streaks, only when the hand went your way. Canvas confetti from two cannons on your match win. A renderer-owned confetti fall on your team's March. | Sparks on the trick that seals a march, and canvas confetti on a March (see Cuts). |
| 4.10 | Match end | The centred panel at 38% over a plain scrim, confetti on a win, and the score roll. A loss cools the felt with the overlay fallback. | The gradient cool (spike-first). |
| 4.11 | Rack hover and select | The plate-child pattern: hover −10 and scale 1.04, selected −18 and scale 1.06 with the gold glow, press 0.96. Each card has a caption line that keeps its height. | None. |
| 4.12 | Screen transitions | Routes slide 70 pt by direction, and the leaving screen exits the way the change goes. Dialogs get a blur scrim and `PANEL_IN`. The settings rail slides. Under reduce motion, routes and dialogs fade only. | The Home → Table hero flight (Home never goes straight to the table). The table build-in stagger (P2). |

## What shipped, by screen (09 §5)

- **5.1 Felt and rail (Lane B).** A lit rail with a brass inlay, and a felt built in layers: nap,
  lamp, grain, vignette, club watermark, rim and the loss-cool overlay. Nothing moves at rest.
- **5.2 Seats (Lane B).** Pinned seat plates that light up. Opponent fans of 30x42 backs, all
  from one back image. Banter bubbles hang under the seat without pushing it.
- **5.3 Trick area (Lane B).** Every trick card and flyer carries the three-shadow ladder, and
  the leading card has a gold outline. The deck tile and the up card share one spot. The bid
  copy has a fixed width, so the card does not jump.
- **5.4 Rack and controls (Lane C).** The rack width rule is `clamp(112, 0.092 W, 140)`. The
  hover, select, legal dim, shake and trump pip are all in. The Play button enters and leaves
  on springs.
- **5.5 Header (Lane C).** The rolling score. The header wraps onto two lines at 700 pt wide or
  narrower.
- **5.6 Dialogs and settings rail (Lane C).** One shared scrim. Sheets enter on
  `panel_transition`. Rules has search and an accordion (06 §5.10). The overlay rail is
  `min(340, W − 32)` wide.
- **5.7 Home, New game, Games, Settings, Match detail (Lane D).**
  - List staggers.
  - Option cards with a sprung gold ring.
  - Press physics.
  - A clean Ace of spades (S-2).
  - The real scrapbook screens on the typed profile seam: lifetime stats and the Your games
    pager.
- **Lane E (06 §10, §13, §14.3, §15).**
  - The coach marks the suggested move, and the bid buttons carry its badge.
  - The sound cues: the place thud pitched per card, and the test cue when sound is turned on.
  - The live-region narration.
  - The first-run tour, with its own scrim and dialog.

## What was cut, and why

| Item | Priority | Why |
|---|---|---|
| Turn marker, halo and ping | P2 | The lit plate already marks the turn. Leaving them out also keeps the AI's think time idle-cheap. |
| Bid bubbles | P2 | Lower priority than every P0 and P1 item in Lane B. The bid shows in the seat's status line. |
| Trump badge `TRUMP_IN` | P2 | Lane C did every P0 and P1 item first. The felt title already names trump, and each trump card carries the pip. |
| Sparks on the trick that seals a march | P2 | The Win shot runs 0.40 s. Giving it 0.9 s of frames changes what waits on the Win. The stamp's own sparks still fire about 2.4 s later. |
| Canvas confetti on a March | P2 | A canvas burst needs 3.7 s of driven frames, and the Stamp shot lives 1.44 s. The renderer-owned fall plays instead and costs no frames. |
| Deck pop and fans dealing in | P1, partial | The rack deals from the deck. The fans show their backs from the first frame. |
| Turned-down flip back, and the Brisk flip tween | P1, partial | The deck takes the up card's matched id and lands on the same spot without a turn. Brisk shows the face at once. |
| Home → Table hero flight | P2 | Home opens New game, not the table, so no build holds both ends of a matched flight. |
| Home parallax, E2 on Home, and the −1 pt hover lifts | P3 | Rune at this pin has no hover offset without per-button state in `AppState`. |
| Spike-first items: inset rim shadow, U-rail, gradient felt cool, trump felt wash, mid-flight face flip, launching the `+N` chip, image warm-up | spike | Each has a working fallback, or its goal is met without it. For example, the first-deal p95 is about 1.2 ms against the 8 ms limit, so no warm-up is needed. |
| Felt buttons' −1 hover offset | polish | The same Rune limit. The hover fill and the key press are in. |
| The scrim dip when the tour reopens from Rules | P1, minor | The table brightens for about 0.1 s on that one path. A fix needs the rules and tour dialogs to share one scrim. |

## Frame times

Each scene was run 3 times with
`euchre --scene X --frames 240 --fps 60 --size 1280x800 --scale 1`, with no `--out`, since
writing PNGs inflates p95. The table gives the best run's animating p50 and p95, the median and
worst p95 across the three runs, and the first frame.

The machine was shared with other sessions: load average 9 to 11 on 14 cores. Limits (09 §2.7):

- animating p95 ≤ 4.0 ms
- first deal p95 ≤ 8.0 ms
- animating p50 ≤ 1.5 ms

| Scene | Animating frames | p50 ms | p95 ms (best) | p95 median | p95 worst | First frame ms |
|---|---|---|---|---|---|---|
| `table-deal-anim` (first deal) | 239 | 0.81 | 1.20 | 1.38 | 1.51 | 33.1 |
| `table-upcard-flip` | 239 | 0.80 | 1.19 | 1.25 | 1.26 | 32.9 |
| `table-play-opp` | 57 | 1.05 | 1.30 | 1.39 | 1.55 | 32.4 |
| `table-play-you` | 239 | 0.76 | 1.29 | 1.30 | 1.34 | 32.6 |
| `table-trick-win` | 166 | 0.91 | 1.39 | 1.43 | 1.45 | 32.3 |
| `table-trick-win-reduced` | 20 | 0.91 | 1.85 | 2.02 | 2.23 | 32.9 |
| `table-pickup` | 239 | 0.84 | 1.35 | 1.36 | 1.38 | 33.1 |
| `table-trump-call` | 239 | 0.76 | 1.20 | 1.33 | 1.40 | 33.5 |
| `table-illegal-shake` | 20 | 0.86 | 1.67 | 1.70 | 2.05 | 32.8 |
| `table-hover` | 36 | 0.56 | 0.66 | 0.68 | 0.71 | 32.2 |
| `table-hand-result` | 149 | 0.79 | 1.77 | 1.81 | 1.87 | 32.3 |
| `table-euchre-stamp` | 149 | 0.82 | 1.82 | 1.83 | 1.91 | 31.6 |
| `game-over-win` | 162 | 0.85 | 1.19 | 1.22 | 1.23 | 33.2 |
| `game-over-loss` | 162 | 0.92 | 1.91 | 1.95 | 2.00 | 32.9 |
| `table-rules-open` | 74 | 0.75 | 0.91 | 0.99 | 1.02 | 32.3 |
| `route-home-new` | 68 | 0.29 | 0.48 | 0.48 | 0.49 | 5.0 |
| `table-reconnect` | 0 | at rest | at rest | at rest | at rest | 32.3 |
| `table-play` | 0 | at rest | at rest | at rest | at rest | 32.3 |

Every scene is inside every limit in every run. The worst p95 is 2.23 ms (`table-trick-win-reduced`,
20 frames). The worst p50 is 1.05 ms.

The first frame (31 to 34 ms) is setup: text shaping and the art upload. That matches the lanes'
reports, and 09 §2.7 counts only animating frames. Scenes with 239 animating frames have an AI
seat thinking: its dots loop for the whole run (`seat-*-thinking`, allowed by `IDLE_OK`).

## Idle

Each scene was traced with `--trace out.jsonl --start 4.0 --frames 30`. Over the 30 frames from
4.0 s, only `IDLE_OK` keys changed:

- `seat-1-thinking` in `table-deal-anim`, `table-upcard-flip`, `table-play-you` and
  `table-trump-call`
- `seat-3-thinking` in `table-pickup`
- nothing in the other 11 scenes

Two scenes are still mid-moment at 4.0 s, so they were traced again from 6.0 s:

- `table-hand-result`: the card's spring settles at 4.17 s
- `table-euchre-stamp`: the stamp shot ends at 4.78 s

From 6.0 s both are fully at rest: 0 animating frames, 0 changes. No idle wake loop was found.
`the_table_sleeps_after_the_last_shot` (motion.rs) also checks that the presentation asks for no
more wakes once the last shot is done.

## Reduce motion (D15: the app preference OR the system setting)

- **Wiring.** `app.rs::view` sets `reduced = prefs.reduce_motion || cx.system_reduced_motion()`
  and passes it to `cx.reduced_motion`. Every screen reads that one value.
- **`table-trick-win-reduced`.**
  - The 0.0 / 0.2 / 0.75 / 2.3 / 3.1 s frames show no flyer and the ring at once.
  - The sweep is instant at `COLLECT_AFTER`.
  - 20 animating frames: opacity tracks only, no travel.
- **A-reduced-none** (`reduce_motion_draws_no_flight_and_no_burst`) passes for both the OS and
  the preference: no flyer, ring pop, sparks or confetti, and the gather is on time.
- **New this pass: `the_system_setting_matches_the_preference`.** Two runtimes play
  `table-trick-win`, `table-euchre-stamp`, `game-over-win` and `table-deal-anim` for 4 s each:
  - one with the in-app toggle on and the OS setting off
  - one with the toggle off and the OS setting on

  Every frame presents the same keyed rects, opacities and scales, and the OS run draws no
  flight or burst. That is the 09 §6.6 "identical" check, done headless.
- **Other reduce-motion tests that pass.**
  - `reduce_motion_lands_everything_at_once` (choreo)
  - `the_sheet_rises_in_and_reduce_motion_only_fades_it` (coach)
  - `route_change_under_reduce_motion_only_fades`
  - `reduce_motion_is_respected`: the rack, by preference and by OS (shell)
  - `reduce_motion_quiets_the_table` (table)
- **Not done.** A manual run with macOS System Settings → Accessibility → Reduce motion. The
  headless runtime's `set_system_reduced_motion` is the same input the window gets from the OS.

## Renders

Every scene in `scenes.rs` (34 in all) was rendered at 1280x800 scale 1. Each scene dir also
holds a 720x520 and a 1440x900 frame at 5 s, and every 09 §6.6 `--at` frame and strip. They are
in the session scratchpad, one dir per scene:
`/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/motion/integration/`.
Each dir has a `log.json` with every run's stats line. Every run exits 0.

What was looked at:

- Every static screen.
- Every motion frame and strip.
- The size variants.
- `table-reconnect`: the 0.0 s and 1.0 s frames differ by 0 px.
- `table-euchre-stamp` at 720x520, 960x640 and 1440x900.

## Requests

Every item in `docs/port/requests/motion-*.md` now ends in DONE or CLOSED with a reason:

- **Already DONE by the lanes:** A→B 1–8, A→C 1–3, C→A 1–4, C→B 1, 2 and 4, D→A 1–3, E→A 1
  and 3, E→C 1.
- **DONE this pass:** B→C 1 and 2, C→B 3 and 6, D→A 4 (Lane D's half), E→A 2 (the sound.rs half).
- **CLOSED with a reason:** B→A 1–4, B→C 3, C→B 5, C→A 6, E→D 1.

## Tests

148 pass, 0 fail:

| Target | Tests |
|---|---|
| lib unit tests | 71 |
| `tests/coach.rs` | 15 |
| `tests/felt.rs` | 8 |
| `tests/motion.rs` | 19 |
| `tests/rack.rs` | 8 |
| `tests/shell.rs` | 13 |
| `tests/table.rs` | 14 |

## Known gaps

- **Short windows.** At 520 to 640 pt tall (the compact felt), the hand-result card is taller
  than the trick box. It covers the middle of the result line ("You win the tr… of this hand.")
  while it shows, and the stamp's top edge reaches the partner's seat plate for about a second.
  At 800 pt and taller, both clear.
- **The pickup copy wraps.** In `table-pickup`, "Right opponent picked up Queen of diamonds."
  wraps to two lines in the fixed-width bid copy, so "wins." drops onto a line of its own just
  above your seat plate. Nothing overlaps.
- **Crossfades.** For about 0.1 s around the trump call, the bid panel's fading You plate and
  the play layout's dealer chip overlap at low opacity. As the stamp fades, its ghost overlaps
  the card's returning title for about 0.2 s.
- **The match-end scrim** dims Play again and the header for the banner's 4.5 s. Play again still
  works under it (`play_again_works_under_the_match_end_panel`).
- **Reduce motion by hand.** No manual macOS check yet (see Reduce motion).
- **Narration and the tour.** Steps that land while the tour is open are not spoken, because
  Rune makes the table behind a modal inert (Lane E).
- **Every cut item above.**

## Review fixes

Four reviewers scored the integrated wave: visual 5.5, behaviour 7.5, spec 7, features 8. This
pass checked each finding against the code and fixed every real P0 and P1, plus the P2s that
were cheap and clearly better. Where findings pulled against each other, 09 and the owner's ask
decided: bold, physical and beautiful, never in the way of play, and calm under reduce motion.
Some rows above ("What was cut", "Known gaps") are out of date where this section says so.

### Gates after the fixes

| Gate | Result |
|---|---|
| check (clippy, all targets, `-D warnings`) | pass |
| test | pass, 154 tests, 0 failed: lib 72, coach 15, felt 9, motion 22, rack 8, shell 14, table 14 |
| lint (`cargo fmt --check`) | pass |

No process under `.cargo-targets/motion-a` was left running (`pgrep -fl` is empty).

### Visual (V1 to V14)

| # | Finding | Outcome |
|---|---|---|
| V1 | P0. Match end was a bare pill over four unswept cards, under a heavy scrim, with confetti only in the corners. | **Fixed.** In play, the panel's `MatchEnd` shot waits for the last trick's sweep and the hand's score roll. A match that ends with nothing to wait for (a first view or a snap, as in the game-over scenes) sweeps its last trick at once, so the cards are gone before the panel is opaque. `b_match_end_panel` asserts no trick card shows once the panel's opacity passes 0.9. The panel is 448 pt wide, centred at 38% of the window's height, and enters on `PANEL_IN` with E5. It holds the star and line, both final scores (they roll to the last hand's points, then the winners' number pops) and Play again. The scrim is 0.25 alpha and sits under the panel. The confetti is now a fountain that rises from behind the panel's top edge once the panel has landed (`CONFETTI_DELAY` 0.55 s). |
| V2 | Score digits rolled outside their cell. | **Fixed.** `header::number` wraps the roll in a `{key}-clip` cell with `clip_fade` at its top and bottom edges. The clip sits outside the pop, so it covers the whole roll. |
| V3 | A played card showed twice for about 0.2 s. | **Fixed.** The rack's cards leave with `child_exit(Transition::none())`, so the slot is empty in the same frame the flyer launches. Only the flyer carries the card. |
| V4 | The picked-up card became an oversized back the frame it left. | **Partly fixed.** The face now flies. It lands at `PICKUP_LAND` (0.40 s), then squeezes over to a back across `PICKUP_TURN` (half of `FLIP`), the same squeeze as the up-card turn. Skipped: "scale to the fan's size only in the last 30% of the flight". A matched flight springs its rect as one, so scale cannot be split from travel without a separate flyer. |
| V5 | The deal had no rhythm, and the bid status started before you held cards. | **Fixed.** The opponents' fans deal in with the rack's `deal_delay`, round-robin, so a card is always in the air. The deck knocks once (`THUMP`) as a Full deal starts. The status line holds "Dealing your cards…" (06 verbatim), and the seats hold their bid status, until the last card lands. |
| V6 | The trick win barely read, and your team's stack sat under the trick. | **Fixed.** The winning card lifts with a gold glow under a 3 pt ring. `WIN_POP` peaks at 1.13 and settles by 0.30 s. Your team's marks and stack moved to the felt's bottom-left corner (`you_marks`), so the gather and sweep carry the cards toward you. **Decision:** 09 §3 says 1.09; 1.13 is what reads at table distance, and the comment on `WIN_POP` says so. **Decision:** the ring stays `GOLD` (`FELT.winner` now points at `motion::GOLD`), not the suggested `GOLD_DEEP`. `GOLD_DEEP` is the stamp's border and the pips' fill, and the lighter gold reads as "won" against the dark felt. |
| V7 | Calling trump had no emphasis, and the old and new copy crossfaded. | **Fixed.** `TRUMP_IN` and a gold flash play on the felt's trump line. The maker chip knocks (`THUMP`). The old copy fades out on `TRUMP_OUT` (0.12 s) before the new line enters, so they never overlap. The `Trump` shot now has a renderer (see S7). |
| V8 | Route changes left a muddy double image, and the navs overlapped. | **Fixed.** The leaving screen fades as it slides, on `ROUTE_OUT` (a 0.16 s spring), so it is mostly gone before `ROUTE_IN` is far along. The pill nav lives in one `nav-layer` outside the route transition. Not done: fading through the target background on a theme change. At go+0.15 the two pages are both faint, and the old one is nearly gone. |
| V9 | Small chips floated under the trick area. | **Fixed.** Your dealer and maker chips sit in `you_marks` with "You". Every other seat's chips sit on its plate. |
| V10 | The result line crossfaded even when its text barely changed. | **Fixed.** The line is keyed on a hash of its copy, so the same text never re-enters. A real change swaps on `LINE_SWAP`: the old line leaves in 0.10 s and the new one enters over 0.30 s. |
| V11 | The bid panel's watermark club ran through the bid copy. | **Fixed.** The watermark fades to 0.3 of its strength while the bid copy shows (`WATERMARK_UNDER_COPY`), on the `UI` spring. |
| V12 | New game has no logo, and the nav pill sits about 10 pt from the right edge. | **Skipped.** 06 §109 sets the pill's 0.6rem inset, and 06 §4's verbatim New game content has no brand row. Changing either is a spec change for the owner. |
| V13 | The stamp's sparks crossed Ned's seat plate. | **Fixed.** The burst is flat on top: sparks that go up start 20 pt out and travel a quarter as far (`SPARK_RISE`), so they stay below the partner's plate. Sideways and downward sparks are unchanged. Seen in `table-euchre-stamp` at 3.07 s. |
| V14 | The up card looked small next to the hand. | **Partly fixed.** The card's margin in the bid row is 8 pt (`BID_CARD_MARGIN`), so it grows to the row's full height, and it carries a 0.2 lift (`UP_CARD_LIFT`). At 1440x900 it is clearly larger. It is still not exactly hand-card size: that needs the felt to take height from the rack, which moves the rack (S-1 territory). |

### Behaviour (B1 to B5)

| # | Finding | Outcome |
|---|---|---|
| B1 | P1. Double-clicking Play in the Home header went to New game and then straight back Home. | **Fixed.** `AppState::nav_to` ignores a nav press for `ROUTE_SETTLE` (0.5 s) after a route change, so the second click lands on nothing. `ROUTE_SETTLE` now has a use; its doc covers both the spec meaning and the guard. |
| B2 | A reconnect snap left entrance fades running. | **Partly fixed.** The snap path runs `present.snap()` (it clears the Director and marks the next commit quiet, so nothing in it flies) plus `tx.settle()`, so the card in the air lands and nothing travels or scales from the next frame. `a_reconnect_lands_everything_at_once` now asserts that with the reconnect at t0+0.1. **Limit:** at Rune 85cd8c6, `settle` does not land a presence (mount) fade that an earlier commit already started. Those opacity fades finish on their own, within about 0.6 s. Landing them needs a Rune change, and Rune is pinned. |
| B3 | Turning reduce motion on mid-flight blanked the card for one frame. | **Fixed.** The trick slot draws the card `Still` whenever `ctx.reduced` holds and the director still says `Hidden`. The director catches up on the next poll. |
| B4 | Springs keep the frame loop running about 0.5 s after things look settled. | **Skipped.** Rune's transitions are springs only (no tween in `rune-motion` at this pin), and the rest epsilon belongs to Rune. `route-home-new` still has 68 animating frames. The cost is about 0.3 ms a frame for half a second, inside every budget. A visible-epsilon rest belongs upstream. |
| B5 | The reduce-motion test looked for a `win-ring-pop` key that never exists. | **Fixed.** See S3. |

### Spec (S1 to S9)

| # | Finding | Outcome |
|---|---|---|
| S1 | P1. When the 4th card won the trick, no ring showed until the 0.95 s hand-off. | **Fixed.** When the Win shot begins (touchdown + `RING_DELAY`, 0.60 s), the flyer is dropped and the slot draws the card `Still`, with its ring and pop. New test `a_ring_shows_when_the_last_card_wins` drives a trick won by its last, non-human card (it tries the fixed seeds in turn). It asserts no ring at 0.3 s, and the ring and the trick card with no flyer at 0.70 s. |
| S2 | The stack knocked about 0.15 s before the last card and `Cue::TrickTake`. | **Fixed.** `THUMP_AFTER` is now `motion::SWEEP_SECS` (0.60 s), the same time `Collect::finish_at` uses for `TrickTake`. New test `b_the_stack_knocks_as_the_last_card_lands` asserts the knock starts within two frames of the take. |
| S3 | Most of the A-reduced-none check could not fail. | **Fixed.** `reduce_breaks` (motion.rs) flags any `fly-`, `sparks` or `confetti` key. It also flags any scale away from 1 (more than 1e-3), or any travel (more than 0.5 pt), on the trick's cards, `win-ring-*`, `stack-glyph-*` and `hand-result-*-chip`. `reduce_motion_draws_no_flight_and_no_burst` first plays the same trick with motion on and asserts that the check catches the flyer and the pop, so the check can fail. Then it runs by the OS and by the preference and asserts nothing is flagged. New `reduce_motion_quiets_the_euchre_stamp` asserts sparks with motion on and nothing flagged under reduce motion. `the_system_setting_matches_the_preference` uses the same check. **Spec wording:** 09 §6.2 should name the check by what it measures (no scale or travel on the trick card, its ring, the stack glyph and the chip), not a `win-ring-pop` key. That edit is for the spec's owner. |
| S4 | Three stale wave-0 doc comments. | **Fixed.** `Presentation.director`, `Hooks` and `AppState.sound` describe what the code does now. The sound doc reads: off until the player turns sound on (D14), and heard only while the window shows. The motion harness header no longer says "wave 0". |
| S5 | Motion and size literals outside the theme, some duplicated. | **Fixed, one kept.** New tokens: `FLIP_GLINT`, `PICKUP_LAND`, `PICKUP_TURN`, `KEY_PRESS`, `BUTTON_IN`, `RISE`, `SEAT_PLATE`, `NAME_FADE`, `FELT_COOL` and `TRUMP_OUT` in `theme/motion.rs`. `FAN_TILT`, `WATERMARK_UNDER_COPY`, `SPARK_RISE` and `STAMP_TURN` are in `felt::size`. The two `FLIP_GLINT`/`GLINT` copies, the two `BUTTON_IN`s and the two 0.25 fades are now one each. Choreo's pickup tilt calls `stage::fan_tilt` with the fan's `FAN_MAX` cap, and the unexplained `- 0.10` is gone. `FELT.winner` points at `motion::GOLD`. The Director's durations are derived: Flip from `FLIP + FLIP_GLINT`, Pickup from `PICKUP_LAND + PICKUP_TURN`, Win from the longer of `WIN_POP` and `RING_IN`, and Stamp from `STAMP_HOLD` plus the `STAMP` beats. **Kept:** `SCORE` (1.0 s) stays a typed constant. It times the hand-result card's arrival, which is a presence spring plus the chip's delayed pop, not one beat list, so there is no single token to derive it from. |
| S6 | P2 bursts shipped while P1 sub-items were cut, against §6.7's order. | **Fixed by finishing two items and giving reasons for two.** Done this pass: the deck's knock and the fans dealing in (V5). Still cut, routed for owner sign-off with S-1 to S-4: (1) **The turn back when the up card is turned down.** The deck takes the up card's matched id and lands on the same spot, so the card already goes home. A turn would play while the copy changes to round two and compete with it. (2) **The Brisk flip tween.** Brisk means the table is behind. Showing the face at once lands it sooner, and the `Flip` shot's Brisk time still gates what follows. **Why the order inverted:** the lanes cut from the bottom of their own lists, so a P2 in Lane B (stamps, sparks, confetti) shipped while a P1 sub-item in another lane was cut. |
| S7 | Two Director shots ran with nothing drawing them. | **Fixed.** `Trump` now has a renderer: the felt's trump line plays `TRUMP_IN` and the flash while it runs, and the maker chip knocks (V7). `Bubble` lasts 0 s (`BUBBLE = 0`), so it never holds the queue or schedules a wake. The seat's detail line and banter bubble already show the bid, so no separate bid chip draws. |
| S8 | A few view functions are long (`rack` 162 lines, `hand_review_dialog` 166, `hero` 128, `receive` 128, `trick` 94). | **Skipped.** Splitting them is a refactor with no change in behaviour. Doing it in the same pass as the motion fixes would mix that risk into this review. It is a follow-up. |
| S9 | Two soft tests. | **Fixed.** The stamp test reads the plate's half-height from its own drawn rect, turned back by `size::STAMP_TURN`, and ends with `.expect("the title is back")`. New `a_click_passes_through_a_live_burst`: while `sparks-*` is mounted, a click opens the table settings, and while `confetti-canvas` is mounted, a click on Play again opens New game. Both canvases cover the whole window. |

### Features (F1 to F4)

| # | Finding | Outcome |
|---|---|---|
| F1 | P1. Headless and scene runs read and wrote the player's real prefs file. | **Fixed.** `PrefsStore::for_run` gives a headless or `--scene` run no store: it starts from the defaults and writes nothing. `EUCHRE_PREFS_DIR` still overrides. Test: `a_scene_run_never_touches_the_platform_prefs`. **Owner action:** delete `~/Library/Application Support/sh.euchre.Euchre/prefs.json` on this machine. The earlier lane runs wrote `"tourSeen": true` there, so the first-run tour will not open until it is gone. This pass did not touch the file. |
| F2 | The sound toggle showed "On"/"Off" text and no tooltip. | **Fixed.** A 20 pt speaker drawn with `canvas`: two waves when on, an X when off. `.help("Sound on" / "Sound off")` is the tooltip. The label, description and checked state are unchanged. |
| F3 | "Try refreshing the page." in a window with no refresh. | **Fixed.** The notices read "Couldn't open your scrapbook just now." and "Couldn't open your settings just now." Each is followed by a "Try again" link that re-runs the route's load (`AppState::reload_route`, the same call `enter` makes). This is a native adaptation of the 06 sentence. |
| F4 | The Settings device switches sat unmarked among fields that apply only on Save. | **Fixed.** Sound, Reduce motion and Hints sit in their own group (`settings-device`) under a hairline, with the heading "On this computer" and the note "These change right away. No need to save." `settings_credit_the_art_and_show_its_license` now scrolls to the license toggle, since the page is taller. |

### Tests changed or added in this pass

- **felt.rs:**
  - `b_match_end_panel`, rewritten for the panel
  - `b_confetti_bounded`, which now allows for the confetti's delay
  - `b_stamp_clears_the_hand_result_line`, hardened (S9)
  - new `b_the_stack_knocks_as_the_last_card_lands` (S2)
- **motion.rs:**
  - new `a_ring_shows_when_the_last_card_wins` (S1)
  - `reduce_breaks` replaces `burst_keys`
  - `reduce_motion_draws_no_flight_and_no_burst`, rewritten with a motion-on control
  - new `reduce_motion_quiets_the_euchre_stamp`
  - new `a_click_passes_through_a_live_burst`
  - `a_reconnect_lands_everything_at_once`, tightened (B2)
- **table.rs:**
  - `reduce_motion_quiets_the_table`, which now checks the panel and watches the whole confetti window
  - `play_again_works_under_the_match_end_panel`, which presses both the panel's and the rack's Play again
- **shell.rs:** `settings_credit_the_art_and_show_its_license` scrolls to the toggle.

### Renders

Every scene was re-rendered, with its 09 §6.6 frames and strips, into the session scratchpad
`motion/fix/` folder. That is 34 scenes, and every run exited 0. I looked at each touched one:

- `game-over-win` at 0.4 s and 3.0 s: the panel over a clear felt, with confetti rising behind
  it.
- `table-trick-win` at 0.75 s: the glow and ring, with your stack bottom-left.
- `table-deal-anim` at 0.5 s: the fans dealing in while the status says "Dealing your cards…".
- `table-pickup`: the face flies, then turns to a back.
- `table-trump-call`
- `table-euchre-stamp` at 3.07 s: the sparks clear Ned's plate.
- `table-bid`: the up card is larger, and the watermark is dimmed.
- `table-settings`: the speaker icon.
- `settings`: the device group.
- `route-home-new`
- `table-hand-result`

### Frame times after the fixes

The method is the same as above: 3 runs per scene, 240 frames, 1280x800, scale 1, no `--out`.
Load average was 4 to 7. Every scene is inside every limit. The worst best-run p95 is 2.02 ms
(`table-hand-result`), and the worst p50 is 0.99 ms.

| Scene | Animating | p50 ms | p95 ms best | p95 median |
|---|---|---|---|---|
| `table-deal-anim` | 239 | 0.74 | 1.21 | 1.25 |
| `table-upcard-flip` | 239 | 0.75 | 1.16 | 1.18 |
| `table-play-opp` | 57 | 0.99 | 1.38 | 1.45 |
| `table-play-you` | 239 | 0.75 | 1.31 | 1.31 |
| `table-trick-win` | 166 | 0.90 | 1.32 | 1.43 |
| `table-trick-win-reduced` | 20 | 0.88 | 1.37 | 1.89 |
| `table-pickup` | 239 | 0.82 | 1.43 | 1.46 |
| `table-trump-call` | 239 | 0.79 | 1.27 | 1.30 |
| `table-illegal-shake` | 20 | 0.89 | 1.57 | 1.69 |
| `table-hover` | 36 | 0.60 | 0.85 | 0.86 |
| `table-hand-result` | 107 | 0.84 | 2.02 | 2.09 |
| `table-euchre-stamp` | 107 | 0.87 | 1.82 | 1.89 |
| `game-over-win` | 89 | 0.93 | 1.41 | 1.47 |
| `game-over-loss` | 86 | 0.91 | 1.52 | 1.59 |
| `table-rules-open` | 74 | 0.85 | 1.19 | 1.26 |
| `route-home-new` | 68 | 0.27 | 0.55 | 0.58 |
| `table-reconnect` | 0 | at rest | at rest | at rest |
| `table-play` | 0 | at rest | at rest | at rest |

`game-over-win` counts fewer animating frames than before (89, was 162). The confetti's frames
come from the app's own wake (`poll` asks for a frame each tick while the burst lives), which
the stats line does not count as animating. A headless check confirms the frame loop runs
until the burst ends at about 4.5 s, so the confetti never freezes in mid-air.

### Idle after the fixes

The trace now records every frame from 0, so the 30 frames from 4.0 s were read out of it by
time.

- **IDLE_OK only:** in `table-deal-anim`, `table-upcard-flip`, `table-play-you`, `table-pickup`
  and `table-trump-call`, only `seat-*-thinking` (the `loop`) changes.
- **Nothing changes** in 11 scenes, including `game-over-win` and `game-over-loss`.
- **Still mid-moment at 4.0 s:**
  - `table-hand-result`: the card's spring settles at 4.17 s.
  - `table-euchre-stamp`: the stamp's designed hold ends at 4.78 s.

  Traced again from 6.0 s, both, plus `game-over-win`, have 0 animating frames and 0 changes.

### Owner actions from this pass

- **Prefs file.** Delete `~/Library/Application Support/sh.euchre.Euchre/prefs.json` (F1).
- **Sign-off.** Sign off the two remaining P1 cuts in S6 (the turned-down turn back, and the
  Brisk flip tween), alongside S-1 to S-4.
- **Decisions to confirm:**
  - `WIN_POP` 1.13 rather than 09's 1.09 (V6)
  - the ring stays `GOLD` (V6)
  - the "Try again" adaptation of the 06 load-failure sentence (F3)
  - the 0.5 s nav guard (B1)
- **Spec edit (09 §6.2).** Reword the A-reduced-none check as in S3.

## Polish round

The second visual critique scored the app 7/10. This pass checked each finding against the
code, then fixed them from most visible to least. Rows above that this section contradicts
(V4's "Skipped" part, V8's slide and fade) are out of date.

### Gates after the polish

| Gate | Result |
|---|---|
| check (clippy, all targets, `-D warnings`) | pass |
| test | pass, 159 tests, 0 failed: lib 72, coach 15, felt 9, motion 25, rack 8, shell 16, table 14 |
| lint (`cargo fmt --check`) | pass |

No process under `.cargo-targets/motion-a` was left running (`pgrep -fl` is empty).

### Findings

| # | Finding | Outcome |
|---|---|---|
| P1 | Route changes: two navs, a double image, a grey smear and an edge strip. The table had no build-in (§4.12). | **Fixed.** The new screen arrives as one sheet with its own background and its own pill nav. It wipes in over the old page with `Transition::reveal`, from the trailing edge going forward and the leading edge going back, on `ROUTE_IN`. The old page stays still under it and fades on `ROUTE_OUT`, so the wipe never shows an empty background. The root fill is the old route's background. Each change raises the new sheet's `paint_order`. When you leave the table, it fades on `ROUTE_OUT_TABLE` (0.12 s), because the match-end panel and confetti paint above every sheet; they are gone by go+0.24. On arrival, the seats and the rack build in after the wipe (`Ctx::build_in`): they rise `RISE` and fade in on `BUILD_IN`, seats 0.04 s apart, the rack 0.2 s in. The loading page blinks out (`motion::blink_out`, 0.06 s), so "Reconnect to this game" never fades over the felt. Under reduce motion, a route change is a plain fade. Files: `src/app.rs`, `src/screens/mod.rs`, `src/theme/motion.rs`, `src/screens/table/stage.rs`, `src/screens/table/hand.rs`, `src/screens/table/mod.rs`. |
| P1 | The result line overprinted: old and new lines both showed. | **Fixed.** The new line waits `LINE_SWAP_GAP` (0.09 s) with `enter_delay`, by which time the old line (out in 0.10 s on `LINE_SWAP`) is nearly gone. New frame test `the_result_line_never_overprints`: in three scenes, at most one `result-*` line is above 0.2 opacity in any frame. File: `src/screens/table/stage.rs`. |
| P2 | Match end: the last sweep flew across the panel. | **Fixed.** A sweep forced by the match ending (`match_now`) lands in place, with no flight. File: `src/screens/table/present.rs`. |
| P2 | Match end: the trick-result line, the hand area's heading and its Play again showed under the panel. | **Fixed.** Once the panel shows, the result line and the hand-result card blink out, and the hand area's heading and line fade and leave the accessibility tree. Play again lives only in the panel. Files: `src/screens/table/stage.rs`, `src/screens/table/hand.rs`. |
| P2 | Match end: the loss panel carried the win star. | **Fixed.** The star shows only when your team won. The loss panel reads "A good game. Well played." with no mark. File: `src/screens/table/mod.rs`. |
| P2 | Deal captions: "5 cards" showed while the cards were still dealing, and "The up card" showed before the flip. | **Fixed.** While the hand deals, each seat reads "Dealing…" (`copy::SEAT_DEALING`). The bid eyebrow and line stay hidden until the Flip shot starts, then fade in on `NAME_FADE`. Under reduce motion they show at once. Files: `src/screens/table/copy.rs`, `src/screens/table/stage.rs`. |
| P2 | Turn marker: none, and no halo on your turn. | **Fixed.** One gold ring (`turn-marker`, 2 pt) frames the seat on turn and travels to the next one on `MARKER`. On your turn only, a halo (`turn-halo`) breathes under it on the `TURN_HALO` cycle; under reduce motion it holds still at `TURN_HALO.still`. The marker hides while the hand deals, while a trick's winner shows, and once the match is over. File: `src/screens/table/stage.rs`. |
| P2 | The 1.6 s bid chip on `CHIP_IN`/`CHIP_OUT`. | **Not built.** The seat status already says each bid in words. The chip is next in line. |
| P2 | At 720x520, the trick cards were bigger than the rack's. | **Fixed.** Below `size::COMPACT_H`, the played cards take `PLAYED_H_SHARE_COMPACT` (0.68) of the trick's height: about 49 pt wide against the rack's 53. `c_rack_not_smaller` now checks 1280x800 and 720x520. Files: `src/theme/felt.rs`, `src/screens/table/stage.rs`, `tests/rack.rs`. |
| P2 | At 720x520, seat names truncated ("Grandpa"). | **Fixed.** At `SEAT_TIGHT_W`, the name and badges take one line and the status takes a second, in compact too. File: `src/screens/table/stage.rs`. |
| P2 | At 720x520, the hand-result card overflowed. | **Fixed.** In compact, the card has tighter padding, a 22 pt title, a 15 pt line, its pips and chip in one row, and a height cap. File: `src/screens/table/hand.rs`. |
| P2 | Pickup: the card shrank for its whole flight. | **Fixed (this was V4's skipped half).** While the pickup runs, the dealer's top back is laid out at the up card's size, so the matched flight only moves it. A poll-driven scale about its bottom centre (mechanism A) holds it at full size until the last `PICKUP_SHRINK` (30%) of `PICKUP_LAND`, then brings it down to a back's size. The Pickup shot is now poll-driven. Before this, the face-to-back squeeze after landing only redrew when something else asked for a frame. New test `the_pickup_flies_at_up_card_size_and_shrinks_last`. Under reduce motion there is no flight and the back is drawn at fan size. Files: `src/screens/table/stage.rs`, `src/screens/table/choreo.rs`, `src/theme/motion.rs`. |
| P2 | Sweep: the winning card went under the others. | **Fixed.** The taker's card leads the sweep order and paints on top in the gather and in the minis (`paint_order`). Files: `src/screens/table/present.rs`, `src/screens/table/stage.rs`. |

### Tests added or changed

- `tests/shell.rs`: new `route_forward_wipes_the_new_sheet_over_the_old_page`,
  `route_back_wipes_the_old_screen_back_over`, `route_wipe_edge_follows_the_direction` and
  `leaving_the_table_clears_its_raised_layers_quickly`. The reduce-motion route test is kept.
  The nav and new-game tests wait `ROUTE_SETTLE` before they click on a new screen.
- `tests/motion.rs`: new `the_result_line_never_overprints`,
  `the_turn_marker_travels_and_the_halo_is_yours` and
  `the_pickup_flies_at_up_card_size_and_shrinks_last`. `IDLE_OK` holds `turn-halo`.
- `tests/felt.rs`: `IDLE_OK` holds `turn-halo` (09 §2.7 allows the one cycle).
- `tests/rack.rs`: `c_rack_not_smaller` runs at 1280x800 and 720x520.
- `tests/table.rs`: new `play_again_works_under_the_match_end_panel` (win, loss and the plain
  game-over scene): no hand-area Play again, no trick-result line, and the panel's Play again
  works. The whole-match test waits `ROUTE_SETTLE` after the route change and ends on the
  panel's Play again.

### Renders

All renders are in
`/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/motion/polish/`.

- `final-new-table-sheet.png`: New game to the table. One nav, the felt wipes in over the
  page, and the seats and rack build in after it.
- `final-gow-again-sheet.png`: Play again from a won match. The table and its panel are faint
  by go+0.12 and gone by go+0.24.
- `final-home-games-sheet.png`: Home to Your games.
- `final-go-win-sheet.png`, `final-go-loss-sheet.png`: the panels with no trick line, no hand
  heading, and no star on the loss.
- `final-deal-sheet.png`, `final-flip-sheet.png`: "Dealing…" on every seat while cards fly. The
  eyebrow and up-card line wait for the flip.
- `final-play-opp-sheet.png`, `final-play-sheet.png`: the marker travels from Grandpa to you,
  and the halo shows on your turn.
- `final-s720-trickwin-sheet.png`, `final-s720-result-sheet.png`, `final-s720-bid-sheet.png`:
  720x520, with trick cards smaller than the rack's, two-line names, and the hand-result card
  inside the felt.
- `pickup-strip.png`: the face flies at the up card's size, shrinks at the end, lands, and
  turns.
- `final-sweep-strip.png`: the winning card stays on top as the cards gather.

### Frame times after the polish

The method is the same as above: 3 runs per scene, 240 frames, 1280x800, scale 1, no `--out`.
Load average was 5 to 7. Every scene is inside every limit. The worst best-run p95 is 2.03 ms
(`table-hand-result`), and the worst p50 is 1.10 ms (`table-play-opp`).

| Scene | Animating | p50 ms | p95 ms best | p95 median |
|---|---|---|---|---|
| `table-deal-anim` (first deal) | 239 | 0.79 | 1.34 | 1.39 |
| `table-upcard-flip` | 239 | 0.79 | 1.34 | 1.38 |
| `table-play-opp` | 75 | 1.05 | 1.33 | 1.38 |
| `table-play-you` | 239 | 0.83 | 1.33 | 1.42 |
| `table-trick-win` | 172 | 0.86 | 1.48 | 1.54 |
| `table-trick-win-reduced` | 26 | 0.95 | 2.16 | 2.18 |
| `table-pickup` | 239 | 0.88 | 1.52 | 1.58 |
| `table-trump-call` | 239 | 0.85 | 1.39 | 1.39 |
| `table-hand-result` | 107 | 0.92 | 2.03 | 2.08 |
| `table-euchre-stamp` | 107 | 0.94 | 1.99 | 2.06 |
| `game-over-win` | 89 | 0.79 | 0.99 | 1.00 |
| `game-over-loss` | 86 | 0.75 | 0.92 | 1.05 |
| `table-bid` | 66 | 0.78 | 1.02 | 1.13 |
| `route-home-new` | 0 | at rest | at rest | at rest |
| `table-play` | 0 | at rest | at rest | at rest |
| New game to table (script) | 120 | 0.64 | 1.32 | 1.40 |
| Play again from a win (script) | 119 | 0.77 | 0.99 | 1.01 |
| `table-deal-anim` at 720x520 | 239 | 0.30 | 0.60 | 0.61 |

`route-home-new` now has 0 animating frames (it was 68): the first screen no longer plays a
route transition. In `table-play` the halo's cycle runs, but the stats line does not count it
as animating.

### Notes

- **Hit-testing follows the wipe.** The part of an incoming sheet that is not yet revealed takes
  no presses. Tests that click right after a route change wait `ROUTE_SETTLE` first. A player
  cannot click faster than the wipe in practice, and the nav guard already ignores a second nav
  press for `ROUTE_SETTLE`.
- **`IDLE_OK` is `["turn-halo"]`.** The halo is the one element allowed to change at rest, and
  only on your turn.
- **Play again is only in the panel** once a match ends. The hand area keeps its place, but its
  heading, line and button leave.
- **A removal can't be made instant.** Kit rules always fade a removed element, so
  `Transition::none()` as an exit still fades. `motion::blink_out()` (0.06 s) is the stand-in.
- **Not built:** the bid chip (above) and a turn-change ping.
