# Request: aiSeat bounds and retries its callbacks into the table (from lane SERVER-TABLE)

**Need.** TS aiSeat sends its answer with `euchreTable.getOrCreate(...).send('aiDecision', ...)`
(and `aiSay` the same way). That is a durable queue send: the message survives a table that is
going to sleep, and the table reads it when it wakes. In the Rust port the former queues are
actions (D6). Seen in the engine e2e (`table_ai_sleep_between_dispatch_and_decision_end_to_end`):

- An action that reaches a table during its sleep grace can fail at once with the rivetkit
  error `stopping` ("Actor is stopping.").
- An action can also hang while the table is in its sleep grace. The in-flight request seems
  to hold the grace open, up to rivetkit's 15 s default `sleep_grace_period`, and the table did
  not wake within 15 s. With a single try bounded at 15 s, this happened in about one full-suite
  run in three.

The table itself never stalls on this while a player is connected: the parked heuristic move
and the `onAiTimeout` watchdog play the turn. But the model's answer is lost.

**Ask.** In the aiSeat port, send `aiDecision` and `aiSay` with a short bound on each try (the
stand-in uses 2 s), on a fresh handle, and retry a few times on an error or timeout (the
stand-in uses 5 tries, 100 ms apart). A retry is safe: the table drops a duplicate or stale
decision (`onAiDecision` checks the turn and that the parked decision is not yet answered) and
keys chat by `msgId`.

**Local workaround.** The stand-in aiSeat in `crates/euchre-table/tests/table_ai_e2e.rs`
(`call_table`) does this. With it, the hang went away in every later run. The table needs no
change.

## Resolution (orchestrator, 2026-11-11)

Done by SERVER-SEAMS in `crates/euchre-ai-seat/src/calls.rs` (`send_to_table`): 2 s per try, up to 5 tries 100 ms apart, refusals final. See `reports/seams.md`. Moved to `done/`.
