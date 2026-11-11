# Send each seat's messages in order (SERVER-AI-SEAT → SERVER-TABLE)

## Problem

TS `euchreTable` awaited every send to a seat in its run loop: `handEnd(N)` (index.ts:986), then `resetHand(N+1)` (:997), then `decide(N+1)` (:847). So a seat always saw them in that order.

03 §16.7 makes every Rust send detached (`register_task`, one task per call). Two tasks for the same seat can then reach it in either order. `crates/euchre-table/src/seams.rs` describes the same detached shape for the client-backed `AiDispatch`.

The seat handles messages in arrival order (one worker, FIFO). When `decide(N+1)` overtakes `handEnd(N)`:

- `absorb_public_state` moves the memory to hand N+1 and overwrites `memory.snapshot`.
- `handEnd(N)` would then note and buffer hand N+1's partial view as hand N's episode.
- A `resetHand(N+1)` that lands after `decide(N+1)` would wipe what that decide absorbed.

## What SERVER-AI-SEAT did locally

Before the pure handlers run, the seat drops two kinds of message and logs `aiSeat dropped an out-of-order lifecycle message` with `{name, handNo, memoryHandNo}`:

- a `handEnd` whose `handNo` is below `memory.handNo`;
- a `resetHand` whose `handNo` is below `memory.handNo`, or equal to it once a decide has absorbed that hand.

This stops wrong notes and wiped memory. But the overtaken hand's episode is still lost, because its view is gone by the time `handEnd(N)` arrives. Only ordered delivery keeps it.

## Ask

**SERVER-TABLE:** give each seat one ordered outbox. That is a single detached task per seat (an `mpsc` channel drained by one `register_task` holding a keep-awake region), which sends `resetHand` / `decide` / `handEnd` / `gameEnd` one after another, each bounded as today (`send_bounded`).

The table lane still never awaits a seat. A stuck seat delays only its own later messages, never the table, and the watchdog covers `decide` as it does now.

Test: an e2e where a slow `handEnd` to seat N does not let that seat's next `decide` arrive first.

## Resolution (orchestrator, 2026-11-11)

Done by SERVER-TABLE (TABLE-AI-PROFILE stage): each AI seat and the profile now get their own ordered outbox. The seat keeps its out-of-order guard as a backstop. Moved to `done/`.
