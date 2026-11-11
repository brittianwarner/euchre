# Desktop table stage report

Lane: DESKTOP. Scope is 06 §5, §6, §7, §9, §12 and §14, plus the table parts of §20. The screen renders only from `TableStore` state plus `Presentation` state (ARCHITECTURE §2.5).

## Gates

All three gates pass, with `CARGO_TARGET_DIR=.cargo-targets/desktop`:

- `cargo fmt -p euchre-desktop -- --check`
- `cargo clippy -p euchre-desktop --all-targets -- -D warnings`
- `cargo test -p euchre-desktop`: 29 unit tests, 5 shell tests and 6 table tests.

## What was built

All of it lives in `apps/desktop/src/screens/table/`:

| File | What it holds |
| --- | --- |
| `copy.rs` | Every 06 copy string, kept verbatim, and every predicate that picks one. |
| `present.rs` | Presentation state, described below. |
| `header.rs` | The cream header: wordmark home link, the six-and-four ScoreCards, sliding score numerals, and "First to 10" when the window is wider than 1050. |
| `stage.rs` | The felt, described below. |
| `hand.rs` | The hand area, described below. |
| `mod.rs` | The screen: header, felt plus hand, the reconnect notice, the banner and confetti, the §14 loading state, and the global table keys. |

`present.rs` holds the presentation state:

- **Rack slots.** Cards keep their places, and a refused card returns to its slot.
- **Trick collect.** It starts after 2.2 s, staggers each card 0.045 s, waits for Continue in review mode, and sends Continue once the cards land.
- **Banners.** A banner shows for 4.5 s, with 2.4 s of confetti.
- **Illegal-card message.** It shows for 3.2 s.
- **Other state:** Go alone per decision, deal-in, hover, and the hook state.

`stage.rs` draws the felt:

- the rail and the status row (trump line with its glyph, led pill and lone line)
- seats with D16 names, the initial, and the maker, lead and dealer markers
- a "Passed" or card-count detail and a thinking dot
- each opponent's fan of card backs, and each seat's trick stack
- the tilted trick slots, with the winner outlined
- the up card or deck tile, with the round 1 and round 2 copy
- the result line, with Continue or "Collecting… →"

`hand.rs` draws the hand area:

- the heading and the turn instruction
- the CardRack, with select-then-play, lift, deal-in and slot retention
- the BidPanel: order up or name a suit, Pass, and Go alone
- a fixed-height controls row, so nothing shifts when it changes
- the Play and Play again buttons
- the error notice

**Changes in `app.rs`:**

- `TableScreen.present`, and an `after_store()` call after every store change.
- New input methods: `press_card`, `play_card`, `play_selected`, `continue_trick`, `toggle_go_alone`, `hover_card`, and `set_review_tricks`.
- The presentation poll runs in `app::poll`.
- Presence events now mark a seat offline.
- `motion_reduced` takes either the preference or the OS setting (D15).
- A sync that carries steps now animates on each element's own spring.

**Motion:**

- Cards fly from hand to trick and from opponent to trick using `matched` ids.
- The up card is picked up, or turns into the deck tile when it is turned down.
- The trick moves to the winner's stack.
- Each hand deals in.
- Score numerals slide to their new value.
- Under reduce motion, nothing is matched, nothing lifts or loops, there is no confetti, and the trick collect happens at once.

## Scenes

Each scene drives a `TableCore` deterministically with `Tempo::INSTANT` and `HeuristicDecider`. The snapshots are in `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/desktop-snaps/`:

- `table-deal.png`
- `table-bid.png`, the second round of bidding with the up card turned down
- `table-play.png`
- `table-trick-won.png`
- `table-hand-scored.png`
- `game-over.png`
- `table-play-720.png` and `table-bid-720.png`, at the 720×520 minimum

## Tests (`tests/table.rs`)

- `a_whole_match_plays_to_game_over_through_the_ui`: plays a whole match against SimBackend through the bridge, clicking only UI keys (`bid-*`, `hand-{c}`, `hand-play` and `trick-continue`) with trick review on. It checks that someone reaches 10, Play again appears, and Play again goes to /new.
- `an_illegal_card_is_refused_and_the_selection_stays`: the 06 reason is shown and nothing is sent. The message clears after 3.2 s.
- `a_reviewed_trick_waits_for_continue`
- `reduce_motion_quiets_the_table`: confetti is off and the trick collect is immediate. The existing shell test still covers the rack lift.
- `number_keys_and_arrows_reach_the_rack`
- `the_table_fits_the_smallest_window`

## Deviations

- The Play button sits in the fixed controls row under the rack, and your seat stays on the felt.
- The "Must follow suit" caption is light on the felt rather than #735548, for contrast.
- The pickup is a straight flight, because the flight does not carry rotation.
- The trick collect fades by opacity rather than turning the cards over (no rotateY).
- The opponents' fans of card backs are an addition. They are the source of each opponent's card flight.
- The deck tile uses a gradient instead of stripes.
- Below a 900 px felt width, the led pill drops under the trump line, positioned out of layout so nothing shifts. In a short window, your trick stack floats beside the trick.
- Scene snapshots are a single frame, so entering transitions show at rest.

## Fixes found by the match test

The table changes the trick's turn id when it opens the trick for review. Continue now answers with the newest turn id.

## Hooks left (state only, no UI)

These are in `Presentation.hooks` and on `AppState`:

- `settings_open`, `rules_open`, `tour_step` and `hand_review_open`
- `hints`
- the `sound` cue queue (D14: rodio, off by default)
- the `announce` step queue, for the announcer
- `offline[4]`, for presence
- `set_review_tricks`, for the settings rail
- banter: `TableStore::visible_table_talk` already feeds the result line. Speech bubbles are not drawn yet.

## Review fixes

Every reviewer finding was checked against the code and the snapshots. None was rejected. Each change is listed below.

### Fixed

| Pri | Finding | Fix |
| --- | --- | --- |
| P0 | Home collapses at narrow widths | The home layout reflows. It was checked at 720x2600 and 1280x1900. |
| P1 | Trick cards too small to read | The trick cards were enlarged. Test `trick_cards_read_at_the_default_window` checks they are at least 100 px wide at 1280x820. |
| P1 | Your seat missing outside bidding | Your seat row shows in every phase. `you_below` moves it beside the bid copy when the felt is short. |
| P1 | Stage elements overlap | The bid copy width now adapts (clamped 200..440). Its second line shows only when there is room. Test: `the_bid_copy_stays_in_its_box_beside_your_seat`. |
| P1 | Side seats spill past the rim | `SIDE_SEAT` widened to 224 px. Test: `seat_rows_stay_inside_the_rim`, at 1280x820 and 720x820. |
| P1 | Header tools, settings rail, rules dialog and hand review missing or broken | All of these are wired. Tests: `header_tools_open_and_close_the_panels` and `the_narrow_settings_rail_lies_over_the_table`. |
| P1 | No live deal-in | `after_change` cues Deal once for a new hand or an empty rack. Unit test: `a_live_deal_into_an_empty_rack_deals_in_once`. |
| P1 | Card art credits and license missing | Settings has an About card with the credits and a "Show the card art license" toggle. Test: `settings_credit_the_art_and_show_its_license`. |
| P2 | Compact h1 and labels | Sizes are tuned for compact windows. |
| P2 | Felt padding and RESULT_H | Adjusted. |
| P2 | Banner position | Fixed. |
| P2 | Play again outside the heading row | Moved into the heading row. |
| P2 | New game: Deal me in falls below the fold | The page tightens below 700 px tall. Test: `deal_me_in_stays_above_the_fold_in_the_smallest_window`. |
| P2 | Prefs writes race | A single writer keeps only the newest prefs. Temp file names are unique (pid plus sequence), and the directory is fsynced after the rename. |
| P2 | `tx.instant()` on every sync | Snaps happen only when the hand, phase or trick changes, or after a reconnect. |
| P2 | Continue failure not tied to its request | The `RequestId` is tracked. Only the matching `ReviewSettled{ok:false}` (or a refused send) puts the trick back. Test: `only_the_continue_failing_puts_the_trick_back`. |
| P2 | Weak tests | Keyboard, fit and whole-match tests now assert focus and full rects, and wait for cards to land. |
| P3 | Opacity, trump glyph, footer and text-link centering | Fixed. |
| P3 | CloseTable racing an open | Each open gets an epoch. A close during the open wins, and the late connection is closed. Test: `a_close_during_the_open_wins`. |
| P3 | No bridge shutdown | Added `Intent::Shutdown`, with a join bounded at 500 ms on exit. Test: `shutdown_ends_the_loop_while_a_bridge_is_alive`. |
| P3 | Sim drift from the TS server | The sim now matches the server in five ways: (1) presence flips only on the first connect and last disconnect; (2) submit checks run in the TS order; (3) move ids are cut to 64 UTF-16 units; (4) the cut id is what gets remembered; (5) a finished table is removed after its summary is recorded. Test: `a_remembered_move_id_acks_and_long_ids_are_cut_to_64`. |

### Extra fixes found while re-snapping

- The header Settings button overflowed at 720. Score cards are now hidden below 800 px wide.
- "Grandpa" was cut to "Gran…" at 1280. This is fixed by the 224 px side seat.
- The bid copy overlapped your seat at 1280x820 and overflowed at 1000x700. This is fixed by the adaptive bid layout above.

### Deviations kept

- The rules dialog has no search and no accordion. It is plain scrolled text.
- Hint suggestions need euchre-ai, which D28 forbids until W3. Only the setting toggle exists.
- The brand mark is hidden below 1000 px wide, and the felt rim is dropped in compact.
- The sim cuts by UTF-16 units without splitting a character. JS `slice` can split a surrogate pair.
- A duplicate of a move that is no longer legal returns `illegal_move`, following the TS check order. The sim test was updated to match.
- Known: rack card names truncate at 1000x700 ("Jack of…"). They read fully at 1280x820 and 720x520.

### Gates after fixes

- `cargo fmt -p euchre-desktop -- --check`: clean
- `cargo clippy -p euchre-desktop --all-targets -- -D warnings`: 0 warnings
- `cargo test -p euchre-desktop`: lib 33, shell 7, table 13 — all pass

The snapshots are 29 PNGs: every scene at 1280x820 and 720x520, home at full height, and the bid at 1000x700. They are in `/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/desktop-snaps/final/`.
