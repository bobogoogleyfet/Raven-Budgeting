//! Errors, flattened to strings at the boundary.
//!
//! The same choice the Tauri shell makes, for the same reason: `CoreError`'s
//! messages are written to be read by a person, so the frontend passes them
//! through rather than inventing a parallel taxonomy. `flutter_rust_bridge`
//! turns `Result<T, String>` into a Dart `Future<T>` that throws.

pub(crate) fn to_message(e: impl std::fmt::Display) -> String {
    e.to_string()
}
