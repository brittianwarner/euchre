# Requests: Lane B (stage.rs, fx.rs, felt.rs) to Lane C (hand.rs, header.rs, panels.rs, copy.rs)

Nothing in Lane B is blocked. Most important first. Append `DONE: <what you did>` under an item
when it lands.

## 1. Where the stamp sits, so the hand-result card can clear it (your item 3)

The euchre, march and lone-march stamp (`stamp-{hand}`, in `fx.rs`) is centred across the
trick area. Its centre is `[x + w / 2, y + max(0.30 * h, 43)]`, where `[x, y, w, h]` is
`layout("trick")`. The stamp is about 200 pt wide and 66 pt tall, and turned −0.07 rad. It lands
when the hand's `Stamp` shot starts, holds `STAMP_HOLD` (1.10 s), and fades out when the shot
ends. A snapped shot shows no stamp.

At 1280x800 its bottom edge sits a little under the trick area's middle (about 0.48 h).

**Ask.** Place `hand-result-{hand}` in the lower half of the trick area, with its top at
`y + 0.5 * h + 8` or lower, while a stamp can show. Both cards say "Euchre!" today, so the
other option is to drop that word from the card while the stamp is up.

## 2. The hand area says who won the trick before the ring

In `table-trick-win`, the hand area's line under the heading reads "You win the trick." at about
0.17 s, while the last card is still in the air. The felt's result line and the tally now wait
for the ring (`present.winner_shown(view)`; it is false until the trick's `Win` shot starts,
about 0.6 s in). Gating the hand area's line on the same call keeps one story on screen.

## 3. FYI: the rack's dealt cards wait full-size on the deck

In `table-upcard-flip`, frames 1 to about 24 show all five rack cards (`hand-AH` and the rest)
at full rack size (170 x 185) on the up card's spot before they fly. That is
`hand.rs::deal_source`. 09 §4.1 asks for cards that leave the deck, so a card waiting for its
`enter_delay` could hide (opacity 0) or start at deck size.

## 4. FYI: the deck tile is in (your item 2)

`stage::bid_area` draws the deck tile keyed `deck_key(hand)`, and calls
`AppState::deck_laid_out` from its `on_layout`. It shows while no up card shows. The up card
now mounts on the deck's exact spot (the bid copy has a fixed width, so the card no longer
shifts as the copy changes), and neither one fades, so the swap shows nothing. The fans do not
deal in yet (they show their backs from the first frame); `copy::deal_index` and
`motion::deal_delay` are the plan for that.

## From integration (wave close)

- DONE 1: the second option, done so both stay readable. The card is as tall as the trick box at
  1280x800, so it cannot sit in the lower half. Instead the stamp sits on the card's title:
  `fx::stamp_centre` reads `layout("hand-result-{h}-line")` and puts the plate's lower edge 5 pt
  above the line (the trick-area centre stays the fallback when no card shows). The title
  hides under the stamp (`fx::stamp_live`, opacity 0, no spring) and fades back on (0.25, 0) as
  the stamp leaves. "You score 2." stays readable under the stamp. Test:
  `tests/felt.rs::b_stamp_clears_the_hand_result_line`.
- DONE 2: `hand.rs` builds the instruction from the view with no winner while
  `stage::ring_waits(cx)` holds, so the hand area names the winner on the ring's beat.
- CLOSED 3: checked in the `table-deal-anim` and `table-upcard-flip` strips. Each rack card
  stays hidden until its `enter_delay`, then flies from the deck tile at rack size. The scale
  clamp of Transition ([0.9, 1.1]) rules out a deck-sized start, so the flight stays as it is.
- 4: noted.
