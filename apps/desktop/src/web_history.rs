//! The browser's address bar and back/forward (web plan §3.5): `history.pushState` and
//! `replaceState` for the screen changes, a `popstate` listener that opens the screen the URL
//! now names, and the `euchre_ui=classic` cookie. Native: every call here does nothing.

use rune::AppHandle;

use crate::app::AppState;
use crate::url_route::Deep;

/// `euchre_ui=classic` for 400 days, the cookie the server reads to pick the Svelte shell.
pub const CLASSIC_COOKIE: &str = "euchre_ui=classic; Max-Age=34560000; Path=/; SameSite=Lax";

/// Whether this is the browser build.
pub const IS_WEB: bool = cfg!(target_arch = "wasm32");

/// `history.pushState` to `path`.
pub fn push(path: &str) {
    #[cfg(target_arch = "wasm32")]
    wasm::state(path, false);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = path;
}

/// `history.replaceState` to `path`.
pub fn replace(path: &str) {
    #[cfg(target_arch = "wasm32")]
    wasm::state(path, true);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = path;
}

/// The page's current path and query (`/` and empty natively).
pub fn location() -> (String, String) {
    #[cfg(target_arch = "wasm32")]
    return wasm::location();
    #[cfg(not(target_arch = "wasm32"))]
    ("/".to_owned(), String::new())
}

/// Opens the screen the URL names on every back or forward.
pub fn listen(handle: AppHandle<AppState>) {
    #[cfg(target_arch = "wasm32")]
    wasm::listen(handle);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = handle;
}

/// Sets the classic-site cookie and reloads, so the server sends the Svelte shell.
pub fn use_classic_table() {
    #[cfg(target_arch = "wasm32")]
    wasm::classic();
}

/// What a popstate or an initial load asks the app to open.
pub fn deep_here() -> Deep {
    let (path, query) = location();
    crate::url_route::parse(&path, &query)
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use rune::AppHandle;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    use super::CLASSIC_COOKIE;
    use crate::app::{AppState, UrlMode};

    pub fn state(path: &str, replace: bool) {
        let Some(history) = web_sys::window().and_then(|w| w.history().ok()) else {
            return;
        };
        let _ = if replace {
            history.replace_state_with_url(&JsValue::NULL, "", Some(path))
        } else {
            history.push_state_with_url(&JsValue::NULL, "", Some(path))
        };
    }

    pub fn location() -> (String, String) {
        let Some(loc) = web_sys::window().map(|w| w.location()) else {
            return ("/".to_owned(), String::new());
        };
        (
            loc.pathname().unwrap_or_else(|_| "/".to_owned()),
            loc.search().unwrap_or_default(),
        )
    }

    pub fn listen(handle: AppHandle<AppState>) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let on_pop = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
            let deep = super::deep_here();
            handle.update(move |s: &mut AppState, tx| s.open_deep(deep, UrlMode::Silent, Some(tx)));
        });
        // `addEventListener` through Reflect: no extra web-sys features for one listener.
        let add = js_sys::Reflect::get(&window, &JsValue::from_str("addEventListener"))
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        if let Some(add) = add {
            let _ = add.call2(
                &window,
                &JsValue::from_str("popstate"),
                on_pop.as_ref().unchecked_ref(),
            );
        }
        on_pop.forget();
    }

    pub fn classic() {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Some(document) = window.document() {
            let _ = js_sys::Reflect::set(
                &document,
                &JsValue::from_str("cookie"),
                &JsValue::from_str(CLASSIC_COOKIE),
            );
        }
        let _ = window.location().reload();
    }
}
