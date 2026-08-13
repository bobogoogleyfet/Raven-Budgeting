//! Black-box tests for `pacc export`.
//!
//! Everything about the *content* of an export is tested in `accounting-core`.
//! What only the binary can be asked is whether the command wires up at all:
//! that the format is picked from the extension, that a file the user already
//! has is not silently destroyed, and that the filters reach the exporter
//! rather than being parsed and dropped.
//!
//! The output is inspected as bytes rather than through the core library on
//! purpose. `pacc export` exists so that a file can be handed to something
//! else, and "does the file on disk start with the zip magic" is exactly the
//! question the next program will ask.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

/// The bytes at the start of every zip, and therefore of every xlsx.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
/// The UTF-8 byte order mark every CSV this app writes begins with.
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

struct Run {
    ok: bool,
    /// stdout and stderr together: which stream a diagnostic lands on is a
    /// formatting decision, but the user reads both.
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

/// A database file with the default taxonomy installed, plus somewhere to
/// write exports.
struct Fixture {
    dir: TempDir,
    db: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("pacc.db");
        let f = Fixture { dir, db };
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

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Three receipts: two in August 2026 and one in June, so a month filter
    /// has something to exclude.
    fn with_receipts(self) -> Fixture {
        self.run(&[
            "add",
            "--date",
            "2026-06-15",
            "--merchant",
            "Costco",
            "--total",
            "20.00",
            "--category",
            "Groceries",
        ])
        .expect_ok("add June");
        self.run(&[
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
        .expect_ok("add August");
        self.run(&[
            "add",
            "--date",
            "2026-08-07",
            "--merchant",
            "Shell",
            "--total",
            "40.00",
            "--category",
            "Gas",
        ])
        .expect_ok("add August fuel");
        self
    }
}

#[track_caller]
fn bytes_at(path: &Path) -> Vec<u8> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("`pacc export` must write {}: {e}", path.display()))
}

/// Data records in a CSV export, header excluded.
///
/// A plain line count is enough here: nothing this file writes contains a
/// quoted newline, and the core suite already proves that a note with one
/// stays inside its record.
#[track_caller]
fn csv_row_count(path: &Path) -> usize {
    let raw = bytes_at(path);
    assert!(
        raw.starts_with(BOM),
        "a CSV written by `pacc export` must carry the UTF-8 BOM; {} starts {:?}",
        path.display(),
        &raw[..raw.len().min(8)]
    );
    let text = String::from_utf8(raw[BOM.len()..].to_vec()).expect("the export must be UTF-8");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "even an empty export has a header line; {} is empty",
        path.display()
    );
    lines.len() - 1
}

// ---------------------------------------------------------------------------
// Writing a file
// ---------------------------------------------------------------------------

#[test]
fn exporting_to_an_xlsx_path_writes_a_workbook() {
    let f = Fixture::new().with_receipts();
    let out = f.path("august.xlsx");

    f.run(&["export", "--to", out.to_str().expect("utf-8 path")])
        .expect_ok("export --to august.xlsx");

    let written = bytes_at(&out);
    assert!(
        !written.is_empty(),
        "an empty file is not a workbook, whatever its extension"
    );
    assert!(
        written.starts_with(ZIP_MAGIC),
        "an xlsx is a zip; {} starts {:?}",
        out.display(),
        &written[..written.len().min(8)]
    );
}

#[test]
fn the_format_is_inferred_from_the_file_extension() {
    // The user typed a name, not a format. Making them say `--format csv`
    // after typing `.csv` is the kind of small friction that gets an export
    // command replaced by a copy-paste.
    let f = Fixture::new().with_receipts();
    let csv = f.path("ledger.csv");
    let xlsx = f.path("ledger.xlsx");

    f.run(&["export", "--to", csv.to_str().expect("utf-8 path")])
        .expect_ok("export --to ledger.csv");
    f.run(&["export", "--to", xlsx.to_str().expect("utf-8 path")])
        .expect_ok("export --to ledger.xlsx");

    let as_csv = bytes_at(&csv);
    assert!(
        as_csv.starts_with(BOM),
        "a `.csv` path must produce CSV, not a workbook with the wrong \
         extension; it starts {:?}",
        &as_csv[..as_csv.len().min(8)]
    );
    assert!(
        !as_csv.starts_with(ZIP_MAGIC),
        "a `.csv` path must not receive zip bytes"
    );
    assert!(bytes_at(&xlsx).starts_with(ZIP_MAGIC));

    // And an explicit `--format` is still obeyed: a user exporting CSV to a
    // path their pipeline names `.txt` must be able to say so.
    let odd = f.path("ledger.txt");
    f.run(&[
        "export",
        "--to",
        odd.to_str().expect("utf-8 path"),
        "--format",
        "csv",
    ])
    .expect_ok("export --format csv --to ledger.txt");
    assert!(bytes_at(&odd).starts_with(BOM));
}

#[test]
fn an_existing_file_is_not_overwritten_without_force() {
    // An export path is typed by hand and reused, and the thing most likely to
    // be sitting there is last month's export — or, if the user is unlucky and
    // the app is careless, something else entirely.
    let f = Fixture::new().with_receipts();
    let out = f.path("ledger.csv");
    let target = out.to_str().expect("utf-8 path");

    std::fs::write(&out, b"something the user cared about").expect("seed the file");

    let message = f
        .run(&["export", "--to", target])
        .expect_failure("export over an existing file");
    assert!(
        message.contains("ledger.csv") || message.to_lowercase().contains("exist"),
        "the refusal must say which file is in the way; got:\n{message}"
    );
    assert_eq!(
        bytes_at(&out),
        b"something the user cared about",
        "a refused export must leave the file exactly as it found it"
    );

    f.run(&["export", "--to", target, "--force"])
        .expect_ok("export --force");
    let after = bytes_at(&out);
    assert!(
        after.starts_with(BOM),
        "--force must actually replace the file; it still holds {:?}",
        String::from_utf8_lossy(&after[..after.len().min(32)])
    );
    assert!(csv_row_count(&out) > 0, "and with the real rows in it");
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[test]
fn a_month_filter_narrows_the_export() {
    // Row counts are the property here, not a proxy for one: "fewer rows come
    // out when you ask for one month" *is* what a filter means, and the core
    // suite already asserts that the rows kept are the right ones.
    let f = Fixture::new().with_receipts();
    let everything = f.path("all.csv");
    let august = f.path("august.csv");

    f.run(&["export", "--to", everything.to_str().expect("utf-8 path")])
        .expect_ok("export everything");
    f.run(&[
        "export",
        "--to",
        august.to_str().expect("utf-8 path"),
        "--month",
        "2026-08",
    ])
    .expect_ok("export --month 2026-08");

    // Three receipts, one split each: three rows in total, two of them in
    // August (2026-08-01 and 2026-08-07) and one in June.
    assert_eq!(csv_row_count(&everything), 3, "every receipt, one row each");
    assert_eq!(
        csv_row_count(&august),
        2,
        "August has two of the three receipts"
    );
}

#[test]
fn a_merchant_filter_narrows_the_export() {
    let f = Fixture::new().with_receipts();
    let out = f.path("shell.csv");

    f.run(&[
        "export",
        "--to",
        out.to_str().expect("utf-8 path"),
        "--merchant",
        "Shell",
    ])
    .expect_ok("export --merchant Shell");

    assert_eq!(
        csv_row_count(&out),
        1,
        "only the one Shell receipt of the three"
    );
    let text = String::from_utf8(bytes_at(&out)[BOM.len()..].to_vec()).expect("UTF-8");
    assert!(text.contains("Shell"), "got:\n{text}");
    assert!(
        !text.contains("Costco"),
        "a merchant filter that let Costco through is not a filter; got:\n{text}"
    );
}

#[test]
fn a_category_filter_narrows_the_export() {
    let f = Fixture::new().with_receipts();
    let out = f.path("fuel.csv");

    f.run(&[
        "export",
        "--to",
        out.to_str().expect("utf-8 path"),
        "--category",
        "Gas",
    ])
    .expect_ok("export --category Gas");

    assert_eq!(csv_row_count(&out), 1, "one fuel receipt of the three");
    let text = String::from_utf8(bytes_at(&out)[BOM.len()..].to_vec()).expect("UTF-8");
    assert!(text.contains("Gas"), "got:\n{text}");
    assert!(!text.contains("Groceries"), "got:\n{text}");
}

#[test]
fn a_year_filter_narrows_the_export() {
    let f = Fixture::new().with_receipts();
    f.run(&[
        "add",
        "--date",
        "2025-11-11",
        "--total",
        "10.00",
        "--category",
        "Groceries",
    ])
    .expect_ok("add a receipt from the year before");

    let this_year = f.path("2026.csv");
    let last_year = f.path("2025.csv");
    f.run(&[
        "export",
        "--to",
        this_year.to_str().expect("utf-8 path"),
        "--year",
        "2026",
    ])
    .expect_ok("export --year 2026");
    f.run(&[
        "export",
        "--to",
        last_year.to_str().expect("utf-8 path"),
        "--year",
        "2025",
    ])
    .expect_ok("export --year 2025");

    // Four receipts in all: three in 2026 and the one just added in 2025.
    assert_eq!(csv_row_count(&this_year), 3);
    assert_eq!(csv_row_count(&last_year), 1);
}

#[test]
fn an_explicit_date_range_narrows_the_export() {
    // Note the flag name. The contract for this milestone called the upper
    // bound `--to`, but `--to` is already the destination *file* (as it is for
    // `pacc backup`), and clap cannot carry the same long flag twice. The
    // upper bound is `--to-date`, aliased `--until`; this test pins that down
    // so the resolution is a decision rather than an accident.
    let f = Fixture::new().with_receipts();
    let out = f.path("window.csv");

    f.run(&[
        "export",
        "--to",
        out.to_str().expect("utf-8 path"),
        "--from",
        "2026-08-01",
        "--to-date",
        "2026-08-01",
    ])
    .expect_ok("export --from --to-date");

    assert_eq!(
        csv_row_count(&out),
        1,
        "both ends are inclusive, so a one-day window catches the 2026-08-01 \
         receipt and nothing else"
    );
    let text = String::from_utf8(bytes_at(&out)[BOM.len()..].to_vec()).expect("UTF-8");
    assert!(text.contains("2026-08-01"), "got:\n{text}");
    assert!(!text.contains("2026-08-07"), "got:\n{text}");
}

#[test]
fn an_archived_receipt_is_exported_only_when_all_is_asked_for() {
    let f = Fixture::new().with_receipts();
    f.run(&[
        "add",
        "--date",
        "2026-08-09",
        "--merchant",
        "Deleted Store",
        "--total",
        "99.00",
        "--category",
        "Groceries",
    ])
    .expect_ok("add the one to archive");

    // Archived through the library, because `pacc` has no archive subcommand
    // yet — the property under test is the export's, not the CLI's.
    {
        use accounting_core::repo::transactions::{self, TxnFilter};
        let db = accounting_core::db::Db::open(&f.db).expect("reopen");
        let victim = transactions::list(&db, &TxnFilter::default())
            .expect("list")
            .into_iter()
            .find(|t| t.merchant_name.as_deref() == Some("Deleted Store"))
            .expect("the receipt just added");
        transactions::archive(&db, victim.id).expect("archive");
    }

    let live = f.path("live.csv");
    let everything = f.path("everything.csv");
    f.run(&["export", "--to", live.to_str().expect("utf-8 path")])
        .expect_ok("export");
    f.run(&[
        "export",
        "--to",
        everything.to_str().expect("utf-8 path"),
        "--all",
    ])
    .expect_ok("export --all");

    let live_text = String::from_utf8(bytes_at(&live)[BOM.len()..].to_vec()).expect("UTF-8");
    assert!(
        !live_text.contains("Deleted Store"),
        "an archived receipt is deleted as far as the user is concerned, and a \
         tax export that still counts it overstates the year; got:\n{live_text}"
    );
    let all_text = String::from_utf8(bytes_at(&everything)[BOM.len()..].to_vec()).expect("UTF-8");
    assert!(
        all_text.contains("Deleted Store"),
        "--all must bring it back; got:\n{all_text}"
    );
    assert_eq!(
        csv_row_count(&everything),
        csv_row_count(&live) + 1,
        "--all widens the export by exactly the archived receipt"
    );
}

// ---------------------------------------------------------------------------
// Nothing to export
// ---------------------------------------------------------------------------

#[test]
fn exporting_an_empty_database_succeeds_and_writes_a_file() {
    // A brand-new database, or a month with no receipts in it, is a question
    // with an answer: a file with column headers and no rows. Failing here
    // would mean the first thing a new user tried was an error message.
    let f = Fixture::new();
    let csv = f.path("empty.csv");
    let xlsx = f.path("empty.xlsx");

    f.run(&["export", "--to", csv.to_str().expect("utf-8 path")])
        .expect_ok("export an empty database to CSV");
    f.run(&["export", "--to", xlsx.to_str().expect("utf-8 path")])
        .expect_ok("export an empty database to xlsx");

    assert_eq!(
        csv_row_count(&csv),
        0,
        "headers, and no data rows to go under them"
    );
    assert!(
        bytes_at(&xlsx).starts_with(ZIP_MAGIC),
        "an empty workbook is still a workbook"
    );
}

#[test]
fn a_month_with_no_receipts_still_writes_a_file() {
    let f = Fixture::new().with_receipts();
    let out = f.path("january.csv");

    f.run(&[
        "export",
        "--to",
        out.to_str().expect("utf-8 path"),
        "--month",
        "2026-01",
    ])
    .expect_ok("export a month with nothing in it");

    assert_eq!(csv_row_count(&out), 0);
}
