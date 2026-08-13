//! Domain model, persistence, and export for the personal accounting app.
//!
//! # Boundary rules
//!
//! This crate is the product. Everything else (CLI, Tauri desktop, a future
//! Android UI) is a rendering layer over it. Four rules keep it that way, and
//! each one maps to a concrete mobile failure mode:
//!
//! 1. **No UI, no path discovery.** This crate never names `tauri`, `tokio`,
//!    `dirs`, or `directories`. [`db::Db::open`] takes a path it is *given* —
//!    the caller supplies it (from `app.path().app_data_dir()` on Tauri, or a
//!    flag on the CLI). Path-discovery crates return meaningless results on
//!    Android.
//! 2. **Fully synchronous.** SQLite is a local file; there is nothing to await.
//!    Async would drag in `tokio`/`mio` for no benefit.
//! 3. **Never reads the clock.** Every date-dependent function takes
//!    `today: NaiveDate` explicitly. This avoids `chrono`'s `iana-time-zone`
//!    JNI dependency on Android *and* makes every test deterministic without
//!    clock mocking.
//! 4. **All money math lives here.** UIs send a draft and render the returned
//!    preview; they never do arithmetic. The moment a frontend computes
//!    `total - subtotal`, floats are back and the logic has to be rewritten
//!    per platform.

#![doc(html_no_source)]
