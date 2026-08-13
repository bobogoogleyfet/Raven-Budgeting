//! Black-box tests for `pacc location`.
//!
//! The command exists to close a loop the app could otherwise only complain
//! about: the seeded `Home` location carries a 0% rate, so a receipt entered
//! without `--rate` records $0.00 tax and the entry service warns that no rate
//! is set — advice that was impossible to act on, because nothing could change
//! a rate. These tests assert the loop actually closes end to end, which is the
//! only claim worth making about this command.

use std::path::PathBuf;

use accounting_core::db::Db;
use accounting_core::repo::locations;
use assert_cmd::Command;
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    db: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("pacc.db");
        let f = Fixture { _dir: dir, db };
        assert!(f.run(&["init"]).0, "init must succeed");
        f
    }

    /// Returns (success, stdout + stderr). Which stream a diagnostic lands on
    /// is a formatting choice; the user reads both.
    fn run(&self, args: &[&str]) -> (bool, String) {
        let mut cmd = Command::cargo_bin("pacc").expect("the pacc binary must build");
        cmd.arg("--db").arg(&self.db);
        let out = cmd.args(args).output().expect("run pacc");
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    fn open(&self) -> Db {
        Db::open(&self.db).expect("reopen the database")
    }
}

#[test]
fn the_seeded_home_location_is_listed_with_no_rate() {
    let f = Fixture::new();
    let (ok, text) = f.run(&["location"]);
    assert!(ok, "listing must succeed:\n{text}");
    assert!(text.contains("Home"), "expected Home in:\n{text}");
    assert!(
        text.contains("(home)"),
        "expected the home marker in:\n{text}"
    );
}

#[test]
fn setting_a_rate_on_an_existing_location_changes_it() {
    let f = Fixture::new();

    // locations::get_or_create deliberately does not overwrite an existing
    // rate, so this is the assertion that `set_rate` is actually reached
    // rather than the create path silently doing nothing.
    let (ok, text) = f.run(&["location", "--name", "Home", "--rate", "7.75"]);
    assert!(ok, "setting a rate must succeed:\n{text}");

    let db = f.open();
    let home = locations::find_by_name(&db, "Home")
        .expect("query")
        .expect("Home is seeded");
    // 7.75% is 77_500 parts per million. ppm rather than a float or
    // percent-with-2-decimals so a rate like 9.4375% stays exact.
    assert_eq!(home.tax_rate_ppm, 77_500);
}

#[test]
fn a_rate_set_here_is_what_the_next_receipt_estimates_with() {
    let f = Fixture::new();
    f.run(&["location", "--name", "Home", "--rate", "7.75"]);

    // The whole point of the command: a bare `add` now estimates a real tax
    // instead of recording $0.00 and warning about it.
    let (ok, text) = f.run(&[
        "add",
        "--date",
        "2026-05-01",
        "--total",
        "10.00",
        "--category",
        "Groceries",
    ]);
    assert!(ok, "add must succeed:\n{text}");

    // $10.00 tax-inclusive at 7.75%: subtotal 10.00 / 1.0775 = 9.2807... which
    // rounds to $9.28, leaving $0.72 of tax.
    assert!(
        text.contains("$0.72"),
        "expected $0.72 of estimated tax in:\n{text}"
    );
    assert!(
        !text.contains("no tax rate set"),
        "the dead-end warning must be gone once a rate exists:\n{text}"
    );
}

#[test]
fn a_new_location_can_be_created_and_made_home() {
    let f = Fixture::new();
    let (ok, text) = f.run(&["location", "--name", "Boise, ID", "--rate", "6", "--home"]);
    assert!(ok, "creating a location must succeed:\n{text}");

    let db = f.open();
    let boise = locations::find_by_name(&db, "Boise, ID")
        .expect("query")
        .expect("just created");
    assert_eq!(boise.tax_rate_ppm, 60_000);

    // The home flag must MOVE, not duplicate: ux_location_home is a partial
    // unique index permitting exactly one home, so a second one would have
    // been a constraint failure rather than a silent second row.
    let homes: Vec<_> = locations::list(&db)
        .expect("list")
        .into_iter()
        .filter(|l| l.is_home)
        .collect();
    assert_eq!(
        homes.len(),
        1,
        "exactly one home location, found: {homes:?}"
    );
    assert_eq!(homes[0].name, "Boise, ID");
}

#[test]
fn a_rate_without_a_name_is_refused_rather_than_silently_ignored() {
    let f = Fixture::new();
    let (ok, text) = f.run(&["location", "--rate", "7.75"]);
    assert!(!ok, "a rate with no target must fail; got:\n{text}");
    assert!(
        text.contains("--name"),
        "the error must say what is missing:\n{text}"
    );
}

/// Closing the pipe early must not look like a failure.
///
/// Rust ignores `SIGPIPE`, so a write to a closed pipe returns `EPIPE` and
/// `println!` unwraps it into a panic — `pacc report | head` printed its first
/// lines and then died with exit code 101, which a shell script reads as the
/// report having failed. Piping into `head`, `grep -m1`, or quitting `less`
/// early is ordinary use of a reporting tool, so it must exit cleanly.
#[test]
fn closing_the_pipe_early_is_not_an_error() {
    use std::process::{Command as Proc, Stdio};

    let f = Fixture::new();
    let bin = assert_cmd::cargo::cargo_bin("pacc");

    let mut child = Proc::new(&bin)
        .arg("--db")
        .arg(&f.db)
        .arg("categories")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn pacc");

    // Drop the read end without reading it. The next write `pacc` attempts
    // gets EPIPE, which is exactly what `head` does after its last line.
    drop(child.stdout.take());

    let status = child.wait().expect("wait for pacc");
    assert_ne!(
        status.code(),
        Some(101),
        "exit 101 is a Rust panic — the broken pipe was not handled"
    );
    assert!(
        status.success(),
        "a reader that stopped listening is not this program's error; got {status:?}"
    );
}
