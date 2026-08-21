//! Black-box tests for `pacc budget` and `pacc report`.
//!
//! The milestone's headline promise is a sentence a user can check: a $1200
//! yearly premium budgeted from January reads, in August, as $100.00 a month
//! — and the same $1200, paid in March, does not wreck March in the amortized
//! report.
//!
//! # A funded fund is set up through the front door
//!
//! A sinking fund's balance is what was actually set aside, not what the
//! schedule planned, so getting one to $800.00 takes eight recorded
//! contributions and `pacc budget contribute` is what records them. The tests
//! below fund the envelope by running that command, never by reaching past the
//! CLI to `budgets::set_contribution`: a test that seeds a state no user can
//! produce would keep passing on the day the command stopped existing, which is
//! exactly the regression this file is here to catch.
//!
//! These tests drive the real binary against a real database file, so they
//! cover the parts nothing else does: clap's wiring for the `--period` family
//! of flags, the exit codes, and the error messages. Where a claim is about
//! *arithmetic* it is asserted through `accounting-core` against the same file
//! after the process exits — the core suite owns the per-month figures, and a
//! CLI test that scrapes a table for them is really a test of the column
//! layout.

use std::path::PathBuf;

use accounting_core::db::{Db, check_all};
use accounting_core::domain::budget_calc::OnTrack;
use accounting_core::repo::budgets;
use accounting_core::repo::categories;
use accounting_core::service::budgets as budget_service;
use accounting_core::{CategoryId, Money, YearMonth};
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

    /// A non-zero exit *for the reason the test is about*.
    ///
    /// Without the second half, every refusal test in this file would pass
    /// against a `pacc` that has no `budget` subcommand at all: clap exits
    /// non-zero for an unknown subcommand too, and "the command does not
    /// exist" is not the rejection any of these tests is asserting.
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

    /// The $1200-a-year insurance line the milestone is written around.
    fn set_yearly_insurance(&self) {
        self.run(&[
            "budget",
            "set",
            "--category",
            "Car Insurance",
            "--amount",
            "1200",
            "--period",
            "yearly",
            "--start",
            "2026-01",
            "--rollover",
        ])
        .expect_ok("budget set");
    }

    /// Save exactly the plan into that line, month by month, through the CLI.
    ///
    /// A bare `contribute` is the whole point of the default: this is what a
    /// person who paid in on schedule actually types, so it is what the tests
    /// type. Eight of these is what turns "behind by the whole schedule" into
    /// the $800.00 the milestone promises by August.
    fn contribute_on_plan(&self, months: &[&str]) {
        for month in months {
            self.run(&[
                "budget",
                "contribute",
                "--category",
                "Car Insurance",
                "--month",
                month,
            ])
            .expect_ok("budget contribute");
        }
    }

    /// January through August 2026, the milestone's window.
    fn fund_through_august(&self) {
        self.contribute_on_plan(&[
            "2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06", "2026-07", "2026-08",
        ]);
    }
}

// ---------------------------------------------------------------------------
// Reading amounts out of a table
// ---------------------------------------------------------------------------

/// `1200.00` written the way a grouped display would write it: `1,200.00`.
fn grouped(major: &str) -> String {
    let (whole, frac) = major.split_once('.').unwrap_or((major, ""));
    let mut out = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// How many times an amount appears, in either spelling.
///
/// `Money` has a plain form (`1200.00`) and a display form (`$1,200.00`), and
/// which one a given table uses is a formatting decision. Counting both is what
/// keeps these tests about the report rather than about the formatter.
fn times_shown(text: &str, major: &str) -> usize {
    let plain = text.matches(major).count();
    let grouped_form = grouped(major);
    if grouped_form == major {
        plain
    } else {
        plain + text.matches(&grouped_form).count()
    }
}

#[track_caller]
fn assert_shows(text: &str, major: &str, what: &str) {
    assert!(
        times_shown(text, major) > 0,
        "{what}: expected `{major}` (or `{}`) in the output; got:\n{text}",
        grouped(major)
    );
}

#[track_caller]
fn assert_does_not_show(text: &str, major: &str, what: &str) {
    assert!(
        times_shown(text, major) == 0,
        "{what}: `{major}` must not appear; got:\n{text}"
    );
}

fn category(db: &Db, name: &str) -> CategoryId {
    categories::resolve(db, name)
        .unwrap_or_else(|e| panic!("resolve `{name}`: {e}"))
        .id
}

fn ym(year: i32, month: u32) -> YearMonth {
    YearMonth::new(year, month).expect("valid month")
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
// budget set / list
// ---------------------------------------------------------------------------

#[test]
fn a_budget_line_that_was_set_shows_up_in_the_list() {
    let f = Fixture::new();
    f.set_yearly_insurance();

    let listed = f.run(&["budget", "list"]).expect_ok("budget list");
    assert!(
        listed.contains("Car Insurance"),
        "the list must name the category; got:\n{listed}"
    );
    assert_shows(&listed, "1200.00", "the budgeted amount");

    // And the stored shape, which the output format cannot tell us about.
    let db = f.open();
    let insurance = category(&db, "Car Insurance");
    let stored = budgets::for_category(&db, insurance, ym(2026, 8))
        .expect("for_category")
        .expect("the line must be in force in August");
    assert_eq!(stored.line.amount, Money::from_cents(120_000));
    assert_eq!(stored.line.period_start, ym(2026, 1));
    assert!(
        stored.line.rollover,
        "--rollover is what makes a sinking fund possible; it must reach the row"
    );
    assert_clean(&db);
}

#[test]
fn budget_list_filtered_by_month_shows_only_the_lines_in_force() {
    let f = Fixture::new();
    f.set_yearly_insurance();

    let in_force = f
        .run(&["budget", "list", "--month", "2026-06"])
        .expect_ok("budget list --month 2026-06");
    assert!(
        in_force.contains("Car Insurance"),
        "the line is in force in June; got:\n{in_force}"
    );

    let before = f
        .run(&["budget", "list", "--month", "2025-12"])
        .expect_ok("budget list --month 2025-12");
    assert!(
        !before.contains("Car Insurance"),
        "the line starts in 2026-01 and must not be listed before that; got:\n{before}"
    );
}

#[test]
fn an_unknown_category_is_refused_and_named() {
    let f = Fixture::new();

    let out = f
        .run(&[
            "budget",
            "set",
            "--category",
            "Zorblatt",
            "--amount",
            "100",
            "--period",
            "monthly",
            "--start",
            "2026-01",
        ])
        .expect_failure("budget set with an unknown category");
    assert!(
        out.contains("Zorblatt"),
        "the message must echo the name that was not found; got:\n{out}"
    );

    let db = f.open();
    assert!(
        budgets::list(&db, None).expect("list").is_empty(),
        "a refused `budget set` must write nothing"
    );
}

#[test]
fn every_n_without_an_interval_is_refused() {
    // `--period every-n` alone is meaningless: every *how many* months? Left
    // to a default it would silently budget something other than what the user
    // meant, and the mistake would only surface as a wrong figure months later.
    let f = Fixture::new();

    let out = f
        .run(&[
            "budget",
            "set",
            "--category",
            "Car Repair",
            "--amount",
            "300",
            "--period",
            "every-n",
            "--start",
            "2026-01",
        ])
        .expect_failure("budget set --period every-n with no --every");
    assert!(
        !out.trim().is_empty(),
        "a refusal must say something; got an empty output"
    );

    let db = f.open();
    assert!(budgets::list(&db, None).expect("list").is_empty());
}

#[test]
fn by_date_without_a_due_date_is_refused() {
    // The same rule for the other flag-dependent cadence: a `by-date` line with
    // no date to save toward has no schedule at all.
    let f = Fixture::new();

    f.run(&[
        "budget",
        "set",
        "--category",
        "Gifts",
        "--amount",
        "600",
        "--period",
        "by-date",
        "--start",
        "2026-01",
    ])
    .expect_failure("budget set --period by-date with no --due");

    let db = f.open();
    assert!(budgets::list(&db, None).expect("list").is_empty());
}

#[test]
fn an_every_n_line_with_its_interval_is_accepted() {
    let f = Fixture::new();
    f.run(&[
        "budget",
        "set",
        "--category",
        "Car Repair",
        "--amount",
        "300",
        "--period",
        "every-n",
        "--every",
        "3",
        "--start",
        "2026-01",
        "--rollover",
    ])
    .expect_ok("budget set --period every-n --every 3");

    let db = f.open();
    let auto = category(&db, "Car Repair");
    let stored = budgets::for_category(&db, auto, ym(2026, 2))
        .expect("for_category")
        .expect("the line is in force");
    assert_eq!(
        stored.line.period,
        accounting_core::domain::budget_calc::Period::EveryNMonths(3)
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// budget status — the milestone's acceptance check
// ---------------------------------------------------------------------------

#[test]
fn budget_status_shows_a_hundred_a_month_and_eight_hundred_funded_by_august() {
    // The milestone's headline sentence, end to end through the binary: $1200 a
    // year from January is $100.00 a month, and eight months of paying in on
    // plan is $800.00 sitting in the fund by August.
    //
    // Both halves are asserted, because the interesting thing is that they are
    // no longer the same statement. Before a single contribution is recorded
    // the same line reads $0.00 funded and behind by the full $800.00 — a
    // sinking fund holds what was paid into it, not what the plan hoped for,
    // and the old reading answered "will the $1,200 be there in December" out
    // of the schedule that asked the question.
    let f = Fixture::new();
    f.set_yearly_insurance();

    let unfunded = f
        .run(&["budget", "status", "--month", "2026-08"])
        .expect_ok("budget status before anything is set aside");
    assert!(
        unfunded.contains("Car Insurance"),
        "the status must name the category; got:\n{unfunded}"
    );
    assert_shows(
        &unfunded,
        "800.00",
        "the shortfall against eight months of schedule",
    );
    // The funding cell itself, not merely the word somewhere on screen: the
    // footer explains what `behind` means and so contains it either way.
    assert!(
        unfunded.contains("behind $800.00"),
        "a fund nobody has paid into must read as behind by the whole schedule; got:\n{unfunded}"
    );

    f.fund_through_august();

    let out = f
        .run(&["budget", "status", "--month", "2026-08"])
        .expect_ok("budget status --month 2026-08");
    assert_shows(
        &out,
        "100.00",
        "the monthly contribution the schedule calls for",
    );
    assert_shows(&out, "800.00", "eight months of contributions, banked");
    assert!(
        out.contains("on track") && !out.contains("behind $"),
        "a fund paid into exactly on plan is level with its schedule, not behind \
         anything; got:\n{out}"
    );

    // The figures themselves, where the table layout cannot get in the way.
    let db = f.open();
    let insurance = category(&db, "Car Insurance");
    let status =
        budget_service::budget_status_for(&db, insurance, ym(2026, 8)).expect("budget_status_for");
    let funded = status.status.as_ref().expect("a line means a status");
    assert_eq!(funded.monthly_contribution, Money::from_cents(10_000));
    assert_eq!(funded.contributed_to_date, Money::from_cents(80_000));
    assert_eq!(funded.funded_balance, Money::from_cents(80_000));
    assert_eq!(
        funded.on_track,
        OnTrack::OnTrack,
        "level with the schedule, because every month of it was actually saved"
    );
    assert_clean(&db);
}

#[test]
fn a_bare_contribution_records_the_plan_and_clear_takes_the_month_back() {
    // The two things `contribute` has to get right beyond writing a number.
    //
    // A bare invocation means "saved exactly the plan", which is the month a
    // person should not have to type an amount for — eleven of those a year is
    // how a fund goes unrecorded and reads `behind` forever.
    //
    // And `--clear` returns a month to *nothing recorded*, which is not the
    // same as a recorded $0.00 even though both leave the balance alone: one
    // says nobody has answered yet, the other says the answer was nothing, and
    // only the first is something a screen should offer to fill in.
    let f = Fixture::new();
    f.set_yearly_insurance();

    let out = f
        .run(&[
            "budget",
            "contribute",
            "--category",
            "Car Insurance",
            "--month",
            "2026-03",
        ])
        .expect_ok("budget contribute with no --amount");
    assert_shows(&out, "100.00", "a bare contribution records the plan");

    // A short month is said out loud instead.
    f.run(&[
        "budget",
        "contribute",
        "--category",
        "Car Insurance",
        "--month",
        "2026-04",
        "--amount",
        "40",
    ])
    .expect_ok("budget contribute --amount 40");

    let db = f.open();
    let insurance = category(&db, "Car Insurance");
    let march = budget_service::budget_status_for(&db, insurance, ym(2026, 3))
        .expect("budget_status_for March");
    assert_eq!(
        march.contributed_this_month,
        Some(Money::from_cents(10_000)),
        "a bare contribution defaults to the month's planned amount"
    );
    let april = budget_service::budget_status_for(&db, insurance, ym(2026, 4))
        .expect("budget_status_for April");
    assert_eq!(april.contributed_this_month, Some(Money::from_cents(4_000)));
    assert_eq!(
        april.status.as_ref().expect("status").funded_balance,
        Money::from_cents(14_000),
        "the fund holds the two figures that were recorded and nothing else"
    );
    drop(db);

    // A second figure for a month already recorded is a correction, not a
    // second deposit.
    f.run(&[
        "budget",
        "contribute",
        "--category",
        "Car Insurance",
        "--month",
        "2026-03",
        "--amount",
        "60",
    ])
    .expect_ok("budget contribute over an existing month");
    let db = f.open();
    assert_eq!(
        budget_service::budget_status_for(&db, insurance, ym(2026, 3))
            .expect("budget_status_for March again")
            .contributed_this_month,
        Some(Money::from_cents(6_000)),
        "recording twice corrects the month rather than adding to it"
    );
    drop(db);

    f.run(&[
        "budget",
        "contribute",
        "--category",
        "Car Insurance",
        "--month",
        "2026-03",
        "--clear",
    ])
    .expect_ok("budget contribute --clear");

    let db = f.open();
    let march = budget_service::budget_status_for(&db, insurance, ym(2026, 3))
        .expect("budget_status_for March after clearing");
    assert_eq!(
        march.contributed_this_month, None,
        "--clear returns the month to nothing-recorded, not to a recorded zero"
    );
    assert_eq!(
        budget_service::budget_status_for(&db, insurance, ym(2026, 4))
            .expect("budget_status_for April after clearing March")
            .status
            .as_ref()
            .expect("status")
            .funded_balance,
        Money::from_cents(4_000),
        "clearing March takes March's money back out of the fund"
    );
    assert_clean(&db);
}

#[test]
fn an_adjustment_changes_what_status_reports() {
    let f = Fixture::new();
    f.set_yearly_insurance();
    f.fund_through_august();

    // $800.00 banked and level with the schedule. An adjustment is deliberately
    // not a contribution: it moves the balance without claiming anybody paid in
    // on plan, which is what makes "moved from Gifts" tellable apart from
    // "saved as scheduled" six months later.
    let before = f
        .run(&["budget", "status", "--month", "2026-08"])
        .expect_ok("budget status before the adjustment");
    assert_shows(&before, "800.00", "the balance before any adjustment");
    assert_does_not_show(&before, "850.00", "the balance the adjustment will leave");

    f.run(&[
        "budget",
        "adjust",
        "--category",
        "Car Insurance",
        "--month",
        "2026-08",
        "--amount",
        "50",
        "--reason",
        "moved from Gifts",
    ])
    .expect_ok("budget adjust");

    let after = f
        .run(&["budget", "status", "--month", "2026-08"])
        .expect_ok("budget status after the adjustment");
    assert_ne!(
        before, after,
        "an adjustment that changes nothing on screen is invisible to the user"
    );
    assert_shows(&after, "850.00", "the balance after the adjustment");
    assert!(
        after.contains("ahead $50.00"),
        "a fund holding more than the schedule asked for is ahead of it; got:\n{after}"
    );

    let db = f.open();
    let insurance = category(&db, "Car Insurance");
    let status =
        budget_service::budget_status_for(&db, insurance, ym(2026, 8)).expect("budget_status_for");
    let funded = status.status.as_ref().expect("status");
    assert_eq!(funded.funded_balance, Money::from_cents(85_000));
    assert_eq!(
        funded.contributed_to_date,
        Money::from_cents(80_000),
        "an adjustment is not a contribution"
    );
    assert_eq!(funded.on_track, OnTrack::Ahead(Money::from_cents(5_000)));
    assert_clean(&db);
}

#[test]
fn a_negative_adjustment_moves_money_out_of_an_envelope() {
    // Half of what adjustments are for: moving money *out* of one envelope into
    // another, and writing an overspend off so it stops propagating through
    // every later rollover balance. A CLI that can only add is half a lever —
    // and `--amount -25` is exactly the shape an argument parser eats as a
    // flag, so it is worth spending a test on.
    let f = Fixture::new();
    f.set_yearly_insurance();
    f.fund_through_august();

    f.run(&[
        "budget",
        "adjust",
        "--category",
        "Car Insurance",
        "--month",
        "2026-08",
        "--amount",
        "-25",
        "--reason",
        "moved to Travel",
    ])
    .expect_ok("budget adjust with a negative amount");

    let db = f.open();
    let insurance = category(&db, "Car Insurance");
    let status =
        budget_service::budget_status_for(&db, insurance, ym(2026, 8)).expect("budget_status_for");
    let funded = status.status.as_ref().expect("status");
    // Eight months paid in, less the $25.00 moved out — and the fund is now
    // short of its own schedule by exactly that, which is the figure that must
    // keep rolling forward rather than being quietly forgotten in September.
    assert_eq!(funded.funded_balance, Money::from_cents(77_500));
    assert_eq!(funded.on_track, OnTrack::Behind(Money::from_cents(2_500)));
    assert_clean(&db);
}

#[test]
fn spending_shows_up_in_the_status_for_the_month_it_was_dated() {
    let f = Fixture::new();
    f.run(&[
        "budget",
        "set",
        "--category",
        "Groceries",
        "--amount",
        "400",
        "--period",
        "monthly",
        "--start",
        "2026-01",
        "--no-rollover",
    ])
    .expect_ok("budget set monthly");
    f.run(&[
        "add",
        "--date",
        "2026-08-09",
        "--merchant",
        "Costco",
        "--total",
        "125.00",
        "--category",
        "Groceries",
    ])
    .expect_ok("add");

    let out = f
        .run(&["budget", "status", "--month", "2026-08"])
        .expect_ok("budget status");
    assert_shows(&out, "125.00", "August's grocery spending");

    let db = f.open();
    let groceries = category(&db, "Groceries");
    let status =
        budget_service::budget_status_for(&db, groceries, ym(2026, 8)).expect("budget_status_for");
    assert_eq!(status.spent_this_month, Money::from_cents(12_500));
    // $400.00 budgeted, $125.00 spent: not over budget, whatever the funding
    // measure says about the envelope being short of its schedule.
    assert_eq!(
        status.status.as_ref().expect("status").spend.overspent_by,
        Money::ZERO
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------

#[test]
fn the_two_report_bases_differ_in_march_and_agree_in_total() {
    // $1200 paid in March against a $1200/yr line. Cash flow puts the whole
    // amount in March; amortized spreads it over the twelve months of the
    // cycle. Both add up to $1,200.00 for the year, so the $1,200.00 figure
    // appears in *both* outputs — but only the cash-flow one shows it as a
    // month's spending, which is what the extra occurrences below are.
    let f = Fixture::new();
    f.set_yearly_insurance();
    f.run(&[
        "add",
        "--date",
        "2026-03-14",
        "--merchant",
        "State Farm",
        "--total",
        "1200.00",
        "--category",
        "Car Insurance",
    ])
    .expect_ok("add the premium");

    let cashflow = f
        .run(&["report", "--year", "2026", "--basis", "cashflow"])
        .expect_ok("report --basis cashflow");
    let amortized = f
        .run(&["report", "--year", "2026", "--basis", "amortized"])
        .expect_ok("report --basis amortized");

    assert_shows(
        &cashflow,
        "1200.00",
        "the year's total in the cash-flow view",
    );
    assert_shows(
        &amortized,
        "1200.00",
        "the year's total in the amortized view",
    );
    assert_shows(&amortized, "100.00", "the amortized monthly share");
    assert_ne!(
        cashflow, amortized,
        "the two bases must not render identically — that is the whole point of \
         having them"
    );
    assert!(
        times_shown(&cashflow, "1200.00") > times_shown(&amortized, "1200.00"),
        "cash flow shows $1,200.00 as a month's spending as well as a total; \
         amortized never shows it in a single month.\ncash flow:\n{cashflow}\n\
         amortized:\n{amortized}"
    );

    // The per-month arithmetic itself is asserted in the core suite, where it
    // does not depend on a column layout.
    let db = f.open();
    assert_clean(&db);
}

#[test]
fn report_without_a_basis_uses_amortized() {
    // The default has to be visible in the output: a reader who cannot tell
    // which basis a report is in cannot interpret a single number in it.
    let f = Fixture::new();
    f.set_yearly_insurance();
    f.run(&[
        "add",
        "--date",
        "2026-03-14",
        "--total",
        "1200.00",
        "--category",
        "Car Insurance",
    ])
    .expect_ok("add the premium");

    let default = f.run(&["report", "--year", "2026"]).expect_ok("report");
    assert!(
        default.to_lowercase().contains("amortiz"),
        "the report must name its basis; got:\n{default}"
    );

    let explicit = f
        .run(&["report", "--year", "2026", "--basis", "amortized"])
        .expect_ok("report --basis amortized");
    assert_eq!(
        default, explicit,
        "no `--basis` must mean exactly `--basis amortized`"
    );
}

#[test]
fn a_report_over_an_explicit_month_range_covers_those_months() {
    let f = Fixture::new();
    f.run(&[
        "add",
        "--date",
        "2026-02-10",
        "--total",
        "60.00",
        "--category",
        "Groceries",
    ])
    .expect_ok("add");
    f.run(&[
        "add",
        "--date",
        "2026-09-10",
        "--total",
        "70.00",
        "--category",
        "Groceries",
    ])
    .expect_ok("add");

    let out = f
        .run(&["report", "--from", "2026-01", "--to", "2026-03"])
        .expect_ok("report --from 2026-01 --to 2026-03");
    assert_shows(&out, "60.00", "February's spending");
    assert_does_not_show(&out, "70.00", "September is outside the range");
}

// ---------------------------------------------------------------------------
// The end-to-end proof
// ---------------------------------------------------------------------------

#[test]
fn the_whole_budget_path_leaves_the_database_healthy() {
    // Every budget command the CLI offers, into one database, then `pacc
    // doctor`. Budget lines and adjustments do not participate in the
    // transaction aggregate invariants, so the risk here is the opposite one —
    // a budget write that disturbs something that *does* — and this is what
    // would catch it.
    let f = Fixture::new();
    f.set_yearly_insurance();
    f.run(&[
        "budget",
        "set",
        "--category",
        "Groceries",
        "--amount",
        "400",
        "--period",
        "monthly",
        "--start",
        "2026-01",
        "--no-rollover",
        "--note",
        "reviewed in January",
    ])
    .expect_ok("budget set monthly");
    f.run(&[
        "budget",
        "set",
        "--category",
        "Car Repair",
        "--amount",
        "300",
        "--period",
        "every-n",
        "--every",
        "3",
        "--start",
        "2026-01",
        "--rollover",
    ])
    .expect_ok("budget set every-n");
    f.run(&[
        "budget",
        "set",
        "--category",
        "Gifts",
        "--amount",
        "600",
        "--period",
        "by-date",
        "--due",
        "2026-12-24",
        "--start",
        "2026-01",
        "--rollover",
    ])
    .expect_ok("budget set by-date");
    f.run(&[
        "budget",
        "adjust",
        "--category",
        "Gifts",
        "--month",
        "2026-05",
        "--amount",
        "25",
        "--reason",
        "moved from Travel",
    ])
    .expect_ok("budget adjust");

    f.run(&[
        "add",
        "--date",
        "2026-03-14",
        "--merchant",
        "State Farm",
        "--total",
        "1200.00",
        "--category",
        "Car Insurance",
    ])
    .expect_ok("add");
    f.run(&[
        "add",
        "--date",
        "2026-08-09",
        "--merchant",
        "Costco",
        "--total",
        "125.00",
        "--category",
        "Groceries",
    ])
    .expect_ok("add");

    f.run(&["budget", "list"]).expect_ok("budget list");
    f.run(&["budget", "status", "--month", "2026-08"])
        .expect_ok("budget status");
    f.run(&["report", "--year", "2026"]).expect_ok("report");
    f.run(&["report", "--year", "2026", "--basis", "cashflow"])
        .expect_ok("report --basis cashflow");

    let doctor = f.run(&["doctor"]).expect_ok("doctor");
    assert!(
        doctor.contains("0 errors, 0 warnings"),
        "the budget commands must leave nothing for `doctor` to report; got:\n{doctor}"
    );

    let db = f.open();
    assert_clean(&db);
    assert_eq!(
        budgets::list(&db, Some(ym(2026, 6)))
            .expect("list")
            .iter()
            .filter(|l| l.category_name == "Gifts")
            .count(),
        1,
        "each category has exactly one line in force in a given month"
    );
}
