# Requests: Lane B (stage.rs, fx.rs, felt.rs) to Lane A (choreo.rs, present.rs)

Nothing in Lane B is blocked. Each item has a local workaround in Lane B's files. Most important
first. Append `DONE: <what you did>` under an item when it lands.

## 1. Sparks for the trick that seals a march (P2, 09 §4.9)

**Problem.** 09 §4.9 throws sparks for a stamp and for the trick win that seals a march. The
`Win` shot is not poll-driven (`Shot::poll_driven` covers Play/Flip at Full, Stamp under
`SPARKS`, MatchEnd won), so a canvas posed from the clock gets no frames during the ring.

**Ask.** Make the `Win` shot poll-driven for its first `motion::SPARKS` seconds at Full when it
is the fifth trick of a hand your team marched (all five to the makers). A
`Director::win_seals_march(&ShotKey) -> bool`, or a flag on the shot, lets `fx::stamp` draw the
sparks keyed `sparks-win-{h}`.

**Workaround.** None. The stamp's own sparks still play, about 0.6 s after the ring.

## 2. Canvas confetti on your team's March (S-4, P2)

**Problem.** S-4 asks for confetti on a March too. A March has no banner, so
`Presentation::confetti_t` is `None`, and the Stamp shot asks for frames only for `SPARKS`
(0.9 s). A canvas burst needs 3.7 s of frames.

**Ask.** Either give a March by your team a banner with `confetti: true` (06 §6.5 has no
March banner copy, so this needs a copy ruling), or keep the Stamp poll-driven for
`CONFETTI_SECS` when `result == March` and the makers are your team.

**Workaround (in).** `fx::confetti` mounts `stage::confetti_fall("confetti-march-{h}")` while the
March stamp is live: 24 renderer-owned pieces (a `LoopSpec` slide plus a one-shot fade), so it
needs no frames from the Director. It is quieter than the cannons.

## 3. A Director read for a live shot's data (P3)

`fx::stamp` reads `view.result` and `view.maker_seat` for the stamp word and the team, not the
shot's own `Shot::Stamp { .. }` data. That is right while the stamp's hand is the displayed
view (it is today, since the Score shot holds it). If a later change lets the view move on
while a stamp runs, please add `Director::stamp_of(&ShotKey) -> Option<HandResult>`.

## 4. FYI: the gather starts 0.033 s late in `table-trick-win`

The collect's `due` is set at its first poll (0.033 s in), so the gather starts at 1.933 s, not
1.90 s. B-gather measures from the fourth card's Play start (`director.started(play)`), which
is 09 §4.4's `t = 0`, and passes (about 29 pt from the centre at +2.05 s). Measured from scene
time 0 it would read 41.9 pt at 2.05 s. No change needed unless you want `due` anchored to the
touchdown rather than the poll.

## 5. FYI: the result line and tally now wait for the ring (your item 8 in motion-A-to-B.md)

`stage::ring_waits(cx)` is `view.trick.winner_seat.is_some() && winner_shown(cx).is_none()`.
While it holds, the result line reads as if no winner were set, and the winner team's count
in "Tricks this hand" is one lower. The line is keyed `result-{h}-{i}-{phase}` plus `-won` once
the winner shows, so the ring's beat gets the 09 §4.8 enter.

## From integration (wave close)

- CLOSED 1: not built (P2). The `Win` shot runs 0.40 s, so frames for 0.9 s of sparks need a
  longer Win or a new shot, and either changes what waits on the Win. The stamp's own sparks
  still fire on your team's March, and the ring and pop mark the sealing trick.
- CLOSED 2: the workaround stays. A canvas burst needs 3.7 s of driven frames; the Stamp shot
  lives 1.44 s, so it would need a new shot kind with its own wait and fast-forward rules. The
  renderer-owned fall (`confetti-march-{h}`) plays while the March stamp shows and costs no
  frames.
- CLOSED 3: not needed. The Score shot holds the hand's view while its Stamp runs.
- CLOSED 4: no change. B-gather measures from the fourth card's Play start and passes.
- 5: noted. The hand area's line now waits on the same `ring_waits` (motion-B-to-C.md, item 2).
