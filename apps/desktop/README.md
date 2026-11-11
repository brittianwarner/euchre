# Euchre desktop

A native Rune window over the shared engine and protocol (D28). The code map is in
`src/lib.rs`; the specs are `docs/port/06-*` (look and voice), `07-*` (desktop architecture)
and `09-motion-visual-spec.md`.

```bash
cargo run -p euchre-desktop                 # the window, with the embedded game server
cargo build -p euchre-desktop --release     # target/release/euchre
cargo fmt -p euchre-desktop --check
cargo clippy -p euchre-desktop --all-targets -- -D warnings
cargo test -p euchre-desktop                # engine-free unless EUCHRE_E2E=1
```

## Modes

The window always talks to a `net::Backend` through the bridge thread (`net/bridge.rs`). No
network or server call ever runs on the UI thread. `BackendKind::choose` (`src/net/mod.rs`)
picks the backend once, at start.

| Mode | How to pick it | What plays |
| --- | --- | --- |
| Embedded (default) | nothing set | `EmbeddedBackend` starts `euchre_server::start(ServerConfig::try_local(<data>/server))` on loopback. It plays through `RemoteBackend` (HTTP + rivetkit-client) against that server. |
| Remote | `EUCHRE_SERVER_URL=https://…` | `RemoteBackend` against that server. No engine starts here. |
| Sim | `EUCHRE_DESKTOP_BACKEND=sim` | the in-process `SimBackend`, with no server and no network. `--scene` and headless runs always use it. |

`EUCHRE_DESKTOP_BACKEND` takes `sim`, `embedded` or `remote` and wins over
`EUCHRE_SERVER_URL`. Any other value is logged and ignored.

### Data dir

- `EUCHRE_DATA_DIR` overrides it. Otherwise it is the platform one: on macOS that is
  `~/Library/Application Support/sh.euchre.Euchre`.
- The data dir holds:
  - `prefs.json`;
  - `device.json`, the enrolled guest device;
  - `recent-tables.json`;
  - `logs/euchre.log` (rotated past 5 MiB);
  - `server/`, the embedded server's engine data, keys and port record.
- `EUCHRE_PREFS_DIR` moves `prefs.json` on its own. When it is unset, the preferences follow
  `EUCHRE_DATA_DIR`.
- Point both at a temp dir for any test or scripted run. Never run against a player's real data
  dir.

The engine port is recorded under `server/` and reused on the next launch; the HTTP port is
new on each launch. The engine needs 4 GiB of free disk. With less, the app shows "Euchre
needs more disk space." and does not hang.

### Engine binary (embedded mode)

The binary is resolved in this order (`net::embedded::engine_source`):

1. `RIVET_ENGINE_BINARY_PATH`.
2. A bundled engine: `<exe dir>/rivet-engine`, else the macOS bundle's
   `<exe dir>/../Resources/rivet-engine` (`.exe` on Windows).
3. rivetkit's cache (`~/.rivetkit/engine/v3/3.1.0/…`) or a dev-tree build, found with
   downloads off.
4. Otherwise it is downloaded (checksum-verified) into rivetkit's cache. If the download fails,
   the player sees "One part of Euchre is missing." with Try again.

### Proxies

The rivetkit and engine HTTP clients read the system proxy. `main` makes sure `NO_PROXY`
covers `127.0.0.1,localhost,::1`, re-executing itself once with the merged list
(`startup::reexec_with_no_proxy`). Set it yourself as well for anything else that talks to
loopback, such as `curl` against `/health` or the tests:

```bash
export NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1
```

### Quit

- A window close or SIGTERM stops the embedded server through the bridge before the process
  ends. That takes about 3 s, and the engine exits with it.
- `kill -9` or a crash of the app leaves the engine running. The next launch on the same data
  dir reclaims it (`reclaim_orphan` in `crates/euchre-server-core/src/rivet.rs`).

## Live end to end

Two engine tests run only with `EUCHRE_E2E=1`. Each needs a real engine, 4 GiB free disk and
its own port block: the engine binds `P`, `P+1` and `P+5`.

```bash
EUCHRE_E2E=1 EUCHRE_TEST_ENGINE_PORT=6870 EUCHRE_TEST_TMP=/private/tmp/euchre-e2e \
NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1 \
  cargo test -p euchre-desktop --test live -- --nocapture
```

`tests/live.rs` plays one full match to game over through the real UI: the bridge, the
`TableStore`, `RemoteBackend` and the embedded server. On the way it checks:

- a made-up move refused by the server;
- the review gate;
- a dropped socket mid-hand;
- an engine stop with Restart;
- Your games and the match detail;
- Quit.

The port defaults to 6870. The run's root is `$EUCHRE_TEST_TMP/integration-<pid>` and is kept.
It holds:

- `data/`;
- `fixtures/*.json`, the real syncs as `scenes::LiveFixture`;
- `snaps/live-*.png`, rendered at 1280x800, scale 1;
- `timings.json`: wall time, per-move ack p50/p95/max, reconnect, Restart and Quit times.

`tests/embedded_live.rs` covers `EmbeddedBackend` alone: boot, stop, Restart and quit. Its port
defaults to 6860 and its root is `$EUCHRE_TEST_TMP/n2-<pid>`.

To render a saved live fixture again:

```bash
EUCHRE_LIVE_FIXTURE=<root>/fixtures/play.json EUCHRE_DESKTOP_BACKEND=sim \
EUCHRE_DATA_DIR=<tmp> EUCHRE_PREFS_DIR=<tmp> \
  target/debug/euchre --scene live-table --snap play.png --size 1280x800 --scale 1 --at 2.5
```

The live scenes are `live-table`, `live-games`, `live-match` and `live-stopped`.
