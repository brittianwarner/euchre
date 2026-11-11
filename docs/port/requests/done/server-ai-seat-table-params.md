# aiSeat callers must pass `{ internalToken }` as params (SERVER-AI-SEAT → SERVER-TABLE, INTEGRATION)

**Resolved.** Asks 1-2 by SERVER-TABLE, ask 3 by W3 S1-BOOT (`W3-checklist.md`); moved to `done/` in the W4 review fixes.

## Problem

TS `aiSeat` let the stateless HTTP publish through and denied WebSocket connections in its queue guard (`externalDenied`). rivetkit 3.0.7 Rust cannot tell a WebSocket connection from a stateless HTTP call: `ctx.conn()` is set for both. So the Rust seat cannot use the TS guard as written.

## What SERVER-AI-SEAT did

- The four message actions (`decide`, `resetHand`, `handEnd`, `gameEnd`) require connection params `{ internalToken }` that match the match's internal token (constant-time, `internal_token_ok`).
- Any other caller of those actions is refused with `user/internal_token_mismatch`, message `aiSeat accepts actor-to-actor calls with the internal token only`.
- Connections are not gated, and the reads (`getStatus`, `getPersona`, `getDecisionLog`) stay open, as in TS. The TS tooling (`scripts/ai-status.mjs`, `scripts/e2e-browser.mjs`) reads `getStatus` with no params.
- Calls with no connection (internal dispatch) are allowed.
- Each message body is still checked by the pure handlers, as in TS.

## Asks

1. **SERVER-TABLE:** call the seat as 03 §16.7 says:
   ```rust
   client.get_or_create(
       "aiSeat",
       vec!["table".into(), game_id.into(), "seat".into(), seat.to_string()],
       GetOrCreateOptions {
           params: Some(json!({ "internalToken": token })),
           create_with_input: Some(serde_json::to_value(&input)?),
           ..Default::default()
       },
   )?
   .action("decide", vec![serde_json::to_value(&req)?])
   ```
   Do the same for `resetHand`, `handEnd` and `gameEnd`. A call without these params is refused. The TS table passed no params.
2. **SERVER-TABLE:** the seat calls the table's `aiDecision` and `aiSay` with params `{ internalToken }`, as TS did, using stateless calls through the server's shared JSON client. The table's `on_before_connect` must accept that connection.
3. **INTEGRATION:** register the seat with `euchre_ai_seat::register(&mut registry, AiSeatRuntime::from_config(SeatClient::new(client), &cfg))`. It takes the models from `cfg.ai` and puts `cfg.secrets.internal` in the profile writes. In local mode that secret lives in `secrets.json`, not in the environment. `AiSeatRuntime::dev_from_process_env` (env only, with the dev fallback secret) is for tools, never for a server: a local server's profile would refuse its writes.
