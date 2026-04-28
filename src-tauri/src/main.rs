// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "desktop")]
fn main() {
    moneypenny_lib::run()
}

// Stub binary for `cargo test --no-default-features`. Without `desktop`
// the runtime can't actually start, but the binary target still has to
// link, and tests don't invoke it.
#[cfg(not(feature = "desktop"))]
fn main() {
    eprintln!("moneypenny built without `desktop` feature; nothing to run.");
}
