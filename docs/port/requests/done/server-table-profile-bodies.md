# Request: playerProfile write bodies for the table (from lane SERVER-TABLE)

**Need.** `euchreTable` sends two writes to `playerProfile` (TS `euchre-table/index.ts`
`recordHand` and `recordMatchIfNeeded`). `euchre-protocol` owns `MatchRecord` and
`HandJournalEntry`, but not the two message bodies that wrap them.
`euchre-profile-model` parses them (`parse_record_game`, `parse_record_hands`) from raw JSON.

**Ask.** Publish these in `euchre-protocol` (the profile module), with the same serde shape
(camelCase, field order as written):

- `RecordGameMessage { internalToken, record: MatchRecord }` (action `recordGame`)
- `RecordHandsMessage { internalToken, matchId, hands: Vec<HandJournalEntry> }` (action `recordHands`)

**Local workaround.** Lane SERVER-TABLE defines borrowing serializers `RecordGameBody<'a>` and
`RecordHandsBody<'a>` (crate-private) in `crates/euchre-table/src/remote.rs`. Unit tests in
`src/core/tests.rs` check the key order and that `euchre_profile_model::parse_record_game` /
`parse_record_hands` accept the bodies. Once the protocol owns the types, `remote.rs` serializes
them instead and drops its own. No public API changes.

## Resolution (orchestrator, 2026-11-11)

Not moved into `euchre-protocol`, which stays frozen. Recorded as D34: each writer keeps its body type, and parser cross-check tests plus the seams e2e are the contract. Moved to `done/`.
