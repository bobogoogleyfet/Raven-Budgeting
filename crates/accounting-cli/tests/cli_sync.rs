//! Black-box tests for `pacc sync`.
//!
//! Two database files, two `pacc` processes, and a file passed between them —
//! which is the whole of the sync design: there is no server, and the
//! transport is deliberately something this program cannot see the far end
//! of. Everything below therefore goes through the binary, and every claim
//! about what *arrived* is checked with a second `pacc` command rather than by
//! reading the database, because "the row is in the file" and "the app shows
//! it" are different statements and only the second one is the promise.
//!
//! # The second device starts empty
//!
//! Pairing hands the joining device the other one's data. Two databases that
//! each ran `pacc init` have each seeded their own chart of accounts — the
//! same names under different uuids — so merging them is not a merge at all,
//! and the schema's unique indexes refuse it. One of the tests below asserts
//! that refusal explains itself.
//!
//! There is no `pacc` command that creates the empty database a joining device
//! needs: `init` always seeds. So [`Pair::new`] makes it with `Db::open`,
//! which migrates the file and mints its device name and does nothing else.
//! That is a gap in the CLI rather than a liberty taken by the test.

use std::path::{Path, PathBuf};

use accounting_core::db::Db;
use assert_cmd::Command;
use tempfile::TempDir;

/// The phrase both devices share. Any real one would be longer; this one is
/// here to be typed identically twice.
const PHRASE: &str = "correct horse battery staple";

/// The outcome of one `pacc` invocation.
struct Run {
    ok: bool,
    /// stdout and stderr together: which stream a diagnostic lands on is a
    /// formatting decision, and the user reads both.
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

    /// A non-zero exit *for the reason the test is about*.
    ///
    /// Without the second half, every refusal test here would also pass
    /// against a `pacc` with no `sync` subcommand at all: clap exits non-zero
    /// for an unknown subcommand too, and "the command does not exist" is not
    /// the rejection any of these tests is asserting.
    #[track_caller]
    fn expect_failure(self, what: &str) -> String {
        assert!(
            !self.ok,
            "`pacc {what}` must exit non-zero; output was:\n{}",
            self.text
        );
        let lower = self.text.to_lowercase();
        assert!(
            !lower.contains("unrecognized subcommand") && !lower.contains("unknown subcommand"),
            "`pacc {what}` must fail on its arguments, not because the command \
             is missing; output was:\n{}",
            self.text
        );
        self.text
    }
}

/// One database file, driven only through the binary.
struct Device {
    db: PathBuf,
}

impl Device {
    /// Run `pacc` with no pairing phrase in the environment.
    ///
    /// `env_remove` is not tidiness: the phrase tests below assert what
    /// happens when none is supplied, and a `PACC_SYNC_PHRASE` set in whatever
    /// shell runs `cargo test` would quietly make them pass for the wrong
    /// reason.
    fn run(&self, args: &[&str]) -> Run {
        self.invoke(args, None)
    }

    /// Run `pacc` with the phrase in the environment, which is how the docs
    /// tell a person to supply it.
    fn run_sealed(&self, args: &[&str]) -> Run {
        self.invoke(args, Some(PHRASE))
    }

    fn invoke(&self, args: &[&str], phrase: Option<&str>) -> Run {
        let mut cmd = Command::cargo_bin("pacc").expect("the pacc binary must build");
        cmd.arg("--db").arg(&self.db);
        match phrase {
            Some(phrase) => cmd.env("PACC_SYNC_PHRASE", phrase),
            None => cmd.env_remove("PACC_SYNC_PHRASE"),
        };
        // `output()` blocks until the process exits, so a `sync` command that
        // ever decided to prompt for a phrase would hang this suite rather
        // than fail it. That is the reason the CLI refuses to prompt at all.
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

    /// This database's device name, as `pacc sync id` prints it.
    fn device_id(&self) -> String {
        let text = self.run(&["sync", "id"]).expect_ok("sync id");
        let first = text.lines().next().unwrap_or_default();
        let (label, id) = first.split_once(char::is_whitespace).unwrap_or_default();
        assert_eq!(label, "device", "`pacc sync id` names the device first");
        id.trim().to_string()
    }
}

/// A laptop and a phone, plus the folder they pass files through.
struct Pair {
    dir: TempDir,
    laptop: Device,
    phone: Device,
}

impl Pair {
    fn new() -> Pair {
        let dir = tempfile::tempdir().expect("tempdir");
        let laptop = Device {
            db: dir.path().join("laptop.db"),
        };
        laptop.run(&["init"]).expect_ok("init");

        // Migrated, named, and empty — see the module docs. `pacc init` would
        // seed a second chart of accounts, which is the one thing a joining
        // device must not do.
        let phone_db = dir.path().join("phone.db");
        drop(Db::open(&phone_db).expect("create the joining device's database"));

        Pair {
            dir,
            laptop,
            phone: Device { db: phone_db },
        }
    }

    /// A path in the shared folder, for a bundle.
    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("{name}.paccsync"))
    }
}

/// The number `pacc sync import` reported under one of its labels.
#[track_caller]
fn reported(text: &str, label: &str) -> usize {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(label))
        .unwrap_or_else(|| panic!("no `{label}` line in:\n{text}"));
    line.split_whitespace()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("`{line}` does not report a count"))
}

fn bytes_of(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read the bundle")
}

#[test]
fn a_device_names_itself_and_says_it_has_no_peers_yet() {
    let pair = Pair::new();
    let text = pair.laptop.run(&["sync", "id"]).expect_ok("sync id");

    let id = pair.laptop.device_id();
    assert!(!id.is_empty(), "a device mints a name on demand:\n{text}");
    assert!(
        text.contains("peers   none"),
        "a database that has synced with nobody says so:\n{text}"
    );
    // The name identifies the replica, so asking twice gives the same answer.
    assert_eq!(id, pair.laptop.device_id());
    assert_ne!(
        id,
        pair.phone.device_id(),
        "two databases are two devices, whatever the hardware"
    );
}

#[test]
fn a_change_made_on_one_device_is_visible_on_the_other() {
    // The whole point of the feature, asserted the way a user would check it:
    // by looking at the other device's screens.
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");

    pair.laptop
        .run(&["location", "--name", "Corner Shop", "--rate", "7.75"])
        .expect_ok("location");

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &pair.phone.device_id(),
        ])
        .expect_ok("sync export");
    assert!(bundle.exists(), "the bundle is written where it was asked");

    let applied = pair
        .phone
        .run_sealed(&["sync", "import", bundle.to_str().unwrap()])
        .expect_ok("sync import");
    assert!(reported(&applied, "applied") > 0, "{applied}");
    assert_eq!(reported(&applied, "deferred"), 0, "{applied}");

    let there = pair.phone.run(&["location"]).expect_ok("location");
    assert!(
        there.contains("Corner Shop"),
        "the shop entered on the laptop must show up on the phone:\n{there}"
    );
    // And the pairing carried the chart of accounts with it, which is what
    // makes the joining device usable at all.
    let categories = pair.phone.run(&["categories"]).expect_ok("categories");
    assert!(
        categories.contains("Groceries"),
        "the phone should have the laptop's categories:\n{categories}"
    );
}

#[test]
fn the_wrong_phrase_is_refused_and_neither_phrase_is_printed() {
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");

    pair.laptop
        .run(&["location", "--name", "Corner Shop", "--rate", "7.75"])
        .expect_ok("location");
    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &pair.phone.device_id(),
        ])
        .expect_ok("sync export");

    let wrong = "hunter2 is not the phrase";
    let text = pair
        .phone
        .run(&[
            "sync",
            "import",
            bundle.to_str().unwrap(),
            "--phrase",
            wrong,
        ])
        .expect_failure("sync import --phrase (wrong)");

    // A secret in an error message is a secret in every bug report that error
    // is ever pasted into.
    assert!(
        !text.contains(wrong) && !text.contains(PHRASE),
        "no phrase may be echoed back:\n{text}"
    );
    // And the refusal says both possibilities, rather than blaming a typo the
    // user may not have made.
    assert!(
        text.contains("does not match") && text.contains("altered"),
        "the refusal should explain itself:\n{text}"
    );

    let there = pair.phone.run(&["location"]).expect_ok("location");
    assert!(
        !there.contains("Corner Shop"),
        "nothing may be applied from a file that could not be opened:\n{there}"
    );
}

#[test]
fn a_missing_phrase_names_both_ways_to_supply_one() {
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");

    let text = pair
        .laptop
        .run(&["sync", "export", "--out", bundle.to_str().unwrap()])
        .expect_failure("sync export (no phrase)");

    assert!(
        text.contains("--phrase") && text.contains("PACC_SYNC_PHRASE"),
        "both ways of supplying a phrase should be named:\n{text}"
    );
    assert!(
        !bundle.exists(),
        "a refused export must not leave a file behind"
    );
}

#[test]
fn the_same_file_imported_twice_changes_nothing_the_second_time() {
    // A synced folder hands the same file over more than once, and a person
    // re-runs a command they are not sure worked. Both must be no-ops.
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");

    pair.laptop
        .run(&["location", "--name", "Corner Shop", "--rate", "7.75"])
        .expect_ok("location");
    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &pair.phone.device_id(),
        ])
        .expect_ok("sync export");

    let first = pair
        .phone
        .run_sealed(&["sync", "import", bundle.to_str().unwrap()])
        .expect_ok("sync import (first)");
    let second = pair
        .phone
        .run_sealed(&["sync", "import", bundle.to_str().unwrap()])
        .expect_ok("sync import (second)");

    assert!(reported(&first, "applied") > 0, "{first}");
    assert_eq!(
        reported(&second, "applied"),
        0,
        "the second read applies nothing:\n{second}"
    );
    assert_eq!(
        reported(&second, "conflicts"),
        0,
        "and re-reading a file is not a disagreement:\n{second}"
    );
    assert!(
        reported(&second, "ignored") > 0,
        "what it skipped should still be reported:\n{second}"
    );

    let there = pair.phone.run(&["location"]).expect_ok("location");
    assert_eq!(
        there.matches("Corner Shop").count(),
        1,
        "and the shop is there once, not twice:\n{there}"
    );
}

#[test]
fn exporting_refuses_to_overwrite_a_file_that_is_already_there() {
    // The file may be one the other device has not read yet, and it is the
    // only copy. `pacc backup` takes the same line for the same reason.
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");
    let peer = pair.phone.device_id();

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &peer,
        ])
        .expect_ok("sync export");
    let first = bytes_of(&bundle);

    let text = pair
        .laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &peer,
        ])
        .expect_failure("sync export (existing file)");

    assert!(
        text.contains("already exists"),
        "the refusal should say what is in the way:\n{text}"
    );
    assert_eq!(
        bytes_of(&bundle),
        first,
        "the file that was already there must be untouched"
    );
}

#[test]
fn a_deferred_change_is_reported_rather_than_swallowed() {
    // Built the way it happens in life: a bundle is written and marked as
    // delivered, then never read on the other device. Everything in it counts
    // as sent, so the next bundle carries only what came after — and what came
    // after points at something the other device has never seen.
    let pair = Pair::new();
    let first = pair.file("laptop-1");
    let second = pair.file("laptop-2");
    let peer = pair.phone.device_id();

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            first.to_str().unwrap(),
            "--peer",
            &peer,
        ])
        .expect_ok("sync export (1)");

    // Nothing imports `first`. This budget line points at a category that
    // travelled only in it.
    pair.laptop
        .run(&[
            "budget",
            "set",
            "--category",
            "Groceries",
            "--amount",
            "400",
        ])
        .expect_ok("budget set");

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            second.to_str().unwrap(),
            "--peer",
            &peer,
        ])
        .expect_ok("sync export (2)");

    let text = pair
        .phone
        .run_sealed(&["sync", "import", second.to_str().unwrap()])
        .expect_ok("sync import (2)");

    assert_eq!(
        reported(&text, "deferred"),
        1,
        "the budget line cannot land without its category:\n{text}"
    );
    assert!(
        text.contains("did NOT arrive"),
        "a deferred change is data that is still only on the other device, and \
         the report has to say so:\n{text}"
    );
    assert!(
        text.contains("budget_line") && text.contains("needs category"),
        "it should name what is missing and what wanted it:\n{text}"
    );

    // The claim behind the number: nothing arrived.
    let budgets = pair.phone.run(&["budget", "list"]).expect_ok("budget list");
    assert!(
        !budgets.contains("Groceries"),
        "a deferred change must not be half-applied:\n{budgets}"
    );
}

#[test]
fn status_shows_where_each_device_has_got_to() {
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");
    let phone_id = pair.phone.device_id();
    let laptop_id = pair.laptop.device_id();

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &phone_id,
        ])
        .expect_ok("sync export");
    pair.phone
        .run_sealed(&["sync", "import", bundle.to_str().unwrap()])
        .expect_ok("sync import");

    let laptop = pair
        .laptop
        .run(&["sync", "status"])
        .expect_ok("sync status");
    assert!(laptop.contains(&laptop_id), "{laptop}");
    assert!(
        laptop.contains(&phone_id),
        "the peer it has written to:\n{laptop}"
    );
    assert!(
        !laptop.contains("sent through  nothing yet"),
        "the laptop has handed something over:\n{laptop}"
    );
    assert!(
        laptop.contains("seen through  nothing yet"),
        "and has read nothing back:\n{laptop}"
    );
    assert!(laptop.contains("conflicts  none open"), "{laptop}");

    let phone = pair.phone.run(&["sync", "status"]).expect_ok("sync status");
    assert!(phone.contains(&laptop_id), "{phone}");
    assert!(
        !phone.contains("seen through  nothing yet"),
        "the phone has read something:\n{phone}"
    );
    assert!(
        phone.contains("sent through  nothing yet"),
        "and has sent nothing:\n{phone}"
    );

    // `sync id` agrees with it: the peer is known, and is the default.
    let id = pair.phone.run(&["sync", "id"]).expect_ok("sync id");
    assert!(id.contains(&laptop_id), "{id}");
    assert!(id.contains("the default for"), "{id}");
}

#[test]
fn the_first_bundle_of_a_pairing_says_so_and_invents_no_peer() {
    // Exporting before either device has heard of the other is how a pairing
    // starts. There is nobody to mark the bundle as delivered to, and making
    // one up would leave a device in `pacc sync status` that does not exist —
    // and would then be the "only peer" that a later `--peer`-less export
    // silently addressed.
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");

    let text = pair
        .laptop
        .run_sealed(&["sync", "export", "--out", bundle.to_str().unwrap()])
        .expect_ok("sync export (unpaired)");
    assert!(
        text.contains("first bundle of a pairing"),
        "an export with nobody to address should say what it is:\n{text}"
    );

    let status = pair
        .laptop
        .run(&["sync", "status"])
        .expect_ok("sync status");
    assert!(
        status.contains("peers      none"),
        "no phantom peer may be recorded:\n{status}"
    );

    // And the file is a real bundle: the placeholder is a label, not a
    // restriction on what went into it.
    let applied = pair
        .phone
        .run_sealed(&["sync", "import", bundle.to_str().unwrap()])
        .expect_ok("sync import");
    assert!(reported(&applied, "applied") > 0, "{applied}");
    let categories = pair.phone.run(&["categories"]).expect_ok("categories");
    assert!(categories.contains("Groceries"), "{categories}");
}

#[test]
fn both_devices_editing_the_same_thing_is_reported_and_the_loser_is_kept() {
    let pair = Pair::new();
    let first = pair.file("laptop-1");
    let second = pair.file("laptop-2");
    let peer = pair.phone.device_id();

    pair.laptop
        .run(&["location", "--name", "Corner Shop", "--rate", "7.75"])
        .expect_ok("location");
    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            first.to_str().unwrap(),
            "--peer",
            &peer,
        ])
        .expect_ok("sync export (1)");
    pair.phone
        .run_sealed(&["sync", "import", first.to_str().unwrap()])
        .expect_ok("sync import (1)");

    // Two people, one shop, two different corrections. The phone's is first in
    // real time, so the laptop's is the one that wins.
    pair.phone
        .run(&["location", "--name", "Corner Shop", "--rate", "8.5"])
        .expect_ok("location (phone)");
    pair.laptop
        .run(&["location", "--name", "Corner Shop", "--rate", "9"])
        .expect_ok("location (laptop)");

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            second.to_str().unwrap(),
            "--peer",
            &peer,
        ])
        .expect_ok("sync export (2)");
    let text = pair
        .phone
        .run_sealed(&["sync", "import", second.to_str().unwrap()])
        .expect_ok("sync import (2)");

    assert_eq!(reported(&text, "conflicts"), 1, "{text}");
    assert!(
        text.contains("kept rather than dropped"),
        "the losing version is kept, and the report should say so:\n{text}"
    );

    let status = pair.phone.run(&["sync", "status"]).expect_ok("sync status");
    assert!(
        status.contains("conflicts  1 open"),
        "an unresolved disagreement stays visible:\n{status}"
    );
}

#[test]
fn two_separately_seeded_databases_are_told_why_they_cannot_merge() {
    // The mistake anybody would make first: run `pacc init` on both devices.
    // Each seeds its own chart of accounts, so the names collide while the
    // uuids do not, and the schema refuses. The raw SQLite text describes the
    // symptom; the CLI has to describe the cause.
    let pair = Pair::new();
    let bundle = pair.file("laptop-1");

    let other = Device {
        db: pair.dir.path().join("second-init.db"),
    };
    other.run(&["init"]).expect_ok("init");

    pair.laptop
        .run_sealed(&[
            "sync",
            "export",
            "--out",
            bundle.to_str().unwrap(),
            "--peer",
            &other.device_id(),
        ])
        .expect_ok("sync export");

    let text = other
        .run_sealed(&["sync", "import", bundle.to_str().unwrap()])
        .expect_failure("sync import (two seeded databases)");
    assert!(
        text.contains("seed"),
        "the refusal should name the cause, not only the constraint:\n{text}"
    );
}
