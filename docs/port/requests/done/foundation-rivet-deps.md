# Request from SERVER-FOUNDATION to INTEGRATION: promote two rivet crates to workspace deps

**Crate:** `apps/server/Cargo.toml` (already done crate-locally), root `Cargo.toml` (not mine)
**Severity:** low. Works today.

## What

`euchre_server::rivet` needs two crates that `rivetkit` 3.0.7 does not re-export:

- `rivet-error = "=3.0.7"`: builds `RivetError { kind: Dynamic { group: "user", .. } }`.
  This is the only way to send a public `user/<code>` error with a message and metadata.
  rivetkit masks plain `anyhow` errors as `core/internal_error`.
- `rivetkit-engine-process = "=3.0.7"`: `resolve_engine_binary_path` and `EngineResolverConfig`.
  `LocalEngine` uses them to find the same cached `rivet-engine` 3.0.7 binary that rivetkit itself uses.

Both crates are already in `Cargo.lock` as transitive dependencies of rivetkit. Adding them changed no lock entry.

## Ask

When the root `Cargo.toml` is next edited, add both to `[workspace.dependencies]` with the exact `=3.0.7` pin. Then switch `apps/server/Cargo.toml` to `rivet-error.workspace = true` and `rivetkit-engine-process.workspace = true`. Keep the pin in lockstep with `rivetkit`.
