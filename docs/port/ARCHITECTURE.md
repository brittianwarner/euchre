# Euchre: Rust + Rune port, build contract

This is the contract every implementation agent works to. The numbered specs in this directory (`01`–`08`) hold the detail. This file fixes the seams, the ownership, the decisions and the gates. If a spec and this file disagree, this file wins. If the TS code and a spec disagree, the code wins (`~/goods/euchre`, read-only).

## 0. Goal

The whole euchre game runs natively:

- **Backend (Rust):** the `euchreTable`, `aiSeat` and `playerProfile` actors run on the rivetkit Rust SDK **3.1.0**. Auth, the HTTP API, the public gateway and AI decisions are also in Rust. Actor names, keys, action names, event names and JSON shapes stay byte-compatible with the TS app, so the existing SvelteKit web client keeps working against the Rust backend.
- **Desktop (Rust + Rune):** a native app (`apps/desktop`, binary `euchre`) connects with `rivetkit-client` **3.1.0**. It can run against a remote server or start the server **embedded** in-process, with a local Rivet engine, for offline play.
- **Web (`apps/web`):** stays SvelteKit. It moves to rivetkit JS **3.1.0**, drops its TS actors, and talks to the Rust API and gateway.
- **Rivet:** 3.1.0 everywhere. That covers the Rust crates, rivetkit JS, `@rivetkit/svelte`, and the engine image in `.railway/railway.ts`.

The bar is "make it great". The desktop app must feel better than the web app: smooth card motion, a calm and legible table, instant local play and no jank. It must keep every rule, word of copy and behaviour the specs pin down.

## 1. Repo layout and ownership

```
Cargo.toml                 workspace (resolver 3). ONLY the orchestrator edits it.
rust-toolchain.toml        1.99.1 (Rune needs it)
turbo.json, package.json   Turborepo 2.11.10 + bun 1.4.4 workspaces
crates/euchre-engine       pure rules engine (01)         — Lane ENGINE
crates/euchre-protocol     wire types + pure profile logic (02, 05 §3.5, §10.3) — Lane PROTOCOL
crates/euchre-ai           heuristic, rollouts, Jev, banter, aiSeat logic (04) — Lane AI
crates/euchre-server-core  config, secrets, auth tokens, rivet helpers, LocalEngine + TestEngine harness (05 §2, 08) — Lane SERVER-FOUNDATION (done)
crates/euchre-profile-model pure profile logic: settings, prefs, personas, stats, records, replay redaction (05 §3.5, §3.10) — Lane SERVER-PROFILE
crates/euchre-profile      playerProfile actor (05 §3)  — Lane SERVER-PROFILE
crates/euchre-table        euchreTable actor (03)       — Lane SERVER-TABLE
crates/euchre-ai-seat      aiSeat actor glue (04 §14)   — Lane SERVER-AI-SEAT
crates/euchre-http         axum API (05 §11), /rivet/** gateway (05 §12), namespace bootstrap (05 §5.4) — Lane SERVER-HTTP
apps/server                euchre-server: composition only (registry, start()/run(), main) — integration wave
apps/desktop               Rune client (06, 07)    — Lane DESKTOP
apps/web                   SvelteKit web client (retarget wave)
packages/rivetkit-svelte   Svelte bindings (bump to rivetkit 3.1.0 in retarget wave)
docs/port/                 specs + this contract
```

**Ownership rule.** An agent edits only the paths its lane owns, plus new files inside them. It may read anything. If it needs a change in another lane's crate, it writes the request to `docs/port/requests/<lane>-<topic>.md` and works around it locally, for example with a private helper. It does not edit the other lane. The orchestrator merges requests between waves.

**Cargo rules.**
- Never edit the root `Cargo.toml`. Every needed dependency is already in `[workspace.dependencies]`. A lane may add a crate-local dependency to its own `Cargo.toml` with an exact version when nothing in the workspace list fits.
- Each agent builds with its own target dir: `export CARGO_TARGET_DIR=$HOME/goods/euchre-rune/.cargo-targets/<lane>`. It is pre-seeded as an APFS clone of `target/`. This avoids build-lock contention between parallel agents.
- Gates per crate: `cargo fmt -p <crate> -- --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo test -p <crate>`. Every one must pass before a lane reports done.

**Process rules.** No git commits, no `git add`, no `git stash`, no pushes. Use bun only, never npm or pnpm. Never stop a process you did not start, including other projects' `rivet-engine` processes on ports 6420 and similar.

## 2. Crate contracts

### 2.1 `euchre-engine` (spec 01)

- Pure and deterministic. No time, IO, env or randomness beyond `(seed, handNo)`. No async. Only `serde` and `serde_json` (dev) as dependencies; `serde_repr` is fine if wanted.
- Module layout and public surface exactly as 01 §1 and §14, snake_case. `project`, `project_steps`, `apply_optimistic` and `assert_invariants` live here.
- `GameState` **must not** be serializable to any client wire. It derives `Serialize/Deserialize` only for actor persistence. Persistence uses the custom `hands` object form (01 §2), so state blobs stay inspectable. The type carries a doc comment saying it is server-only.
- Golden vectors (01 §15) and every listed TS test (01 §16) are ported as `#[test]`s with snake_case names. That includes the four full-match replays with exact `moveHash` and final `turnId`.
- `examples/e2e.rs` ports `scripts/e2e-play.ts`.

### 2.2 `euchre-protocol` (spec 02, 04 §2, 05 §3.5/§3.10/§10.3)

- Depends on `euchre-engine` and re-exports the engine wire vocabulary (`Card`, `Seat`, `Move`, `Step`, `LegalMove`, `PublicGameView`, …), so clients depend on `euchre-protocol` alone.
- Owns: `PROTOCOL_VERSION`, connection params, `TableConnState`, `TableCreateInput`, event payloads (`SyncEvent`, `ThinkingEvent`, `ChatEvent`, `ChatDeltaEvent`, `PresenceEvent`), action request/response types (`SubmitMoveRequest`, `MoveAck`, `ReviewTrickRequest`, `CompletedHandReview`), the hand-review builder (02 §7), pacing constants (02 §9.1), `ProtocolErrorCode`, the AI wire types (`AIDecideRequest`, `AIDecision`, `AiSayMsg`, lifecycle messages), personas (`PersonaConfig`, `PersonaAssignment`, `PersonaView`, defaults and presets), and profile settings (`ProfileSettings`, `TablePrefs`, patch, sanitize, normalize). It also owns `MatchRecord`, `HandJournalEntry`, `StoredEpisode`, `ProfileStats`, `MatchListPage` and `ClientMatchReplay`, plus the replay redaction. Each type has exactly one definition, and every lane imports it from here.
- Also owns the client store state machine as pure logic: `TableStore` (02 §6). It covers `apply_sync`, optimistic preview keyed by `forV`, the submit guard, chat dedupe and the reconnect rules, so the desktop is a thin UI over a tested store. Only the TS client (which keeps its own copy) does not use it.
- `serde_json` is built with `preserve_order` workspace-wide, so object key order matches JS insertion order.
- JSON golden tests use fixtures captured from the TS app (null vs absent, tags, key order).

### 2.3 `euchre-ai` (spec 04)

- Depends on engine and protocol. Pure logic plus a Jev HTTP client behind `trait JevClient` (reqwest impl + scripted mock) and a banter text client behind `trait TalkClient` (OpenRouter SSE, Anthropic fallback). No rivetkit dependency.
- `ai_seat` module: state, memory, difficulty narrowing, the decision ladder (`run_ladder`, with deadlines as `tokio::time::Instant` or an injected clock), and usage/budget metering. These are the pure handlers that the server's `aiSeat` actor calls.
- Rollouts run on `tokio::task::spawn_blocking` inside the ladder budget.
- Every listed test (04 §16) is ported, with fixtures in `crates/euchre-ai/tests/fixtures/*.json`.
- With no `OPENROUTER_API_KEY`, decisions fall back to the heuristic and banter is silent. The game must be fully playable offline.

### 2.4 `euchre-server` (specs 03, 04 §14, 05)

Library API used by `main.rs`, by the desktop's embedded mode and by integration tests:

```rust
pub struct ServerConfig { /* from env or explicit; see config.rs */ }
impl ServerConfig {
    pub fn from_env() -> anyhow::Result<Self>;
    pub fn local(data_dir: PathBuf) -> Self;   // embedded/dev: spawn engine, loopback, generated secrets persisted in data_dir
}
pub struct RunningServer { pub http_addr: SocketAddr, pub engine_endpoint: String, pub namespace: String, /* … */ }
pub async fn start(cfg: ServerConfig, shutdown: CancellationToken) -> anyhow::Result<RunningServer>;
pub async fn run(cfg: ServerConfig) -> anyhow::Result<()>;   // start + wait for SIGTERM/ctrl-c, graceful drain
```

- **Actors.** `euchreTable` (key `["table", gameId]`), `aiSeat` (key `["table", gameId, "seat", "1"|"2"|"3"]`) and `playerProfile` (key `["user", userId]`) keep the TS action and event names (03 §16.4, 05 §0.6). TS queues become actions with the same names. Single-writer semantics come from a `tokio::sync::Mutex` lane per actor. Persist before emit, and emit before ack (03 §13). Timers use `ctx.schedule()`, with a due-time ledger in state, and reconcile on wake (03 §16.5). Schedule-target actions reject calls that arrive on a client connection.
- **Storage.** Use actor SQLite (`ctx.sql()`) for the table (state row + `hand_review`) and the profile (05 §3.6 schema verbatim). `aiSeat` state is one SQLite row. Never use `ctx.kv()` (deprecated, 128 KiB cap). Persisted JSON uses the protocol types. No `run` handler: reconcile in `on_start` (08 G35).
- **Auth.** Two JWT families plus the internal token, exactly as 05 §2 and §10.2, in separate modules with cross-language fixtures.
- **HTTP.** The axum API in 05 §11.2 covers both web and desktop. Browser callers use cookies, the desktop uses bearer tokens.
- **Gateway.** `/rivet/**` is an allowlist proxy to the private engine (05 §12.2). It is used in production, where the engine is private. In local or embedded mode clients may connect to the engine endpoint directly.

### 2.5 `euchre-desktop` (specs 06, 07)

- **Threading.** Rune owns the main thread (`rune::App`). A tokio runtime on a background thread owns all networking: HTTP API, rivetkit-client connections, and optionally the embedded server. Network results reach the UI through `AppHandle::update(|state, tx| …)`. UI intents reach the network through an `mpsc` channel. The UI thread never blocks.
- **State.** `AppState { route, identity, server, table: Option<TableScreen>, games, settings, toasts, prefs }`. The table screen wraps `euchre_protocol::TableStore` plus presentation state: animations, selection, hint, banter bubbles, announcer queue.
- **Modes.** If `EUCHRE_SERVER_URL` is set, or a server is configured in the app's settings, connect remotely. Otherwise start `euchre_server::start(ServerConfig::local(app_data_dir))` on loopback with a spawned engine and connect to it. The UI shows the brief "A seat at the table" state while the engine boots.
- **Identity.** The device enrolls once with `POST /api/device/enroll`. The device record is stored in `directories::ProjectDirs::from("sh","euchre","Euchre").data_dir()/device.json`, mode 0600, written atomically (05 §11.4).
- **Screens.** Home, New game, Table (with settings rail, rules dialog, tour, hand review), Games, Match detail, Settings (06 §2–§18). Copy, timings and accessibility labels follow 06 verbatim.
- **Assets.** Card art is embedded from `apps/web/static/art/cards/*.png` via `include_bytes!`, with the LICENSE text shown in an About/credits section. Fonts come from Rune's bundled set or are embedded (see 07).
- **Tests.** `rune::Runtime` headless tests drive the table: click card, play, bid, continue. Named `--scene` snapshots (`home`, `new`, `table-deal`, `table-bid`, `table-play`, `table-trick-won`, `table-hand-scored`, `games`, `settings`, `game-over`) render with deterministic fixture data from `euchre-engine` replays. No network is needed for scenes.

## 3. Decisions (resolving the specs' open questions)

| # | Question | Decision |
|---|---|---|
| D1 | Native clients send no `Origin` (02 §0.1, 03 Q4) | An Origin header, when present, must be allowlisted. When absent, the token is the only gate. Browsers always send Origin, so cross-site WebSocket hijacking stays blocked. |
| D2 | Email-nickname cookie is unsigned (05 Q2) | Fix it. Issue a signed `identityToken` (05 §11.4) with secret `EUCHRE_IDENTITY_SECRET` and keep `userId = sha256hex(email)`. The legacy plain cookie is accepted only when `EUCHRE_ALLOW_UNSIGNED_EMAIL=1` (default off). |
| D3 | Gateway token (05 Q3) | SERVER-HTTP probes Option A (engine `POST /auth/tokens`) against the 3.1.0 engine through `LocalEngine`. Use Option A if it works, otherwise Option B. Record the probe evidence and the choice in `docs/port/requests/done/gateway-token.md`. Desktop and web clients use `json` encoding (D19), not the `Cbor` named in 05 §11.4. |
| D4 | Web hosting after retarget (05 Q4) | SvelteKit `adapter-static` SPA (fallback `index.html`), served by the Rust server with `ServeDir`. Page server loads become client `fetch`es to `/api/*`. `GET /play` stays a client route that calls `POST /api/tables` and then navigates. |
| D5 | Desktop identity (05 Q6) | Bearer session token in app-data (0600). No keychain in v1. |
| D6 | `snapshot` for non-players (03) | Player role required. |
| D7 | Legacy `onNudge`/`onAbandon` (03 Q7) | Keep both as no-op schedule targets for wire parity, rejecting conn callers. |
| D8 | Table needs a heuristic top move for the parked move (03 Q8) | `euchreTable` depends on `euchre-ai` (heuristic only). |
| D9 | `submitMove` without `turnId` (03 Q9) | Accept it, as TS does, and treat it as stale-checked by `moveId` legality. |
| D10 | Migrating existing TS tables (03 Q10) | Not migrated. Rust actors start fresh. The TS deployment's tables end when it is replaced. |
| D11 | Rate limits (05 Q9) | Per-IP token bucket (in-process) on `device/enroll`, `identity/identify`, `tables` and `table-session`. |
| D12 | `dailyTokens`/`activeGameId` read APIs (05 Q7) | No new read APIs. `setActiveGame` stays an action for parity. |
| D13 | Pure profile logic location (05 Q10) | Superseded by D29: `crates/euchre-profile-model` (settings, sanitize, patch, normalize, personas from settings, stats math, cursor, records, replay redaction). `euchre-protocol::profile` stays an empty doc stub. |
| D14 | Sound | Synthesized cues (06 §15) via `rodio` in the desktop. This matches the web: a local table toggle, off by default, persisted in the desktop prefs file. `TablePrefs.sound` stays a saved profile field and is not consumed. |
| D15 | Reduce motion | `TablePrefs.reduceMotion` OR the OS preference disables flights and tweens (instant layout). |
| D16 | Seat names | Live-table names everywhere in the table UI (`Left opponent`, `Your partner`, `Right opponent`). Persona headings keep `West`, `North · your partner`, `East`. |
| D17 | Per-connection identity (critic C5) | Re-derive the role on every action from `ctx.conn().params()` (verify the token each time; HS256 is cheap). `ctx.conn() == None` means a scheduled fire. Keep only an in-memory presence map, rebuilt on `on_connect`. Never persist conn state and never rely on `conn.inner().set_state`. |
| D18 | WS vs HTTP conns (critic gap 1) | No discriminator is needed. Presence is transition-based: broadcast `online` only when the player's live conn count goes 0→1, and `offline` only on 1→0. The initial `sync` uses `conn.send`, which is silently dropped for stateless HTTP calls (08 G6). The HTTP API checks ownership with the internal token, so it never counts as a player conn. |
| D19 | Wire encoding (critic C9) | `json` everywhere: the Rust client (`EncodingKind::Json`) and the JS client (`encoding: "json"`, 08 G34). |
| D20 | Scheduled-action failure (Q6) | Treat a fire as lost if its handler fails. Handlers never return `Err` for transient problems: they log, re-arm the same timer with backoff, and return `Ok`. `on_start` reconcile re-derives every due timer from the persisted ledger. |
| D21 | Error exposure | Never set `RIVET_EXPOSE_ERRORS`. Every protocol code is a public `user/<code>` error via the helper in `euchre_server_core::rivet`. Clients parse error strings with the shared parser (08 G16). |
| D22 | `snapshot` hardening (D6 detail) | Allowed for a valid player token matching the owner, or a valid internal token. Anything else gets `forbidden`. |
| D23 | `thinking` escalation | Parity: the table emits only the on/off edges with `extended: false`, as TS does. |
| D24 | AI budgets | Keep the actor ladder constants (2200/1400/400/300 ms, 4000 ms hard cap) for parity. The heuristic must therefore be strong on its own; it is the path that plays offline. |
| D25 | Desktop minimum window | 720×520 (verified in 07). The primary design is 1280×820. |
| D26 | Fonts | Done by the orchestrator: `apps/desktop/assets/fonts/` holds static DM Sans 400/500/600/700, Source Serif 4 400/500/600/700 + italic 400 and IBM Plex Mono 400, with OFL files and a README. Register all ten in `AppSpec::fonts` with `families: ("DM Sans", "IBM Plex Mono")`, and select serif with `Family::Named("Source Serif 4")`. |
| D27 | Byte-exact strings | Heuristic `why` strings, banter prompts, chat lines and UI copy are ported byte for byte from the TS source (typographic ’ where TS has it). Golden tests assert them. UTF-16 caps (140/120 and similar) count UTF-16 code units, as JS `.length` does, and never split a surrogate pair. |
| D28 | Desktop backends | The desktop UI talks to one `Backend` trait in `apps/desktop/src/net/`. It covers enroll or identity, create table, open table, list games, match detail and settings. Open table returns a handle that streams protocol `TableEvent`s and accepts submit, review, snapshot and hand-review calls. `RemoteBackend` (HTTP API + rivetkit-client) serves both remote and embedded mode and is wired in W3. `SimBackend` is an in-process dev double: engine plus a pluggable `SimDecider`, deterministic seeds, the same `SyncEvent` sequence and pacing as the table actor (03 §6–§9). Headless tests, `--scene` and `EUCHRE_DESKTOP_BACKEND=sim` use it. Until W3 the desktop depends only on engine, protocol, rune, rune-kit and rivetkit-client, never on `euchre-ai` or the server crates, because those are being edited in parallel. **Amended after W2:** `euchre-ai` (finished, frozen) and `euchre-profile-model` (finished, frozen) are now allowed, for hints (`rank_moves`), the sim's AI decider, and typed settings / match detail (closes `requests/desktop-profile-types.md`). The actor and HTTP crates stay off-limits until W3 wires `RemoteBackend`. |
| D29 | Server crate split (supersedes the `apps/server/src/*` lane map in §1) | Each W2 server lane gets its own crate, so parallel lanes never break each other's builds. Foundation moves to `crates/euchre-server-core` (config, auth, rivet helpers, testing harness). `crates/euchre-profile-model` holds the pure profile logic (replaces D13's `euchre-protocol::profile`). `crates/euchre-profile` is the `playerProfile` actor, `crates/euchre-table` is `euchreTable`, `crates/euchre-ai-seat` is the `aiSeat` glue, and `crates/euchre-http` holds the axum API, gateway and namespace bootstrap. `apps/server` (`euchre-server`) is composition only: `start`/`run`, the registry and `main`. Actors talk to each other by actor name over the client, never through Rust types from another actor crate. |

| D30 | `forgetEpisode` cannot delete anything (TS bug found by SERVER-PROFILE) | Fix it. Stored ids are `role.matchId.handNo.kind`, so the id pattern must allow `.`. Deliberate deviation from TS, with a test (W3). |
| D31 | Motion + visual spec sign-offs (09 §6.8) | Owner asked for a bolder look ("make it look better … badass transitions and gameplay animations"), so the orchestrator approves all four: S-1 rack lifts −10 hover / −18 selected; S-2 clean Ace of spades art (CC0, so editing is allowed): a desktop-owned `AS.png` without the QR/URL; S-3 `PICKUP` 0.55 s and `SWEEP` 0.45 s spring, inside the tempo holds; S-4 confetti on a March too. Rune pin stays 85cd8c6 this wave (09 §1). |
| D32 | Remaining desktop features | The polish wave folds in what the first desktop wave left: tour, banter bubbles, announcer, hints (`euchre_ai::rank_moves`), synthesized sound (D14, `rodio`), rules search/accordion, and real Games / Match detail / Settings screens on `euchre-profile-model` types. |
| D33 | Table review amendments to D17, D20, D22 (requests/done/server-table-d17-amendment.md) | **D17:** a live WebSocket keeps the identity it proved at connect (token signature, expiry, protocol version and match token checked once in `on_before_connect`/`on_connect`, held in memory per conn); later actions on that socket re-check only the role against the owner. Stateless HTTP calls are still verified in full on every call. This matches TS `createConnState` and stops a game that outlives the 2 h guest token from failing every move. **D20:** a session that fails after its first save rolls back the working copy and spawns one keep-awake `reconcile` (up to `FIRE_RETRY_LIMIT` tries) that re-derives the open turn's timers from saved state; a session that failed before any save keeps the same-arguments retry; a move whose save landed is acknowledged. **D22:** an internal-token `snapshot` returns the seat-0 view with `hand` and `legal` cleared, because only the opposing aiSeats hold that token. |
| D34 | Profile write bodies (requests/done/server-table-profile-bodies.md, ai-profile-bodies.md) | Not moved into `euchre-protocol` (frozen). Each writer keeps its crate-private body type, and tests in the writer's crate parse every body with the `euchre-profile-model` parsers (`parse_record_game`, `parse_record_hands`, episodes, tokens). The seams e2e proves all four writes against the real `playerProfile`. Those parsers are the contract. |
| D35 | Web = the Rune app on WebGPU at euchre.sh; Svelte SPA is the no-WebGPU fallback (owner decision; build plan `10-web-rune-plan.md`) | **App:** `apps/desktop` itself builds for `wasm32-unknown-unknown` (cdylib, `src/web.rs` start entry, `net/rt.rs` runtime seam, native-only dependencies and `euchre-ai` LLM/seat modules gated off wasm); no UI fork. **Network:** `net/web/WebBackend` over `fetch` with the browser's cookies, and the official rivetkit 3.1.0 JS client through a bun-bundled bridge (`apps/desktop/web/rivet-bridge.ts`); the Rust `rivetkit-client` does not build for wasm and a hand-written actor protocol is rejected. `Backend` gains `identify`/`sign_out`; web reads `profile=email` when an email identity exists. **Serving:** same `euchre-server`, one URL space, two shells: `index.html` is the Rune loader, the unchanged Svelte build is `classic.html`, picked in `static_files.rs` by the `euchre_ui=classic*` cookie (`Vary: Cookie`). The loader sets it when there is no WebGPU, no adapter, a fallback (software) adapter, a viewport under 720×520 (phones, until a compact table lands), no first frame in 10 s, or a lost device. No Svelte base-path move; deep links are identical in both shells. **Assets:** Rune files under `/_rune/immutable/<hash>/`, art by absolute URL under `/_rune/immutable/art-<hash>/`, no `wasm-opt`. **Deploy:** new Docker `rune-web` and `site` stages, no Railway config change; rollback = redeploy the previous deployment. |
| D36 | Minimal homepage + desktop downloads (owner decisions 2026-11-11, after D35) | **Home:** `/` is always the minimal homepage for every browser: the euchre wordmark, the fanned-cards picture, Play now and Download, nothing else (Svelte route `apps/web/src/routes/+page.svelte`). It amends D35/W-4: the Rune loader is no longer `index.html`; it is served only for app routes (`/new`, `/play/<id>`, `/games`, `/games/<id>`, `/settings`) when no `euchre_ui=classic*` cookie is set, and `/` never runs the WebGPU gates on load. Play now goes through one function (`startPlaying`) that later routes WebGPU browsers into the Rune app and everyone else into the Svelte flow. On the web the Rune app does not show its own marketing Home; its home links go to `/`. **Downloads:** Mac (universal .dmg), Windows and Linux builds of `apps/desktop`, built on the owner's Mac (Windows/Linux via Docker or cross-compiling), served by `euchre-http` under `/download/*` from `EUCHRE_DOWNLOADS_DIR` with `manifest.json` (`docs/port/downloads/*.md`). Unsigned for now; the homepage shows the one-time open steps. Nothing is committed or pushed to publish them. **Desktop default:** release downloads play online against `https://euchre.sh` (same server, accounts and history as the website); the embedded server stays the default for dev builds and is selectable by env. **Approvals for the D35 plan:** the `euchre-ai` cfg gates (wasm) and the `rust-toolchain.toml` wasm target are approved. |

## 4. Rivet Rust SDK facts every server agent relies on

See `08-rivet-rust-cookbook.md` for verified, runnable recipes. Summary:

- Actions run concurrently (`tokio::spawn` per call), so serialize with the lane mutex.
- No queues, workflows or managed state. Persist with `ctx.sql()`; reload in `create`.
- `ctx.schedule().after(dur, "actionName", &cbor_args)` is at-most-once and fires with no conn.
- Per-conn identity: authenticate in `on_before_connect`; re-derive the role per action from `ctx.conn().params()` (D17). Fan out per-conn `sync` with `ctx.conns()` and `conn.send`, filtering each conn by its own params.
- User-facing errors carry `{group:"user", code, message}`. Use the helper in `euchre_server_core::rivet`.
- Local engine: `EngineSpawnMode::Always`, `engine_auto_download`, a unique `engine_port`, a unique `RIVETKIT_STORAGE_PATH`, and `RIVET_ENGINE_BINARY_PATH=~/.rivetkit/engine/v3/3.1.0/rivet-engine-aarch64-apple-darwin`.

## 5. Ports and storage for agents that spawn engines

| Lane | Engine port | Storage |
|---|---|---|
| SERVER-FOUNDATION | 6701 | `$SCRATCH/rivet-store-foundation` |
| SERVER-AI-SEAT | 6771 | `$SCRATCH/rivet-store-ai-seat` |
| SERVER-TABLE | 6711 | `$SCRATCH/rivet-store-table` |
| SERVER-PROFILE | 6721 | `$SCRATCH/rivet-store-profile` |
| SERVER-HTTP | 6731 | `$SCRATCH/rivet-store-http` |
| DESKTOP | 6741 | `$SCRATCH/rivet-store-desktop` |
| INTEGRATION | 6751 | `$SCRATCH/rivet-store-integration` |
| WEB | 6761 | `$SCRATCH/rivet-store-web` |
| Reviewers (W2) | 6781, 6791, 6801, 6811, 6821, 6831 | `$SCRATCH/rivet-store-review-*` or `<lane>-review-<lens>` |
| SERVER-SEAMS | 6841 | `$SCRATCH/rivet-store-seams` |
| Overflow (when a block is busy) | 6850–6990, step 10 | per agent |

Engine integration tests read `EUCHRE_TEST_ENGINE_PORT` and `EUCHRE_TEST_STORAGE` (defaults: 6751 and a temp dir). They are plain `#[tokio::test]`s that print a `SKIP` line and return unless `EUCHRE_E2E=1` (`euchre_server_core::testing::e2e_enabled()` / `e2e_skip()`), so a plain `cargo test` stays engine-free and `EUCHRE_E2E=1 cargo test` runs them without `--ignored`. All `TestEngine`s in one test binary share one engine (refcounted), each test on its own pool, so tests may run in parallel.

Engines bind P, P+1 (peer) and P+5 (metrics), so each lane stays inside its own P..P+9 block. Start engines only through `euchre_server_core::testing::TestEngine` or `euchre_server_core::rivet::LocalEngine`, never through rivetkit's `EngineSpawnMode::Always` (it uses P+10, shared default storage and the well-known admin token).

Always wrap engine-touching commands in a hard timeout (`perl -e 'alarm N; exec @ARGV' …`). macOS has no `timeout`. Kill only the engine processes you started.

## 6. Waves

1. **W1 foundation.** ENGINE-CORE, then ENGINE-REDUCER and PROTOCOL in parallel. SERVER-FOUNDATION runs alongside from the start, since it has no engine dependency: config, secrets, all four token families with cross-language fixtures, `rivet.rs` helpers and the `testing.rs` engine harness. Gate: engine goldens and full-match replays pass, protocol JSON goldens pass, auth goldens pass, and a probe actor round-trips through `testing.rs`.
2. **W2 build.** In parallel: AI-CORE then AI-LLM; SERVER-HTTP; SERVER-PROFILE; SERVER-TABLE (needs AI-CORE for the heuristic; aiSeat glue after AI-LLM); DESKTOP (develops against engine fixtures and the protocol store; wires networking against the server lib API).
3. **W3 integration.** Registry and main wiring, end-to-end tests with a spawned engine (full match vs AI over rivetkit-client), desktop embedded mode, and live headless snapshots of a real match.
4. **W4 web + deploy.** Web retarget (rivetkit 3.1.0, static SPA, `/api/*`), removal of TS actors, patch and overrides, Dockerfile for the Rust server, `.railway/railway.ts` engine 3.1.0, docs and README.
5. **W5 review.** Adversarial review panels (rules parity, security, concurrency and durability, UI fidelity, code quality), fix loops, final acceptance gate.

## 7. Definition of done (whole port)

- `bun run check`, `bun run test` and `bun run build` pass from the repo root (turbo runs cargo + web).
- The engine passes all 01 goldens and replays. The AI passes all 04 tests. The profile and auth goldens (05 §14) pass.
- `EUCHRE_E2E=1 cargo test -p euchre-server --test e2e` plays a full match human-vs-3-AI through a real 3.1.0 engine via rivetkit-client, including reconnect, illegal moves, review gate and profile recording.
- The desktop app launches, plays a full match against the embedded server with smooth animations, and its scenes render headless.
- The web app plays a full match against the Rust backend (browser check).
- No TS actor code or rivetkit 2.x remains. Rivet is 3.1.0 everywhere.
