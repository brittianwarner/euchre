# Request: playerProfile write bodies for the aiSeat flushes (from lane AI-LLM)

**Need.** aiSeat sends two writes to `playerProfile` at hand and game end
(`actors/ai-seat/index.ts` `flushEpisodes` and `flushTokens`). `euchre-protocol` has no
types for those bodies yet.

**Ask.** Publish these in `euchre-protocol` (the profile module), with the same serde shape
(camelCase, field order as written):

- `RecordEpisodesBody { internalToken, episodes: Vec<EpisodeOut> }`
- `EpisodeOut { role, matchId, handNo, ts, kind: EpisodeKind, summary, salience }`.
  `salience` is a JS number (`js_f64`).
- `RecordTokensBody { internalToken, tokensIn, tokensOut, cacheReadTokens, calls }`

**Local workaround.** Lane AI-LLM defines all three in `crates/euchre-ai/src/ai_seat/handlers.rs`
and covers them with serde shape tests in `tests/seat_handlers.rs`. Once the protocol owns
them, `handlers.rs` re-exports the protocol types under the same names and drops its own
definitions. The public API does not change.

## Resolution (orchestrator, 2026-11-11)

Closed by D34. The AI-SEAT lane checks its `recordEpisodes`/`recordTokens` bodies against the `euchre-profile-model` parsers. `seams_full_match_end_to_end` proves them against the real `playerProfile`: `listEpisodes` matches each seat's notes, and `dailyTokens` equals the seats' budgets. Moved to `done/`.
