//! The match-end panel's score never reads below the header's: it shows the last hand's points
//! arriving only while the header is still rolling them in. A frame clock that stops (a web
//! page that is not redrawing) must not leave the panel on the totals before that hand.

use euchre_desktop::app::AppState;
use euchre_desktop::{scenes, spec, theme};
use rune::{App, Env, Runtime};

const DT: f64 = 1.0 / 60.0;

fn scene_runtime(name: &str) -> Runtime<AppState> {
    let (_, apply) = scenes::SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("scene {name} exists"));
    let mut state = AppState::default();
    apply(&mut state);
    state.wait_for_art = true;
    let mut rt = App::new(spec())
        .init(move || state)
        .headless_runtime(theme::window::SIZE, Env::new(false, 1.0));
    rt.frame(0.0);
    rt
}

fn value(rt: &Runtime<AppState>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.label.or(e.text))
}

#[test]
fn the_panel_shows_the_final_score_once_the_header_does() {
    for name in ["game-over-win", "game-over-loss"] {
        let mut rt = scene_runtime(name);
        let mut checked = 0;
        for _ in 0..240 {
            rt.frame(DT);
            let (Some(panel_us), Some(panel_them)) = (
                value(&rt, "match-end-us-roll"),
                value(&rt, "match-end-them-roll"),
            ) else {
                continue;
            };
            let (Some(head_us), Some(head_them)) =
                (value(&rt, "score-us-roll"), value(&rt, "score-them-roll"))
            else {
                continue;
            };
            let view = rt
                .state
                .table
                .as_ref()
                .and_then(|t| t.store.display_view())
                .cloned()
                .expect("a view");
            let team = view.you.index() % 2;
            let (us, them) = (view.score[team].to_string(), view.score[1 - team].to_string());
            if head_us == us && head_them == them {
                assert_eq!(
                    (panel_us, panel_them),
                    (us, them),
                    "{name}: the header shows the final score, so the panel does"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "{name}: the panel and the header were both up");
    }
}
