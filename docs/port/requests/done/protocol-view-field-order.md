# Request from PROTOCOL to ENGINE: `PublicGameView` field order

**Crate:** `crates/euchre-engine` (`src/types.rs`, `PublicGameView`)
**Severity:** low. Protocol works around it today.

## What

The engine struct declares the five actor metadata fields first:
`players`, `observations`, `reviewTricks`, `rules`, `awaitingTrickReview`.
The TS table builds its view as
`{ ...project(game, seat), players, observations, reviewTricks, rules, awaitingTrickReview }`
(`actors/euchre-table/index.ts`, `tableView`), so on the wire those keys come
**after** `winnerTeam`.

`serde_json` uses `preserve_order` across the workspace, so a plain
`serde_json::to_string(&view)` with metadata present does not byte-match the TS
JSON.

## Ask

Move the five `Option` fields to the end of `PublicGameView`, after
`winner_team`. Keep their `skip_serializing_if = "Option::is_none"`. A pure
`project()` output, with no metadata, is unaffected.

## Workaround in place

`euchre_protocol::wire` (`WireView`, `serialize_view`, `view_to_value`) moves
the five keys to the end. `SyncEvent.view` and `AIDecideRequest.view` use it,
and the table's `snapshot()` should return `WireView(&view)`. Once the engine
order changes, the helper does nothing and can stay or go.

Goldens covering this: `crates/euchre-protocol/tests/golden.rs`
`table_view_round_trips_with_metadata_last`, `sync_events_round_trip` and
`ai_decide_requests_round_trip_with_view_in_ts_order`.
