# Desktop net: a shorter registry grace once the engine is gone (SERVER)

**From:** the desktop net wave, lane N2 (`apps/desktop/src/net/embedded.rs`). Contract:
`docs/port/reports/desktop-net-contract.md` §N2. Report: `docs/port/reports/desktop-net-embedded.md`.
**Owner:** `apps/server/src/server.rs` (W4). The desktop does not edit it.
**Blocking:** no. The desktop works today; a quick Restart just waits about 5 s longer.

## What

When the spawned engine dies, `RunningServer::stop` still waits
`REGISTRY_GRACE_ENGINE_GONE` (5 s) for the registry to stop. With the engine gone, no actor can
drain, so the full 5 s passes every time. The log shows `registry did not stop in time
after=5s`, then the stop finishes (`stop_ms=5001` in the desktop's log).

The desktop cannot start the next server until that stop ends. The HTTP port is sticky, and the
process-global table client is unbound only after the registry wait. So a player who presses
**Restart** as soon as "The game stopped" appears waits for:

- the 5 s grace;
- then the boot (about 4.5 s).

That is 9.6 s in `tests/embedded_live.rs`. It should be about 4.5 s.

## Why

The embedded server is the desktop's only server. An engine crash is the one failure a player
fixes with a button, so the wait after that press is the part they notice.

## Proposed change

In `apps/server/src/server.rs`:

```rust
/// Registry grace once the engine is gone: no actor can be drained through it.
const REGISTRY_GRACE_ENGINE_GONE: Duration = Duration::from_millis(500);
```

If the 5 s is there for a case the desktop does not see, there is another way. Keep 5 s for the
deployed server, and use the short grace only when `self.registry_grace ==
REGISTRY_GRACE_LOCAL`:

```rust
let grace = if self.supervision.engine_gone() {
    let gone = if self.registry_grace == REGISTRY_GRACE_LOCAL {
        Duration::from_millis(500)
    } else {
        REGISTRY_GRACE_ENGINE_GONE
    };
    gone.min(self.registry_grace)
} else {
    self.registry_grace
};
```

**How to check:** run
`EUCHRE_E2E=1 cargo test -p euchre-desktop --test embedded_live -- --nocapture`. The
`restart_ready_ms` it prints should drop from about 9.6 s to about 5 s.

## Review update, 2026-11-11 (review fix PX-5)

**No longer needed by the desktop.** After an engine death the desktop does not call
`RunningServer::shutdown` any more. With the engine gone there is nothing to drain or deregister,
so it drops the server on the server's own runtime and shuts that runtime down within 2 s
(`clear_dead`, `DEAD_STOP` in `apps/desktop/src/net/embedded.rs`). That ends the HTTP side, the
registry and every task the server spawned, and frees the HTTP port before Restart boots again.

- Measured: the stop after a crash takes 0–2 ms (it was 5001 ms). Restart to ready dropped from
  13.6–14.3 s to 9.3–10.3 s in `tests/live.rs`.
- Covered by the unit test `closed_stops_and_ends_tables`: a dead server is cleared (its drop
  runs) and never drained (no `shutdown`).
- The rest of the Restart wait is the engine's boot after a crash
  (`desktop-net-stale-registry.md`, "Review update").

The 500 ms grace is still worth landing for any other host that calls `shutdown` after an engine
death, such as `apps/server`'s own restart path.

