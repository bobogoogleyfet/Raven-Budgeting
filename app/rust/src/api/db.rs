//! The open database, and the path it lives at.
//!
//! Path resolution lives here, exactly as it lives in `accounting-cli` and in
//! the Tauri shell. `accounting-core` is handed a path and never discovers one
//! — boundary rule 1, which exists because path-discovery crates return
//! meaningless results on Android. The rules below are copied from those two
//! frontends deliberately: all three must land on the same `pacc.db`, and a
//! fourth answer would read to the user as data loss.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use accounting_core::db::{Db, seed_defaults};
use accounting_core::service::rates as rates_service;

use crate::error::to_message;

/// The handle, opened once and kept for the life of the process.
///
/// `rusqlite::Connection` is `Send` but not `Sync`, so the mutex is required
/// rather than defensive — the same shape the Tauri shell's `AppState` has.
/// SQLite serialises at the file level anyway, and the core is synchronous.
struct Handle {
    db: Mutex<Db>,
    path: PathBuf,
}

static HANDLE: OnceLock<Handle> = OnceLock::new();

/// Open (creating if absent), migrate and seed the database.
///
/// Returns the resolved path, so the UI can show which file it is looking at.
///
/// Deliberately not `#[frb(sync)]`: every generated function is a `Future`
/// that runs on a worker isolate, which is what makes a blocking SQLite call
/// acceptable to a UI. Marking it `sync` would put the migration on the UI
/// isolate instead.
///
/// `app_data_dir` is the platform's per-app directory, which only Dart can ask
/// for (`path_provider`) and Rust has no business guessing. Desktop passes
/// `null` and the XDG rules apply.
pub fn open_database(app_data_dir: Option<String>) -> Result<String, String> {
    if let Some(open) = HANDLE.get() {
        // A hot restart re-runs Dart's `main()` against a library that is
        // still loaded. Re-opening would be wrong, not merely wasteful.
        log::info!("database already open at {}", open.path.display());
        return Ok(open.path.display().to_string());
    }

    let path = database_path(app_data_dir)?;

    // `Db::open` takes the path it is given and will not create the directory
    // above it — that is a filesystem policy decision, and policy belongs to
    // the frontend.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(to_message)?;
    }

    let db = Db::open(&path).map_err(to_message)?;
    seed_defaults(&db).map_err(to_message)?;
    rates_service::ensure_installed(&db).map_err(to_message)?;
    accounting_core::adopt_published_areas(&db).map_err(to_message)?;
    log::info!("opened {}", path.display());

    let open = HANDLE.get_or_init(|| Handle {
        db: Mutex::new(db),
        path,
    });
    Ok(open.path.display().to_string())
}

/// The open database, for the duration of one call.
pub(crate) fn locked() -> Result<MutexGuard<'static, Db>, String> {
    let open = HANDLE
        .get()
        .ok_or_else(|| "the database is not open yet".to_string())?;
    open.db.lock().map_err(to_message)
}

/// `$PACC_DB`, else `$XDG_DATA_HOME`, else `~/.local/share` — the CLI's rules,
/// in the CLI's order. On mobile the platform directory wins outright, which
/// is how the Tauri shell resolves it there too.
fn database_path(app_data_dir: Option<String>) -> Result<PathBuf, String> {
    if let Some(dir) = app_data_dir.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("pacc.db"));
    }
    if let Some(explicit) = non_empty_env("PACC_DB") {
        return Ok(PathBuf::from(explicit));
    }
    let base = match non_empty_env("XDG_DATA_HOME") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(non_empty_env("HOME").ok_or_else(|| {
            "neither $PACC_DB, $XDG_DATA_HOME nor $HOME is set".to_string()
        })?)
        .join(".local")
        .join("share"),
    };
    Ok(base.join("personal-accounting").join("pacc.db"))
}

fn non_empty_env(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}
