# Match detail and stats show "Tricks won 0–0" (W4 review → orchestrator, SERVER-PROFILE)

**From:** the W4 integration review (web-flows, P3), checked by the W4 fix agent.
**Owner:** `crates/euchre-profile-model/src/records.rs` (`parse_stats`), and the orchestrator for the parity decision.
**Blocking:** no. It is the TS behaviour, copied on purpose.

## What happens

- `euchre-table` adds up tricks per team and sends `stats.tricks` as an array `[a, b]` (`crates/euchre-table/src/state.rs`, `accumulate_stats`).
- `parse_stats` reads it with `pick(tricks, "0")` and `pick(tricks, "1")`. `pick` only sees a plain object, so an array reads as `[0, 0]`.
- So every stored match has `tricks: [0, 0]`. The match detail page and `GET /api/profile/stats` show "Tricks won 0–0", while the hand rows on the same page add up to the real totals.

The doc comment on `parse_stats` says this mirrors TS exactly: TS `guards.ts` `pick` uses `isRecord`, which rejects arrays, and `repository.ts` stores 0 the same way.

## Why the W4 fix agent did not change it

The profile model crate is frozen for W4, and it copies TS on purpose. Fixing it is a deliberate departure from TS, the same kind of call as D30. That needs an orchestrator decision.

## Ask

Decide whether the port fixes it. If yes:

1. In `parse_stats`, read `stats.tricks` as an array of two counts. Keep the object form as a fallback.
2. Update the parity comment, and add a test that a recorded match keeps its trick totals.
3. Existing rows stay at `[0, 0]` unless they are backfilled from the `hands` table.
