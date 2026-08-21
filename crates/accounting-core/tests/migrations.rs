//! Migration tests: the ladder is valid, it climbs to the expected rung, it is
//! idempotent, its rungs never change, and the connection pragmas that the rest
//! of the suite depends on are actually in force.

mod common;

use accounting_core::db::Db;
use accounting_core::db::migrations::{MIGRATION_SOURCES, MIGRATIONS, SCHEMA_VERSION};
use common::{pragma_i64, pragma_string};
use sha2::{Digest, Sha256};

/// SHA-256 of every migration that has ever shipped, in apply order.
///
/// WHY THIS EXISTS: `rusqlite_migration` records progress in
/// `PRAGMA user_version` — a counter, not a content hash. If you edit
/// `001_init.sql` after it has been applied somewhere, a *fresh* database gets
/// the new text while every existing database keeps the old, and both report
/// `user_version = 1`. Nothing complains. The two schemas diverge silently, and
/// you find out on the device you migrate to next year.
///
/// **A failure here means you edited a shipped migration. Revert the edit and
/// add a new numbered migration instead.**
///
/// When you legitimately ADD a migration, append one row to this array:
///
/// ```text
/// sha256sum crates/accounting-core/src/db/schema/*.sql
/// ```
///
/// A migration is "shipped" the moment it is committed — this repo's database
/// holds real financial data and is never recreated from scratch.
const MIGRATION_HASHES: &[(&str, &str)] = &[
    (
        "001_init.sql",
        "9e974f128e4499bf50b8ef420edbbae9677cbf812d512a93cc4d88fccc903eb1",
    ),
    (
        "002_food_rate.sql",
        "d01bbd5a0f1e44d3ec8449ce65fb67169649db8628f1a890bf0d0aee3cc67a2d",
    ),
    (
        "003_utah_rates.sql",
        "90564f669b5581bff1c7931d2dca42f0eb7dbde1cc2a6f68363c6fd082428b74",
    ),
    (
        "004_taxable.sql",
        "2000560dbbf402a80e25f2f5d26ce34296ccceb3a17c69ea7a710d1033350c42",
    ),
    (
        "005_restaurant_rate.sql",
        "2ec26bf7dbc32eb57424af6b9b32ddda6dbfbb9722b068d0a2fa83ccec8c9628",
    ),
    (
        "006_budget_contribution.sql",
        "70ce72e96de216c3ff77c5e2cf2ac58058fe38d78ddb01f4ee0a97d5df0ba6d3",
    ),
    (
        "007_group_obligation.sql",
        "37f0640a50f70f75082f9d16987efaa7f72f71b2275c651095cfb2f2801b7a42",
    ),
];

/// `PRAGMA user_version` after `to_latest()`. `rusqlite_migration` sets it to
/// the number of applied migrations, so this must track `MIGRATION_HASHES`.
/// `pacc doctor` prints it.
const EXPECTED_USER_VERSION: i64 = 7;

fn sha256_hex(bytes: &str) -> String {
    let digest = Sha256::digest(bytes.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// The ladder itself
// ---------------------------------------------------------------------------

#[test]
fn migration_set_is_internally_valid() {
    MIGRATIONS
        .validate()
        .expect("MIGRATIONS.validate() — a migration is malformed or out of order");
}

#[test]
fn fresh_database_migrates_to_the_expected_user_version() {
    let db = Db::open_in_memory().expect("open_in_memory");
    assert_eq!(
        pragma_i64(common::raw(&db), "user_version"),
        EXPECTED_USER_VERSION,
        "a freshly opened DB must be at the latest schema version"
    );
}

#[test]
fn user_version_matches_the_number_of_shipped_migrations() {
    // rusqlite_migration's user_version *is* the count of applied migrations.
    // If these disagree, either a migration was added without bumping the
    // expectation or one is not being applied at all.
    assert_eq!(
        EXPECTED_USER_VERSION,
        i64::try_from(MIGRATION_HASHES.len()).expect("migration count fits i64"),
    );
    assert_eq!(
        EXPECTED_USER_VERSION,
        i64::try_from(SCHEMA_VERSION).expect("SCHEMA_VERSION fits i64"),
    );
    assert_eq!(MIGRATION_SOURCES.len(), MIGRATION_HASHES.len());
}

#[test]
fn reopening_an_already_current_database_is_a_no_op() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");

    let schema_before = {
        let db = Db::open(&path).expect("first open");
        let c = common::raw(&db);
        assert_eq!(pragma_i64(c, "user_version"), EXPECTED_USER_VERSION);
        dump_schema(c)
    };

    let db = Db::open(&path).expect("reopen");
    let c = common::raw(&db);
    assert_eq!(
        pragma_i64(c, "user_version"),
        EXPECTED_USER_VERSION,
        "reopening must not re-run or advance migrations"
    );
    assert_eq!(
        dump_schema(c),
        schema_before,
        "reopening an up-to-date database must not alter the schema"
    );
}

fn dump_schema(c: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = c
        .prepare("SELECT type || ' ' || name || ' ' || COALESCE(sql, '') FROM sqlite_master ORDER BY type, name")
        .expect("prepare schema dump");
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query schema dump");
    rows.map(|r| r.expect("schema row")).collect()
}

// ---------------------------------------------------------------------------
// Immutability
// ---------------------------------------------------------------------------

#[test]
fn shipped_migrations_are_byte_for_byte_immutable() {
    // Hashes the `include_str!`ed text the binary actually ships, not the file
    // on disk — those are the same bytes, but only one of them is what a
    // released build carries.
    let actual: Vec<(String, String)> = MIGRATION_SOURCES
        .iter()
        .map(|(name, sql)| ((*name).to_string(), sha256_hex(sql)))
        .collect();

    let expected: Vec<(String, String)> = MIGRATION_HASHES
        .iter()
        .map(|(n, h)| ((*n).to_string(), (*h).to_string()))
        .collect();

    assert_eq!(
        actual, expected,
        "\n\n\
         A shipped migration changed.\n\
         \n\
         If you EDITED an existing migration: revert it. user_version is a\n\
         counter, not a content hash, so the edit reaches new databases only —\n\
         every database that already applied it keeps the old schema and still\n\
         reports the same version. Add a new numbered migration instead.\n\
         \n\
         If you ADDED a migration: append its (name, hash) to MIGRATION_HASHES\n\
         in this file and bump EXPECTED_USER_VERSION. Current on-disk hashes:\n\
         {actual:#?}\n"
    );
}

#[test]
fn every_shipped_migration_carries_sql() {
    // A truncated migration hashes fine on a fresh checkout but produces an
    // empty schema, and the failure surfaces far from the cause.
    //
    // Checked for *any* schema-changing statement rather than CREATE TABLE
    // specifically: only the first migration creates tables, and every one
    // after it alters or indexes them. Requiring CREATE TABLE would make this
    // a test that the schema never evolves.
    for (name, sql) in MIGRATION_SOURCES {
        let upper = sql.to_ascii_uppercase();
        assert!(
            [
                "CREATE TABLE",
                "ALTER TABLE",
                "CREATE INDEX",
                "CREATE TRIGGER"
            ]
            .iter()
            .any(|stmt| upper.contains(stmt)),
            "{name} changes no schema — is it truncated?"
        );
    }
}

// ---------------------------------------------------------------------------
// Connection pragmas
// ---------------------------------------------------------------------------

#[test]
fn file_backed_database_uses_wal_journal_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Db::open(&dir.path().join("accounting.db")).expect("open");
    assert_eq!(
        pragma_string(common::raw(&db), "journal_mode").to_ascii_lowercase(),
        "wal",
        "WAL is what lets a reader run while a write is in flight"
    );
}

#[test]
fn foreign_keys_are_enforced_on_an_open_connection() {
    // LOAD-BEARING. `PRAGMA foreign_keys` is a no-op inside a transaction and
    // SQLite reports no error when it is ignored. `to_latest()` runs in a
    // transaction, so setting the pragma after migrating leaves FK enforcement
    // silently OFF for the life of the connection — and every FK test in
    // schema.rs would pass vacuously. This asserts the ordering directly.
    let db = Db::open_in_memory().expect("open_in_memory");
    assert_eq!(
        pragma_i64(common::raw(&db), "foreign_keys"),
        1,
        "foreign_keys must be ON; it is ignored if set inside a transaction"
    );
}

#[test]
fn foreign_keys_are_enforced_on_a_file_backed_connection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Db::open(&dir.path().join("accounting.db")).expect("open");
    assert_eq!(pragma_i64(common::raw(&db), "foreign_keys"), 1);
}
