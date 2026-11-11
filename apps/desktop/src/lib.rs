//! Euchre for the desktop: a native Rune window over the shared engine and protocol (D28).
//!
//! - [`theme`] holds every design token (06 §1) and text style; nothing else names a colour or
//!   a size.
//! - [`app`] is the state, the router and the global chrome (nav, toasts, keys).
//! - [`screens`] draws Home, New game, the Table and the history and settings screens.
//! - [`net`] is the [`net::Backend`] trait, the embedded game server
//!   ([`net::embedded::EmbeddedBackend`], the default), the remote backend, the in-process
//!   [`net::sim::SimBackend`] and the [`net::bridge`] that keeps every call off the UI thread.
//! - [`startup`] keeps loopback off any proxy (it runs first in `main`) and starts the log.
//! - [`prefs`] keeps the per-device preferences; [`assets`] embeds the card art.
//! - [`sound`] is the sound player (silent until Lane E; D14: off by default).
//!
//! `cargo run -p euchre-desktop` opens the window and starts the game server on loopback
//! (`EUCHRE_SERVER_URL` plays against a remote one instead). `--scene NAME --snap out.png`
//! renders a named state headless; scenes and headless runs always use the sim and never touch
//! the network.
//!
//! The same crate builds for the browser (`wasm32-unknown-unknown`, the cdylib): `web` is the
//! entry there, and `run` and `startup` are native-only (docs/port/10-web-rune-plan.md).

#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod assets;
pub mod net;
pub mod prefs;
pub mod scenes;
pub mod screens;
pub mod sound;
#[cfg(not(target_arch = "wasm32"))]
pub mod startup;
pub mod theme;
pub mod url_route;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub mod web_history;
#[cfg(all(target_arch = "wasm32", feature = "web-proof"))]
pub mod web_probe;

#[cfg(not(target_arch = "wasm32"))]
use rune::App;
use rune::{AppSpec, Waker};

use crate::app::AppState;
#[cfg(not(target_arch = "wasm32"))]
use crate::net::BackendKind;
#[cfg(not(target_arch = "wasm32"))]
use crate::net::bridge::{Bridge, BridgeConfig, BridgeThread, Intent, UiSink};

#[cfg(not(target_arch = "wasm32"))]
use crate::prefs::PrefsStore;

/// How long the quit waits for the bridge to close tables and write preferences, with no
/// local server.
pub const QUIT_WAIT: std::time::Duration = std::time::Duration::from_millis(500);
/// How long the quit waits with the embedded server: its stop budget, the bounded drop after
/// it, and a margin. The window is already gone; only the process stays a little longer.
pub const QUIT_WAIT_HOST: std::time::Duration = std::time::Duration::from_secs(45);

/// The window title.
pub const TITLE: &str = "Euchre";

/// The app id the window reports on Linux (Wayland `app_id`, X11 `WM_CLASS`). It equals the
/// macOS bundle id and the Linux `.desktop` file's name, so the desktop shows the app's icon.
pub const APP_ID: &str = "sh.euchre.desktop";

/// The app icon (512 px PNG). Windows draws it in the title bar, the taskbar and Alt+Tab; macOS
/// takes the bundle's `.icns` and Linux the `.desktop` entry's icon instead.
#[cfg(windows)]
const ICON_PNG: &[u8] = include_bytes!("../assets/icon/euchre-512.png");

/// Every bundled face. A non-empty list replaces Rune's built-in Inter, so all eleven are here;
/// the last is the suits-only symbol face (♠♣♥♦) so the suits never draw as boxes.
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/DMSans-Regular.ttf"),
    include_bytes!("../assets/fonts/DMSans-Medium.ttf"),
    include_bytes!("../assets/fonts/DMSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/DMSans-Bold.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4-Regular.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4-Medium.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4-SemiBold.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4-Bold.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4-Italic.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSansSymbols2-Suits.ttf"),
];

/// The sans and mono families; the serif is named per style ([`theme::SERIF`]).
pub const FAMILIES: (&str, &str) = (theme::SANS_FAMILY, theme::MONO_FAMILY);

/// `AppSpec::setup`: starts decoding the art. Headless renders wait for it so the first
/// snapshot has every picture (07 §4.3).
fn setup(state: &mut AppState, waker: Waker) {
    let art = assets::Art::load(waker);
    if state.wait_for_art || rune::is_headless() {
        art.wait_all();
    }
    state.art = Some(art);
}

/// The app: title, size, fonts, scenes and hooks. Tests build their runtime from it.
pub fn spec() -> AppSpec<AppState> {
    AppSpec {
        min_size: Some(theme::window::MIN),
        scenes: scenes::SCENES,
        fonts: FONTS,
        families: FAMILIES,
        setup: Some(setup),
        poll: Some(app::poll),
        on_key: Some(app::on_key),
        ..AppSpec::new(TITLE, theme::window::SIZE, AppState::default, app::view)
    }
}

/// Runs the app until the window closes.
///
/// `main` calls [`startup::reexec_with_no_proxy`] first. The backend is the embedded game server
/// unless `EUCHRE_DESKTOP_BACKEND` or `EUCHRE_SERVER_URL` says otherwise
/// ([`BackendKind::choose`]); a headless render or a `--scene` run always uses the sim, so it
/// never boots an engine and stays deterministic. The bridge thread starts with the window and
/// boots the server while the window draws; a quit stops it before the process ends.
///
/// Native only: the browser starts at `web::start`.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() -> anyhow::Result<()> {
    let scene_run = startup::scene_run();
    rune::set_app_id(APP_ID);
    let data_dir = net::data_dir();
    startup::init_logging((!scene_run).then_some(data_dir.as_path()));
    #[cfg(windows)]
    if let Err(err) = rune::set_app_icon(ICON_PNG) {
        tracing::warn!(%err, "the window icon could not be set");
    }
    rune::prewarm_gpu();

    // A headless render or a `--scene` run starts from the default preferences and saves
    // nothing, so its frames never depend on (or change) the player's own file.
    let prefs_store = PrefsStore::for_run(scene_run, PrefsStore::dir_override());
    let prefs = prefs_store
        .as_ref()
        .map(PrefsStore::load)
        .unwrap_or_default();
    let backend = BackendKind::for_run(scene_run).build();
    let backend_name = backend.name();
    let quit_wait = if backend.host().is_some() {
        QUIT_WAIT_HOST
    } else {
        QUIT_WAIT
    };
    if !scene_run {
        tracing::info!(backend = backend_name, data_dir = %data_dir.display(), "starting");
    }
    let (bridge, intents) = Bridge::channel();
    let quit = bridge.clone();

    let app = App::new(spec()).init(move || AppState::new(prefs, Some(bridge), backend_name));
    let handle = app.handle();
    let sink =
        UiSink::new(move |reply| handle.update(move |s: &mut AppState, tx| s.receive(reply, tx)));
    let thread = BridgeThread::spawn(
        BridgeConfig {
            backend,
            sink,
            prefs: prefs_store,
            quit_on_signals: true,
        },
        intents,
    )?;

    let ready = app.handle();
    let mut started = false;
    let ran = app
        .on_ready(move |_window| {
            if !std::mem::replace(&mut started, true) {
                ready.update(|s: &mut AppState, _| s.on_ready());
            }
        })
        .on_visibility(app::on_visibility)
        // The server's stop starts with the quit, overlapping the window's teardown.
        .on_quit(app::on_quit)
        .try_run();
    // Again (it is idempotent), for a loop that ended without a quit hook; then a bounded wait
    // for the tables to close, the preferences to land and the server to stop. A thread still
    // busy after that goes with the process exit.
    quit.send(Intent::Shutdown);
    let began = std::time::Instant::now();
    let done = thread.join_within(quit_wait);
    if !scene_run {
        tracing::info!(
            stopped = done,
            wait_ms = began.elapsed().as_millis(),
            "quit"
        );
    }
    ran.map_err(anyhow::Error::msg)?;
    Ok(())
}
