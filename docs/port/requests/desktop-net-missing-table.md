# Desktop net: tell a missing table from a passing outage (SERVER-HTTP)

**From:** the desktop net wave, lane N1 (`apps/desktop/src/net/remote/**`). Contract:
`docs/port/reports/desktop-net-contract.md` §N1.4, §N1.6, §N1.7.
**Owner:** `crates/euchre-http/src/api/tables.rs` (W4). The desktop does not edit it.
**Blocking:** no. The desktop works around all three gaps (below). With these changes it can stop
retrying tables that will never come back, and stop guessing why an answer is empty.

## 1. `POST /api/table-session/{gameId}` answers 503 for a table that does not exist

**What happens now.**
- The handler does `client.get(EUCHRE_TABLE, ["table", gameId])` and then `snapshot`.
- Any error from that call except `forbidden` becomes `unavailable(e.code())`:
  `503 "Your table is temporarily unavailable. Please reconnect."`.
- So a game id that never existed, or a table that is gone, reads the same as a passing engine
  hiccup.

**Why it matters.**
- The desktop reconnect supervisor treats 503 as passing. It retries with backoff for 2 minutes
  before it gives up and shows "We couldn’t reach this table."
- A saved-table row for a gone table spins for the same 2 minutes.
- The browser has the same problem: its reconnect banner never settles.

**Proposed change** (`table_session`, the `match call(&handle, SNAPSHOT, ..)` arm):

```rust
Err(ActorCallError::Rivet(e)) if is_actor_not_found(&e) => {
    return Err(ApiError::not_found(TABLE_NOT_FOUND)); // 404 {"error":{"code":"not_found"}}
}
```

- `is_actor_not_found` matches the engine's structured error for a `get` on a key with no actor.
  It is expected to be group `actor`, code `not_found`; confirm the exact pair with a case in
  `crates/euchre-http/tests/http_e2e.rs` that mints a session for a random UUID.
- Suggested copy for `TABLE_NOT_FOUND`: "That table could not be found."
- Add one test: a valid UUID that was never created answers 404, not 503.

## 2. `GET /api/tables/recent` cannot tell "gone" from "asleep"

**What happens now.** Every failed lookup (missing, foreign, timeout, engine down) becomes the same
row, `{"gameId": id, "status": null, "score": null, "handNo": null}`, and sets
`unavailable: true`.

**Why it matters.** The desktop must never prune a saved table on a `null` row, because the table
may only be waking up. So gone tables stay in the band, as "Reconnect saved table · …", until the
12-entry cap pushes them out.

**Proposed change (additive, the web client keeps working).** Add an optional
`"reason": "missing" | "forbidden" | "unavailable"` to a `null` row. Set `unavailable: true` only
for `"unavailable"` rows. Clients that ignore `reason` see the same JSON as today.

## 3. `GET /api/tables/recent` answers an unverified bearer with an empty 200

**What happens now.** When `Caller::resolve` finds no guest (a missing, expired or foreign bearer),
the handler returns `200 {"tables": [], "unavailable": false}`. That is the same answer as a caller
with no saved tables.

**Why it matters.** An expired device session cannot be told apart from "nothing to show". The
desktop's 401 recovery (refresh, then enroll again, then retry once) never runs for this route.

**Proposed change.** When the request carries an `Authorization: Bearer` header that does not
verify, answer `401` with the standard error body, as every other bearer route does. Keep the empty
200 for a caller with no credentials at all (the browser's first visit), so the web stays as it is.

## Desktop workarounds (in place until this lands)

| Gap | Workaround (contract section) |
|---|---|
| 1 | Reconnect treats 503 as passing and gives up after 120 s with `Ended(Unavailable)` ("We couldn’t reach this table. Your game is saved."). A 404, when it comes, already ends at once with `Ended(NotFound)` (§N1.6). |
| 2 | `null` rows are never pruned. They render as "Reconnect saved table · {first 8} →", and `unavailable` shows the 06 §17.1 status line (§N1.7, §N2.5). |
| 3 | An empty `tables` list for a non-empty `ids` runs the session recovery once and asks again (§N1.4). |

When each change lands, the desktop needs only small changes: gap 1 needs none (404 already maps
to `NotFound`). Gap 2 can prune `"missing"` rows. Gap 3 can drop the empty-list check.

## Review update, 2026-11-11 (review fix PP-1)

Change 1 is now rated P2 by the desktop review: it is the one thing that lets a gone table leave
the player's list. The desktop side is done and waits only on the 404.

- A table-session mint answered 404 ends the stream with `Ended(NotFound)` at once, and the id
  is dropped from the recent tables file (`RemoteBackend`'s `TableDialer::mint` calls
  `RecentStore::forget`). The next Your games read no longer asks for it.
- A 403 never drops the id: the table may be another device's or this device's before a sign-out
  (the `SignedOut` copy, PP-7).
- Covered by `tests/remote_http.rs::a_gone_table_is_forgotten_and_a_signed_out_device_says_so`
  (wiremock answers 404 and the id leaves `recent-tables.json`) and the unit test
  `forget_drops_one_id_and_keeps_the_order`.
- `tests/remote_live.rs` already proves the 404 shape on a real server for `GET
  /api/matches/{id}` (a random id answers `NotFound`). Once `table_session` answers 404 too, add
  the same check there for a random game id.

