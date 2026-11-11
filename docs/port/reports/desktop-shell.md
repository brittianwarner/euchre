# Report: DESKTOP-SHELL (lane DESKTOP)

Native Rune desktop shell for euchre: the app frame, router and chrome; Home and New game
with 06 copy verbatim; a first table (felt, seats, up card or trick, hand, bid buttons); the
Backend trait with an in-process SimBackend and a bridge that keeps all IO off the UI thread.
Only `apps/desktop/**` changed. No git writes.

## Gates (all green)

| Gate | Result |
|---|---|
| `cargo fmt -p euchre-desktop -- --check` | clean |
| `cargo clippy -p euchre-desktop --all-targets -- -D warnings` | clean |
| `cargo test -p euchre-desktop` | 20 unit + 5 headless runtime tests pass |

Crate lints: `#![deny(missing_docs)]`, plus `clippy::unwrap_used` and `clippy::expect_used`
denied outside tests. There is no `unsafe` and no `todo!`.

## Module map (`apps/desktop/src`)

| Module | Job |
|---|---|
| `main.rs` | `fn main() { euchre_desktop::run() }` |
| `lib.rs` | `spec()` builds the AppSpec: "Euchre", 1280x820, min 720x520, 10 bundled faces, families ("DM Sans", "IBM Plex Mono"), serif `Family::Named("Source Serif 4")`, scenes, `setup`/`poll`/`on_key`. `run()` adds tracing, prefs, backend, bridge thread, `on_ready`→Identify, `on_visibility`. |
| `theme.rs` | Every 06 §1 token as a named constant. Hexes go to `PAL` (linear). Also `radius`, `space`, `card`, `window`, `shadow`, `motion`, `ty` (type scale with vw clamps), and `home`/`table` layout measures. |
| `assets.rs` | 24 faces, the back, the hero art, the license and credits. `Art` decodes them off-thread. |
| `prefs.rs` | `Prefs` (sound, hints, hint streaks, reduce motion, tour seen). `PrefsStore` writes atomically with mode 0600 in the platform config dir. |
| `app.rs` | `AppState` and `Route` (Home, NewGame, Table, Games, MatchDetail, Settings), with history and Escape back. Also `TableScreen`: the protocol `TableStore` plus selection, effects→bridge and one armed timer. Also the pill GlobalNav (hidden on Home and the table), toasts, the focus ring and reduced motion. |
| `screens/home.rs` | 06 §3: header, hero (eyebrow, h1, lede, actions, art with vignette, caption, rotated stamp, strip), intro, how to play with the jacks disclosure, footer. Single column at <=760. |
| `screens/new_game.rs` | 06 §4: two checkboxes (role checkbox), the both-on note in a reserved line (no shift), and Deal me in. |
| `screens/table/mod.rs` | 06 §5/§14: loading (store `loading_notice`, failure line, Reconnect, saved games), reconnect banner, cream header with score, wood rail + brass + radial felt + rim, seats, up card / deck tile, tilted trick slots, `turn_instruction` (§12 in full), hand heading strings, Play/Discard button, rack, bid buttons. Cards are `matched("card-{id}")` shared elements. |
| `screens/other.rs` | Dark scrapbook frames for Your games, one match and Settings (device prefs toggles). |
| `scenes.rs` | `home`, `new`, `table-deal` (deterministic SimBackend fixture, no network). |
| `net/mod.rs` | `Backend`, `TableCalls`, `TableConn`, `ConnEvent`, `TableConfig`, `Identity`, `NetError`, `BackendKind` (`EUCHRE_DESKTOP_BACKEND=sim` default, or `remote`). |
| `net/sim.rs` | `TableCore`: the authoritative in-process table (engine reducer, persona robots with a heuristic decider, Normal/Fast/Instant tempos, review gate, idempotent moves). `SimBackend` runs one tokio task per table. |
| `net/bridge.rs` | `Intent` → background tokio runtime → `Reply` delivered through `UiSink` (`AppHandle::update` in the app, a channel in tests). |
| `net/remote.rs` | `RemoteBackend` stub. Every call returns `NotWired`. |

## Backend trait and bridge API

```rust
#[async_trait] pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn identity(&self) -> Result<Identity, NetError>;
    async fn create_table(&self, cfg: TableConfig) -> Result<String, NetError>;
    async fn open_table(&self, game_id: &str) -> Result<TableConn, NetError>; // events + calls
    async fn list_games(&self, cursor: Option<String>) -> Result<MatchListPage, NetError>;
    async fn match_detail(&self, match_id: &str) -> Result<Value, NetError>;
    async fn get_settings(&self) -> Result<Value, NetError>;
    async fn patch_settings(&self, patch: Value) -> Result<Value, NetError>;
}
```

`TableCalls` provides `submit_move`, `review_trick`, `snapshot`, `get_hand_review` and
`close`. The UI sends `Intent`s over `Bridge::send`: Identify, CreateTable, OpenTable,
CloseTable, Store(command), ArmTimer, ListGames, MatchDetail, GetSettings, PatchSettings and
SavePrefs. `BridgeThread` (thread "euchre-bridge", 2 workers) answers with `Reply`s, which
`AppState::receive` applies on the UI thread.

The store owns the protocol timeouts. The bridge adds a 1 s grace, and a lost call becomes
NoAck/None. The UI thread never blocks.

## Scenes

Run `cargo run -p euchre-desktop -- --scene <name> --snap <png>`. The snaps are in the
scratchpad (`desktop-snaps/home.png`, `new.png`, `table-deal.png`) and were checked by eye
against 06.

## Tests (`tests/shell.rs`, headless `rune::Runtime`)

- Navigation reaches every screen through the header and the pill, and Escape goes back.
- New game: the toggles work, and the note never moves the submit button.
- The table hides the nav, and leaving the table closes it.
- Full flow against SimBackend (Fast tempo) through the real bridge thread:
  1. Deal me in.
  2. Wait for the hand and the turn.
  3. Make a move from the UI.
  4. The ack arrives and the view advances.
- Reduce motion (D15): a selected card lifts on a spring. With either the app preference or the
  system setting on, it does not travel.

## Deviations

- Home art mask: Rune paints a forest vignette instead of an alpha mask. It reaches full forest
  at the image edge, so no seam shows; the centre is solid as on the web.
- Intro copy: "made for the desktop". New game footer: "saved on this device". Both follow the
  06 port notes.
- Rack: a click selects and Play submits. Double-click to play, and arrow-key roving between
  cards, are not built yet.
- The `Must follow suit` and `unavailableReason` a11y copy is simplified. `whyIllegal` is
  owed.
- Table header: a plain score instead of the cribbage ScoreCards.

## Open items

- Table lane:
  - ScoreCards, the settings side panel, the rules, hand review and hints (§5.3, §5.9–5.10,
    §11, §13);
  - the Continue button;
  - table talk in the result line;
  - illegal-card messages (§5.7);
  - the deal, pickup and collect animations (§6);
  - responsive table layouts (§5.2).
- Remote backend: wire `RemoteBackend` to the server API and Rivet (integration lane).
- Profile lane: the identity gate, stats, history paging and the persona editor. The typed
  settings are requested in `requests/desktop-profile-types.md`.
- First-run tour and sound (§14.3, §15).
