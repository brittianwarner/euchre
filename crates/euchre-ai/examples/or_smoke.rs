//! Opt-in live smoke test of the Jev decision path (`scripts/or-smoke.ts`).
//!
//! Never run by `cargo test`. Needs a real key in the environment:
//!
//! ```text
//! OPENROUTER_API_KEY=... cargo run -p euchre-ai --example or_smoke
//! ```
//!
//! `OPENROUTER_BASE_URL` is honoured. Prints the model, the choice and the
//! elapsed time; exits non-zero on any failure.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use euchre_ai::decision_context::JEV_MODEL;
use euchre_ai::model::model_factory_from_process_env;
use euchre_ai::types::JevDecisionInput;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::var("OPENROUTER_API_KEY").map_or(true, |k| k.trim().is_empty()) {
        eprintln!("OPENROUTER_API_KEY is not set; nothing to do.");
        return ExitCode::FAILURE;
    }
    let Some(decision) = model_factory_from_process_env().and_then(|f| f.decision) else {
        eprintln!("No decision client could be built from the environment.");
        return ExitCode::FAILURE;
    };
    let criteria = json!({
        "move_0": "Play jack of spades",
        "move_1": "Play ace of hearts",
        "move_2": "Play nine of clubs"
    });
    let input = JevDecisionInput {
        state: json!(
            "Euchre. Hearts trump. Lead a new trick. Your hand: JS, AH, 9C. Choose a legal lead."
        ),
        // As in the TS script: no instructions (omitted from the body).
        instructions: serde_json::Value::Null,
        criteria: criteria.as_object().cloned().unwrap_or_default(),
        additional_questions: None,
    };
    let cancel = CancellationToken::new();
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_millis(5000),
        decision.decide(input, cancel.clone()),
    )
    .await;
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Err(_) => {
            cancel.cancel();
            eprintln!("timed out after {elapsed_ms} ms");
            ExitCode::FAILURE
        }
        Ok(Err(e)) => {
            eprintln!("{e} ({elapsed_ms} ms)");
            ExitCode::FAILURE
        }
        Ok(Ok(out)) => {
            println!(
                "{}",
                json!({ "model": JEV_MODEL, "choice": out.choice, "elapsedMs": elapsed_ms })
            );
            ExitCode::SUCCESS
        }
    }
}
