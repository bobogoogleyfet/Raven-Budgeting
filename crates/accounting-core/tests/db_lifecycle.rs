//! Lifecycle tests: opening, reopening, backing up, and reading concurrently.

mod common;

use accounting_core::db::Db;
use common::{count, insert_group, insert_group_with_category, pragma_string, raw};

// ---------------------------------------------------------------------------
// In-memory
// ---------------------------------------------------------------------------

#[test]
fn an_in_memory_database_opens_migrated_and_usable() {
    let db = Db::open_in_memory().expect("open_in_memory");
    let c = raw(&db);
    insert_group(c, "Monthly", 100);
    assert_eq!(count(c, "SELECT COUNT(*) FROM category_group"), 1);
}

#[test]
fn two_in_memory_databases_do_not_share_state() {
    // `:memory:` is per-connection, but a shared-cache URI would silently make
    // every test in the suite see every other test's rows.
    let a = Db::open_in_memory().expect("open a");
    let b = Db::open_in_memory().expect("open b");

    insert_group(raw(&a), "Only In A", 100);

    assert_eq!(count(raw(&a), "SELECT COUNT(*) FROM category_group"), 1);
    assert_eq!(
        count(raw(&b), "SELECT COUNT(*) FROM category_group"),
        0,
        "in-memory databases must be isolated from one another"
    );
}

// ---------------------------------------------------------------------------
// File-backed
// ---------------------------------------------------------------------------

#[test]
fn opening_a_path_that_does_not_exist_creates_and_migrates_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");
    assert!(!path.exists());

    let db = Db::open(&path).expect("open");
    assert!(path.exists(), "open must create the database file");
    assert_eq!(count(raw(&db), "SELECT COUNT(*) FROM category_group"), 0);
}

#[test]
fn data_written_before_a_close_is_there_after_reopening() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");

    {
        let db = Db::open(&path).expect("first open");
        insert_group_with_category(raw(&db), "Monthly", "Groceries");
    }

    let db = Db::open(&path).expect("reopen");
    let c = raw(&db);
    assert_eq!(count(c, "SELECT COUNT(*) FROM category_group"), 1);
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM category WHERE name = 'Groceries'"),
        1,
        "durability is the entire point of a file-backed ledger"
    );
}

// ---------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------

#[test]
fn backup_writes_a_single_self_contained_file() {
    // `VACUUM INTO` produces one consistent file. A naive `fs::copy` of a WAL
    // database copies the main file only and leaves the most recent commits
    // behind in the -wal — a backup that looks fine and is stale.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");
    let db = Db::open(&path).expect("open");
    insert_group_with_category(raw(&db), "Monthly", "Groceries");

    let backup_dir = tempfile::tempdir().expect("backup tempdir");
    let dest = backup_dir.path().join("backup.db");
    db.backup(&dest).expect("backup");

    let produced: Vec<String> = std::fs::read_dir(backup_dir.path())
        .expect("read backup dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        produced,
        vec!["backup.db".to_string()],
        "backup must produce exactly one file — no -wal, no -shm sidecars"
    );
}

#[test]
fn a_backup_opens_cleanly_and_holds_the_same_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");

    let db = Db::open(&path).expect("open");
    accounting_core::db::seed::seed_defaults(&db).expect("seed");
    insert_group_with_category(raw(&db), "Fixture", "Fixture Category");
    let expected_groups = count(raw(&db), "SELECT COUNT(*) FROM category_group");
    let expected_categories = count(raw(&db), "SELECT COUNT(*) FROM category");

    let backup_dir = tempfile::tempdir().expect("backup tempdir");
    let dest = backup_dir.path().join("backup.db");
    db.backup(&dest).expect("backup");

    let restored = Db::open(&dest).expect("open the backup");
    let c = raw(&restored);
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM category_group"),
        expected_groups
    );
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM category"),
        expected_categories
    );
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM category WHERE name = 'Fixture Category'"
        ),
        1,
    );
    assert!(
        check_ok(&restored),
        "the backup must pass SQLite's own integrity_check"
    );
}

fn check_ok(db: &Db) -> bool {
    pragma_string(raw(db), "integrity_check").eq_ignore_ascii_case("ok")
}

#[test]
fn backing_up_does_not_disturb_the_source_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");
    let db = Db::open(&path).expect("open");
    insert_group(raw(&db), "Monthly", 100);

    let backup_dir = tempfile::tempdir().expect("backup tempdir");
    db.backup(&backup_dir.path().join("backup.db"))
        .expect("backup");

    // VACUUM resets journal_mode on the *target*; it must not touch the source.
    assert_eq!(
        pragma_string(raw(&db), "journal_mode").to_ascii_lowercase(),
        "wal"
    );
    insert_group(raw(&db), "Non-Monthly", 200);
    assert_eq!(count(raw(&db), "SELECT COUNT(*) FROM category_group"), 2);
}

#[test]
fn backing_up_over_an_existing_file_fails_rather_than_clobbering_it() {
    // `VACUUM INTO` refuses an existing target. Surfacing that as an error is
    // the right behaviour for a financial backup: a silent overwrite of
    // yesterday's good backup with today's is how one bad day becomes two.
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Db::open(&dir.path().join("accounting.db")).expect("open");

    let backup_dir = tempfile::tempdir().expect("backup tempdir");
    let dest = backup_dir.path().join("backup.db");
    db.backup(&dest).expect("first backup");
    assert!(
        db.backup(&dest).is_err(),
        "a second backup to the same path must not silently overwrite"
    );
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[test]
fn a_reader_is_not_blocked_by_an_open_write_transaction_under_wal() {
    // This is what WAL buys: the CLI can run `pacc doctor` while the GUI holds
    // a write open. Under the default rollback journal the reader would get
    // SQLITE_BUSY.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");

    let writer = Db::open(&path).expect("open writer");
    insert_group(raw(&writer), "Monthly", 100);

    let reader = Db::open(&path).expect("open reader");

    let wc = raw(&writer);
    wc.execute_batch("BEGIN IMMEDIATE").expect("begin write");
    insert_group(wc, "Uncommitted", 200);

    // The reader sees the last committed snapshot, not the in-flight write.
    assert_eq!(
        count(raw(&reader), "SELECT COUNT(*) FROM category_group"),
        1,
        "a WAL reader must proceed against the committed snapshot"
    );

    wc.execute_batch("COMMIT").expect("commit");

    assert_eq!(
        count(raw(&reader), "SELECT COUNT(*) FROM category_group"),
        2,
        "after the commit the reader must see the new row"
    );
}

#[test]
fn two_readers_can_query_the_same_file_at_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("accounting.db");
    {
        let db = Db::open(&path).expect("open");
        insert_group(raw(&db), "Monthly", 100);
    }

    let a = Db::open(&path).expect("open a");
    let b = Db::open(&path).expect("open b");
    assert_eq!(count(raw(&a), "SELECT COUNT(*) FROM category_group"), 1);
    assert_eq!(count(raw(&b), "SELECT COUNT(*) FROM category_group"), 1);
}
