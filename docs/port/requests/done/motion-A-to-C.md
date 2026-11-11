# Requests: Lane A (choreography) to Lane C (hand.rs, header.rs)

Append `DONE: <what you did>` here when each lands.

## 1. Card-sized holes (P0, 09 §6.4 first item)

Your play's flyer starts from `cx.layout("hole-{card}")` (`choreo::resolve_flight`). Today the
hole is 1 pt tall at the top of the row, so the flyer guesses the card's centre from the cell
width. Please make the hole the card's own rect: `el().key(hole_key(c)).w(cell_w).h(card_h)`
(the same `w / card::ASPECT` the art uses, top at the row's `pt(10.0)`), with nothing drawn in
it. `resolve_flight` takes a hole of 4 pt or taller as the card (centre = hole centre, width =
`min(hole.w, hole.h * ASPECT)`).

Your card leaves the rack in the click's own commit (the store's optimistic view), and its
flyer starts in that same commit, so there is no frame without it.

DONE (Lane C): `hole-{c}` is `el().key(hole_key(c)).size(cell_w, h)`, with `h = w / ASPECT` the
art's own height. Nothing is drawn in it, and it sits in the same row as the cards. The row's top
padding is now `RACK_TOP` = 6 (it was 10). That leaves room in the 1280x800 height budget for a
card at least as large as the trick cards. The hole is still the card's rect, because
`resolve_flight` reads the laid-out hole.

## 2. The score roll (P1, 09 §4.8, C-score-rolls)

`present.score` now steps: `shown` moves one point per `motion::SCORE_STEP` toward `target`
once the hand's `Score` shot starts (it waits for the trick sweep's last landing), and
`landed_at` is set when it reaches the target (fire `PULSE` and `NUM_POP` from it). The first
view, a snap (reconnect) and reduce motion show the target at once with `landed_at` untouched.
Read `present.score.shown` for the `RollingText` value.

DONE (Lane C): `header::header_with(.., Some((&present.score, clock)))` reads `shown` for the
`RollingText` value, and `landed_at` (once `shown == target`) for the `PULSE` ring and `NUM_POP`.
The number that pulses is the side whose `view.delta` is above 0. mod.rs still calls the plain
`header::header`, which rolls once from the old value to the new one. See motion-C-to-A.md item 1.

## 3. The rack card's matched spring

The rack art's `.matched(hcard-…)` is the source of a Brisk or Skim play (09 §4.11 sketch uses
`.motion(motion::PLAY_BRISK)`). Full plays do not use it: the slot is hidden while the flyer
lives, and mounts without a partner at the hand-off.

DONE (Lane C): the art keeps `.matched(hcard-{hand}-{c})`. Its spring is `PLAY_BRISK`, or
`PICKUP` for the sixth card, the dealer's pickup. The lift, the scale and the hover spring are on
the outer `plate-{c}`. The shadows are on the art, so a matched flight carries its own shadow.

CLOSED (integration, wave close): all three items are DONE.
