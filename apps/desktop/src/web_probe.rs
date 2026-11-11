//! The sim proof's probe (web plan W-15): with `?proof` in the URL the view also records a small
//! JSON snapshot of the route and the table, read from JS through [`euchre_probe`]. Built only
//! with the `web-proof` feature, never shipped, and never used by a done-proof (those drive the
//! shipping build through Rune's DOM mirror).
//!
//! It reads no layout, so it adds no view re-runs: the view runs as it always does, only on
//! state changes.

use std::cell::RefCell;

use rune::{AppSpec, Cx, El};
use serde_json::json;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::app::{self, AppState};

thread_local! {
    static PROBE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// `spec` with the recording view when the page URL asks for `?proof`, else unchanged.
pub fn wrap(mut spec: AppSpec<AppState>) -> AppSpec<AppState> {
    if proof_requested() {
        spec.view = proof_view;
    }
    spec
}

fn proof_requested() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.contains("proof"))
}

fn proof_view(state: &AppState, cx: &mut Cx<AppState>) -> El {
    let root = app::view(state, cx);
    let table = state.table.as_ref().map(|t| {
        let shown = t.store.display_view().map(|v| {
            json!({
                "v": v.v, "phase": v.phase, "status": v.status, "you": v.you,
                "turn": v.turn_seat, "dealer": v.dealer_seat, "hand_no": v.hand_no,
                "score": v.score, "tricks_won": v.tricks_won, "trump": v.trump,
                "hand": v.hand.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "trick_cards": v.trick.plays.len(), "tricks_done": v.trick_log.len(),
                "legal": v.legal.iter().map(|m| json!({ "id": m.id, "label": m.label })).collect::<Vec<_>>(),
            })
        });
        json!({
            "shown": shown,
            "latest_v": t.store.view().map(|v| v.v),
            "selected": t.selected.map(|c| c.to_string()),
            "controls": t.store.controls_enabled(),
            "tour_step": t.present.hooks.tour_step,
        })
    });
    let snapshot = json!({ "route": format!("{:?}", state.route), "table": table });
    PROBE.with(|p| *p.borrow_mut() = snapshot.to_string());
    root
}

/// The last snapshot the probe recorded (empty without `?proof`).
#[wasm_bindgen]
pub fn euchre_probe() -> String {
    PROBE.with(|p| p.borrow().clone())
}
