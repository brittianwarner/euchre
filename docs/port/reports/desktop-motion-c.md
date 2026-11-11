# Report: DESKTOP MOTION / LANE C (rack, header, panels, rules)

Crate: `apps/desktop` (`euchre-desktop`), Rune pinned at 85cd8c6. Spec:
`docs/port/09-motion-visual-spec.md` §6.4, with the 06 copy in `docs/port/06-ui-spec.md` §5.3 to
§5.10. The TS reference was read only: `~/goods/euchre/src/lib/ui/RulesPanel.svelte`,
`ScoreBoard.svelte`, and `src/routes/play/[gameId]/+page.svelte`.

Lane C edited only its own files:

- `src/screens/table/hand.rs`
- `src/screens/table/header.rs`
- `src/screens/table/panels.rs`
- `src/screens/table/copy.rs`
- `tests/rack.rs` (new)

Outside the crate, Lane C wrote this report and request files under `docs/port/requests/`. No git
writes were made. Formatting used `rustfmt --edition 2024` on these files only, so no other lane's
file was rewritten.

## Gates

Run with `CARGO_TARGET_DIR=/Users/brittianwarner/goods/euchre-rune/.cargo-targets/motion-c` on
2026-11-11, between 01:00 and 01:30, while Lanes A, B, D and E were still editing. The load
average was 13 to 17.

| Gate | Result |
| --- | --- |
| `check` (clippy `-D warnings`, all targets) | **Green** (final run at 01:20). Earlier passes failed only in Lane B's files: `stage.rs:1412` (`confetti` unused), then `tests/felt.rs:191` (`clippy::map_or_identity`). Both cleared on a 60 s retry, once Lane B landed its fixes. |
| `test` (`--no-fail-fast`) | Every suite passes but one test. Pass: lib (71), `coach` (15), `felt` (6), `motion` (18), `rack` (8/8), `shell` (13). The one failure is `tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat`, at 720x820: `bid-copy [254.5, 215.0, 267.6, 83.5]` is not inside the trick box `[181.9, 224.0, 356.2, 65.6]`. The bid copy and the trick box are laid out in `stage.rs` (Lane B), and the test is Lane A's. Lane C's header does not wrap at 720, and the hand area is `t::hand_h(820) = 344.4`, so the felt's height does not come from Lane C. Lane E's report saw the same failure. It is reported in `motion-C-to-B.md` §4 and `motion-C-to-A.md` §6. |
| `lint` (`fmt --check`) | **Green.** |

`pgrep -fl motion-c` was empty at the end: no test or build process is left under Lane C's
target dir.

## What is done, in the 6.4 order

1. **Card-sized holes.** `hand::hole_key(c)` keeps a card-sized hole in the rack while a card is
   out (`hole-{c}`, `cell_w × h`). Lane A's play flyer leaves from it.
2. **Plate-child hover and select, S-1 lifts.** The outer `plate-{c}` carries the lift on
   `motion::HOVER`:
   - hover: −10, scale 1.04;
   - selected: −18, scale 1.06, gold glow and a focus outline;
   - origin is the bottom centre.

   The art `card-{c}` keeps `.matched(hcard-{hand}-{c})` and carries the elevation shadows
   (`motion::card_shadows`), so a matched play carries its own shadow. Reduce motion keeps the
   card in place, and the glow still marks the selection.
3. **Legal dim.** A card you may not play is drawn at 0.55 over a plain white plate face, not
   over the felt. Its line reads `Must follow suit`.
4. **Shake.** This is poll-driven from `present.shake_t(clock)` and `choreo::shake_x`. The
   offset and a 0.004 rad/pt tilt are on the plate, with `instant_changes`, and the plate springs
   home afterwards. Moving the shake from the art to the plate stops the dimmed card's white face
   showing as a ghost edge. There is no shake under reduce motion.
5. **Deal-in from the deck.**
   - Each cell travels from the deck centre on `DEAL_CARD` / `DEAL_CARD_OUT`. The source is
     `layout(deck_key(hand))`, then `up-{hand}`, then `present.deck_at`.
   - Timing is `deal_delay(j)`, in the real 3-2 / 2-3 packet order from the dealer's left
     (`copy::deal_index`). Brisk halves the delays.
   - The tier is read from the deal's own shot, `director.feel_of(&ShotKey::deal(hand))`.
   - Skim, Snap and reduce motion skip the deal. Lane A clears `deal_in` on Snap.
   - **Fixed this pass:** the cards still waiting on the deck showed face up. A child's entrance
     does not play when its parent is inserted in the same commit, so the plate now holds a
     delayed `Sequence` (`DEAL_SHOW`: opacity 0 → 1 in 0.1 s, `.delay(deal_delay)`) in place of
     an opacity transition. Each card stays hidden until its turn.
6. **Rack width rule.** The rack card width is `clamp(112, 0.092 W, 140)`, then capped by the
   rack's room. C-rack-not-smaller holds at 1280x800: the rack card is 111.5 wide, and the trick
   card is no wider.
7. **Captions on hover.** There is one line under each card:
   - the reason it is selected or held back;
   - otherwise, while the pointer is on it, its name (`Jack of spades`).

   The line keeps its height when empty, so nothing moves.
8. **Trump pip.** A 20 pt disc sits on every trump card, the left bower included. It fades in on
   the call (`table-trump-call`: opacity 0.62 at 0.10, 0.92 at 0.20, 1.0 at 0.50).
9. **Play button presence.** It enters and leaves on `BUTTON_IN` springs, and the key press uses
   `KEY_PRESS`.
10. **Result line.** This is in `stage.rs` (Lane B), so it is a request, not done here
    (`motion-C-to-B.md` §1).
11. **Hand-result card** (`hand-result-{hand}`, in a raised `hand-result-layer` over the trick
    area). It holds:
    - the title, from `announcer::result_label` (`Point.`, `March!`, `Euchre!` and so on);
    - the line (`They score 1.`);
    - five trick pips, gold for the maker's tricks;
    - a gold `+N` chip.

    It enters on `panel_transition`. **Fixed this pass:**
    - It now waits for the hand's Score shot (`!director.pending(&ShotKey::score(hand))`). Before,
      it sat over the last trick while that trick was still on the felt (seen at 1.40 s).
    - The chip pops in with a delayed `Sequence` (opacity 0 → 1, scale 0.8 → 1 over 0.22 s,
      0.18 s after the card), for the same nested-entrance reason as the deal.
12. **RollingText score.** The header numbers step one point per `SCORE_STEP` from
    `present.score`. `RollingText` rolls the glyph in on `ROLL`. When the roll lands, the number
    pops (`NUM_POP`, sampled on the frame clock) and a gold `PULSE` ring plays behind it, keyed
    by value so each landing replays it. **Fixed this pass:**
    - The pop wrapper used `instant_changes`. That also landed `RollingText`'s glyph insert and
      exit at once, so no digit ever rolled. It now uses `.motion(Spring::NONE)`, which lands
      only the wrapper's own scale.
    - Under reduce motion the number is one stable key (`score-us-value`), so the new value lands
      in place with no crossfade.
    - The dead `header::header` wrapper is deleted, now that mod.rs calls `header_with`.
13. **Dialogs and scrim.**
    - One shared scrim (`panels::scrim`): `#0b1d16` at 0.45, blur 10 (none when reduced), on
      `SCRIM_IN` / `SCRIM_OUT`.
    - Sheets enter on `panel_transition`: scale 0.9 + offset (0, 30) + opacity in, scale 0.96 +
      opacity out, `PANEL_IN` / `PANEL_OUT`. Reduce motion fades only.
    - Escape and a backdrop click now animate out, like the close button (no `tx.instant()`).
    - The rules dialog follows 06 §5.10:
      - a search box (`Search the rules…`);
      - sections closed by default with a rotating chevron, and every match open while there is
        a query;
      - `No section matches "{query}".`;
      - a dashed `New to euchre? Take the quick tour again` button.

      Lane A wired the `rules_search` `TextEdit` and the `RulesState` hooks (`motion-C-to-A.md`,
      items 1 to 4 DONE).
14. **Settings width fix.** The overlay rail is `rail_width(W) = min(340, W − 32)`. **Fixed this
    pass:** at 360 wide the header overflowed, and `Settings` sat at x = 466, off a 360-wide
    window. At 700 points or narrower, the header now wraps the way the web table does:
    - the score spans the top line, your team at the left and theirs at the right, with compact
      numerals;
    - `How to play` and `Settings ⌄` sit at the two ends of a second line (44 pt, 15 pt type).

    `header_tools` gained a `narrow` flag. Above 700 wide the header is unchanged.

Also done, for Lane E: the bid buttons carry the coach mark (`hint::move_badge`).

## Tests (`tests/rack.rs`, 8/8 pass)

| Test | What it checks |
| --- | --- |
| `c_hover_lift` | On `table-hover`, after +0.5 s of hover, the plate's bottom edge moves −10 ± 1 and its width scales 1.04 ± 0.01. It is back at rest 0.6 s after the pointer leaves. |
| `c_shake` | The scene sets `present.shake`. `card-{c}` swings ≥ 4 pt in its first 8 frames and is within 0.5 pt by +0.4. Under reduce motion it does not move (≤ 0.01). |
| `c_deal_from_deck` | On `table-deal-anim` at 0.10: rack card 0 is within 60 pt of the deck (or the up card standing in for it), and at least one waiting card is hidden (opacity < 0.01). At 2.0 every card is at rest and fully shown. |
| `c_score_rolls` | On `table-hand-result`: a glyph is mid-roll 0.1 s after the first step. The labels equal the view's score and nothing rolls 1.3 s after the step. Under reduce motion the final number is shown in place on the first frame. |
| `c_rack_not_smaller` | At 1280x800, every rack card is at least as wide as the widest trick card. |
| `c_settings_720` | At 720x520 and 360x640, Settings opens a rail whose laid-out and drawn rects are inside the window, and Escape closes it. It also checks `rail_width`. |
| `c_dialog_scrim` | On `table-rules-open` at 0.12, the scrim's opacity is between 0 and 1 and the sheet's scale is below 1. Both are settled by +1.0. Under reduce motion the sheet does not scale. |
| `the_rules_search_filters_and_the_sections_toggle` | The matching, open-and-flip, and no-match copy rules, with the 06 string verbatim. |

The helpers run one extra frame after the first, because a scene's moment lands on the frame
after the first. Without it, the hand is empty and `present.shake` is unset.

Three deviations from the 6.4 text:

- **C-score-rolls is timed from the step, not from 0.3 s.** In `table-hand-result` the Score
  shot starts at about 2.95 s, after the last trick is played and gathered. Lane A's Director
  orders it that way, so nothing has rolled at 0.3.
- **The settings rail's key is `table-settings`, not `settings-rail`.**
- **The 360 case is added to C-settings-720.**

## Scenes looked at

All scenes are 1280x800, scale 1, unless a size is given.

| Scene | At | Seen |
| --- | --- | --- |
| `table-deal-anim` | 0.10, 0.75, 2.0 | At 0.10 all five rack cards are hidden at the deck. At 0.75 the ace of hearts is in flight and the queen is leaving the deck, with nothing waiting face up. At 2.0 the rack is at rest. |
| `table-illegal-shake` | 0.03, 0.06, 0.20, 0.40 | At 0.03 the king of hearts (dimmed, `Must follow suit`) is shifted right and tilted, and its face moves with it. The instruction reads `You still hold the jack of spades — you have to follow suit.` It is home by 0.40. |
| `table-hover` (`scripts/hover.json`) | hover+0.1, hover+0.6 | The jack of spades is lifted −10 and scaled 1.04, with the caption `Jack of spades`. The king stays dimmed. |
| `table-hand-result` | 1.40, 2.90, 3.05, 3.20, 3.40 | At 1.40 the last trick is on the felt with no card over it. At 3.05 the card is entering and the chip is still hidden. At 3.20 the full card shows `Point.`, `They score 1.`, 4 of 5 pips and the chip popping. The header `1` has the gold ring. |
| `table-rules-open` | 0.12, 0.60 | At 0.12 the sheet is growing in over a fading scrim. At 0.60 the title, table line, `Right now` box, search box and closed sections with chevrons all show. |
| `table-trump-call` | 0.10, 0.20, 0.50 | The trump pips fade in on the jack of diamonds, the jack of hearts and the ten of diamonds, and the bid panel fades out. |
| `table-settings` | 1.0 at 1280x800, 720x520, 360x640 | The rail is inline at 1280 and an overlay with a blurred scrim at 720. At 360 it is 328 wide at x = 32. |
| `table-play` | 1.0 at 360x640, 700x560, 720x520 | The header wraps at 700 and narrower, and both buttons are inside the window. At 360, the felt's own text overlaps (`motion-C-to-B.md` §5). |

## Frame times

These are `rune:` stats lines from the debug binary, 1280x800, run while other lanes were
compiling (load average about 15). The first-deal budget is p95 ≤ 8 ms.

| Scene (to) | Animating frames | cpu p50 | cpu p95 |
| --- | --- | --- | --- |
| `table-deal-anim` (2.0) | 120 | 0.90 ms | 1.92 ms (a quieter run: 0.82 / 1.80) |
| `table-rules-open` (0.60) | 36 | 0.86 ms | 1.07 ms (other runs: p95 2.29 to 4.53) |
| `table-trump-call` (0.50) | 30 | 1.11 ms | 1.83 ms |
| `table-hand-result` (3.40) | 72 | 0.49 ms | 2.93 ms |
| `table-hover` (hover+0.6) | 36 | 0.78 ms | 3.04 ms |
| `table-illegal-shake` (0.40) | 0 (poll-driven commits) | 0.90 ms, forced at rest | 1.40 ms |
| `table-settings` (1.0) | 0 | 0.61 ms, forced at rest | 1.72 ms |

The first frame is 32 to 46 ms (text shaping and art), as in the other lanes. One
`table-rules-open` run had a 113.7 ms max frame under the heaviest load. It did not come back in
two later runs, where the max was the first frame.

## Cuts (09 §6.7 priority)

- **P2, the trump badge in the header (`TRUMP_IN`): not done.** It is the lowest priority, and
  every P0 and P1 item in Lane C's list came first. The felt title already names trump, and each
  trump card carries the pip.
- **The felt buttons' −1 hover offset: cut.** Rune at P has no hover offset: `.hover()` changes
  only the fill. The buttons keep the hover fill and `KEY_PRESS`. This is polish outside the
  priority list.
- **P1, the result line: moved to Lane B.** `stage.rs` draws it (`motion-C-to-B.md` §1).
- **The chip's 14 pt rise and the plate's deal fade-in are simplified to Sequences** (opacity
  plus scale, no offset). A child's entrance does not play when its parent is inserted in the same
  commit, and a `Sequence` carries no offset. The card itself still rises in on
  `panel_transition`.

## Open requests

- **`motion-C-to-B.md`.**
  - §1, the result line's keyed 09 §4.8 transition: open.
  - §2, a `deck-{hand}` tile: drawn now, and the rack deals from it.
  - §3 and §6, a stamp that lands with the Score shot meets the hand-result card at the trick
    centre: FYI.
  - §4, the bid copy outgrows the trick box at 720x820 (the failing `tests/table.rs` test):
    open.
  - §5, the felt's text overlaps at 360 wide: FYI.
- **`motion-C-to-A.md`.**
  - Items 1 to 4 are DONE by Lane A and taken up by Lane C.
  - §6 is the `table.rs` failure, for Lane A's information.
