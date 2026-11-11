# Requests: Lane E (coach, sound, announcer, tour) to Lane A (app.rs, present.rs)

Three asks, most important first. Nothing in Lane E is blocked: each item says what is missing
until it lands. Lane E's code for each one is already in `src/sound.rs`.

## 1. Turning sound on plays the test cue (06 §15.1)

**Problem.** 06 §15.1: "Turning ON plays `playTrumpCalled('H')` as a test cue." `toggle_sound`
(app.rs) flips the pref and `set_enabled`, but plays nothing, so the player does not hear
whether sound works until a card lands.

**Ask.** In `AppState::toggle_sound`, after `self.sound.set_enabled(self.prefs.sound)`:

```rust
if self.prefs.sound && self.visible {
    self.sound.play(&Cue::TrumpCalled(Suit::Hearts));
}
```

Use whatever flag `poll` already uses for "the window is shown" (`audible`) in place of
`self.visible` if it is named differently. In a test or a headless run `Sound::play` opens no
output (`sound::live_app()` is false there), so the existing
`tests/motion.rs::the_sound_toggle_turns_the_player_on_and_off` stays green.

## 2. The place cue carries its card (06 §15.2 `playPlace(cardId)`)

**Problem.** The web shifts the place thud by a hash of the card id (a few semitones either
way), so a trick of four cards sounds like four cards, not one sample four times. `Cue::Place`
has no card, so `sound::recipe(&Cue::Place)` plays the centre pitch every time.

**Ask.** Make it `Cue::Place(Card)` (present.rs) and push the landed card. 09 §4.3 wants the
cue on the landing, not the launch. In `sound::recipe`, Lane E then changes one arm:

```rust
Cue::Place(card) => place(place_shift(card)),
```

`sound::place_shift` is the web `hashSemitone`, already written and tested (JS = -2.028,
AH = -2.592, all within ±3). Tell Lane E when the variant changes, or make that one-line edit
in sound.rs yourself as part of the same change, since the match must be exhaustive.

## 3. Live-sim tests start with the tour seen (optional)

**Problem.** The tour opens by itself over the first real hand (`tour::should_auto_open` in
`poll`). Tests that build `AppState::new(Prefs::default(), Some(bridge), ...)` and click by
position (`tests/table.rs::sim_table`, `tests/shell.rs` around line 169) would have their
clicks eaten by the modal. To keep them green today, `should_auto_open` also requires
`sound::live_app()`: not headless and the running binary is `euchre`.

**Ask.** Build those tests' prefs as `Prefs { tour_seen: true, ..Prefs::default() }`. Once that
lands (and Lane D's, see `motion-E-to-D.md`), Lane E can drop the binary-name half of the gate
and keep only `!rune::is_headless()`.

DONE (Lane A, 01:08): item 1. `toggle_sound` plays `Cue::TrumpCalled(Suit::Hearts)` when it turns
sound on and the window shows (`self.visible`).

NOT YET (Lane A): item 2. Changing `Cue::Place` to `Cue::Place(Card)` breaks the exhaustive match
in sound.rs in the same build, and Lane A's rules forbid edits outside its own files, so the
change cannot land atomically from either side. The cue already fires on the landing (the
Director counts touchdowns; `present.poll` pushes one `Cue::Place` per poll that saw any).
There is no shape your arm can take first that compiles before and after the change, so the
cleanest path is one edit to both files after the lanes finish (the orchestrator, or either
lane with leave). Lane A's half is small: `take_touchdowns` returns the landed cards and
`present.poll` pushes `Cue::Place(card)` for each.

DONE (Lane A, 01:08): item 3. `tests/table.rs::sim_table` builds its prefs with
`tour_seen: true`. tests/shell.rs is Lane D's.

LANE E REPLY (01:15): item 2 is unblocked. sound.rs now matches the cue as `Cue::Place { .. }`,
which compiles with the unit variant and with `Place(Card)`, and tests/coach.rs no longer
builds a `Cue::Place` value. So Lane A can change the variant (and `take_touchdowns`) on its own
at any time without breaking Lane E's files. Afterwards the one-line follow-up in sound.rs is
`Cue::Place(card) => place(place_shift(card))` (Lane E, or the orchestrator if Lane E has
finished). Items 1 and 3: thanks, seen.

LANE E NOTE (01:20): item 3 seen. Lane E keeps the `sound::live_app()` gate on the auto-open
anyway: the sound player needs the same check (a cargo test is not headless, and the toggle's
test cue would otherwise open the speaker in `the_sound_toggle_turns_the_player_on_and_off`).
Open from this file: item 2 only.

DONE (Lane A, 01:17): item 2. `Cue::Place(Card)` (present.rs). `Director::take_touchdowns` now
returns the landed cards in order (`Vec<Card>`, the reduce-motion and skim paths included), and
`present.poll` pushes one `Cue::Place(card)` per poll that saw a landing, pitched by the last card
that landed in it (so a skimmed backlog still makes one thud, not four at once). Your
`Cue::Place { .. }` arm compiles as is; the one-line follow-up in sound.rs is yours:
`Cue::Place(card) => place(place_shift(card))` (then drop the `#[allow(clippy::unneeded_struct_pattern)]`).

## From integration (wave close)

- DONE 2, the sound.rs half: `Cue::Place(card) => place(place_shift(card))`, and the clippy
  allow is gone. `tests/coach.rs` checks that four cards give four pitched place cues.
- All three items are closed.
