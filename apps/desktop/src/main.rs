//! `euchre`: the desktop app. Everything lives in the library so tests share it.
// A release build on Windows is a GUI app: no console window opens beside it. Debug builds keep
// the console for logs and `--scene` output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    // Before any thread, runtime, logger or GPU work: keep loopback off any proxy (it may
    // replace this process with itself, same pid).
    euchre_desktop::startup::reexec_with_no_proxy();
    euchre_desktop::run()
}

/// The browser build is the library's cdylib (`web::start`); this binary does nothing there.
#[cfg(target_arch = "wasm32")]
fn main() {}
