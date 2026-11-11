# Cookbook additions: Motion (for 07, applied by the 07 owner; 07 is not edited here)

These facts were newly VERIFIED by the motion spike: `scratchpad/motion-spike`, built at Rune `85cd8c6b797f68d600f9f3c1cd3ce074e4415ddb`, window 1000x640, headless `--snap/--strip` at `--scale 1`. Evidence is in `research/motion/spike.md`, techniques S1-S11. Rune paths are `crates/…` at that rev.

## 5.6 Motion: verified at 85cd8c6

| # | Fact | Rule |
|---|---|---|
| M1 | **The poll-driven pose works and is deterministic headless.** `AppSpec::poll` (`rune/src/tooling.rs:35`) stores `now`. The view computes x/y/rotate/scale from it. The element gets `.instant_changes()` (`rune/src/el/motion.rs:181`) and `.transition(Transition::none())` (`rune-motion/src/transition.rs:194`). Wake with `tx.wake_after(Duration::ZERO)` (`el/tx.rs:226`) while moving, and `tx.wake_at(t)` (`:242`) for the next beat. Stop waking at the end and the app idles. | Use it for arcs, flips, shakes and bursts. `--at T` is exact. |
| M2 | **`enter_delay` does not stagger matched flights.** All of them fly at once. | Stagger by sending one element per commit, with `wake_at` 45-60 ms apart. |
| M3 | **A one-commit multi-flight ignores the source `paint_order`** (`motion.rs:142`). | The staggered commits of M2 fix the order. One commit: 0.33/0.75 ms. Staggered: 0.63/1.19 ms (p50/p95 CPU). |
| M4 | **A re-keyed element cross-fades and ghosts** under the default 0.2 s in / 0.15 s out. | Add `.transition(Transition::none())` to anything re-keyed to replay a `Sequence`, or swapped at a hand-off. |
| M5 | **A matched flight carries no rotation.** The rotation snaps at mount (seen in the Home→Table hero). | Give both ends equal rotation, or fly it with M1. |
| M6 | **`Tx::unclipped_flights()`** (`rune/src/flight.rs:36`) is needed for a matched flight that crosses a clip. Proven with trick → seat minis. | Call it in the commit that mounts the destinations. |
| M7 | **The flip works two ways.** (A) Poll: width = `w·|cos(πu)|` floored at 1 pt, the face swaps at u ≥ 0.5, inside a fixed outer element, art via `.pixels` (stretch, `el/visuals.rs:215`). (B) Two commits: `tx.motion_curve(Curve::tween(0.18, EASE_IN))` (`tx.rs:536`) to width 1, then the face + `tween(0.18, EASE_OUT)`. | `.pixels_fit` (`:223`) letterboxes instead of squeezing. Do not use it for flips. |
| M8 | **Shadow geometry springs on `.motion(Spring)`, not on `.offset_motion`,** and only when the shadow count is unchanged. | Always emit the same number of shadows (pad with alpha 0). Carry them on a plate child. |
| M9 | **Deal:** 20 cards mounted in one commit with `Transition::offset(deck − own centre)` + `SpringPair` + `enter_delay(Stagger::new(0.055).delay(i, 20))` + `paint_order(20 − i)` deals cleanly. Cost: 0.91 ms p50; the 6.1 ms p95 is the first image upload. | Re-deals need new keys. Pre-touch the images. |
| M10 | **`Cycle` wakes at its own `fps`** (`rune-motion/src/cycle.rs:8`, `El::cycle` `el/visuals.rs:556`). It is the only piece that breaks the idle floor (30 fps in the spike). | One `Cycle` per screen, and only when it means something. |
| M11 | **`backdrop_blur(σ)`** (`el/backdrop.rs:34`): the element's opacity also fades the blur. That gives a clean scrim in and out with `Transition::opacity()`. | A maximum of 4 per frame. |
| M12 | **Canvas paint order is tree order.** A `canvas(Shape)` (`rune/src/el.rs:359`) mounted before a card draws under it. Sparks: 28 strokes. Confetti: 120 filled quads at about 60 fps on the CPU budget. | Mount bursts before the subject and confetti last. Seed with an integer hash, never a random number generator. |
| M13 | **Kit clamps on `Transition`** (`kit_rules`, `rune-motion/src/transition.rs:148`): scale is clamped to [0.9, 1.1] (or [0.5, 1.1] with `allow_small_scale`), removal opacity is forced to 0, and blur is dropped when the transition also travels. `Sequence` beats are not clamped. | For a > 1.1 pop, use a `Sequence`. |
| M14 | **`RollingText`** (`rune-kit/src/rolling_text.rs:93-159`): digits spill below the line mid-roll. | Clip the cell and use `TextStyle::features("'tnum' 1")`. |
| M15 | **Spring feel:** `Spring::new(period, bounce)` (`rune-motion/src/spring.rs:30`) settles in about 1.2-1.5× the period. A bounce under 0.3 barely reads. `Animated::new` defaults to `PropClass::Other` and settles about 0.2 s late. | Use `.with_class(PropClass::Opacity)` for opacity. Use bounce ≥ 0.3 where overshoot should show. |
