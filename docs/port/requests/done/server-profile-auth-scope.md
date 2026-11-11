# playerProfile keys are one set per process (SERVER-PROFILE → INTEGRATION, FOUNDATION)

**Resolved.** Boot fails on `KeysConflict` before any engine starts, and test binaries share one `secrets.json` (W3 S1-BOOT, `W3-checklist.md`); moved to `done/` in the W4 review fixes.

## Problem

rivetkit 3.0.7 builds an actor with `Actor::create(ctx)` and no factory argument, so `euchre_profile::register` installs the `Authenticator` in a process-wide static. Each `playerProfile` reads it on every wake.

`ServerConfig::local(data_dir)` generates different secrets for each `data_dir`. If one test binary starts two servers with different data dirs, the second server's keys would replace the first server's keys. The first server's profiles that wake afterwards would then verify profile JWTs and `EUCHRE_INTERNAL_TOKEN` with the wrong keys. The failures would be intermittent, because profiles that are already awake keep the old keys.

## What SERVER-PROFILE did

- `euchre_profile::configure`, `register` and `register_with` now return `Result<(), ConfigureError>`.
- The same keys again return `Ok`, so several registries with one config are fine.
- Different keys return `ConfigureError::KeysConflict` and log `playerProfile keys already configured`. The first key set stays and nothing is registered.

## Asks

1. **INTEGRATION:** in a test binary that starts more than one `euchre_server::start(cfg)`, share one secrets set. For example, use one `data_dir` for the secrets, or pass explicit `AuthKeys`. Handle the `Result` from `euchre_profile::register`, and fail start-up on `KeysConflict`.
2. **FOUNDATION (optional):** if rivetkit gains a per-registry way to pass state into `Actor::create`, such as a factory or registry-scoped extensions, expose it so the keys can be scoped to one registry. The static can then go.
