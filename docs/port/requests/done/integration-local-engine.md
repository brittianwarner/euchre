# Request from SERVER-FOUNDATION to the owner of `src/server.rs` / `src/bootstrap/`: local-mode engine

**Resolved.** `server.rs` boots `rivet::LocalEngine` for `EngineMode::Spawn` (W3 S1-BOOT, `W3-checklist.md`); moved to `done/` in the W4 review fixes. Same text as `done/foundation-local-engine.md`.

**Files:** `apps/server/src/server.rs`, `apps/server/src/bootstrap/` (not mine)
**Severity:** medium. This affects desktop embedded mode and local dev.

## What

`ServerConfig::local(data_dir)` describes a loopback engine with `EngineMode::Spawn`. Since the review fixes it:

- picks a **free port block** (`config::pick_local_engine_port`: P, P+1 and P+5 all bindable) instead of 6420, which other engines on dev machines already hold;
- carries a **random admin token** in `cfg.engine.token`, persisted in `secrets.json` under `RIVET_TOKEN`. The well-known token `default` is never used.

If `server.rs` uses rivetkit's `EngineSpawnMode::Always` for this instead, then:

1. rivetkit spawns the engine on its shared default storage (`$HOME/.rivetkit`) unless `RIVETKIT_STORAGE_PATH` is set. That variable can only be set through the environment, and the workspace forbids `unsafe`, so it cannot be set from inside the process. Every local server and test would then share one engine database.
2. Its readiness probe can miss an engine that is already running on the port. It then either adopts a foreign engine or spawns a second one.
3. Its metrics port is `P + 10`, which collides with the lane port layout (see `foundation-engine-ports.md`).
4. It starts the engine with the well-known admin token `default`.

## Ask

For `EngineMode::Spawn`:

1. Start the engine with
   `rivet::LocalEngine::start(LocalEngineOptions::new(cfg.engine.engine_port, data_dir.join("engine")).with_admin_token(token))`,
   where `token` is `cfg.engine.token` (always `Some` in local mode). It refuses a port that already answers `/health`, and it reclaims an engine orphaned by a killed earlier run of the same data dir (pid file `euchre-engine.pid`; an engine whose owner process is still alive is never touched). Then serve the registry with `ServeConfig::from_env()`, overriding:
   - `endpoint = engine.endpoint()`;
   - `token = Some(engine.admin_token().expose().to_owned())`;
   - `namespace = LOCAL_ENGINE_NAMESPACE`;
   - `engine_spawn = EngineSpawnMode::Never`;
   - `engine_host`/`engine_port` = the loopback pair;
   - `engine_auto_download = false`;
   - `start_services = false`;
   - a fresh `version`.

   `testing::TestEngine::start` is a working reference: the foundation probe passes against it on port 6701.
2. Keep the `LocalEngine` in `RunningServer` and call `shutdown().await` after the registry stops. Dropping it also stops the engine, bounded at 15 s.
3. If the free-port pick fails (it logs a warning and falls back to 6420), let `LocalEngine::start`'s "port already in use" error surface as a clear UI error. `ServerConfig::with_engine_port` exists to pin a port.

## API changes from the review fixes (for every lane)

- `OriginPolicy::is_local_dev()` is gone; it is `allows_unlisted()`. Local mode is now **strict** (a present `Origin` must be allowlisted, an absent one passes, D1). Add the desktop/dev origins with `OriginPolicy::allow(origin)` or build one with `OriginPolicy::strict(list)`.
- `rivet::LOCAL_ENGINE_TOKEN` is gone. Use `cfg.engine.token` / `LocalEngine::admin_token()`. `rivet::DEFAULT_ENGINE_TOKEN` names the well-known value only so code can refuse it.
- `testing::TestEngine::engine()` is gone; use `token()` and `engine_pid()`. All `TestEngine`s in one test binary share one engine (refcounted; the last one stops it), each on its own pool, so tests may run in parallel.
- Engine tests are plain `#[tokio::test]`s that call `testing::e2e_skip(name)` and return when `!e2e_enabled()`; see `foundation-e2e-skip.md`.
- `rivet::CallError::Rejected { message }`: `connect_bounded` now fails fast when the first WebSocket attempt is refused, instead of waiting out its timeout. The message carries no `group/code`; make one stateless call to classify it.
- `rivet::wait_ready` returns at once on a `user/*` error.
- `rivet::JsonTable` quotes its table name and rejects names starting with `sqlite_` or `_rivet_`.
- Deployments (`EUCHRE_PRODUCTION=1`) now also require `RIVET_TOKEN`, `RIVET_NAMESPACE` and `RIVET_ENDPOINT`, and refuse URL userinfo and a public endpoint that embeds the token (TS `assertRivetEnvSafety`).
