//! Runs the `examples/e2e.rs` harness (the port of the TS `scripts/e2e-play.ts`)
//! inside the test gate, so its own checks (substring redaction, follow-suit
//! replay, independent trick winners, the illegal-move probe, the score-delta
//! check and determinism) fail `cargo test`, and compares its whole report with
//! the TS script's.
//!
//! The goldens are the TS output with the `elapsed` line removed:
//!
//! ```text
//! bun run ~/goods/euchre/scripts/e2e-play.ts 20       | grep -v '^elapsed ' > tests/fixtures/e2e-20.txt
//! bun run ~/goods/euchre/scripts/e2e-play.ts 20 quiet | grep -v '^elapsed ' > tests/fixtures/e2e-20-quiet.txt
//! ```

#[allow(dead_code)]
#[path = "../examples/e2e.rs"]
mod e2e;

const NARRATED: &str = include_str!("fixtures/e2e-20.txt");
const QUIET: &str = include_str!("fixtures/e2e-20-quiet.txt");

/// Runs the harness with `args`; returns its pass flag and its report without
/// the timing line (the only line that legitimately differs from TS).
fn run(args: &[&str]) -> (bool, String) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let mut out = String::new();
    let ok = e2e::run(&args, &mut out);
    let report: String = out
        .lines()
        .filter(|l| !l.starts_with("elapsed "))
        .map(|l| format!("{l}\n"))
        .collect();
    (ok, report)
}

#[test]
fn the_narrated_harness_passes_and_matches_ts_byte_for_byte() {
    let (ok, report) = run(&["20"]);
    assert!(ok, "the e2e harness reported failures:\n{report}");
    assert!(report.contains("✓ ALL INVARIANTS HELD"));
    assert_eq!(report, NARRATED);
}

#[test]
fn the_quiet_harness_passes_and_matches_ts_byte_for_byte() {
    let (ok, report) = run(&["20", "quiet"]);
    assert!(ok, "the e2e harness reported failures:\n{report}");
    assert_eq!(report, QUIET);
    assert!(!report.contains("NARRATED GAME"));
}
