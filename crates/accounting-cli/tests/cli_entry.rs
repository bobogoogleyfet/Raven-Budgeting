//! Black-box tests for the `pacc` write path.
//!
//! These run the real binary against a real database file, which is the only
//! way to exercise the parts nothing else covers: clap's argument wiring, the
//! split/item mini-languages, the exit codes, and — most importantly — the
//! error messages. A CLI that refuses a receipt without saying which category
//! was ambiguous or how much money is unassigned has told the user only that
//! they have failed.
//!
//! State is inspected two ways on purpose: through the CLI itself (which is
//! what a user sees) and, where output formatting would make an assertion
//! brittle or where an id is needed, through `accounting-core` against the same
//! file once the process has exited.

use std::path::PathBuf;

use accounting_core::Money;
use accounting_core::db::{Db, check_all};
use accounting_core::repo::transactions::TxnFilter;
use accounting_core::repo::{categories, merchants, transactions};
use accounting_core::{CategoryGroupId, TxnId};
use assert_cmd::Command;
use tempfile::TempDir;

/// The outcome of one `pacc` invocation.
struct Run {
    ok: bool,
    /// stdout and stderr together: which stream a diagnostic lands on is a
    /// formatting decision, but the user reads both, and so does this test.
    text: String,
}

impl Run {
    #[track_caller]
    fn expect_ok(self, what: &str) -> String {
        assert!(
            self.ok,
            "`pacc {what}` must succeed; output was:\n{}",
            self.text
        );
        self.text
    }

    #[track_caller]
    fn expect_failure(self, what: &str) -> String {
        assert!(
            !self.ok,
            "`pacc {what}` must exit non-zero; output was:\n{}",
            self.text
        );
        self.text
    }
}

/// A database file with the default taxonomy installed.
struct Fixture {
    _dir: TempDir,
    db: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("pacc.db");
        let f = Fixture { _dir: dir, db };
        f.run(&["init"]).expect_ok("init");
        f
    }

    fn run(&self, args: &[&str]) -> Run {
        let mut cmd = Command::cargo_bin("pacc").expect("the pacc binary must build");
        cmd.arg("--db").arg(&self.db);
        let out = cmd.args(args).output().expect("run pacc");
        Run {
            ok: out.status.success(),
            text: format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        }
    }

    /// Reopen the file from the test process, once `pacc` has exited.
    fn open(&self) -> Db {
        Db::open(&self.db).expect("reopen the database")
    }
}

fn group_id(db: &Db, name: &str) -> CategoryGroupId {
    categories::list_groups(db)
        .expect("list_groups")
        .into_iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("the seed must contain a `{name}` group"))
        .id
}

/// The single transaction on file, for the tests that saved exactly one.
fn only_txn(db: &Db) -> accounting_core::repo::transactions::Txn {
    let rows = transactions::list(db, &TxnFilter::default()).expect("list");
    match rows.len() {
        1 => rows.into_iter().next().expect("checked"),
        n => panic!("expected exactly one transaction on file, found {n}"),
    }
}

#[track_caller]
fn assert_clean(db: &Db) {
    let violations = check_all(db).expect("check_all");
    assert!(
        violations.is_empty(),
        "the CLI write path must leave every invariant intact; got {violations:?}"
    );
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[test]
fn an_added_receipt_shows_up_in_the_list() {
    let f = Fixture::new();

    f.run(&[
        "add",
        "--date",
        "2026-08-13",
        "--merchant",
        "Costco",
        "--total",
        "54.32",
        "--tax",
        "4.20",
        "--category",
        "Groceries",
    ])
    .expect_ok("add");

    let listed = f.run(&["list"]).expect_ok("list");
    assert!(
        listed.contains("Costco"),
        "the list must show the merchant; got:\n{listed}"
    );
    assert!(
        listed.contains("54.32"),
        "the list must show the amount; got:\n{listed}"
    );

    // And the stored shape, which the output format cannot tell us about.
    let db = f.open();
    let saved = only_txn(&db);
    assert_eq!(saved.total, Money::from_cents(5432));
    assert_eq!(saved.tax, Money::from_cents(420));
    assert_eq!(
        saved.subtotal,
        Money::from_cents(5012),
        "$54.32 with $4.20 of tax leaves a $50.12 subtotal"
    );
    assert!(
        !saved.tax_is_estimated,
        "the tax was given on the command line, so it is a fact, not an estimate"
    );
    assert_eq!(saved.splits.len(), 1);
    assert_eq!(saved.splits[0].category_name, "Groceries");
    assert_clean(&db);
}

#[test]
fn a_dry_run_add_writes_nothing() {
    let f = Fixture::new();

    let out = f
        .run(&[
            "add",
            "--date",
            "2026-08-13",
            "--merchant",
            "Dry Run Store",
            "--total",
            "54.32",
            "--category",
            "Groceries",
            "--dry-run",
        ])
        .expect_ok("add --dry-run");
    assert!(
        out.contains("54.32"),
        "a dry run must still show what it would have saved; got:\n{out}"
    );

    let listed = f.run(&["list"]).expect_ok("list");
    assert!(
        !listed.contains("Dry Run Store"),
        "a dry run must not appear in the list; got:\n{listed}"
    );

    // The merchant is the giveaway: an implementation that creates the store
    // before deciding not to save the receipt leaves that behind.
    let db = f.open();
    assert!(
        transactions::list(&db, &TxnFilter::default())
            .expect("list")
            .is_empty(),
        "a dry run must leave the database empty"
    );
    // Named rather than counted: the seed installs suggested stores, so an
    // empty merchant table stopped being the property here. What matters is
    // that *this* store was not created — the store is created on the way to a
    // save, and there was no save.
    assert!(
        merchants::find_by_name(&db, "Dry Run Store")
            .expect("find merchant")
            .is_none(),
        "a dry run must not create the merchant either"
    );
}

#[test]
fn an_unknown_category_is_refused_and_named() {
    let f = Fixture::new();

    let out = f
        .run(&[
            "add",
            "--date",
            "2026-08-13",
            "--total",
            "10.00",
            "--split",
            "Zorblatt=remainder",
        ])
        .expect_failure("add with an unknown category");
    assert!(
        out.contains("Zorblatt"),
        "the message must echo the name that was not found; got:\n{out}"
    );

    let db = f.open();
    assert!(
        transactions::list(&db, &TxnFilter::default())
            .expect("list")
            .is_empty(),
        "a rejected add must write nothing"
    );
}

#[test]
fn an_ambiguous_category_lists_the_candidates() {
    // Two groups, one category name. Without both candidates in the message
    // the user has no way to know what to type instead — the qualified
    // `Group/Name` form is only discoverable from this error.
    let f = Fixture::new();
    {
        let db = f.open();
        let bills = group_id(&db, "Bills");
        let qol = group_id(&db, "Quality of Life");
        categories::create(&db, bills, "Utilities").expect("create Bills/Utilities");
        categories::create(&db, qol, "Utilities").expect("create Quality of Life/Utilities");
    }

    let out = f
        .run(&[
            "add",
            "--date",
            "2026-08-13",
            "--total",
            "10.00",
            "--split",
            "Utilities=remainder",
        ])
        .expect_failure("add with an ambiguous category");
    assert!(
        out.contains("Bills/Utilities"),
        "the message must list the first candidate; got:\n{out}"
    );
    assert!(
        out.contains("Quality of Life/Utilities"),
        "the message must list the second candidate; got:\n{out}"
    );

    // And the qualified form the message points at must actually work.
    f.run(&[
        "add",
        "--date",
        "2026-08-13",
        "--total",
        "10.00",
        "--split",
        "Bills/Utilities=remainder",
    ])
    .expect_ok("add with a qualified category");
}

#[test]
fn splits_that_do_not_cover_the_total_are_refused_with_the_shortfall() {
    let f = Fixture::new();

    let out = f
        .run(&[
            "add",
            "--date",
            "2026-08-13",
            "--total",
            "100.00",
            "--split",
            "Groceries=60.00",
        ])
        .expect_failure("add with an unassigned remainder");
    assert!(
        out.contains("40.00"),
        "the message must say how much is unassigned — a refusal without the \
         number leaves the user to do the arithmetic the app exists to do; got:\n{out}"
    );

    let db = f.open();
    assert!(
        transactions::list(&db, &TxnFilter::default())
            .expect("list")
            .is_empty()
    );
}

#[test]
fn percent_and_remainder_splits_consume_the_total_exactly() {
    // 33.33% + 33.33% + remainder of $100.00. Rounding each share on its own
    // gives 3333 + 3333 + 3334 = 10000 here, but the guarantee under test is
    // that the three stored rows sum to the total to the cent whatever the
    // rounding did.
    let f = Fixture::new();

    f.run(&[
        "add",
        "--date",
        "2026-08-13",
        "--total",
        "100.00",
        "--split",
        "Groceries=33.33%",
        "--split",
        "Household=33.33%",
        "--split",
        "Toiletries=remainder",
    ])
    .expect_ok("add with percent and remainder splits");

    let db = f.open();
    let saved = only_txn(&db);
    assert_eq!(saved.total, Money::from_cents(10_000));
    assert_eq!(saved.splits.len(), 3);
    assert_eq!(
        saved.splits.iter().map(|s| s.amount).sum::<Money>(),
        saved.total,
        "not one cent may be created or destroyed by a split"
    );
    let names: Vec<&str> = saved
        .splits
        .iter()
        .map(|s| s.category_name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Groceries", "Household", "Toiletries"],
        "splits must keep the order they were given on the command line"
    );
    assert_clean(&db);
}

#[test]
fn show_prints_the_splits_of_a_receipt() {
    let f = Fixture::new();
    f.run(&[
        "add",
        "--date",
        "2026-08-13",
        "--merchant",
        "Costco",
        "--total",
        "100.00",
        "--split",
        "Groceries=60.00",
        "--split",
        "Household=remainder",
    ])
    .expect_ok("add");

    let id = {
        let db = f.open();
        only_txn(&db).id
    };

    let out = f.run(&["show", &id.to_string()]);
    let out = out.expect_ok("show");
    assert!(out.contains("Groceries"), "got:\n{out}");
    assert!(out.contains("Household"), "got:\n{out}");
    assert!(out.contains("60.00"), "got:\n{out}");
    assert!(out.contains("40.00"), "the remainder's amount; got:\n{out}");
}

// ---------------------------------------------------------------------------
// quick-add / inbox / categorize
// ---------------------------------------------------------------------------

#[test]
fn quick_add_lands_in_the_inbox_and_categorize_empties_it() {
    let f = Fixture::new();
    f.run(&["quick-add", "20.00"]).expect_ok("quick-add");

    let inbox = f.run(&["inbox"]).expect_ok("inbox");
    assert!(
        inbox.contains("20.00"),
        "a quick-added receipt must be waiting in the inbox; got:\n{inbox}"
    );

    // The id comes from the database rather than by scraping the output: the
    // test is about the write path, not about a column layout.
    let id: TxnId = {
        let db = f.open();
        let waiting = transactions::list(
            &db,
            &TxnFilter {
                needs_review: Some(true),
                ..Default::default()
            },
        )
        .expect("list");
        assert_eq!(waiting.len(), 1, "one quick-add, one row awaiting review");
        waiting[0].id
    };

    f.run(&[
        "categorize",
        &id.to_string(),
        "--split",
        "Groceries=remainder",
    ])
    .expect_ok("categorize");

    let inbox = f.run(&["inbox"]).expect_ok("inbox after categorize");
    assert!(
        !inbox.contains("20.00"),
        "a categorised receipt must leave the inbox; got:\n{inbox}"
    );

    let db = f.open();
    let saved = transactions::get(&db, id).expect("get");
    assert!(!saved.needs_review);
    assert_eq!(
        saved.total,
        Money::from_cents(2000),
        "categorising re-plans a known total; it must never move it"
    );
    assert_eq!(saved.splits[0].category_name, "Groceries");
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// The end-to-end proof
// ---------------------------------------------------------------------------

#[test]
fn the_whole_write_path_leaves_the_database_healthy() {
    // Everything the CLI can write, into one database, then `pacc doctor`.
    // The invariants that no SQLite constraint can express are exactly the
    // ones a write path breaks quietly, so this is the assertion that makes
    // all the others mean something.
    let f = Fixture::new();

    f.run(&[
        "add",
        "--date",
        "2026-08-01",
        "--merchant",
        "Costco",
        "--total",
        "54.32",
        "--tax",
        "4.20",
        "--category",
        "Groceries",
    ])
    .expect_ok("add");

    f.run(&[
        "add",
        "--date",
        "2026-08-05",
        "--merchant",
        "Costco",
        "--total",
        "100.00",
        "--split",
        "Groceries=33.33%",
        "--split",
        "Household=33.33%",
        "--split",
        "Toiletries=remainder",
    ])
    .expect_ok("add with splits");

    f.run(&[
        "add",
        "--date",
        "2026-08-07",
        "--merchant",
        "Costco",
        "--total",
        "30.00",
        "--itemized",
        "--split",
        "Groceries=remainder",
        "--item",
        "1:Apples=12.00",
        "--item",
        "1:Bread=8.00",
        "--item",
        "1:Cheese=10.00",
    ])
    .expect_ok("add itemized");

    f.run(&["quick-add", "20.00"]).expect_ok("quick-add");
    let id = {
        let db = f.open();
        transactions::list(
            &db,
            &TxnFilter {
                needs_review: Some(true),
                ..Default::default()
            },
        )
        .expect("list")[0]
            .id
    };
    f.run(&[
        "categorize",
        &id.to_string(),
        "--split",
        "Groceries=remainder",
    ])
    .expect_ok("categorize");

    let doctor = f.run(&["doctor"]).expect_ok("doctor");
    assert!(
        doctor.contains("0 errors, 0 warnings"),
        "every receipt above went through the one write path, so `doctor` must \
         be silent; got:\n{doctor}"
    );

    let db = f.open();
    assert_clean(&db);
    for row in transactions::list(&db, &TxnFilter::default()).expect("list") {
        assert_eq!(
            row.splits.iter().map(|s| s.amount).sum::<Money>(),
            row.total,
            "txn {:?} does not add up",
            row.id
        );
    }
}
