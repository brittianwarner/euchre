# Desktop net: what an engine crash and Restart leave behind (SERVER)

**From:** the desktop net integrator (`apps/desktop/src/net/embedded.rs`,
`apps/desktop/tests/live.rs`). Report: `docs/port/reports/desktop-net.md`.
**Owner:**
- `apps/server/src/server.rs` (stop, registry abort);
- rivetkit / `rivet-envoy-client` 3.0.7 (the envoy's connection loop);
- `crates/euchre-table` (`recordHand`, the aiSeat dispatch).

These belong to W4. The desktop does not edit them.

**Blocking:** no. Everything here happens only after the engine dies mid-match and the player
presses **Restart** in the same process. That is the one recovery the desktop offers. The match
then plays on to the right result, and it is recorded.

## The setup

The live test (`tests/live.rs`) sends SIGTERM to the engine at the start of hand 2 (0-based)
and presses Restart. The same process then starts a new server on the same data dir and engine
port, and the match plays on to game over.

## 1. The old engine client rejoins the new engine (fixed in the desktop)

- `RunningServer::shutdown` aborts the registry task after `REGISTRY_GRACE_ENGINE_GONE` (5 s).
- rivet-envoy-client runs its connection loop as a detached `tokio::spawn`
  (`connection/native.rs`), and the abort does not reach it.
- On a shared runtime the loop keeps dialling the engine port (`connection failed … Connection
  refused`, about one line a second).
- Once Restart brings an engine up on that port, it registers again next to the new server's
  runner. Run final3 logged:
  `reconnected after the envoy lost threshold without a ping; stopping all actors silence_ms=12396 lost_threshold_ms=7000`.

**Desktop workaround (in place).** Each embedded server now runs on a tokio runtime of its own
(`ServerRuntime` in `embedded.rs`). Once the stop returns, `shutdown_background` drops that
runtime, which ends every task the server spawned. Since then, no run has logged a connection
attempt after the old server stopped.

Any other host that starts a second server in one process will hit the same thing. That
includes `apps/server`'s own restart test, if it ever restarts after an engine death.

**Correction.** An earlier draft of this file blamed this loop for a "Resume game" row on a
finished match. That row was the desktop's own cached answer from the Restart moment. It is
fixed in the desktop: `AppState::note_open_table` keeps the open table's row in step with the
live view. A fresh read from the server was right every time (run F: right on the first re-read;
run E's data dir reopened in a new process: `complete`, 10–3).

## 2. Faults after a Restart (9 live runs since the desktop fix)

- **A hand is dropped from the history (6 of 9 runs).** The log shows
  `recordHand failed; hand journal entry dropped hand_no=2 err=Actor queue delivery interrupted`.
  The match detail then lists hands 1, 2, 4… and skips the hand that was in play.
- **The AI seats fall back (9 of 9 runs, 2–3 seats).** The log shows
  `dispatch to aiSeat failed; heuristic stands … Actor queue delivery interrupted`, then
  `AI deadline reached; releasing parked safety move`. The computer seats play the heuristic for
  the rest of that hand.
- **Restart is slow.** Restart to ready takes 13.6–14.3 s in most runs, and 10.4–10.6 s in
  two. That breaks down as:
  - the 5 s registry grace (`desktop-net-dead-engine-grace.md`);
  - a boot of about 8.5 s, against about 5 s for a cold boot. It seems to wait for the engine
    to give up on the old runner (`lost_threshold_ms=7000`).

  Before the desktop fix, Restart took about 9.6 s in all. The old runner rejoining probably
  spared this wait, at the cost of problem 1.
- **Engine warnings.** The restarted engine logs:
  - `depot filesystem fenced; its owner must reopen the volume` (5 times);
  - `durable alarm delivery is for a revision that is no longer armed` (18–33 times);
  - `failed to send stop command; the stop timer resends it` (1–2 times).

  Every Restart run shows them, green ones too.

## Proposed changes

1. **Stop the engine client with the registry** (rivetkit / `rivet-envoy-client`).
   - Tie the connection loop to the registry's cancellation token, so an abort ends it.
   - Until then, `apps/server` could run each `start` on a runtime it owns and drops at stop,
     as the desktop now does around it.
2. **Retry `recordHand` on `Actor queue delivery interrupted`** (`euchre-table`). It already
   retries `recordGame` ("recordGame failed; will retry"). Today a lost hand journal entry is
   gone for good.
3. **Retry the aiSeat dispatch once** after `Actor queue delivery interrupted`, before falling
   back to the heuristic.
4. **Shorten the Restart.**
   - Let the new runner take over the old runner's actors without waiting out the lost
     threshold: reuse the runner identity across a same-process restart, or use a short lost
     threshold for the local engine.
   - Land the 500 ms grace in `desktop-net-dead-engine-grace.md`.
5. **Cover the crash case in `apps/server/tests/e2e.rs`.** Its drain-and-restart case is
   graceful. The new case should:
   1. send SIGTERM to the engine mid-match;
   2. restart in the same process;
   3. finish the match;
   4. check that the match's replay has every hand.

## How to check

```bash
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6870 EUCHRE_TEST_TMP=/private/tmp/euchre-e2e \
NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1 \
  cargo test -p euchre-desktop --test live -- --nocapture
```

Expected after the fixes:

- `timings.json` has `"missing_hands": []` in every run;
- `restart_ready_ms` is about 5 s;
- the log has no `dispatch to aiSeat failed` after `the game server is back`.

## Review update, 2026-11-11 (review fixes PX-2 and PX-5)

**PX-2 (P1): a Restart can drop the hand in play from the history.** This is proposed change 2
above, now rated P1 by the desktop review: a player who restarts loses that hand from their
match detail for good. The fix belongs in `crates/euchre-table` (W4): retry `recordHand` on
`Actor queue delivery interrupted`, as `recordGame` already does.

- The desktop cannot fix it. `tests/live.rs` now pins how far it may go: the only hand allowed
  to be missing is the one the engine stopped in (`stopped_hand`, from the last sync before the
  stop). Any other gap fails the run.
- In the review-fix runs, 2 of 4 live runs lost hand 2 (the hand in play at the stop:
  `missing hands [2] of 12` and `[2] of 13`); the other 2 lost none.
- When the fix lands, tighten the assert in `tests/live.rs` to `missing_hands.is_empty()`.

**PX-5: Restart is shorter; the rest is the engine's boot after a crash.**

- The desktop no longer waits out the 5 s registry grace after an engine death: a server whose
  engine died is dropped on its own runtime instead of shut down (`clear_dead` in
  `apps/desktop/src/net/embedded.rs`). The stop now takes 0–2 ms in every run, down from 5001 ms.
- Restart to ready in `tests/live.rs` is now 9.3–10.3 s (4 runs), down from 13.6–14.3 s.
- What is left is the boot itself: 9.3–10.3 s after a crash, against 5.2–6.1 s cold and
  4.5–4.6 s after a clean stop on the same data dir (`tests/embedded_live.rs`, 4 runs each).
- The restarted engine's own log (`<storage>/.rivetkit/var/logs/rivet-engine/*-stdout.log`)
  shows, in that window:
  - `depot filesystem fenced; its owner must reopen the volume`;
  - `failed to send stop command; the stop timer resends it … no_responders` twice, about 4.9 s
    apart.

  So the new engine seems to wait one stop-timer period for the dead runner's actors before it
  serves. Proposed change 4 above (take over the old runner's actors, or a short lost threshold
  for the local engine) is the fix. It is server and rivetkit work; the desktop has nothing left
  to shorten.

