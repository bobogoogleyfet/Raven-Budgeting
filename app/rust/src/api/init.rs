//! Library initialisation.

/// Runs once when the Dart side loads the library.
///
/// Idempotent on purpose. A Flutter hot restart resets the Dart isolate but
/// not the loaded `.so`, so this runs again against Rust globals that were
/// never cleared — `try_init` returns `Err` on the second call where `init`
/// would panic.
#[flutter_rust_bridge::frb(init)]
pub async fn init_app() {
    // `setup_default_user_utils()` installs a log sink via
    // `setup_log_to_console`, which has arms for Android, iOS/macOS and wasm
    // and then ends in `// TODO add more platforms`. On Linux and Windows it
    // is a no-op, so without our own logger every `log::` call from Rust
    // vanishes on the platform we develop on. Not in a browser, where the
    // wasm arm is the logger that reaches the console and this one would take
    // its place.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    let _ = env_logger::try_init();
    flutter_rust_bridge::setup_default_user_utils();
}
