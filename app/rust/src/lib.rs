//! Dart bindings for `accounting-core`.
//!
//! This crate contains no business logic, on purpose. It is the third frontend
//! beside `accounting-cli` and `accounting-tauri`, not a layer underneath
//! them: logic placed here would have to be written again for every platform,
//! which is the thing the core exists to prevent.

pub mod api;
mod error;
mod frb_generated;
