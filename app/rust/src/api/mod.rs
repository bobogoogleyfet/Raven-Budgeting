//! Everything Dart can call.
//!
//! # Every function is `async`, and almost none of them awaits anything
//!
//! flutter_rust_bridge runs an ordinary `fn` on a pool of threads. In a
//! browser those threads are Web Workers sharing the module's memory, which
//! needs `SharedArrayBuffer`, which a page only gets when it is served with
//! cross-origin isolation headers — and GitHub Pages, where the web build
//! lives, cannot send them. An `async fn` is run by the bridge's executor
//! instead: the page's own event loop on the web, tokio on a desktop or phone.
//! The pool is created the first time an ordinary `fn` is called, so a single
//! one of them is enough to break the web build, and only the web build.
//! `every_bridged_function_is_async_or_sync` below holds the line.
//!
//! On a desktop or a phone this is the same arrangement it replaced. tokio's
//! runtime has one worker thread per core, as the pool did, and each body
//! still blocks on SQLite exactly as it did there; `async` is how a function
//! reaches that thread, not a promise that it yields while it runs.
//!
//! The one exception is `#[frb(sync)]`, which runs on the caller's own thread
//! and touches neither the pool nor the executor.

pub mod budget;
pub mod categories;
pub mod db;
pub mod entry;
pub mod init;
pub mod rates;
pub mod sync;
pub mod transactions;

#[cfg(test)]
mod tests {
    /// The generated dispatcher, which is the ground truth for how each
    /// function is run: `wrap_normal` is the thread pool, `wrap_async` the
    /// executor and `wrap_sync` the caller's thread.
    const GENERATED: &str = include_str!("../frb_generated.rs");

    #[test]
    fn every_bridged_function_is_async_or_sync() {
        assert!(
            !GENERATED.contains("FLUTTER_RUST_BRIDGE_HANDLER.wrap_normal"),
            "a bridged function is an ordinary `fn`, which the web build cannot \
             run — make it `pub async fn` (or `#[frb(sync)]` if it must answer \
             on the UI thread), then regenerate the bindings"
        );
        assert!(GENERATED.contains("FLUTTER_RUST_BRIDGE_HANDLER.wrap_async"));
    }
}
