# Requests: Lane C (rack, header, panels) to Lane A (table/mod.rs, app.rs, present.rs)

Most important first. Each item has a local workaround in Lane C's files, so nothing is
blocked. Each item says what changes once it lands. Append `DONE: <what you did>` under an item
when it lands.

## 1. Pass the score roll to the header (P1, 09 §4.8, C-score-rolls)

**Problem.** `header::header_with` reads `present.score` (`shown`, `landed_at`) so the score rolls
one point per `SCORE_STEP`, and the `PULSE` ring and `NUM_POP` fire when the roll lands. mod.rs
still calls `header::header(...)`, which passes no roll. The numbers still roll from the old value
to the new one (the `RollingText` remembers it), but in one jump, and with no pulse or pop.

**Ask.** In `table/mod.rs::view`, replace the `header::header(...)` call with:

```rust
.child(header::header_with(
    view,
    width,
    compact,
    reduced,
    hooks.settings_open,
    Some((&table.present.score, state.clock)),
))
```

When it lands, tell Lane C. Lane C then deletes the `header` wrapper.

## 2. The narrow settings rail's width (09 §5.6, C-settings-720)

**Problem.** `panels::settings_rail(.., overlay: true, ..)` now sets its own width:
`panels::rail_width(window_w)` = `min(RAIL_W, width − 32)`. mod.rs then calls
`.w(panels::RAIL_W.min(width))` on it, which wins. That width is right at 720 wide. Below 372 wide,
though, the rail meets the left edge with no gutter.

**Ask.** In `table/mod.rs`, the settings overlay: drop the `.w(panels::RAIL_W.min(width))` call,
or change it to `.w(panels::rail_width(width))`.

## 3. The rules dialog's search and accordion state (06 §5.10)

**Problem.** The rules dialog now has a search box and an accordion: sections are closed by
default, every match opens while there is a query, `No section matches "{query}".`, and the
`New to euchre? Take the quick tour again` button. It reads `hooks.rules_query` and
`hooks.rules_flipped` and sends `RulesMsg`. Two things are missing.

- `panels::rules_dialog(view, height, reduced)` (the call in mod.rs today) has no hooks. It
  draws every section open, with no toggles and no search box. That is the old behaviour, plus
  the tour button.
- A Rune text field edits a `TextEdit` that lives in app state (`El::on_edit`). There is no such
  field yet.

**Ask.**

(a) Add the field and an accessor to app state. It must return a `&mut TextEdit` even with no
table, so keep it on `AppState` (or keep a fallback):

```rust
// AppState
pub rules_search: rune::TextEdit, // TextEdit::new("") at start
pub fn rules_search(&mut self) -> &mut rune::TextEdit { &mut self.rules_search }
```

(b) In `table/mod.rs`, replace the `panels::rules_dialog(view, height, reduced)` call with:

```rust
panels::rules_dialog_with(
    view,
    Some(panels::RulesState {
        query: &hooks.rules_query,
        flipped: hooks.rules_flipped,
        search: Some((&state.rules_search, AppState::rules_search)),
    }),
    height,
    reduced,
)
```

The box sends `RulesMsg::Query(text)` on every change. It also clears `rules_flipped` when the
query goes from empty to not empty, or back, as the web `<details open>` binding does. It does
this through `RulesMsg::Toggle`, so `AppState::rules` needs no change. If (a) has to wait, pass
`search: None` for now: the accordion and the filter still work. Keep the query when the dialog
closes, as the web does.

## 4. Deal gating under the tiers (09 §4.1)

The rack deals in from the deck when `present.deal_in == Some(hand)`, the motion is not reduced,
and `director.feel() != Feel::Skim`. Brisk halves the packet delays. In wave 0 `feel()` is always
Snap, so Snap does **not** skip the deal. If Snap means "land in place" (a reconnect, say), clear
`deal_in` in that case, or tell Lane C and the rack will skip the deal on Snap too.

## 5. FYI: what Lane C mounts itself

- The hand-result card `hand-result-{hand}` mounts in `hand.rs::hand_area`. It sits in a layer
  `hand-result-layer`, placed over the trick area from `layout("trick")` and
  `layout("hand-area")`. Nothing is needed from mod.rs.
- `panels::panel_transition(reduced)` is public, for any other dialog panel (PANEL_IN/OUT, scale
  0.9 + offset(0, 30) + opacity; reduced fades only).
- `panels::scrim(key, reduced)` is the shared SCRIM (`#0b1d16` a0.45, blur 10, SCRIM_IN/OUT).
- The modal close no longer calls `tx.instant()`: Escape and a click on the backdrop now
  animate out like the close button does. `rt.find` skips exiting elements, so the Escape tests
  in tests/table.rs stay green.

DONE (Lane A, 01:08): item 1. mod.rs calls `header::header_with(.., Some((&table.present.score,
state.clock)))`. The `header` wrapper is now unused (a dead-code warning); delete it.

DONE (Lane A, 01:08): item 2. The overlay rail uses `.w(panels::rail_width(width))`.

DONE (Lane A, 01:08): item 3. `AppState.rules_search: rune::TextEdit` (starts empty, outlives the
dialog and the table) and `AppState::rules_search(&mut self) -> &mut TextEdit`; mod.rs calls
`panels::rules_dialog_with(view, Some(RulesState { query, flipped, search: Some((&state.rules_search,
AppState::rules_search)) }), height, reduced)`.

DONE (Lane A, 01:08): item 4. Snap means "landed in place": `present.deal_in` is cleared when the
hand's Deal shot lands as Snap (a skimmed backlog, a reconnect snap, reduce motion) and on every
quiet change (first view, snap). Suggestion: read the deal's own tier,
`director.feel_of(&ShotKey::deal(view.hand_no))`, rather than `director.feel()` (the feel in
force now, which moves as the backlog drains, so `pace` could change mid-deal). Full and Brisk
are the only tiers you will see while `deal_in` is set.

Lane C (01:40): items 1 to 4 are taken up on Lane C's side.
- The `header` wrapper is deleted. Only `header_with` is left.
- The rack reads the deal's own tier, `director.feel_of(&ShotKey::deal(hand))`, and falls back to
  `feel()` while the shot is queued.
- The hand-result card waits for the hand's Score shot (`!director.pending(&ShotKey::score(hand))`),
  so it lands with the score roll, not over the last trick.

## 6. FYI: `tests/table.rs::the_bid_copy_stays_in_its_box_beside_your_seat` fails at 720x820

In Lane C's target, at 01:35:

```
table-bid at (720.0, 820.0): bid-copy [254.51563, 215.04688, 267.5625, 83.5] inside [181.92188, 224.0, 356.15625, 65.59375]
```

The trick box is 65.6 points tall there, and the bid copy is 83.5. The hand area is
`t::hand_h(820) = 344.4` and the header is 64, as before, so Lane C's files do not change the felt's
height. The layout belongs to `stage.rs` (Lane B). The same note is in `motion-C-to-B.md`.

## From integration (wave close)

- 5: noted.
- CLOSED 6: Lane B fixed it (motion-A-to-B.md, DONE 7). The test passes in the integration
  run.
