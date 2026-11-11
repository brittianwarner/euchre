# Requests: Lane A (choreography) to Lane B (stage.rs, fx.rs)

The Director is live (choreo.rs, present.rs). Lane A's tests in `tests/motion.rs` read the keys
below, so these are what A-layout-rects, A-play-arcs, A-ring-waits, A-sweep-inside-hold,
A-skim-last-only, A-fast-forward and A-reduced-none need from your files. Most important
first. Append `DONE: <what you did>` here when each lands, or a note if you want it different.

## 1. The play flyer in `fx::layer` (P0, 09 §4.3)

```rust
use super::choreo::{self, play_pose};
use crate::theme::motion;

for (key, spec, t) in ctx.present().director.flyers(ctx.state.clock) {
    let Some(f) = choreo::resolve_flight(spec, ctx.view, cx, ctx.height) else { continue };
    let p = play_pose(&f, t);
    let [w, h] = f.size;                       // the trick card's size
    layer = layer.child(
        motion::card_shadows(stage::face(ctx.state, spec.card, w), p.lift, 0.0)
            .key(format!("fly-{key}"))          // key text: "fly-play-{h}-{i}-{seat}"
            .absolute(p.x - w / 2.0, p.y - h / 2.0)
            .rotate(p.rot).scale(p.scale).origin(0.5, 0.5)
            .instant_changes()
            .transition(rune::Transition::none()),
    );
}
```

- `flyers` yields only Full-tier plays, `t` already includes fast-forward (seconds into the
  flight). The flyer lives `motion::PLAY_END` (0.95 s), then the slot takes over.
- `resolve_flight` returns `None` only before the `trick` box is laid out.
- `play_pose` is no longer the 09 sketch's eased Bezier (that put the peak at t = 0.1 and failed
  A-play-arcs). It is a toss: drag on the ground track, a time parabola for height (peak
  `arc/2` at mid-travel), a skid, and the same squash. Same inputs, same `Pose` output.
- Opponents' source is the centre of `seat-{i}-fan`; `s0` = (fan height x card aspect) / slot
  width, so 30x42 backs give about 0.4 automatically. Compact mode (no fan) falls back to
  `seat-{i}`.

## 2. The trick slot follows `director.play_slot` (P0, 09 §4.3)

In `stage::trick`, per play:

```rust
use super::choreo::PlaySlot;
match cx.present().director.play_slot(view.hand_no, view.trick.index, play.seat) {
    PlaySlot::Hidden => continue,                                   // a flyer owns it
    PlaySlot::Matched => slot = slot.matched(play_id(..)).motion(motion::PLAY_BRISK),
    PlaySlot::Still => {}                                           // no .matched
}
// and on every slot:
slot = slot.transition(rune::Transition::none());
```

`Transition::none()` on every slot matters: at the 0.95 s hand-off the static card must not
fade in under the vanishing flyer. Today's `.motion(PLAY)` (0.7) becomes `PLAY_BRISK`.

## 3. The fan keeps its unlaunched backs (P0, 09 §4.3)

In `stage::fan`:

- `n = hand_counts[s] + director.unlaunched_from(s)`: a back leaves only when its play starts.
- The top back's matched id: `director.unlaunched_play(s)` when it is `Some(p)`, as
  `format!("play-{}-{}-{}", p.hand_no, p.index, s.index())`, else today's `next` rule. A Brisk
  or Skim play then flies from it.

## 4. Ring and pop gated on the Win shot (P0, 09 §4.4)

- Show the winner outline / `win-ring` / seat plate only when `present.winner_shown(view)`.
  It is false while the trick's `Win` shot waits for the last card's touchdown (+0.05 s).
- The pop's start: `director.started(&ShotKey::win(h, i))` (Some while live, 0.40 s).
- Key the ring `win-ring` (A-ring-waits looks for it; absent at 0.3 s, present at 0.7 s in
  `table-trick-win`). A-reduced-none checks that no key containing `win-ring-pop`, `sparks`
  or `confetti` exists under reduce motion, so do not build the pop element there.

## 5. Gather and sweep (P0, 09 §4.4)

- Gather: `present.gathering(view)` is true from `GATHER_LEAD` before the sweep, Full win
  only (false under reduce motion, Brisk, Skim). Move the remaining slots with
  `.motion(motion::GATHER)` while it holds.
- Stack minis: `.motion(motion::SWEEP)` instead of `COLLECT`. `app::poll` already calls
  `tx.unclipped_flights()` on each commit that sends a card, and `tx.instant()` for a Skim win
  (all four in one commit).
- `Collect` gained `gather` and `took` (the last card landed; `Cue::TrickTake` fires then).

## 6. Bursts (P2)

- Confetti: `present.confetti_t(state.clock) -> Option<f32>` is the burst time while it runs
  (3.7 s from the banner's first poll, `motion::CONFETTI_SECS`; `None` under reduce motion).
  The presentation now asks for a frame every tick while it runs, so a canvas confetti can be
  posed from `state.clock`. `Banner.since` is the banner's first poll.
- Stamp sparks: `director.started(&ShotKey::stamp(h))`; the Director asks for frames for the
  first `motion::SPARKS` seconds of a stamp.
- `table/mod.rs` still mounts `stage::confetti` (the old one); tell me when the fx layer draws
  its own and I will drop that mount.

## Other reads you may want

- `director.progress(&ShotKey::flip(h), now)`: 0 while queued, 0..=1 live, 1 landed. The Flip
  runs `FLIP + 0.40` s at Full (the last 0.40 is the glint) and starts at deal end − 0.10.
- `director.feel_of(key)`: the tier a shot ran at (`Snap` = it landed in place).
- `director.elapsed(key, now)`: seconds into a live shot, fast-forward included.

## 7. From Lane A (01:11)

- Items 1 to 5 are in and Lane A's tests read them (A-layout-rects, A-play-arcs, A-ring-waits,
  A-sweep-inside-hold, A-skim-last-only, A-fast-forward, A-reduced-none all pass on your code).
- Confetti: `table/mod.rs` no longer mounts `stage::confetti`. That fn is now dead code
  (`dead_code` warning, so `clippy -D warnings` fails): please delete it.
- `tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` fails at 720x820:
  `table-bid: bid-copy [254.5, 215.0, 267.6, 83.5] inside [181.9, 224.0, 356.2, 65.6]`. The trick
  box is 65 pt tall there, so the bid copy (83 pt) overflows it. This follows the felt/seat
  changes in stage.rs/felt.rs; tell me if you want the test's expectation changed instead.

## 8. From Lane A (01:17): the result line and the tally wait for the ring (09 §4.4 t = 0.60)

Seen on the `table-trick-win` strip: at 0.2 s, with Ned's card still in the air, the result line
already reads "You win the trick. You lead next." and "Tricks this hand" already counts the trick.
09 §4.4 puts "The result line changes (§4.8)" on the 0.60 beat with the ring. Both are yours
(`stage.rs::result_row`, the `felt-tricks` tally in the status row). Suggested shape: while
`view.trick.winner_seat.is_some() && winner_shown(cx).is_none()`, build the result line from a
view with `winner_seat = None` (it then reads "Trick N of 5 · The winner leads the next trick."),
and show the winner team's count one lower. The confetti and header items in 7 are resolved
(seen 01:16: `stage::confetti` and `header::header` are gone).

Still failing at 01:21 on `.cargo-targets/motion-a` (the only failing test in the crate):
`table-bid at (720.0, 820.0): bid-copy [254.51563, 215.04688, 267.5625, 83.5] inside [181.92188,
224.0, 356.15625, 65.59375]` (tests/table.rs:673). The copy overflows the trick box by about 9 pt
above and below.

## From Lane B (01:45): DONE notes

- DONE 1: `fx::flyers` draws each Director flyer as you sketched, keyed `fly-{key.as_str()}`,
  with `card_shadows(face, p.lift, 0)`. B-shadow-count checks the three shadows on every `fly-*`.
- DONE 2: `stage::trick` follows `play_slot` (Hidden skips, Matched gets `PLAY_BRISK`, or
  `GATHER` while gathering), and every slot has `Transition::none()`.
- DONE 3: `stage::fan` keeps `hand_counts + unlaunched_from` backs (30x42, step 9), and the top
  back carries `unlaunched_play`'s id, else the `next` rule, else `pickup-{h}` for the dealer.
- DONE 4: the ring, pop, winner outline and the seat plate's lit state all wait for
  `winner_shown`. The ring is keyed `win-ring-{h}-{i}` (A-ring-waits passes); nothing is built
  for the pop under reduce motion.
- DONE 5: while `gathering`, the slot holders move 18 pt toward the winner's slot on `GATHER`
  (offset and rotation), with the winner's card on top. Stack minis use `SWEEP`, and the stack
  glyph knocks once (`THUMP`) as the last sweep card lands.
- DONE 6: the confetti is a canvas in `fx::confetti`, posed from `confetti_t`. The stamp
  sparks are a canvas in `fx::stamp` for the first `SPARKS` seconds.
- DONE 7: `stage::confetti` is gone. The 720x820 bid copy now fits: the plate's vertical pad
  is 4 pt in a narrow felt, and your bidding block keeps no stack (it never shows one there).
  The trick box is about 100 pt tall there. `the_bid_copy_stays_in_its_box_beside_your_seat`
  passes on `.cargo-targets/motion-b`.
- DONE 8: `stage::ring_waits` holds the result line (built from the view with no winner) and
  the tally (the winner team's count one lower) until `winner_shown`. The line is keyed
  `result-{h}-{i}-{phase}`, plus `-won` once the winner shows, so the ring's beat gets the 09
  §4.8 enter.

## From integration (wave close)

All eight items are DONE (see Lane B's notes above). Integration checked them in the renders and
the frame-time runs (docs/port/reports/desktop-motion.md).
