//! The browser entry (web plan W-1, §3.2): the same app, state, views and motion as the window,
//! drawn by Rune's WebGPU web target into its full-viewport canvas.
//!
//! - No threads: the bridge loop is a task on the page's event loop ([`crate::net::rt`]).
//! - The backend is the server the page came from ([`BackendKind::Remote`]); until the web
//!   backend lands (lane W2b) that plays the in-process sim.
//! - Preferences live in `localStorage` ([`crate::prefs`]); the art is fetched from
//!   [`asset_base`] (S2).
//! - A failed start hands the page to the classic site through the loader's
//!   `globalThis.euchreFallback("start")` ([`fallback`], S3).

use std::sync::OnceLock;

use rune::App;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::app::{self, AppState, UrlMode};
use crate::net::BackendKind;
use crate::net::bridge::{Bridge, BridgeConfig, BridgeThread, UiSink};
use crate::prefs::PrefsStore;
use crate::url_route::{self, Deep};
use crate::web_history;

/// The global the loader sets before it imports `app.js`: `{art: "/_rune/immutable/art-<h>/"}`.
pub const LOADER_CONFIG: &str = "__EUCHRE_RUNE__";

/// The global the loader defines to send the browser to the classic site (S3).
pub const FALLBACK_FN: &str = "euchreFallback";

/// The art base used when the loader set none (a console warning says so).
pub const FALLBACK_ART_BASE: &str = "/_rune/art/";

/// Starts the app when the module loads (`wasm_bindgen(start)`).
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| console_error(&format!("euchre: {info}"))));
    let prefs_store = PrefsStore::platform();
    let prefs = prefs_store
        .as_ref()
        .map(PrefsStore::load)
        .unwrap_or_default();
    let backend = BackendKind::Remote.build();
    let backend_name = backend.name();
    let (bridge, intents) = Bridge::channel();

    let spec = crate::spec();
    #[cfg(feature = "web-proof")]
    let spec = crate::web_probe::wrap(spec);
    // Rune's web target refuses `on_ready` (a native window hook), so the intent the window
    // sends on its first frame (`Identify`) goes out as the state is built; the bridge queues it
    // until its loop starts.
    let app = App::new(spec).init(move || {
        let mut state = AppState::new(prefs, Some(bridge), backend_name);
        state.on_ready();
        state.open_deep(web_history::deep_here(), UrlMode::Replace, None);
        state
    });
    let handle = app.handle();
    web_history::listen(handle.clone());
    let router = handle.clone();
    let app = app.same_tab_router(move |url| route_link(&router, url));
    let sink =
        UiSink::new(move |reply| handle.update(move |s: &mut AppState, tx| s.receive(reply, tx)));
    if let Err(err) = BridgeThread::spawn(
        BridgeConfig {
            backend,
            sink,
            prefs: prefs_store,
            quit_on_signals: false,
        },
        intents,
    ) {
        console_error(&format!("euchre: the network bridge did not start: {err}"));
    }

    let ran = app
        .on_visibility(app::on_visibility)
        .on_quit(app::on_quit)
        .try_run();
    if let Err(err) = ran {
        console_error(&format!("euchre: the app did not start: {err}"));
        fallback("start");
    }
}

/// Routes a same-origin link in the app (`history.pushState` and a screen); false for the pages
/// the app does not own (the homepage, downloads), which the browser loads.
fn route_link(handle: &rune::AppHandle<AppState>, url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path_and_query = rest.find('/').map_or("/", |i| &rest[i..]);
    let path_and_query = path_and_query.split('#').next().unwrap_or("/");
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let deep = url_route::parse(path, query);
    if matches!(deep, Deep::Home | Deep::Unknown(_)) {
        return false;
    }
    handle.update(move |s: &mut AppState, tx| s.open_deep(deep, UrlMode::Push, Some(tx)));
    true
}

/// The absolute URL prefix the art lives under, ending in `/`: the loader's
/// `globalThis.__EUCHRE_RUNE__.art`, read once (S2). Without it, [`FALLBACK_ART_BASE`].
pub fn asset_base() -> &'static str {
    static BASE: OnceLock<String> = OnceLock::new();
    BASE.get_or_init(|| match loader_config("art").filter(|b| !b.is_empty()) {
        Some(mut base) => {
            if !base.ends_with('/') {
                base.push('/');
            }
            base
        }
        None => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "euchre: no {LOADER_CONFIG}.art on this page; art from {FALLBACK_ART_BASE}"
            )));
            FALLBACK_ART_BASE.to_owned()
        }
    })
}

/// Sends the browser to the classic site: calls the loader's `globalThis.euchreFallback(why)`
/// (it sets the `euchre_ui=classic-auto` cookie and reloads, or swaps the page in place). A
/// page without the loader only logs.
pub fn fallback(why: &str) {
    let f = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str(FALLBACK_FN))
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    match f {
        Some(f) => {
            if let Err(err) = f.call1(&JsValue::NULL, &JsValue::from_str(why)) {
                web_sys::console::error_2(&JsValue::from_str("euchre: fallback failed"), &err);
            }
        }
        None => console_error(&format!("euchre: no {FALLBACK_FN} on this page ({why})")),
    }
}

/// `globalThis.__EUCHRE_RUNE__[key]` as a string, when the loader set it.
fn loader_config(key: &str) -> Option<String> {
    let config = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str(LOADER_CONFIG)).ok()?;
    if !config.is_object() {
        return None;
    }
    js_sys::Reflect::get(&config, &JsValue::from_str(key))
        .ok()?
        .as_string()
}

/// One line on the page's console as an error.
fn console_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}
