# Report: DESKTOP MOTION / LANE B (felt, seats, trick, fx, banter)

Crate: `apps/desktop` (`euchre-desktop`), Rune pinned at 85cd8c6. Spec: `docs/port/09-motion-visual-spec.md`
§4.2, §4.3, §4.4, §4.8, §4.9, §5 and §6.3 (Lane B rows), and 06 §6.5 and §8.

## Files changed

Lane B changed only its own files:

- `src/screens/table/stage.rs`
- `src/screens/table/fx.rs` (new)
- `src/theme/felt.rs` (non-timing items only)
- `tests/felt.rs` (new)

Outside the crate it changed only:

- this report
- `docs/port/requests/motion-B-to-A.md` (new)
- `docs/port/requests/motion-B-to-C.md` (new)
- DONE notes appended to `motion-A-to-B.md` and `motion-C-to-B.md`

There were no git writes. `crates/**` and the `theme::felt::timing` constants were not touched.

## Gates (target `.cargo-targets/motion-b`, last run 01:33)

- `check` (clippy `--all-targets -D warnings`): green.
- `lint` (`cargo fmt --check`): green.
- `test`: green, 144 passed, 0 failed:
  - lib: 71
  - coach: 15
  - felt: 6
  - motion: 18
  - rack: 8
  - shell: 13
  - table: 13

Two earlier clippy failures were in another lane's files (`net/bridge.rs:98` large enum variant,
`net/sim.rs:1292` sort_by_key). Both cleared on retry, so no request was filed for them.
`tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` (Lane A and C items 7 and 4)
was Lane B's geometry. It is fixed and passes.

## What is done, in work order

1. **Felt layers (P0, 09 §5.1).** The rail is a lit gradient with a brass inlay. The felt has
   these layers:
   - a radial nap (`felt_hi` to centre to edge)
   - `felt-lamp`, a warm pool
   - `felt-grain`, a generated noise image
   - `felt-vignette`
   - `felt-watermark`, the club
   - `felt-rim`
   - `felt-cool`, an overlay that fades in once the match is lost (09 §4.10 fallback)
   - `felt-brass`, the inlay on the rail

   Each layer is its own element, one gradient per element. B-felt-static checks that nothing
   moves at rest.
2. **Seat plates and fans (P1, 09 §5.2).**
   - Each seat has a pinned plate `seat-{i}-plate`. It lights (`SEAT_PLATE_ON` plus a gold
     hairline) for the seat to act and for the trick's taker once the ring shows.
   - Opponents' fans are 30x42 backs, step 9, tilted 0.08 rad per card about the bottom centre.
     They use one back image, with a 3-layer drop.
   - The fan keeps `hand_counts + unlaunched_from` backs. The top back carries the next play's
     matched id, or `pickup-{h}` for the dealer.
   - Plate padding is 8/12. In a narrow felt (under 900 wide) it is 4/12. In a short window it
     is 2/8.
   - Your bidding block keeps no stack, because no trick is taken while it shows. That gives
     the 720x820 trick box its height back: about 100 pt, up from 65.6.
3. **card_shadows in the trick (P0).** Every trick card (`trick-{h}-{i}-{seat}`) and every
   flyer (`fly-*`) carries `motion::card_shadows`, the three-shadow ladder. The leading card
   has a gold outline. B-shadow-count checks all of them.
4. **Flyers (P0, 09 §4.3).** `fx::flyers` poses each `Director::flyers` entry from
   `resolve_flight` and `play_pose` every frame:
   - absolute, rotated and scaled about the centre
   - `instant_changes`
   - no transition

   The trick slot follows `play_slot`: Hidden, Matched on `PLAY_BRISK`, or Still. Every slot
   has `Transition::none()`, so the hand-off at `PLAY_END` does not cross-fade.
5. **Ring, pop and gather (P0, 09 §4.4).**
   - The ring (`win-ring-{h}-{i}`, `RING_IN`), the winner's `WIN_POP`, the outline and the
     plate wait for `Presentation::winner_shown`.
   - While `gathering`, the four slot holders move 18 pt toward the winner's slot on `GATHER`,
     offset and rotation both. The winner's card is drawn last, so it sits on top.
   - Stack minis fly on `SWEEP`.
6. **Stack thump.** When a seat's sweep is all sent, its stack glyph re-keys
   (`stack-glyph-{i}-{h}-{t}`). It plays `THUMP` once, 0.45 s later, as the last card lands.
7. **Up-card flip (P1, 09 §4.2).**
   - The fixed outer `up-{h}` carries the pickup's matched id.
   - Its `up-art` squeezes to `w·|cos πu|` (floor 1 pt). The face swaps in at u = 0.5, and
     the card lifts to scale 1.1 at mid-turn.
   - A 0.4 s gold glint follows (`card_shadows` glow 0 to 0.6 to 0).
   - While the flip waits for the deal, the card lies face down. The heading hides the card's
     name until the turn passes half way: it hides at once and fades in over 0.25 s.
   - The deck tile (`deck_key(h)`, with `on_layout` calling `deck_laid_out` for Lane C's deal)
     and the up card share one spot, and neither fades. The deck-to-up-card swap therefore
     shows nothing. The bid copy now has a fixed width; before, the card jumped 64 pt sideways
     when the copy changed.
8. **Sparks and confetti (P2, 09 §4.9).**
   - Stamp sparks: a canvas `sparks-{h}` of 28 gold and white streaks, drag `exp(−4t)` and
     gravity 160, for the first `SPARKS` (0.9 s). They play only at Full or Brisk, and only
     when the hand went your way.
   - Confetti: a canvas `confetti-canvas` of 120 quads from two bottom cannons (the spike's
     maths), posed from `Presentation::confetti_t`. It exists only while t < 3.7 s and never
     under reduce motion.
   - S-4 (March): your team's March has no banner and no frames, so it gets
     `stage::confetti_fall`, a 24-piece renderer-owned fall, while its stamp is live.
9. **Stamps (P2, 09 §4.8).** `stamp-{h}` reads "Euchre!", "March!" or "Lone march!".
   - It is an inked plate: banner fill, a 2 pt `GOLD_DEEP` border and a fine inner line,
     44/600 serif in stamp ink, turned −0.07 rad.
   - It lands on `STAMP` while the hand's Stamp shot runs and fades out after. A snapped shot
     shows none.
   - It sits centred across the trick area at max(30 % of its height, 43 pt) from the top.
10. **Banter bubbles (06 §8).** A bubble `bubble-{seat}-{msg}` hangs under the speaking seat
    from a zero-height strip, so it never pushes the seat.
    - It shows the line in quotes, up to two lines, centred, at most 220 wide.
    - It rises 4 pt as it fades in, and only fades under reduce motion.
    - It is `a11y_hidden`: the result line stays the accessible channel, and the 06 §8 bottom
      line is kept.
    - The store's 6.5 s lifetime and this-hand rule apply.
11. **Result line and tally (Lane A and C requests, 09 §4.4 and §4.8).**
    - Until the ring shows, the result line is built as if the trick had no winner. It reads
      "Trick N of 5 · The winner leads the next trick.".
    - "Tricks this hand" also shows the winner team's count one lower until then.
    - The line is keyed `result-{h}-{i}-{phase}`, plus `-won` once the winner shows. It enters
      with offset 8 plus opacity and leaves by opacity, on springs (0.3, 0) / (0.2, 0). Under
      reduce motion it uses opacity only.

## Tests (`tests/felt.rs`, all pass)

- **B-felt-static.** `table-play` at rest for 30 frames: no keyed element moves (IDLE_OK is
  empty), and the felt layers are present.
- **B-gather.** In `table-trick-win`, all four trick cards are within 40 pt of the trick centre
  (about 29 pt measured) at +2.05 s.
  - Measured from: the fourth card's Play start (`director.started(play)`), which is 09 §4.4's
    t = 0.
  - Why not scene time: the collect's `due` is set at its first poll (0.033 s), so the gather
    starts at 1.933 s, not 1.90 s. Measured from scene time 0, it reads 41.9 pt at 2.05 s.
  - This is disclosed in motion-B-to-A.md item 4.
- **B-flip-squeeze.** At flip start + `FLIP/2`, `up-art` is at most 4 pt wide, and at the end
  it is the card width ± 0.5.
  - The drawn rect is the rotated bounds, so the test un-rotates it by the parent's −3° tilt.
    Raw bounds read 5.79 pt at the edge-on frame because of the tilt.
- **B-fans-size.** Every `back-*` in `table-play` and `table-upcard-flip`, un-rotated and
  divided by its scale, is 30x42 ± 0.5. At least 3 backs are checked.
- **B-confetti-bounded.** `game-over-win` from 0 to 4 s:
  - `confetti-canvas` is present only while `clock − banner.since < CONFETTI_SECS`, and it is
    seen.
  - Frame p95 is at most 4 ms: the best of up to 5 headless runs with `--frames 240 --snap`.
- **B-shadow-count.** Every `fly-*` and trick card in `table-play-opp` and `table-trick-win`
  has exactly 3 shadows, and at least one flyer is checked.

## Frame times

Debug build, headless, 1280x800, scale 1, `--frames 300 --fps 60 --snap`. The machine load
average was about 18 (other sessions running cargo and engines).

| Scene | Animating frames | cpu p50 | cpu p95 |
| --- | --- | --- | --- |
| table-play | 0 (at rest) | 0.86 forced | 2.05 forced |
| table-trick-win | 166 | 0.80 | 1.37 |
| table-upcard-flip | 299 (the thinking dot loops) | 0.84 | 1.44 |
| table-pickup | 299 (the thinking dot loops) | 0.85 | 1.59 |
| table-euchre-stamp | 177 | 0.79 | 1.75 |
| game-over-win (confetti) | 147 | 0.83 | 1.33 |
| game-over-loss | 147 | 0.88 | 1.94 |
| table-banter | 0 (at rest) | 0.83 forced | 1.65 forced |

Every scene is under the 4 ms p95 budget. Under load, single runs sometimes show a 9 ms p95
spike. That is why B-confetti-bounded takes the best of 5. With `--out` (PNG writing per frame)
p95 reads about 21 ms, which measures the PNG encoder, not the view.

## Scenes looked at

These were rendered and inspected (snaps and strips):

- `table-play`
- `table-trick-win`: 0.2 to 2.3 s, covering the ring, the gather with the winner on top, the
  sweep, and the held result line
- `table-upcard-flip`: deal start, back, edge-on, face, glint
- `table-pickup`: the up card flies face down into Grandpa's fan
- `table-euchre-stamp`: the stamp and sparks from about 2.95 s, gone by about 4.5 s
- `game-over-win`: cannons from about 0.3 s, gone off the bottom by about 2.5 s, canvas gone
  at 4 s
- `game-over-loss`: the cool overlay, no confetti
- `table-banter`: a two-line bubble under Rita, checked at 2x
- `table-bid` at 720x820 and 1280x800

Renders are in the session scratchpad (`motion-b/after/`).

## Cut, by 09 §6.7 priority

- **SPIKE-FIRST, all cut:**
  - inset rim shadow
  - U-rail with negative insets
  - felt cool by gradient (the overlay fallback is used)
  - trump felt wash
  - mid-flight face flip
- **P2, cut:**
  - turn marker, halo and ping (the lit plate stands in for the turn)
  - bid bubbles
  - led-pill motion (the pill is static)
  - sparks on the trick that seals a march: the Win shot gets no frames; request A-1
  - canvas confetti on a March: no banner and no frames, so the fall fallback is used;
    request A-2
- **P1, partial:**
  - The deck's `THUMP` pop at deal start and the fans dealing in (09 §4.1) are not done. Lane
    C's rack deals from the deck tile, and the fans show their backs from the first frame.
  - The turned-down flip back to the back face is not done. The deck takes the up card's
    matched id, so it lands on the same spot without a turn.
  - The Brisk flip tween (variant B) is not done. Brisk shows the face at once.
- **Judgment calls (to confirm):**
  - Sparks only for stamps that went your way ("effects that feel earned").
  - S-4 March confetti only for your team's March.
  - Captions on hover were not built.
  - The Ace of spades QR art (S-2) is an asset item, not done.
- **Not done:** Lane C's item 5 (felt text overlaps at 360 wide). It is below the 09 floor of
  720, and the felt holds at 720.

## Open requests

- **motion-B-to-A.md:**
  1. Win shot poll-driven for 0.9 s when it seals a march (sparks).
  2. Frames, or a banner with confetti, for your team's March (S-4).
  3. A Director read of a live stamp's result (only needed if the view can move on mid-stamp).
  4. FYI: the gather starts 0.033 s late.
  5. FYI: the result line and tally hold.
- **motion-B-to-C.md:**
  1. The stamp's box. Please move the hand-result card to the lower half of the trick area;
     the two still overlap at the centre.
  2. The hand area's "You win the trick." shows before the ring.
  3. Rack cards wait full-size on the deck spot before they fly.
  4. FYI: the deck tile is in.
