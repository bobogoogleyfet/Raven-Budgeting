//! The open database, and the path it lives at.
//!
//! Path resolution lives here, exactly as it lives in `accounting-cli` and in
//! the CLI. `accounting-core` is handed a path and never discovers one
//! — boundary rule 1, which exists because path-discovery crates return
//! meaningless results on Android. The rules below are copied from those two
//! frontends deliberately: all three must land on the same `pacc.db`, and a
//! fourth answer would read to the user as data loss. The demo build is the
//! one deliberate fourth answer — it must never land on that file.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use accounting_core::db::{Db, seed_defaults};
use accounting_core::service::rates as rates_service;

use crate::error::to_message;

/// The handle, opened once and kept for the life of the process.
///
/// `rusqlite::Connection` is `Send` but not `Sync`, so the mutex is required
/// rather than defensive: one process, one database, opened once.
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
///
/// `demo` is the `PACC_DEMO` build flag, which only Dart can see: it is a
/// `--dart-define`, compiled into the Dart snapshot, and this library is built
/// by Cargo without it. `database_path` says what it changes.
pub fn open_database(app_data_dir: Option<String>, demo: bool) -> Result<String, String> {
    if let Some(open) = HANDLE.get() {
        // A hot restart re-runs Dart's `main()` against a library that is
        // still loaded. Re-opening would be wrong, not merely wasteful.
        log::info!("database already open at {}", open.path.display());
        return Ok(open.path.display().to_string());
    }

    let path = database_path(app_data_dir, demo, |key| {
        std::env::var_os(key).filter(|v| !v.is_empty())
    })?;

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
/// is how the CLI resolves it there too.
///
/// The demo is a separate app and gets a separate answer on desktop, where
/// nothing else would keep the two apart: `personal-accounting-demo` beside
/// the real directory, and `$PACC_DEMO_DB` in place of `$PACC_DB`. Not
/// `$PACC_DB` as well, because that variable exists to name the real books —
/// it is how the CLI and the app are pointed at the same file, so a user who
/// exports it in a shell profile has made it the real default, and a demo that
/// honoured it would open that file and let a demonstration write into it. On
/// mobile the per-app directory already belongs to the demo, because the demo
/// is installed under a package and bundle id of its own.
///
/// `env` is the environment, passed in so the rules can be tested without
/// mutating the process's own — which every other test thread can see.
fn database_path(
    app_data_dir: Option<String>,
    demo: bool,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, String> {
    if let Some(dir) = app_data_dir.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("pacc.db"));
    }
    let (explicit, dir) = if demo {
        ("PACC_DEMO_DB", "personal-accounting-demo")
    } else {
        ("PACC_DB", "personal-accounting")
    };
    if let Some(path) = env(explicit) {
        return Ok(PathBuf::from(path));
    }
    let base = match env("XDG_DATA_HOME") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(
            env("HOME")
                .ok_or_else(|| format!("neither ${explicit}, $XDG_DATA_HOME nor $HOME is set"))?,
        )
        .join(".local")
        .join("share"),
    };
    Ok(base.join(dir).join("pacc.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment holding exactly `vars`, and nothing from the process's.
    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key| {
            vars.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        }
    }

    fn path(app_data_dir: Option<&str>, demo: bool, vars: &[(&str, &str)]) -> PathBuf {
        database_path(app_data_dir.map(str::to_string), demo, env(vars)).unwrap()
    }

    #[test]
    fn the_real_app_keeps_the_clis_rules() {
        let home = [("HOME", "/home/u")];
        assert_eq!(
            path(None, false, &home),
            PathBuf::from("/home/u/.local/share/personal-accounting/pacc.db")
        );
        assert_eq!(
            path(None, false, &[("HOME", "/home/u"), ("XDG_DATA_HOME", "/x")]),
            PathBuf::from("/x/personal-accounting/pacc.db")
        );
        assert_eq!(
            path(
                None,
                false,
                &[("PACC_DB", "/books.db"), ("XDG_DATA_HOME", "/x")]
            ),
            PathBuf::from("/books.db")
        );
    }

    #[test]
    fn the_demo_lives_beside_the_real_directory_not_in_it() {
        assert_eq!(
            path(None, true, &[("HOME", "/home/u")]),
            PathBuf::from("/home/u/.local/share/personal-accounting-demo/pacc.db")
        );
        assert_eq!(
            path(None, true, &[("HOME", "/home/u"), ("XDG_DATA_HOME", "/x")]),
            PathBuf::from("/x/personal-accounting-demo/pacc.db")
        );
    }

    #[test]
    fn the_demo_does_not_follow_pacc_db_to_the_real_books() {
        let real = "/home/u/.local/share/personal-accounting/pacc.db";
        assert_eq!(
            path(None, true, &[("PACC_DB", real), ("HOME", "/home/u")]),
            PathBuf::from("/home/u/.local/share/personal-accounting-demo/pacc.db")
        );
    }

    #[test]
    fn the_demo_takes_its_own_explicit_path() {
        let vars = [
            ("PACC_DEMO_DB", "/tmp/demo/pacc.db"),
            ("PACC_DB", "/books.db"),
        ];
        assert_eq!(path(None, true, &vars), PathBuf::from("/tmp/demo/pacc.db"));
        // And the real app does not take the demo's.
        assert_eq!(path(None, false, &vars), PathBuf::from("/books.db"));
    }

    #[test]
    fn the_platform_directory_wins_on_mobile() {
        let vars = [("PACC_DB", "/books.db"), ("PACC_DEMO_DB", "/demo.db")];
        for demo in [false, true] {
            assert_eq!(
                path(Some("/data/app/files"), demo, &vars),
                PathBuf::from("/data/app/files/pacc.db")
            );
        }
    }

    #[test]
    fn with_no_home_at_all_it_says_which_variable_would_have_done() {
        let err = database_path(None, true, env(&[])).unwrap_err();
        assert!(err.contains("$PACC_DEMO_DB"), "{err}");
        let err = database_path(None, false, env(&[])).unwrap_err();
        assert!(err.contains("$PACC_DB"), "{err}");
    }
}
