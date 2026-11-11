# Requests: Lane E (coach) to Lane C (hand.rs)

## 1. The coach marks the suggested bid button (06 §13)

**Problem.** The rack already wears the coach's "Try" badge on the suggested card
(`hint::badge`, hand.rs line ~309). When the suggestion is a bid (round 1 or 2, or a cut),
there is no card to mark, so only the settings rail's suggestion box shows it. With the rail
closed, the player sees nothing.

**Ask.** In `bid_panel`, compute the hint once and give each bid button its mark:

```rust
let coach = hint::for_view(view, &cx.state.prefs);
// ... inside the `for m in moves...` loop, after `b` is built:
if let Some(mark) = coach.as_ref().and_then(|h| hint::move_badge(&m.id, h)) {
    b = b.child(mark);
}
```

`move_badge` compares base ids (it strips `+alone`), so the button is marked whether or not
Go alone is ticked. It returns `None` for a card suggestion, so cards and buttons never both
wear it. The mark is pinned across the top edge out of flow (`pin`, `a11y_hidden`), so the
button's size and the row's layout do not change, and it fades in with a small scale on the
`CHIP_IN`/`CHIP_OUT` springs. Its tooltip is the coach line (`Try: Order it up — ...`).

**Test.** Lane E's `tests/coach.rs` checks `move_badge` itself. Once this lands, a check that
`rt.find("hint-orderUp")` (or whichever id the AI picks) is on screen in a bid scene with hints
on would cover the wiring.

DONE (Lane C): `bid_panel` computes `hint::for_view` once and adds `hint::move_badge(&m.id, h)`
to each bid button (after the enabled or disabled styling), so the suggested call wears the
mark. The rack's card badge is unchanged.

CLOSED (integration, wave close): DONE.
