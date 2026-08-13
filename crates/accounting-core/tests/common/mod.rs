//! Shared fixtures for the M2 storage suite.
//!
//! Every raw-SQL `INSERT` the storage tests need lives here rather than being
//! spelled out at each call site. The tests are about *behaviour* (does the
//! CHECK fire, does the CASCADE reach the line items); the column list is
//! incidental, and centralising it means a schema change is a one-file edit
//! instead of a suite-wide one.
//!
//! Not every test uses every helper, and Cargo compiles this module separately
//! into each test binary, so unused-code warnings here are noise.
#![allow(dead_code)]

use accounting_core::db::{Db, check_all, new_uuid};
use rusqlite::Connection;

/// Raw connection access, for the tests that must reach *around* any repository
/// layer — corrupting a row on purpose, or asserting a constraint that no safe
/// API would ever let you violate.
pub fn raw(db: &Db) -> &Connection {
    db.conn()
}

/// An in-memory database, migrated but empty.
pub fn empty_db() -> Db {
    Db::open_in_memory().expect("open_in_memory")
}

/// An in-memory database with the default taxonomy seeded.
pub fn seeded_db() -> Db {
    let db = empty_db();
    accounting_core::db::seed_defaults(&db).expect("seed_defaults");
    db
}

// ---------------------------------------------------------------------------
// Fixture inserts
//
// Every table carries `uuid TEXT NOT NULL UNIQUE` — the stable cross-device
// identity — so no helper may omit it.
// ---------------------------------------------------------------------------

pub fn insert_group(c: &Connection, name: &str, sort_order: i64) -> i64 {
    c.execute(
        "INSERT INTO category_group (uuid, name, sort_order) VALUES (?1, ?2, ?3)",
        rusqlite::params![new_uuid(), name, sort_order],
    )
    .unwrap_or_else(|e| panic!("insert category_group {name}: {e}"));
    c.last_insert_rowid()
}

pub fn insert_category(c: &Connection, group_id: i64, name: &str, sort_order: i64) -> i64 {
    c.execute(
        "INSERT INTO category (uuid, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![new_uuid(), group_id, name, sort_order],
    )
    .unwrap_or_else(|e| panic!("insert category {name}: {e}"));
    c.last_insert_rowid()
}

/// A category group plus one category inside it — the minimum taxonomy a split
/// needs to exist.
pub fn insert_group_with_category(c: &Connection, group: &str, category: &str) -> (i64, i64) {
    let g = insert_group(c, group, 100);
    let cat = insert_category(c, g, category, 100);
    (g, cat)
}

pub fn insert_location(c: &Connection, name: &str, tax_rate_ppm: i64, is_home: bool) -> i64 {
    c.execute(
        "INSERT INTO location (uuid, name, tax_rate_ppm, is_home) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![new_uuid(), name, tax_rate_ppm, i64::from(is_home)],
    )
    .unwrap_or_else(|e| panic!("insert location {name}: {e}"));
    c.last_insert_rowid()
}

/// A transaction with a consistent `subtotal + tax = total`.
pub fn insert_txn(c: &Connection, subtotal: i64, tax: i64) -> i64 {
    insert_txn_at_detail(c, subtotal, tax, "grouped")
}

/// As [`insert_txn`], but declaring a detail level. `check_all` cross-checks the
/// declared level against whether line items actually exist, so anything that
/// adds line items must say `"itemized"` here or trip that warning.
pub fn insert_txn_at_detail(c: &Connection, subtotal: i64, tax: i64, detail: &str) -> i64 {
    c.execute(
        "INSERT INTO txn (uuid, txn_date, subtotal_cents, tax_cents, total_cents, detail_level)
         VALUES (?1, '2026-03-14', ?2, ?3, ?4, ?5)",
        rusqlite::params![new_uuid(), subtotal, tax, subtotal + tax, detail],
    )
    .unwrap_or_else(|e| panic!("insert txn: {e}"));
    c.last_insert_rowid()
}

/// One split of `txn_id`, again internally consistent.
pub fn insert_split(c: &Connection, txn_id: i64, category_id: i64, subtotal: i64, tax: i64) -> i64 {
    c.execute(
        "INSERT INTO txn_split (uuid, txn_id, category_id, subtotal_cents, tax_cents, amount_cents)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            new_uuid(),
            txn_id,
            category_id,
            subtotal,
            tax,
            subtotal + tax
        ],
    )
    .unwrap_or_else(|e| panic!("insert txn_split: {e}"));
    c.last_insert_rowid()
}

pub fn insert_line_item(c: &Connection, split_id: i64, description: &str, amount: i64) -> i64 {
    c.execute(
        "INSERT INTO line_item (uuid, split_id, description,
                                subtotal_cents, tax_cents, amount_cents)
         VALUES (?1, ?2, ?3, ?4, 0, ?4)",
        rusqlite::params![new_uuid(), split_id, description, amount],
    )
    .unwrap_or_else(|e| panic!("insert line_item {description}: {e}"));
    c.last_insert_rowid()
}

/// A whole consistent aggregate: one itemized txn, one split covering it, and
/// two line items summing to the split. The starting point for the corruption
/// tests, and the shape `check_all` must call clean.
pub struct Aggregate {
    pub txn_id: i64,
    pub split_id: i64,
    pub category_id: i64,
}

pub fn insert_balanced_aggregate(c: &Connection) -> Aggregate {
    let (_, category_id) = insert_group_with_category(c, "Fixture Group", "Fixture Category");
    let txn_id = insert_txn_at_detail(c, 1084, 0, "itemized");
    let split_id = insert_split(c, txn_id, category_id, 1084, 0);
    insert_line_item(c, split_id, "first", 600);
    insert_line_item(c, split_id, "second", 484);
    Aggregate {
        txn_id,
        split_id,
        category_id,
    }
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

pub fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("count `{sql}`: {e}"))
}

/// Assert that every aggregate invariant still holds.
///
/// The two aggregate invariants cannot be constraints (CHECK cannot
/// aggregate), so they are held by construction in the write path and audited
/// by `check_all`. That audit is only worth anything if it is actually run:
/// every test that writes data ends with this call, so a write path that
/// leaves an orphaned split, a half-replaced aggregate, or a transaction whose
/// splits no longer sum is caught here rather than in a report months later.
#[track_caller]
pub fn assert_clean(db: &Db) {
    let violations = check_all(db).expect("check_all");
    assert!(
        violations.is_empty(),
        "the database must satisfy every aggregate invariant; got {violations:?}"
    );
}

/// A census of every table a transaction write can touch.
///
/// Used by the "this call must not write" tests — preview, `--dry-run`,
/// `repeat_last`, a rejected `save`. Comparing two whole snapshots proves the
/// absence of a write far better than checking the one table the test happens
/// to be thinking about: a `save` that validates late could easily have
/// inserted the merchant before rejecting the splits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowCounts {
    pub txns: i64,
    pub splits: i64,
    pub line_items: i64,
    pub merchants: i64,
    pub locations: i64,
    pub categories: i64,
}

pub fn row_counts(db: &Db) -> RowCounts {
    let c = raw(db);
    RowCounts {
        txns: count(c, "SELECT COUNT(*) FROM txn"),
        splits: count(c, "SELECT COUNT(*) FROM txn_split"),
        line_items: count(c, "SELECT COUNT(*) FROM line_item"),
        merchants: count(c, "SELECT COUNT(*) FROM merchant"),
        locations: count(c, "SELECT COUNT(*) FROM location"),
        categories: count(c, "SELECT COUNT(*) FROM category"),
    }
}

pub fn pragma_string(c: &Connection, pragma: &str) -> String {
    c.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("PRAGMA {pragma}: {e}"))
}

pub fn pragma_i64(c: &Connection, pragma: &str) -> i64 {
    c.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("PRAGMA {pragma}: {e}"))
}

/// Assert that `result` failed, and that the message mentions `needle`.
///
/// Used where the *identity* of the rule matters and only the message carries
/// it — which trigger fired, for instance. Where the class of constraint is
/// what is being asserted, prefer [`assert_constraint`]: SQLite's messages are
/// not a stable API, and this suite has already been bitten once by a `STRICT`
/// violation reported as "cannot store REAL value in INTEGER column …" in one
/// statement and "datatype mismatch" in another.
#[track_caller]
pub fn assert_err_mentions<T: std::fmt::Debug>(
    result: std::result::Result<T, rusqlite::Error>,
    needle: &str,
) {
    match result {
        Ok(v) => panic!("expected an error mentioning {needle:?}, got Ok({v:?})"),
        Err(e) => {
            let msg = e.to_string().to_ascii_lowercase();
            assert!(
                msg.contains(&needle.to_ascii_lowercase()),
                "expected error mentioning {needle:?}, got: {e}"
            );
        }
    }
}

/// Assert that `result` failed with a specific SQLite *extended* result code.
///
/// The extended code is the stable, machine-readable part: a STRICT violation
/// is always `SQLITE_CONSTRAINT_DATATYPE` however SQLite chooses to word it,
/// and a bare `SQLITE_CONSTRAINT` would not distinguish a CHECK from a UNIQUE
/// from a trigger — which is exactly the distinction each of these tests is
/// making.
#[track_caller]
pub fn assert_constraint<T: std::fmt::Debug>(
    result: std::result::Result<T, rusqlite::Error>,
    expected: std::ffi::c_int,
    label: &str,
) {
    match result {
        Ok(v) => panic!("expected a {label} failure, got Ok({v:?})"),
        Err(rusqlite::Error::SqliteFailure(e, msg)) => assert_eq!(
            e.extended_code,
            expected,
            "expected a {label} failure ({expected}), got {} — {}",
            e.extended_code,
            msg.unwrap_or_default(),
        ),
        Err(e) => panic!("expected a {label} failure, got: {e}"),
    }
}

/// `STRICT` rejected the value's type.
pub const DATATYPE: std::ffi::c_int = rusqlite::ffi::SQLITE_CONSTRAINT_DATATYPE;
/// A row-local `CHECK (...)` rejected the row.
pub const CHECK: std::ffi::c_int = rusqlite::ffi::SQLITE_CONSTRAINT_CHECK;
/// A `REFERENCES` clause rejected an insert or update pointing at a missing
/// parent.
pub const FOREIGN_KEY: std::ffi::c_int = rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY;
/// An `ON DELETE RESTRICT` clause blocked a delete.
///
/// Deliberately *not* `SQLITE_CONSTRAINT_FOREIGNKEY`. SQLite implements the FK
/// actions (RESTRICT, CASCADE, SET NULL) as implicit triggers, so a blocked
/// delete surfaces as `SQLITE_CONSTRAINT_TRIGGER` even though the message reads
/// "FOREIGN KEY constraint failed". Named separately so the difference is a
/// documented fact rather than a puzzling assertion.
pub const RESTRICT: std::ffi::c_int = rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER;
/// A `UNIQUE` index rejected the row.
pub const UNIQUE: std::ffi::c_int = rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE;
/// A trigger called `RAISE(ABORT, …)`.
pub const TRIGGER: std::ffi::c_int = rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER;
