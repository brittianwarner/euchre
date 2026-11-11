# ai-llm lane report

Lane: **AI-LLM**. This is the model-facing half of `crates/euchre-ai`: the Jev decision client, `callModel`/`decide`, banter streaming, the talk transports, prompts and model params, and the aiSeat ladder, bridge and queue handlers.
TS source of truth (read-only): `~/goods/euchre/src/lib/ai/*`, `src/lib/actors/ai-seat/*` and `scripts/or-smoke.ts`.

## Gates

All three gates pass, run with `CARGO_TARGET_DIR=.cargo-targets/ai`.

| Gate | Result |
| --- | --- |
| `cargo fmt -p euchre-ai -- --check` | pass |
| `cargo clippy -p euchre-ai --all-targets -- -D warnings` | pass (the example is included) |
| `cargo test -p euchre-ai` | pass, 169 tests, 0 failed (after the review fixes). This lane added 94 of them. |

- Library code has no `unsafe`, no `unwrap`/`expect` and no `todo!()`, and every public item has a doc comment.
- The engine crate, the protocol crate and the root `Cargo.toml` are untouched. New deps use `{ workspace = true }`.
- No test reaches the network. The HTTP paths run against local `wiremock` servers with the key `"test"`.
- Every timing test runs on `tokio::time::pause` with `TokioClock` (test-only; production uses `SystemClock`).

## Tests: TS vs Rust

| TS | Rust | Notes |
| --- | --- | --- |
| `jev.test.ts` (6 `it` + 7 `it.each` rows) | `tests/jev.rs` (10) | 1:1 port. The 7 `it.each` rows run in one loop that checks the reason and the violation code. Added tests: endpoint derivation, an abort mid-flight, zod schema edges (`id: null`, fractional or negative tokens, the ±0.02 and 1e-6 slacks), and undefined `instructions`. |
| `decision-context.test.ts` "Jev owns bid and loner choices" (3) | `tests/decide.rs` | 1:1 port, plus a typed test of every composition failure message. |
| `table-context.test.ts` (callModel observations) | `tests/decide.rs` | Runs through the real `call_model` request. |
| `table-context.test.ts` (conversational `buildLayers`) | `tests/prompt.rs` | Grandpa, voice and tone only. |
| `partnership.test.ts` "Jev integration" (2) | `tests/decide.rs` | Case 0 uses `call_model` over `model_factory_from_env` with the decision and talk swapped. Case 1 uses `run_ladder` with a scripted decider. |
| none (alternate `decide`) | `tests/decide.rs` (9) | Covers the forced, no provider, degraded, `ai_ok`, timeout plus escalation, play-never-escalates, `ai_error` stop, deadline floor and empty-legal paths. |
| none (`runLadder`) | `tests/ladder.rs` (16) | Covers every rung, `ai_no_budget`, `ai_attempt_failed` and `jev_failure` reasons, and the deadline edges +4000 (fallback at 3600 ms), +2900 (second budget exactly 400 runs), +2899 (399 is skipped), +1000, +699 and a past deadline. Also runs `JevDecider` end to end. |
| none (SSE) | `tests/sse.rs` (16) | Recorded OpenRouter and Anthropic streams split at every byte, every single byte, and every 3-way split. Also covers CRLF, comments, `[DONE]`, in-band errors, EOF flush, body shapes and the wiremock clients. |
| none (`generateBanter`) | `tests/banter.rs` (9) | Covers the salience gate and threshold, no provider, the stream screen, the final screen, provider errors, the timeout, aborts and `on_delta`. |
| `scripts/prompts.ts` golden | `tests/prompt.rs`, `tests/model.rs` | Checks L0, the banter line, 20 layer cases (layers, instructions, logs and `cachePrefixTokens`), `sanitizePersona`, `publicStateOnly`, the allowlist, 88 `modelParams` rows and the clamp. |
| none (aiSeat `index.ts`) | `tests/seat_handlers.rs` (16) | Covers the token, game, seat and stale checks, an empty legal set, a `Send` check via `tokio::spawn`, the banter gate and job, lifecycle messages, and the episode and token flushes. |

## Public API for the aiSeat glue lane

```rust
// Composition root (call once)
model::model_factory_from_process_env() -> Option<ModelFactory>
model::model_factory_from_env_with(env: &dyn Fn(&str) -> Option<String>, http: Option<reqwest::Client>) -> Option<ModelFactory>
ai_seat::bridge::LlmProvider::from_factory(factory: Option<Arc<ModelFactory>>, deps: ProviderDeps { clock, log }) -> LlmProvider
ai_seat::bridge::LlmProvider::none() -> LlmProvider
ai_seat::bridge::JevDecider::new(Arc<ModelFactory>, Arc<dyn Clock + Send + Sync>).with_cancel(CancellationToken)
ai_seat::handlers::profile_internal_token(env: &dyn Fn(&str) -> Option<String>) -> Option<String>
clock::SystemClock;  log::TracingLog   // SystemClock reads Date.now per call; implements AiLog + SeatLog
// clock::TokioClock is test-only: it drifts from the wall clock that stamps deadlineAt

// Queue handlers (`state` is the actor state; the glue saves and sends)
struct SeatEnv<'a> { clock: &'a (dyn Clock + Send + Sync), log: &'a (dyn SeatLog + Send + Sync), provider: &'a LlmProvider }
async fn on_decide(state: &mut AiSeatState, body: &AIDecideRequest, env: SeatEnv<'_>) -> DecideHandled
//   Answered(Box<Answered { decision: AIDecision, outcome }>): save, send `decision` on aiDecision,
//   then begin_banter. Dropped(_): persist only if `state_changed()`.
fn begin_banter(state: &mut AiSeatState, body: &AIDecideRequest, provider: &LlmProvider) -> Option<BanterJob>
//   Some: save (last_hand_no moved), then spawn run_banter_job.
async fn run_banter_job(provider: &LlmProvider, job: &BanterJob) -> Option<AiSayMsg>   // send on aiSay
fn on_reset_hand(state: &mut AiSeatState, body: &AiSeatLifecycleMessage) -> bool
fn on_hand_end(state, body, now_ms: u64, profile_token: Option<&str>) -> Option<EpisodeFlush>
fn on_game_end(state, body, profile_token: Option<&str>) -> Option<GameEndFlush { episodes, tokens }>
fn complete_episode_flush(state, result: Result<(), &str>, log)   // after the recordEpisodes send
enum EpisodeFlush { Nothing, Dropped, Deferred, Send { profile_key: Vec<String>, body: RecordEpisodesBody } }
struct TokenFlush { profile_key: Vec<String>, body: RecordTokensBody }
fn internal_token_ok(expected: &str, got: &str) -> bool
mod messages { FORGED_DECIDE, WRONG_SEAT, EMPTY_LEGAL, BANTER_UNDELIVERED, FLUSH_EPISODES_FAILED, FLUSH_TOKENS_FAILED, QUEUE_HANDLER_FAILED }

// Lower-level seams
ai_seat::ladder::run_ladder(deps: LadderDeps<'_>, req: &AIDecideRequest) -> Option<LadderOutcome>
ai_seat::bridge::call_decider(&dyn LlmDecider, &LlmDecideInput, &(dyn SeatLog + Send + Sync)) -> Option<LlmDecideResult>
decide::call_model(&DecideDeps, CallModelRequest, &[LegalMove], CallModelOptions) -> Result<DecideOutcome, CallModelError>
decide::decide(&DecideDeps, &AIDecideRequest, &[LegalMove], DecideOptions) -> Result<DecideOutcome, CallModelError>
banter::generate_banter(&DecideDeps, BanterRequest) -> BanterOutcome
#[async_trait] trait jev::JevClient { async fn decide(&self, JevDecisionInput, CancellationToken) -> Result<JevDecisionOutput, JevError> }
#[async_trait] trait talk::TalkClient { async fn stream(&self, TalkRequest, CancellationToken) -> Result<TalkStream, TalkError> }
#[async_trait] trait LlmDecider / LlmBanterWriter   // for scripted providers (see `testing`)
```

`testing` provides the following stand-ins for the glue lane's own tests: `RecordingLog`, `ManualClock`, `ScriptedJev`, `HangingJev`, `ScriptedTalk`, `HangingTalk`, `ScriptedDecider`, `ScriptedBanter`, `jev_factory` and `tiered_factory`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `OPENROUTER_API_KEY` | When it is non-blank, OpenRouter is the provider. This is the only way to get a Jev decision client. |
| `OPENROUTER_BASE_URL` | The talk base, default `https://openrouter.ai/api/v1`. Jev uses `{base minus one "/" minus one "/v1"}/alpha/decisions`. |
| `OPENROUTER_MODEL_TALK` | The talk slug for both tiers. An empty value still counts as an override. The default is `google/gemini-3.6-flash`. |
| `OPENROUTER_MODEL_PLAY` | Ignored (legacy). |
| `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL` | The fallback text-only provider. It has no decision client. |
| `EUCHRE_INTERNAL_TOKEN` | The profile secret when it is at least 16 UTF-16 units long. Otherwise `DEV_PROFILE_TOKEN` is used, but only when `RIVET_ENDPOINT` is unset or empty. |

`examples/or_smoke.rs` is the opt-in live check. It reads `OPENROUTER_API_KEY` and is never run by the tests.

## Deviations from TS

1. **Banter timeout or abort.** These give a `banter_error` (`e: "TimeoutError"` / `"AbortError"`) and a phrasebook line, never a truncated model line.
2. **SSE.** A pending unterminated event is flushed at EOF. Only LF and CRLF line endings are supported (a bare CR is not).
3. **Anthropic `effort`.** `providerOptions.anthropic.effort` is sent as `output_config.effort` on the raw Messages request. The TS side leaves this mapping to the ai-sdk, so a live check against the Anthropic fallback is still owed.
4. **Error branches.** The Jev path has no source for the ai-sdk `NoObjectGeneratedError`/`APICallError` branches of `shouldContinueAfter`. `JevDecisionError` and transport errors log `ai_error` and stop.
5. **Transport errors.** These are reduced to coarse kinds (`connect`, `request`, `body`). Response bodies are never kept on failure.
6. **Global provider.** `setLlmProvider` (a module global) is replaced by an injected `LlmProvider`.
7. **`personasFromSettings`.** Deferred to the profile lane; its test belongs there.
8. **Logging.** There are two log traits. `AiLog` takes the closed `AiLogEvent` enum and `SeatLog` takes free-form actor log names. `SeatAiLog` and `TracingLog` adapt between them.
9. **Undefined `instructions`.** A `Null` `instructions` stands for `undefined` and is omitted from the Jev body, as `JSON.stringify` does.
10. **Timer ties.** If the banter writer and the 1800 ms deadline become ready on the same tick, the writer wins (tokio `timeout` polls the inner future first).
11. **A non-positive budget makes no call.** TS evaluates `decider(input)` before `withDeadline` rejects with `no budget`, so it starts a model call and abandons it. Rust futures are lazy, and `call_decider` returns `None` and logs `jev_failure` (`transport_or_adapter`) without calling the decider. The result and the log match TS. Only the abandoned request (and its token spend) is gone. `runLadder` never reaches this case because of its 400 ms floor. `call_banter_writer` behaves the same way.

## Requests

- `docs/port/requests/ai-profile-bodies.md` asks for `RecordEpisodesBody`, `EpisodeOut` and `RecordTokensBody` in the protocol. They are defined locally in `ai_seat/handlers.rs` until then.

## Review fixes

| Finding | Outcome |
| --- | --- |
| P2 `clock.rs`: the recommended clock `TokioClock::from_system` drifts from the wall clock that stamps `deadlineAt` | **Fixed.** The public API above now points the composition root at `SystemClock`, which reads `SystemTime` on every call (`Date.now`). The `TokioClock` docs mark it test-only and say why. Nothing in the crate defaults to `TokioClock`, and no other crate uses it. Regression test: `clock::tests::system_clock_reads_wall_time_on_every_call`. |
| P3 `talk.rs`: the OpenRouter body drops `cache_control` and adds `stream_options` | **Fixed.** Checked against `@openrouter/ai-sdk-provider` 3.0.0 (`convertToOpenRouterChatMessages`, and `doStream` with the default `compatibility: 'compatible'`). Each system message is now `content: [{ type: "text", text, cache_control? }]`, with `cache_control: { type: "ephemeral", ttl: "1h" }` when the layer is cached. `stream_options` is gone. The `openrouter_body_shape` golden now asserts the TS shape, including one cached and one uncached layer. |
| P3 `tests/seat_handlers.rs`: the episode-buffer cap check could not fail | **Fixed.** The test now keeps a real snapshot (hand 12, maker made the point), so `on_hand_end` pushes a 13th episode. It asserts the length is 12, hand 0 is gone, hand 1 is first and the new episode is last. |
| P3 `tests/ladder.rs`: the non-positive-budget test asserts no call, but TS makes one | **Kept, documented.** Skipping the call is deliberate. It is natural for lazy Rust futures, and it saves a request that TS abandons anyway. The result (`None`) and the `jev_failure` reason match TS. Recorded as deviation 11 above, and as a comment in the test. |
| P3 `heuristic.rs`: a play id that cannot be parsed gets an empty why | **Documented, not changed.** The TS why for a garbage card comes from running `base()` on an invalid `CardId`, and which branch it takes depends on how `power`, `effectiveSuit` and `rankOf` treat garbage. Reproducing that is not worth it for an id the engine never produces. The score (0) and the ranking order already match. `ai-core.md` deviation 4 now says the why is empty, and the code comment says so too. |

Gates after the fixes: `cargo fmt -p euchre-ai -- --check` pass. `cargo clippy -p euchre-ai --all-targets -- -D warnings` pass. `cargo test -p euchre-ai` pass, 169 tests, 0 failed. That includes `tests/differential.rs` (9 tests).
