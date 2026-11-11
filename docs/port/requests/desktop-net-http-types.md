# Desktop net: shared types for the untyped HTTP bodies (SERVER-HTTP, PROTOCOL)

**From:** the desktop net wave, lane N1 (`apps/desktop/src/net/remote/http.rs`). Contract:
`docs/port/reports/desktop-net-contract.md` §N1.3.
**Owner:** `crates/euchre-http/src/api/**` and `crates/euchre-protocol` (W4). The desktop does not
edit them.
**Blocking:** no. The desktop mirrors the bodies (below), and tests pin each mirror.

## What

The desktop is the first Rust client of the HTTP API. Most bodies are already shared types:
- `DeviceEnrollment` and `TableSessionResponse`.
- `MatchListPage`, `ClientMatchReplay`, `ProfileStats`, `ProfileSettings` and `SettingsPatch`.

Five bodies are built with `json!` in the handlers, so there is no type for a client to decode
into:

| Route | Body today (handler) | Proposed type (in `euchre-protocol`, camelCase serde) |
|---|---|---|
| `POST /api/tables` request | `CreateBody { stick: Option<bool>, natural: Option<bool> }`, private to `api/tables.rs` | `CreateTableRequest { stick: Option<bool>, natural: Option<bool> }` |
| `POST /api/tables` 201 | `json!({ "gameId": game_id })` | `CreateTableResponse { game_id: String }` |
| `GET /api/tables/recent` | `json!({ "tables": [{ "gameId", "status", "score", "handNo" }], "unavailable" })` | `RecentTablesResponse { tables: Vec<RecentTableRow>, unavailable: bool }`, `RecentTableRow { game_id, status: Option<GameStatus>, score: Option<[u8; 2]>, hand_no: Option<u32> }` |
| `GET /api/rivet/metadata` | `json!({ "clientEndpoint", "clientNamespace" })` | `RivetMetadata { client_endpoint: String, client_namespace: String }` |
| `GET /api/session` | `json!({ "guestUserId", "identity" })` | `SessionResponse { guest_user_id: Option<String>, identity: Option<..> }` |
| every JSON error | `json!({ "message", "error": { "code", "message" } })` in `error.rs` | `ApiErrorBody { message, error: ApiErrorDetail { code, message } }` |

## Why

- Today a change to a handler's JSON key compiles on both sides and breaks the desktop at run
  time.
- With shared types:
  - the handlers serialize the type, and the desktop decodes the same type;
  - a rename becomes a compile error;
  - the web's generated types can come from the same source.

## Proposed change

1. Add the types above to `euchre-protocol` (an `http` module is the natural home), with
   `#[serde(rename_all = "camelCase")]`. Keep `RecentTableRow`'s nullable fields as `Option`.
2. Make the five handlers return `Json(<type>)` in place of `json!`. The bytes on the wire do not
   change; the existing `crates/euchre-http/src/api/tests.rs` cases prove it.
3. Optional, with `desktop-net-missing-table.md` §2: `RecentTableRow.reason: Option<RecentReason>`.

## Desktop workaround (in place)

- `apps/desktop/src/net/remote/http.rs` `pub mod wire` mirrors each body:
  - `CreateTableBody` and `CreateTableResponse`;
  - `RecentTableRow` and `RecentTablesResponse`;
  - `RivetMetadata` and `SessionResponse`;
  - `ApiErrorBody` and `ApiErrorDetail`.
- N1's `wire_bodies_match_the_handlers` test decodes the exact JSON each handler writes today.
- When the shared types land, the desktop drops `wire` and imports them. This is a one-file change
  in `http.rs`.
