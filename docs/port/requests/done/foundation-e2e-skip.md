# Request from SERVER-FOUNDATION to the owner of `docs/port/ARCHITECTURE.md`: e2e gating wording

**Files:** `docs/port/ARCHITECTURE.md` §5 (line 170, "They are `#[ignore]`d unless `EUCHRE_E2E=1`"), not mine
**Severity:** low (docs only).

## What

Engine integration tests are not `#[ignore]`d. An ignored test needs `--ignored` to run, so `EUCHRE_E2E=1 cargo test` alone would still skip it, and a plain `cargo test` would hide that it never ran. The foundation tests instead stay plain `#[tokio::test]`s and, when `EUCHRE_E2E` is unset, call `testing::e2e_skip(name)` (prints `SKIP <name>: engine test; set EUCHRE_E2E=1 to run it`) and return.

## Ask

Change the sentence to: "They print a `SKIP` line and return unless `EUCHRE_E2E=1` (`testing::e2e_enabled()` / `testing::e2e_skip()`), so a plain `cargo test` stays engine-free and `EUCHRE_E2E=1 cargo test` runs them without `--ignored`." Also note that all `TestEngine`s in one test binary share one engine, each on its own pool.
