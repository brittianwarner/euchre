# Requests: Lane C (rack, header, panels) to Lane B (stage.rs)

Most important first. Nothing in Lane C is blocked. Append `DONE: <what you did>` under an item
when it lands.

## 1. The result line enters and leaves (P1, 09 §4.8)

**Problem.** 09 §6.4 puts the result line in Lane C's order of work, but the line is drawn in
`stage.rs::result_row` (keys `trick-result`, `trick-result-line`, `trick-result-text`), so Lane C
cannot change it. Today it swaps in place when the text changes (r6).

**Ask.** Key the line by its moment and give it the 09 §4.8 transition:

```rust
let id = format!(
    "result-{}-{}-{}",
    view.hand_no,
    view.trick.index,
    format!("{:?}", view.phase).to_lowercase()
);
line.key(id).transition(if cx.reduced {
    Transition::opacity()
} else {
    Transition::asymmetric(
        Transition::offset(0.0, 8.0).combined(Transition::opacity()),
        Transition::opacity(),
    )
}
.springs(SpringPair::new(Spring::new(0.3, 0.0), Spring::new(0.2, 0.0))))
```

The old and new lines overlap while one leaves. Put the line in a fixed-height holder (the
`trick-result-line` row already has one), with the leaving line pinned
(`child_exit` or `pin`), so the row does not grow for a frame.

## 2. A deck to deal from (09 §4.1, C-deal-from-deck)

**Problem.** The rack deals each card from the deck (`hand.rs::deal_source`). It reads
`layout(deck_key(hand))`, then `layout("up-{hand}")`, then `present.deck_at`. In wave 0 no
`deck-{hand}` tile is drawn while the up card shows, so the rack deals from the up card. That
reads well, and C-deal-from-deck accepts it.

**Ask (optional).** If the felt draws a deck tile under the up card, key it `deck_key(hand)` and
call `AppState::deck_laid_out` from its `on_layout`. The rack then deals from it with no change.
`copy::deal_index(dealer, seat, k)` gives every seat's k-th card its place in the 20-card deal
(packets 3-2-3-2 / 2-3-2-3 from the dealer's left), and `motion::deal_delay(j)` gives its time.
Use them if the fans deal in too, so the seats and the rack deal in one order.

## 3. FYI: the hand-result card sits over the trick area

`hand-result-{hand}` mounts from hand.rs in a raised layer `hand-result-layer`. The layer has the
trick area's rect, from `layout("trick")`, while the phase is `hand_score`. It has no handlers, so
presses reach the trick below. If a stamp (`stamp-{hand}`) lands at the same time, the card and
the stamp overlap at the trick's centre. Tell Lane C where the stamp sits, and the card will move
to make room. One option: the card sits in the lower half of the trick area.

## 4. FYI: the bid copy outgrows the trick box at 720x820

`tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` fails in Lane C's target (01:35):

```
table-bid at (720.0, 820.0): bid-copy [254.51563, 215.04688, 267.5625, 83.5] inside [181.92188, 224.0, 356.15625, 65.59375]
```

The felt is 359.6 points tall there, and the trick box is 65.6. The hand area (344.4, from
`t::hand_h`) and the header (64) are unchanged.

## 5. FYI: the felt at 360 points wide

The header now wraps at 700 points wide or narrower: the score across the top, and the two buttons
under it. At 360x640 (`table-play`), the felt's own text still overlaps. The title runs under
`Tricks this hand`, `Your team 1 · Other team 2` sits over the partner seat, and the trick line
crosses the rim. This is not urgent. The 09 floor is 720 wide, and only C-settings-720 opens the
rail at 360.

## 6. FYI: the hand-result card now waits for the score

`hand-result-{hand}` now mounts when the hand's Score shot starts (about 2.95 s into
`table-hand-result`). Before that, it waited for nothing. So it no longer sits over the last trick
while that trick is gathered. A stamp that lands with the Score shot would still meet it at the
trick centre (item 3).

## From Lane B (01:45): DONE notes

- DONE 1: the line sits in a keyed child `result-{h}-{i}-{phase}` (plus `-won` once the trick's
  winner shows), with the 09 §4.8 transition: offset 8 plus opacity in, opacity out, springs
  (0.3, 0) / (0.2, 0), opacity only under reduce motion. Leaving lines drop out of flow (no
  `hold_layout`), so the `trick-result-line` row keeps its height. `trick-result-text` is still
  the text's key.
- DONE 2: the deck tile is keyed `deck_key(hand)` and calls `deck_laid_out` from `on_layout`
  while no up card shows. The up card now mounts on the deck's exact spot. The fans do not deal
  in yet. See motion-B-to-C.md item 4.
- 3: the stamp's place is in motion-B-to-C.md item 1.
- DONE 4: fixed in stage.rs and felt.rs (see motion-A-to-B.md, DONE 7).
- 5: not done. The 09 floor is 720 wide, and the felt holds at 720.
- 6: noted. The stamp and the card still meet; motion-B-to-C.md item 1 gives the stamp's box.

## From integration (wave close)

- DONE 3 and 6: the stamp now sits on the card's title, and the title hides under it. See
  motion-B-to-C.md, item 1.
- CLOSED 5: the 09 floor is 720 wide. The felt holds at 720.
