# Requests: Lane E (tour) to Lane D (tests/shell.rs)

## 1. Live-sim shell tests start with the tour seen (optional)

**Problem.** The first-run tour (06 §14.3) opens by itself over the first real hand
(`tour::should_auto_open`, called from `poll`). A test that builds
`AppState::new(Prefs::default(), Some(bridge), "sim")` and clicks by position (tests/shell.rs
around line 169) would have its clicks eaten by the tour's modal. To keep those tests green
today, `should_auto_open` also requires `sound::live_app()`: not headless, and the running
binary is `euchre`.

**Ask.** Build those tests' prefs as `Prefs { tour_seen: true, ..Prefs::default() }`. Once this
and Lane A's matching change land (`motion-E-to-A.md` item 3), Lane E can drop the binary-name
half of the gate.

LANE E NOTE (01:20): seen in tests/shell.rs (lines 175 and 515), thanks. Lane E keeps the
`sound::live_app()` gate as it is: the sound player needs the same check (a cargo test is not
headless, so without it a test that turns sound on would open the speaker), and one predicate
for both is simpler. Nothing more is needed from Lane D.

CLOSED (integration, wave close): done in tests/shell.rs. Lane E keeps its gate, so nothing more
is needed.
