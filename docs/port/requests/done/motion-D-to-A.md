# Requests: Lane D (shell) to Lane A (app.rs, scenes.rs)

Three asks, most important first. Each has a local workaround in Lane D's files, so nothing is
blocked; each item says what changes once it lands.

## 1. The leaving screen must exit the way the route change says (P1, route transitions)

**Problem.** `screens::route_transition(dir, reduced)` gives the entering screen the right
transition, but a screen that is removed leaves with the transition from its last render: the
direction of the change that brought it in. Back from New game: Home slides in from the left
(right), but New game also leaves to the left (wrong; it should leave to the right).
Measured at +0.15 s: `screen-new` x = -47.7. Rune's answer is `El::child_exit`
(`rune/src/el/child_exit.rs:12`: "a node normally leaves the way it was last drawn ... Set it on
each build with that build's direction").

**Ask.** In `app.rs::view`, give the parent of the screen the same transition as `child_exit`:

```rust
let enter = screens::route_transition(state.route_dir, reduced);
if let Some(t) = enter {
    screen = screen.transition(t);
}
// ...
let mut root = col().key("root") /* ... */ .child(screen);
if let Some(t) = enter {
    root = root.child_exit(t);
}
```

`child_exit` applies to every child that `root` removes in that build. The pill nav leaves with
the screen, which reads fine. When the last toast goes, the empty `toasts` holder leaves too, but
nothing is drawn in it. To keep it tighter, wrap the screen in its own holder (for example
`col().key("screens").w_full().h_full().child_exit(t).child(screen)`) and put that in `root`.

**Test.** `tests/shell.rs::route_back_mirrors_the_slide` asserts the mirror. Until this lands,
that test checks only the entering half (Home comes in from the left). The leaving half is
`route_back_leaving_screen_exits_right`, marked `#[ignore]` with a pointer here. Remove the
`#[ignore]` when this is in.

## 2. Lifetime stats and the pager on Your games (06 §17.1)

**Problem.** `GamesState` holds only the page. The stats grid needs `ProfileStats`, and
`← Newer games` needs the cursors of the pages before the one shown. `Reply` cannot gain an arm
without breaking the exhaustive match in `AppState::receive`.

**Ask.**

- `GamesState.stats: Option<euchre_profile_model::ProfileStats>`.
- `GamesState.newer: Vec<Option<String>>`: the cursors of the pages before this one (empty on
  the first page).
- In `receive`, a `Reply::Stats(Result<ProfileStats, NetError>)` arm that sets `games.stats`.
  Lane D adds the variant and the bridge handler in the same change once you say go. Or add the
  arm yourself with the variant in `net/bridge.rs`; Lane D won't touch that enum until then.
- In `enter(Route::Games)`, also send `Intent::GetStats`. This intent already exists in Lane D's
  bridge.

**Local workaround (in place).**

- `Bridge::shell()` locks a small `ShellSlots` beside the UI handle: `stats` (written by the
  bridge with each first-page `ListGames` and with `GetStats`), `newer` (the back-stack of
  cursors) and `shown` (the cursor of the page on screen). `Bridge::send(ListGames)` keeps
  `shown` and clears `newer` on a first-page load, so `enter(Route::Games)` resets it.
- The Games screen reads stats and the back-stack from there. `Older games →` pushes `shown`
  and sends `ListGames { cursor }`; `← Newer games` pops and sends the popped cursor.

Once your fields exist, the screen reads `state.games.stats` and `state.games.newer`, and the
bridge-side slots go away.

## 3. Scenes for the profile screens (09 §6.6, render checks)

**Problem.** The `games` and `settings` scenes only set the route, so they render empty (no data
without a bridge). There is no match-detail scene.

**Ask.** In `scenes.rs`:

- `games`: call `crate::screens::other::fixture_games(state)` (sets the route, the first page of
  the sim's seeded history and the stats).
- `settings`: call `crate::screens::other::fixture_settings(state)` (the route and loaded
  default settings).
- New scene `match-detail`: call `crate::screens::other::fixture_match_detail(state)` (the route
  and one finished match with its hands).

All three are pure and deterministic, and they never touch the network.

## Not asked: the Home → Table hero flight

Dropped, so there is nothing to wire. Home never goes straight to the table: Let's play opens New
game (06 §4). A matched flight pairs an element removed in one build with one inserted in the same
build, so a Jack on Home cannot meet a card on the loading table two route changes later. The
hero art is also one picture with no separate Jack of spades. `D-hero-flies` is skipped.

DONE (Lane A, 01:08): item 1. `app.rs::view` sets `screen.transition(t)` and `root.child_exit(t)`
with the same `route_transition(...)`, so the leaving screen exits the way the change goes.
Remove the `#[ignore]` on `route_back_leaving_screen_exits_right` (tests/shell.rs is yours).
`navigate`/`back` no longer call `tx.motion(UI)` (it overrode every matched spring in the change).

DONE (Lane A, 01:08): item 2, Lane A's half. `GamesState.stats` and `GamesState.newer` exist;
`enter(Route::Games)` sends `Intent::GetStats` after `ListGames`. GO for `Reply::Stats`: add the
variant and its bridge handler whenever you like. `AppState::receive` has an
`#[allow(unreachable_patterns)] _ => {}` arm, so a new variant compiles before its arm exists;
tell me here when it lands and I add `Reply::Stats(Ok(s)) => self.games.stats = Some(s)` and
drop the catch-all.

DONE (Lane A, 01:08): item 3. Scenes `games` -> `fixture_games`, `settings` ->
`fixture_settings`, new `match-detail` -> `fixture_match_detail`.

LANDED (Lane D, after 01:08): `Reply::Stats(Result<ProfileStats, NetError>)`. `Intent::GetStats`
now posts it (it no longer writes `Bridge::shell()`). Add
`Reply::Stats(Ok(s)) => self.games.stats = Some(s)` and drop the catch-all. Until then the Games
screen falls back to the copy that rides with the first page; it reads `state.games.stats` first.
`fixture_games` now sets `state.games.stats`. The leaving-half test is folded into
`route_back_mirrors_the_slide` and passes; nothing is ignored in tests/shell.rs.

## 4. The pager's back-stack (small, optional)

`GamesState.newer` alone cannot page: `Older games →` has to push the cursor of the page on
screen, and `GamesState` does not hold it. `enter(Route::Games)` also keeps `newer` from the last
visit. So the pager still keeps its stack in `Bridge::shell()` (`newer`, `shown`), which
`Bridge::send(ListGames { cursor: None })` resets.

**Ask.** `GamesState.shown: Option<String>` (the cursor of the page on screen, set when the
`ListGames` is sent), and `self.games.newer.clear()` in `enter(Route::Games)`. Then the screen
pushes `shown` onto `state.games.newer`, and `ShellSlots` goes away.

DONE (Lane A, 01:23): `Reply::Stats`. `AppState::receive` has `Reply::Stats(Ok(stats)) =>
self.games.stats = Some(stats)` and `Reply::Stats(Err(err))` (a `tracing::warn!`, and the screen
keeps its first-page fallback). The catch-all arm is gone, so the match is exhaustive again.

DONE (Lane A, 01:23): item 4. `GamesState.shown: Option<String>` (the cursor of the page on
screen; `None` for the first page). `enter(Route::Games)` now clears `self.games.newer` and sets
`shown = None` before sending `ListGames { cursor: None }`. Your pager sets `s.games.shown` when it
sends a `ListGames` and pushes it onto `s.games.newer` for `Older games →`; then `ShellSlots` can go.

## From integration (wave close)

- DONE 4, Lane D's half. The Your games pager sets `s.games.shown` when it sends `ListGames` and
  pushes that cursor onto `s.games.newer` for `Older games →`. `ShellSlots`,
  `Intent::AttachShell` and `Bridge::shell()` are gone; `Bridge::detached()` takes no arguments.
  `bridge.rs` has a test that pages through the sim's games and reads the lifetime stats
  (`your_games_pages_and_reads_the_lifetime_stats`). `tests/shell.rs` waits for both the stats
  and the page.
