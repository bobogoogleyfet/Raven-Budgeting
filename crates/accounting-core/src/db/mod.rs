//! SQLite storage: connection setup, migrations, seed data, integrity checks.
//!
//! # This module never discovers a path
//!
//! [`Db::open`] takes the path it is given and nothing else. There is no
//! `dirs`, no `directories`, no `$HOME` lookup anywhere in this crate. The
//! caller decides: the CLI resolves `--db` (defaulting to an XDG path), and
//! the Flutter app passes what Dart's `path_provider` reports. Path-discovery
//! crates return meaningless answers on Android — the app's real data
//! directory is only obtainable through the Java `Context` — so a core that
//! discovers its own path is a core that has to be rewritten for the second
//! platform.
//!
//! # Connection setup order is load-bearing
//!
//! Two of the four pragmas below are traps rather than preferences:
//!
//! * `journal_mode = WAL` **must** go through [`rusqlite::Connection::pragma_update`].
//!   `execute()` fails with `ExecuteReturnedResults` because this particular
//!   PRAGMA reports the resulting mode as a row.
//! * `foreign_keys = ON` **must** be set before [`migrations::MIGRATIONS`]
//!   runs. It is a no-op inside a transaction, and `to_latest()` opens one, so
//!   setting it afterwards leaves the connection with foreign keys *off* and
//!   every `REFERENCES` clause in the schema decorative.
//!
//! # Aggregate invariants
//!
//! `SUM(txn_split.amount_cents) = txn.total_cents` cannot be a CHECK
//! constraint — CHECK cannot aggregate — and an AFTER INSERT trigger is worse
//! than nothing, because it fires between the first split and the second and
//! rejects a legal edit. The strategy is instead: row-local CHECKs in the
//! schema, one single write path that replaces a whole transaction aggregate
//! inside one SQLite transaction, and [`integrity::check_all`] as the audit.

pub mod integrity;
pub mod migrations;
pub mod seed;
mod types;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction};
use uuid::Uuid;

use crate::error::{CoreError, Result};

pub use integrity::{InvariantViolation, Severity, check_all};
pub use migrations::{MIGRATION_SOURCES, MIGRATIONS, SCHEMA_VERSION};
pub use seed::seed_defaults;

/// How long a writer waits for a competing writer before giving up.
///
/// Without this, a second connection touching the file — `pacc doctor` run
/// while the GUI is open — fails instantly with `SQLITE_BUSY` instead of
/// waiting the few milliseconds a write actually takes.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// An open, migrated database.
///
/// Not `Sync`: `rusqlite::Connection` is single-threaded by construction. That
/// is the right shape here — the core is fully synchronous, and SQLite's own
/// serialization is at the file level anyway.
pub struct Db {
    conn: Connection,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field(
                "path",
                &self.path.as_deref().unwrap_or(Path::new(":memory:")),
            )
            .finish_non_exhaustive()
    }
}

impl Db {
    /// Open (creating if absent) the database at `path`, then migrate it to
    /// the latest schema.
    ///
    /// The parent directory must already exist — creating it is a filesystem
    /// policy decision that belongs to the caller, which is also the only
    /// layer that knows whether the path came from a flag, an app data dir, or
    /// a test.
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path)?;
        let mut db = Db {
            conn,
            path: Some(path.to_path_buf()),
        };
        db.configure(true)?;
        Ok(db)
    }

    /// Open a private in-memory database, migrated to the latest schema.
    ///
    /// For tests and for `pacc calc`-style dry runs. The contents vanish when
    /// the [`Db`] drops.
    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        let mut db = Db { conn, path: None };
        db.configure(false)?;
        Ok(db)
    }

    /// The order of these statements is explained in the module docs. Do not
    /// rearrange them.
    fn configure(&mut self, on_disk: bool) -> Result<()> {
        if on_disk {
            // WAL lets a reader (a report, `pacc doctor`) run while a write is
            // in flight, and survives a crash mid-write. An in-memory database
            // has no journal file to write, so SQLite silently keeps "memory"
            // mode; asking for it there is noise, not an error.
            //
            // pragma_update, not execute: this PRAGMA returns a row.
            self.conn.pragma_update(None, "journal_mode", "WAL")?;
        }

        // Before to_latest(). Inside a transaction this is ignored outright.
        self.conn.pragma_update(None, "foreign_keys", "ON")?;

        // NORMAL is the documented-safe pairing with WAL: a commit no longer
        // fsyncs, but the WAL still guarantees no corruption on power loss —
        // at most the last few committed transactions are lost. FULL costs an
        // fsync per receipt for a durability window nobody types fast enough
        // to notice.
        self.conn.pragma_update(None, "synchronous", "NORMAL")?;

        self.conn.busy_timeout(BUSY_TIMEOUT)?;

        MIGRATIONS.to_latest(&mut self.conn)?;

        // Before anything can write, and that ordering is the whole point.
        // The change-log triggers stamp every write with this device's name,
        // and the seed runs on the very next call — so a database that named
        // itself later would carry a pile of unattributable entries that no
        // bundle could ever send. Minted here rather than in the migration
        // because a migration runs inside a transaction with no good source of
        // randomness.
        //
        // Conditional, so reopening keeps the name: it identifies the
        // *replica*, and peers track how far along it is by that name.
        self.conn.execute(
            "UPDATE sync_clock SET device = ?1 WHERE id = 1 AND device = ''",
            [new_uuid()],
        )?;
        Ok(())
    }

    /// The underlying connection, for repository code.
    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The file this database lives in, or `None` for an in-memory one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run `f` inside a single SQLite transaction, committing on `Ok` and
    /// rolling back on `Err`.
    ///
    /// This is the mechanism that keeps the aggregate invariants true: a
    /// transaction and all of its splits and line items are written as one
    /// unit, so no reader — and no integrity check — ever observes a receipt
    /// whose splits are half-replaced.
    ///
    /// Takes `&self` rather than `&mut self` (via
    /// [`rusqlite::Connection::unchecked_transaction`]) so that repository
    /// functions can borrow the database immutably. The "unchecked" part is
    /// that nesting is not prevented at compile time; a nested call returns
    /// SQLite's "cannot start a transaction within a transaction" error
    /// instead.
    pub fn with_tx<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T>,
    {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Write a consistent copy of the database to `dest` using `VACUUM INTO`.
    ///
    /// `VACUUM INTO` rather than a file copy or the backup API: it produces
    /// **one** defragmented file, with the WAL already folded in. Copying
    /// `pacc.db` alone in WAL mode gives you a backup that is missing every
    /// committed write still sitting in `pacc.db-wal` — which looks fine until
    /// the day you need it. It also takes a read lock rather than blocking
    /// writers.
    ///
    /// `dest` must not already exist; SQLite refuses to overwrite, which is
    /// the correct default for a backup command.
    pub fn backup(&self, dest: &Path) -> Result<()> {
        let dest = path_str(dest)?;
        self.conn.execute("VACUUM INTO ?1", [dest])?;
        Ok(())
    }

    /// `PRAGMA user_version` — the number of migrations applied.
    pub fn user_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// `PRAGMA journal_mode` — `"wal"` on disk, `"memory"` in memory.
    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }

    /// `PRAGMA foreign_keys`. Must be true, or the schema's `REFERENCES`
    /// clauses are documentation rather than constraints.
    pub fn foreign_keys_enabled(&self) -> Result<bool> {
        let v: i64 = self
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        Ok(v != 0)
    }
}

/// A fresh stable identifier for a new row, as a `TEXT` uuid.
///
/// Version 7: the leading 48 bits are a millisecond timestamp, so uuids sort
/// in creation order and index without the random-insert page churn a v4 uuid
/// causes. This reads the system clock — that is fine, and is not the thing
/// rule 3 forbids. What rule 3 forbids is *date logic* derived from the clock
/// (`Local::now().date()`), which is untestable and pulls chrono's timezone
/// lookup, i.e. JNI on Android. `SystemTime` has neither problem.
#[must_use]
pub fn new_uuid() -> String {
    Uuid::now_v7().to_string()
}

/// SQLite's API is UTF-8; a path that is not valid UTF-8 cannot be passed to
/// it, and silently lossy-converting it would open or clobber the wrong file.
fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| CoreError::Invalid(format!("path {} is not valid UTF-8", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_opens_migrated_with_foreign_keys_on() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            usize::try_from(db.user_version().unwrap()).unwrap(),
            SCHEMA_VERSION
        );
        assert!(db.foreign_keys_enabled().unwrap());
        assert_eq!(db.path(), None);
    }

    #[test]
    fn on_disk_opens_in_wal_and_reopening_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pacc.db");

        {
            let db = Db::open(&path).unwrap();
            assert_eq!(db.journal_mode().unwrap(), "wal");
            assert!(db.foreign_keys_enabled().unwrap());
            assert_eq!(db.path(), Some(path.as_path()));
        }

        // Running the migrations a second time must be a no-op, not an error:
        // every launch of the app takes this path.
        let db = Db::open(&path).unwrap();
        assert_eq!(
            usize::try_from(db.user_version().unwrap()).unwrap(),
            SCHEMA_VERSION
        );
    }

    #[test]
    fn foreign_keys_are_actually_enforced() {
        let db = Db::open_in_memory().unwrap();
        let err = db
            .conn()
            .execute(
                "INSERT INTO category (uuid, group_id, name) VALUES ('u', 999, 'x')",
                [],
            )
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("foreign key"),
            "expected an FK violation, got: {err}"
        );
    }

    #[test]
    fn every_table_is_strict() {
        let db = Db::open_in_memory().unwrap();
        let mut stmt = db
            .conn()
            .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        assert!(rows.len() >= 11, "expected 11 tables, found {}", rows.len());
        for (name, sql) in rows {
            assert!(
                sql.to_uppercase().contains("STRICT"),
                "table `{name}` is not STRICT"
            );
        }
    }

    #[test]
    fn a_text_amount_is_rejected_rather_than_stored() {
        // The entire reason for STRICT. In a legacy table an INTEGER column
        // has INTEGER *affinity*, which is advisory: '12.34' would be coerced
        // to the REAL 12.34 and stored, and 'twelve' would be stored verbatim
        // as TEXT. Every later SUM() over the column would then be silently
        // wrong. STRICT turns both into errors at write time.
        let db = Db::open_in_memory().unwrap();
        seed_defaults(&db).unwrap();

        for bad in ["'12.34'", "'twelve'", "12.34"] {
            let err = db
                .conn()
                .execute(
                    &format!(
                        "INSERT INTO txn (uuid, txn_date, subtotal_cents, tax_cents, total_cents)
                         VALUES ('u', '2026-08-12', {bad}, 0, {bad})"
                    ),
                    [],
                )
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("subtotal_cents"),
                "expected {bad} to be refused by STRICT, got: {err}"
            );
        }
    }

    /// `ON DELETE RESTRICT` blocks the delete, but SQLite reports it with
    /// extended code `SQLITE_CONSTRAINT_TRIGGER` (1811), *not*
    /// `SQLITE_CONSTRAINT_FOREIGNKEY` (787), because RESTRICT is implemented
    /// as an implicit trigger program. Only the default `NO ACTION` yields
    /// 787. The human-readable message is "FOREIGN KEY constraint failed" in
    /// both cases, which is exactly why this trips people up.
    ///
    /// Asserting on the message rather than the numeric code is the portable
    /// choice; this test pins the behaviour so the distinction is discovered
    /// here rather than in a confusing test failure elsewhere.
    #[test]
    fn on_delete_restrict_reports_a_trigger_constraint_code() {
        const SQLITE_CONSTRAINT_TRIGGER: i32 = 1811;

        let db = Db::open_in_memory().unwrap();
        seed_defaults(&db).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO txn (id, uuid, txn_date, subtotal_cents, tax_cents, total_cents)
                 VALUES (1, 't-1', '2026-08-12', 1000, 0, 1000);
                 INSERT INTO txn_split (id, uuid, txn_id, category_id,
                                        subtotal_cents, tax_cents, amount_cents)
                 VALUES (1, 's-1', 1, (SELECT id FROM category WHERE name = 'Groceries'),
                         1000, 0, 1000);",
            )
            .unwrap();

        let err = db
            .conn()
            .execute("DELETE FROM category WHERE name = 'Groceries'", [])
            .unwrap_err();

        let rusqlite::Error::SqliteFailure(code, msg) = &err else {
            panic!("expected a constraint failure, got {err}");
        };
        assert_eq!(code.extended_code, SQLITE_CONSTRAINT_TRIGGER);
        assert_eq!(msg.as_deref(), Some("FOREIGN KEY constraint failed"));
    }

    #[test]
    fn with_tx_rolls_back_on_error() {
        let db = Db::open_in_memory().unwrap();
        let before: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM category_group", [], |r| r.get(0))
            .unwrap();

        let res: Result<()> = db.with_tx(|tx| {
            tx.execute(
                "INSERT INTO category_group (uuid, name) VALUES ('u', 'Doomed')",
                [],
            )?;
            Err(CoreError::Invalid("nope".into()))
        });
        assert!(res.is_err());

        let after: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM category_group", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn backup_writes_one_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("pacc.db");
        let dest = dir.path().join("backup.db");

        let db = Db::open(&src).unwrap();
        seed_defaults(&db).unwrap();
        db.backup(&dest).unwrap();

        // One file, not the three WAL leaves behind.
        assert!(dest.exists());
        assert!(!dir.path().join("backup.db-wal").exists());
        assert!(!dir.path().join("backup.db-shm").exists());

        let copy = Db::open(&dest).unwrap();
        let n: i64 = copy
            .conn()
            .query_row("SELECT count(*) FROM category", [], |r| r.get(0))
            .unwrap();
        assert!(n > 0);

        // Refusing to overwrite is the correct default for a backup.
        assert!(db.backup(&dest).is_err());
    }

    #[test]
    fn uuids_are_distinct_and_time_ordered() {
        let a = new_uuid();
        let b = new_uuid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(Uuid::parse_str(&a).unwrap().get_version_num(), 7);
    }
}
