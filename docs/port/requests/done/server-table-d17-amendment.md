# Request: amend D17, D20 and D22 to match the table after review (from lane SERVER-TABLE)

**Need.** The table review (`reports/table.md`, "Review fixes") changed three behaviours that
ARCHITECTURE.md describes. The table crate now does the following. ARCHITECTURE.md is outside
this lane, so the owner of that file needs to record it.

1. **D17: a live WebSocket keeps the identity it proved at connect.** The table checks the
   player token (signature, expiry, protocol version, match token) once, in
   `on_before_connect` / `on_connect`, and stores the result per connection. Later actions on
   that socket check only the role against the owner. A stateless HTTP call has no connection
   to keep, so it is checked in full on every call. TS did the same: `createConnState` ran once
   per connection. Without this, a game that outlives the 2 h guest token on one socket
   (`exp` has zero leeway) failed every move with `invalid_token`.
2. **D20: after a partial commit, the table runs a recovery reconcile instead of retrying the
   same fire.** A session that fails after its first save rolls back the working copy. If any
   save landed, the table spawns one `reconcile` task (keep-awake, up to `FIRE_RETRY_LIMIT`
   tries) that re-derives the open turn's timers from saved state. It does not retry with the
   same arguments, because the saved `turnId` is already new. A session that failed before
   any save keeps the old same-arguments retry. A move whose save landed is acknowledged
   `{ ok: true, v }` even if a later step failed.
3. **D22: an internal-token reader gets a redacted view.** `snapshot` with the internal token
   returns the seat 0 table view with `hand` and `legal` cleared. The only holders of the
   internal token are the opposing aiSeats, so they must never see the human's cards.

**Ask.** Record these three points in ARCHITECTURE.md under D17, D20 and D22.

**Local workaround.** None is needed. The table already behaves this way and its tests cover
it (`auth.rs` `caller_is_rederived_and_roles_gate_actions`, `core/tests.rs`
`a_turn_that_fails_after_its_first_save_is_recovered_by_reconcile`,
`an_internal_reader_never_sees_a_private_card`, and the e2e `auth_checks` expiry case).

## Resolution (orchestrator, 2026-11-11)

Recorded as D33 in ARCHITECTURE.md. Moved to `done/`.
