# Request from SERVER-FOUNDATION to ARCHITECTURE / all lanes: engine metrics port

**Doc:** `docs/port/ARCHITECTURE.md` §5 (port table)
**Severity:** medium for parallel lanes. Already mitigated in `rivet::LocalEngine`.

## What

A `rivet-engine` bound to guard port `P` also binds a peer port and a metrics port.
rivetkit's own engine spawner puts metrics at `P + 10`. The lane ports are spaced by exactly 10 (6701, 6711, ...), so that layout makes one lane's engine grab the next lane's guard port.

## Done

`euchre_server::rivet::LocalEngine`, and so `testing::TestEngine`, bind:

- the guard at `P`;
- the peer at `P + 1` (`PEER_PORT_OFFSET`);
- metrics at `P + 5` (`METRICS_PORT_OFFSET`).

With those offsets every lane stays inside its own `P .. P+9` block.

## Ask

- Add one line under the port table: "engines bind P, P+1 and P+5; do not start an engine through rivetkit's `EngineSpawnMode::Always` in lane tests (it uses P+10)".
- Lanes should start engines only through `TestEngine` / `LocalEngine`.
