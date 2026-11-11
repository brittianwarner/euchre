# Desktop net wave, lane N2: the embedded server and the app

The desktop now plays against the real Rust server. It picks its backend like this:

- With `EUCHRE_SERVER_URL` set, it plays remote (N1's `RemoteBackend`).
- Otherwise it starts `euchre_server::start(ServerConfig::try_local(<data>/server))` in its own
  process, on loopback, with a spawned engine. It shows "A seat at the table." while that boots.
- The sim stays for tests, scenes and `EUCHRE_DESKTOP_BACKEND=sim`.

This is built to `desktop-net-contract.md` §N2.

**Gates, all green:**

- `cargo fmt -p euchre-desktop --check`.
- `cargo clippy -p euchre-desktop --all-targets -- -D warnings`.
- `cargo test -p euchre-desktop`: 235 tests, engine-free without `EUCHRE_E2E`, and the motion
  budgets in `tests/motion.rs` pass.

**The gated live test** (`tests/embedded_live.rs` /
`embedded_server_boots_restarts_and_stops`, `EUCHRE_E2E=1`, port block 6860) passed 6 times:

- 2 while building;
- the 3 final runs in a row;
- 1 true first launch with an empty engine cache.

Before and after every run, nothing listened on 6860, 6861 or 6865, and 113 GiB was free.

## Files

| File | What it holds |
|---|---|
| `apps/desktop/src/net/embedded.rs` | `EmbeddedConfig`, `Supervisor` (boot, restart, shutdown, the phase machine with its allowed edges), `EmbeddedBackend: Backend` (waits for Ready, then delegates to the running server's `RemoteBackend`), `BootError → BootProblem`, `EngineSource` and the first-launch download, 12 unit tests |
| `apps/desktop/src/net/mod.rs` | `BackendKind` (`parse → Option`, `choose`, `for_run`, `build`), `data_dir()`, `ServerPhase`, `BootProblem`; `RecentTable(s)` serde camelCase |
| `apps/desktop/src/net/bridge.rs` | `start_host` (forwards phases, boots), `RestartServer`, `Shutdown` (closes tables, stops the server), SIGINT/SIGTERM → `Reply::Quit` |
| `apps/desktop/src/startup.rs` | `NO_PROXY` re-exec, logging to `<data>/logs/euchre.log` |
| `apps/desktop/src/lib.rs`, `src/main.rs` | start order, `on_quit`, the bounded quit wait (`QUIT_WAIT_HOST` 45 s with a server, 500 ms without) |
| `apps/desktop/src/prefs.rs` | `EUCHRE_PREFS_DIR` (`PrefsStore::dir_override`) |
| `apps/desktop/src/app.rs` | `on_server_phase` and the `server_lost` recovery, `on_quit`, the notice placement |
| `apps/desktop/src/screens/server.rs` (new) | all server copy (`problem_copy`, `notice_copy`, `booting_line`, `ended`), the notice, the ended-table banner, 4 unit tests |
| `apps/desktop/src/screens/table/mod.rs` | the boot screen and the ended table use `screens::server` |
| `apps/desktop/src/screens/other.rs` | "Your saved tables" on Your games |
| `apps/desktop/src/scenes.rs` | 13 new scenes (`SERVER_SCENES`) and `LiveFixture` (`EUCHRE_LIVE_FIXTURE`) |
| `apps/desktop/tests/server_states.rs` (new) | 10 headless tests of what the player sees in each phase |
| `apps/desktop/tests/shell.rs` | `server_scenes_render` (one key per new scene) |
| `apps/desktop/tests/embedded_live.rs` (new) | the gated live test and the gated offline first-launch test |

## Timings (Apple silicon, debug build, `fast_tempo`, offline AI)

| Measure | Final 3 runs | Notes |
|---|---|---|
| Cold boot: Starting → Ready, fresh data dir, engine already cached | 5389 / 4827 / 5436 ms | About 3.2 s to spawn the engine, 0.1 s for the namespace, about 2.1 s until the runner serves |
| First launch, nothing cached: download + boot | 8197 ms (one run) | 58.7 MB engine, 2.5 s download (checksum-verified), then 5.6 s boot |
| Warm boot: same data dir, same process, port freed | 4574 / 4554 / 4576 ms | Measured as "Try again" after `PortInUse` |
| Engine killed → Stopped notice | 3236 / 3916 / 3540 ms | The server polls the engine every 1 s; the engine takes about 3 s to exit on SIGTERM. The table socket drops at once and shows "Reconnecting" meanwhile |
| Restart pressed at once → Ready | 9568 / 10581 / 9605 ms | 5.0 s is the server waiting out its registry grace against the dead engine, then about 4.5 s of boot. Request filed (below). A player who reads the notice first waits only the 4.5 s |
| Quit: `Shutdown` → bridge ended | 3129 / 3168 / 3125 ms | Clean stop: HTTP drain, registry, then the engine. The window is already gone, so the process finishes this in the background |
| Bridge ended → engine pid gone and every port closed (6860, 6861, 6865, HTTP) | +7 / +2 / +5 ms | The engine never outlives the app |
| Port held by another program → `Failed(PortInUse)` | 105 / 95 / 107 ms | |

After a restart the table comes back where it was. The live test checks this: it reopens on a new
connection and gets the same `hand_no`, `turn_id` and score.

The server keeps its HTTP port across a restart (the URL is the same). The device token stays
valid, and N1's client needs no rebuild.

## First launch offline

`first_launch_offline_reads_engine_missing` checks this. It is gated with
`EUCHRE_E2E_OFFLINE=1` and run with `HOME` on an empty dir and the proxies on a closed loopback
port. It passed.

1. **Starting**, then **Downloading**:
   - The boot screen says "Getting the table ready. The first start takes a minute."
   - The home page stays usable.
2. **Download fails**, after 291 ms (when it fails fast, as here):
   - The phase goes to `Failed(EngineMissing)`.
   - The notice says **"One part of Euchre is missing."** / "The first start downloads it.
     Connect to the internet, then try again." It offers **Try again**.
3. **No engine starts**, and nothing is written to the engine cache: no partial file.
4. **Quitting** from there takes 5 ms.

On a network that hangs rather than refuses, the download waits up to 60 s per request
(rivetkit's limit) before the same notice.

## What the player sees

| State | Where | Copy |
|---|---|---|
| Starting | the table's boot screen ("A seat at the table.") | "Getting the table ready…" and a "Your saved games" link. The home page and Your games stay usable |
| Downloading | same | "Getting the table ready. The first start takes a minute." |
| Stopped | a notice on every page except the boot screen | **"The game server stopped."** / "Restart to keep playing. Your games are saved." [Restart]. On the table it is one line: "The game server stopped. Restart to keep playing." |
| Failed | the same notice | One heading, line and action per `BootProblem` (`screens::server::problem_copy`). `Relaunch` offers [Reopen Euchre] (`Tx::relaunch`); every other problem offers [Try again] |
| Ready after a loss | — | The app re-identifies, reloads saved tables and the current page, and reopens the open table |

**Where the notice sits:**

- **Home:** it sits in the header band, between the wordmark and the links, when it fits.
  Otherwise it sits under the header.
- **Dark pages:** it sits beside the pill nav.
- **Minimum window (720 × 520):** it covers the page heading. It is a raised overlay, and while
  the server is down it is the thing to read.

**When the table connection ends** (`TableStage::Failed(NetError)`), the banner follows the
contract (`screens::server::ended`):

| Error | Banner |
|---|---|
| `ServerStopped` | Restart. While the server is already restarting, the booting line and no button |
| `Forbidden` / `NotFound` | Your saved games |
| `Unauthorized` | New game |
| Unavailable, offline, timeout, rate limited or closed | "We couldn’t reach this table. Your game is saved." [Reconnect] |

No internal error text reaches the player. Details go to the log at warn, never tokens or bodies.

## Process start, logs, data

- **`NO_PROXY`:** the first line of `main` re-execs the process (same pid, same args) with
  `127.0.0.1,localhost,::1` merged into `NO_PROXY` and `no_proxy`. It skips this when they are
  already there, for scene or headless runs, and when `EUCHRE_REEXEC` is set. If `exec` fails,
  it logs a warning and runs on.
- **Logs:**
  - A window run appends to `<data>/logs/euchre.log` (0600, dir 0700). The file rotates to
    `euchre.log.1` past 5 MiB.
  - The filter is `RUST_LOG`, else `warn,euchre_desktop=info`.
  - A scene or headless run logs to stderr and creates nothing.
- **Data dir:**
  - `EUCHRE_DATA_DIR`, else `~/Library/Application Support/sh.euchre.Euchre` on macOS. It holds
    `prefs.json`, `device.json` (0600), `recent-tables.json`, `logs/` and the server's `server/`.
  - `EUCHRE_PREFS_DIR` overrides the prefs location alone.
- **Engine source, in order:** `RIVET_ENGINE_BINARY_PATH`, then bundled
  (`<exe dir>/rivet-engine`, or `../Resources/rivet-engine` in a macOS bundle), then rivetkit's
  cache, then download.
- **Packaging:** this wave bundles nothing. Shipping the engine in `Resources/` makes the first
  launch work offline.

## Screenshots

These are headless scene renders, sim-backed and engine-free. The `live-*` scenes use real server
data that the live test wrote (`EUCHRE_LIVE_FIXTURE_OUT`): a real dealt hand at the player's bid,
and the real saved table. They are in
`/private/tmp/claude-501/-Users-brittianwarner-goods-layerr/25be462f-e35c-4ef6-8730-eade499742b1/scratchpad/net/n2/shots/`.

**At 1280 × 800:**

- `server-starting.png`
- `server-downloading.png`
- `server-stopped.png`
- `server-stopped-table.png`
- `server-stopped-games.png`
- `server-failed.png`
- `server-failed-disk.png`
- `server-failed-port.png`
- `server-relaunch.png`
- `games-saved-tables.png`
- `live-table.png`
- `live-games.png`
- `live-match.png`

**At the 720 × 520 minimum:**

- `server-stopped-min.png`
- `server-stopped-table-min.png`
- `server-stopped-games-min.png`
- `server-failed-disk-min.png`
- `games-saved-tables-min.png`

To render one:

```
EUCHRE_DATA_DIR=<tmp> EUCHRE_PREFS_DIR=<tmp> euchre --scene <name> --snap <png> --size 1280x800 --scale 1
```

`live-match` has no finished match in the fixture, so it falls back to the sim replay.

## Requests to the server

`docs/port/requests/desktop-net-dead-engine-grace.md` asks for a registry grace of about 500 ms
once the engine is gone, instead of 5 s. That would bring "Restart pressed at once" from about
9.6 s to about 5 s. It is not blocking.

## Known limits

- **A crash or `kill -9` of the app** leaves the engine running until the next launch on the same
  data dir. The server then reclaims it (`reclaim_orphan` in
  `crates/euchre-server-core/src/rivet.rs`). This wave's tests do not exercise that.
- **Windows:** if another app refuses a logoff, rune has already run `on_quit`. The bridge has
  then stopped the server and ended, and the app keeps running without a server. macOS is this
  wave's target.
- **Engine death takes about 3.5 s to show as Stopped.** Meanwhile the table reads "Reconnecting".
  It is bounded by the server's 1 s engine poll and the engine's own exit time.
