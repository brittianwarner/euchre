# W3 integration checklist (orchestrator-owned)

Items the W2 lanes handed to the integration wave. Each must be done or explicitly closed before W3 reports done.

## Server boot (`apps/server`)

- [x] `server.rs` boots the engine with `rivet::LocalEngine` for `EngineMode::Spawn` — `requests/done/integration-local-engine.md`. Done by S1-BOOT (`reports/server-boot.md`): admin token from config, data under `<data_dir>/engine`, no download, `ServeConfig::from_env()` with the loopback overrides; a busy port is `BootError::EnginePortInUse`.
- [x] Registry: register `euchreTable`, `aiSeat`, `playerProfile` (`euchre_profile::register` returns `Result`; fail start-up on `KeysConflict`) — `requests/done/server-profile-auth-scope.md`. Test binaries that start several servers share one secrets set. Done by S1-BOOT: plus `health`; `KeysConflict` fails boot before any engine starts; the e2e shares one `secrets.json` and proves the conflict.
- [x] Compose `euchre_http::router` / `ApiState` / `Health`; draining on shutdown; static dir for the web SPA (D4). Done by S1-BOOT: drain = `/health` 503 and new tables refused, then HTTP, registry, engine; static dir only when it exists (`build/client` for the binary).
- [x] Namespace bootstrap uses `AdminDialect::V3` against engine 3.0.7 — `requests/done/server-http-engine-admin-api.md`; update 05 §5.4 text. Done by S1-BOOT: 05 §5.4 now describes `POST /namespaces {name}` and `PUT /pools/{pool}` with the flat body, with a V2 note.
- [x] Gateway tokens: Option A (engine `POST /auth/tokens`) — `requests/done/gateway-token.md`. Clients use json. Done by S1-BOOT: `ApiConfig.engine_token` is set from the engine token; the e2e connects with the issued `gatewayToken` and JSON encoding.

- [x] Register aiSeat with `euchre_ai_seat::register(&mut registry, AiSeatRuntime::from_config(SeatClient::new(client), &cfg))`. Never use `dev_from_process_env` in a server: a local server keeps `secrets.internal` in `secrets.json`. Source: `requests/done/server-ai-seat-table-params.md` ask 3; asks 1-2 are done by SERVER-TABLE. Done by S1-BOOT, on the one shared client.
- [x] Rivet engine 3.0.7 refuses to start with < 4 GiB free disk (`RIVET__STORAGE__LOG_SHARD__MAX_DISK_BYTES` does not lower it). The embedded desktop mode must surface that as a plain error, not hang. Done by S1-BOOT: checked before the spawn, `BootError::LowDisk` names the free space and path; showing it in the desktop UI is W4.

## Foundation follow-ups (`crates/euchre-server-core`)

- [x] Add an explicit trusted-proxy setting (`EUCHRE_TRUST_FORWARDED_FOR=1` or trusted CIDRs) to `ServerConfig`; `euchre-http::ApiConfig::from_server_config` reads it — `requests/server-http-trusted-proxy.md`. Done by S2-CRATES 2026-11-11 (`reports/w3-crates.md` §1). It ships off by default; `EUCHRE_TRUSTED_PROXIES` is optional. The request is in `done/`. W4 still owes `EUCHRE_TRUST_FORWARDED_FOR=1` in `.railway/railway.ts`. Done in W4: it is a literal `"1"` on the `euchre` service (`reports/w4-integration.md`).

## Cross-actor seams

- [x] SEAMS stage of W2-server-b proves table ↔ aiSeat ↔ playerProfile with the real actors (`reports/seams.md`; `seams_full_match_end_to_end` passed twice, orchestrator re-ran it green 2026-11-11 in 82 s).
- [x] `requests/ai-profile-bodies.md`: closed (D34), moved to `done/`.

## Decisions taken by the orchestrator

- [x] D30 (recorded in ARCHITECTURE 2026-11-10): fix the TS `forgetEpisode` bug — the id pattern must allow `.` because stored ids are `role.matchId.handNo.kind`; otherwise no episode can ever be forgotten. Change in `euchre-profile-model` guards + a test; note it as a deliberate deviation from TS. Model side done by S2-CRATES 2026-11-11: `require_episode_id` / `is_valid_episode_id`, with tests and `reports/profile-model.md` deviation 10. Callers done by S1-BOOT 2026-11-11 in the W3 review fixes: the actor uses `require_episode_id`, `DELETE /api/episodes/{id}` uses `is_valid_episode_id`, and `profile_e2e` forgets a stored id (`requests/done/server-profile-d30-forget-episode-callers.md`).
- [x] `recordGame` and rivetkit's 64 KiB argument limit: check the largest realistic match (gameTo 15, many hands) in the e2e; split `recordHands` if it can exceed it. Checked by S2-CRATES 2026-11-11 (`reports/w3-crates.md` §3). The limit is 65,536 B per action body. The worst reachable `recordGame` is 39,965 to 40,190 B; with dossiers maxed, which nothing produces, it is 59,049 B. `recordHands` is one hand per call, about 5.7 KB at `JOURNAL_CAP`. A real-engine probe stored all of these, and a 92,721 B body was refused with `message/incoming_too_long`. No split is needed and `euchre-table` is unchanged. Recommended: cap personas at table create.

## Desktop

- [x] `requests/desktop-profile-types.md`: desktop depends on `euchre-profile-model` for `ProfileSettings` / `SettingsPatch` / `MatchDetail` (D29 replaces the protocol ask). Done by motion Lane D (`apps/desktop/src/net/mod.rs` speaks `ProfileSettings` / `SettingsPatch` / `ProfileStats` / `ClientMatchReplay`).
- [x] `RemoteBackend` (HTTP API + rivetkit-client, json) for remote and embedded modes; embedded mode starts `euchre_server::start(ServerConfig::local(app_data_dir))`. Done by the desktop net wave 2026-11-11 (`reports/desktop-net-remote.md`, `desktop-net-embedded.md`, `desktop-net.md`). Embedded is the default, `EUCHRE_SERVER_URL` selects remote, and `EUCHRE_DESKTOP_BACKEND=sim` selects the sim (`apps/desktop/README.md` "Modes"). Evidence on port 6870, final code, all with `EUCHRE_E2E=1`:
  - `tests/live.rs` played a full match to game over through the app, 3 of 3 green in a row (runs G, H, I). Move ack p50 31–32 ms and p95 41–61 ms; reconnect 379–453 ms.
  - `tests/embedded_live.rs` and `tests/remote_live.rs` are green.
  - A release window booted cold in 6.6 s and warm in 4.8 s, enrolled, answered `/health` 200 and quit with its engine in about 3.2 s.
  - Server follow-ups after an engine crash and Restart are in `requests/desktop-net-stale-registry.md` and `desktop-net-dead-engine-grace.md`.
- [x] Motion + visual implementation per `09-motion-visual-spec.md`. Done by the desktop-motion wave (`reports/desktop-motion.md`, including "Review fixes" and "Polish round"); orchestrator re-ran the gates green 2026-11-11 (fmt, clippy, 159 tests) and built the release binary. The motion requests moved to `requests/done/`. Carried to W5: bid chip, turn ping, hero flight, trump badge, home hero Ace contrast (S-2).

## Follow-ups found in W3 (small, owner crates)

- [x] `euchre-table`: cap persona size at table create (S2-CRATES, `reports/w3-crates.md` §3). An oversized persona would make every `recordGame` for that match fail and retry forever. Add a regression test at the cap. Done by W-C (`reports/w3-crates.md` §4): `TableState::create` bounds the input in `src/create_input.rs`. Personas are re-sanitized to their caps (code points, as TS), one per AI seat. The seed is cut to 256 UTF-16 units, and an off-pattern match id or a user id over 128 units is refused. `worst_case_record_game_fits_the_action_limit` measures 63,617 B against 65,536.
- [x] `euchre-profile`: clamp the token counters (or use saturating sums) so two near-maximum matches cannot overflow the stats sum and wedge every later `recordGame` for that profile (S2-CRATES probe). Done by W-C (`reports/w3-crates.md` §4): `upsert_match` stores each counter clamped to `MAX_SAFE_INTEGER`, and `compute_stats` sums with `TOTAL()`, capped at the same value. `profile_token_overflow_end_to_end` fails without the fix and passes with it.

## End-to-end gate

- [x] `EUCHRE_E2E=1 cargo test -p euchre-server --test e2e`: full match human vs 3 AI through a real 3.0.7 engine via rivetkit-client, incl. reconnect, illegal moves, review gate and profile recording. Done by S1-BOOT: also a drain and restart mid-match on the same data dir; green 4 runs in a row (`reports/server-boot.md`); orchestrator re-ran it green 2026-11-11 (2/2, 85 s).
- [x] Live headless snapshots of a real match against the embedded server. Done by the desktop net integrator 2026-11-11. `tests/live.rs` records the real syncs as `LiveFixture` files and renders them with `--scene live-*` at 1280x800, scale 1:
  - table moments: bid, play, trick won, trick review, hand result, game over, server stopped;
  - Your games and the match detail.

  The set, with boot and Home stopped and the `overview.png` contact sheet, is in `scratchpad/net/gallery/` (paths in `reports/desktop-net.md`). Every PNG was opened and checked; each run also keeps its own under `integration-<pid>/snaps/`.
