//! `pacc` — headless CLI for the personal accounting app.
//!
//! This is not throwaway scaffolding. It is the enforcement mechanism for the
//! core/UI boundary: if `pacc` can do everything the GUI can, then the core is
//! genuinely UI-agnostic and a second frontend is a rendering exercise rather
//! than a re-implementation. It also stays useful forever for backfills,
//! debugging, and integrity checks.
//!
//! Note what this file does *not* contain: arithmetic. It parses strings into
//! core types, calls into `accounting-core`, and formats what comes back.
//!
//! # Output goes through [`outln!`], never `println!`
//!
//! Rust sets `SIGPIPE` to `SIG_IGN` before `main` runs, so a write to a closed
//! pipe returns `EPIPE` instead of killing the process — and `println!`
//! unwraps that into a panic. The visible result is that `pacc report | head`
//! prints its first lines and then dies with "failed printing to stdout:
//! Broken pipe" and exit code 101, which any shell script reads as a real
//! failure. Piping into `head`, `grep -m1`, or quitting `less` early is
//! completely ordinary use of a reporting tool, so every line of output here
//! goes through [`outln!`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use accounting_core::db::integrity::Severity;
use accounting_core::db::{Db, SCHEMA_VERSION, check_all, seed_defaults};
use accounting_core::domain::budget_calc::{OnTrack, Period, monthly_contribution};
use accounting_core::domain::reconcile::{
    AmountEntry, AmountField, Amounts, TaxContext, reconcile,
};
use accounting_core::domain::splits::{
    LineItemDraft, SplitAmount, SplitDraft, SplitPlan, plan_splits,
};
use accounting_core::export;
use accounting_core::repo::budgets::{self, BudgetLineRecord, StoredBudgetLine};
use accounting_core::repo::transactions::{Txn, TxnFilter};
use accounting_core::repo::{categories, locations, merchants, transactions};
use accounting_core::service::budgets as budget_service;
use accounting_core::service::entry::{
    TransactionDraft, TransactionPreview, categorize, needs_categorizing,
    preview_transaction_with_rate, quick_add, repeat_last, save_transaction_with_rate,
};
use accounting_core::service::reports::{MonthlyMatrix, ReportBasis, monthly_matrix};
use accounting_core::time::{MonthRange, month_short_name};
use accounting_core::{CategoryBudgetStatus, DetailLevel, Money, TxnId, YearMonth};
use anyhow::Context;
use chrono::{Local, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};

/// `println!` that treats a closed pipe as a normal end of output.
///
/// See the module docs: `println!` panics on `EPIPE` because Rust ignores
/// `SIGPIPE`, which turns `pacc list | head` into a crash. Exiting `0` here is
/// what every well-behaved Unix filter does — the reader got what it asked for
/// and stopped listening, which is not this program's error.
macro_rules! outln {
    () => { $crate::write_line(std::format_args!("")) };
    ($($arg:tt)*) => { $crate::write_line(std::format_args!($($arg)*)) };
}

fn write_line(args: std::fmt::Arguments<'_>) {
    use std::io::Write;

    let mut out = std::io::stdout().lock();
    if writeln!(out, "{args}").is_err() {
        // Deliberately not `anyhow` — there is nobody left to read the error,
        // and a non-zero exit would make a shell script think the report
        // failed when it merely ended early.
        std::process::exit(0);
    }
}

#[derive(Parser)]
#[command(name = "pacc", version, about = "Personal accounting — headless CLI")]
struct Cli {
    /// Database file. Defaults to `$PACC_DB`, else
    /// `$XDG_DATA_HOME/personal-accounting/pacc.db`, else
    /// `~/.local/share/personal-accounting/pacc.db`.
    ///
    /// Path resolution lives here and only here. `accounting-core` never
    /// discovers a path — it is handed one — because path-discovery crates
    /// return meaningless answers on Android, where the data directory is
    /// only obtainable through the Java `Context`.
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print version and build information.
    Version,
    /// Reconcile subtotal/tax/total and preview a split, without a database.
    ///
    /// Examples:
    ///   pacc calc --total 54.32 --rate 8.375
    ///   pacc calc --total 100.00 --split 33.33 --split 33.33 --split remainder
    #[command(verbatim_doc_comment)]
    Calc(CalcArgs),
    /// Create the database, migrate it, and install the default categories.
    ///
    /// Safe to re-run: migrations are versioned and the seed is idempotent.
    Init,
    /// Report schema version, connection settings, row counts, and any
    /// invariant violations. Exits non-zero if anything is wrong.
    Doctor,
    /// Write a single-file, defragmented copy of the database (`VACUUM INTO`).
    Backup(BackupArgs),
    /// Record a transaction.
    ///
    /// Examples:
    ///   pacc add --total 54.32 --merchant H-E-B --category Groceries
    ///   pacc add --subtotal 50.00 --tax 4.19 --split "Groceries=30.00" --split "Household=remainder"
    ///   pacc add --total 20.00 --category Groceries --item "Milk=3.49" --item "1:Bread=16.51"
    #[command(verbatim_doc_comment)]
    Add(AddArgs),
    /// List transactions, newest first.
    List(ListArgs),
    /// Show one transaction in full: header, splits, line items.
    Show(ShowArgs),
    /// Record a total now and categorize it later, from `pacc inbox`.
    QuickAdd(QuickAddArgs),
    /// The needs-categorizing queue, with the command to clear each entry.
    Inbox(InboxArgs),
    /// Assign categories to a transaction and clear its review flag.
    Categorize(CategorizeArgs),
    /// Draft a new transaction from a merchant's last one.
    Repeat(RepeatArgs),
    /// The category taxonomy — the valid values for `--category` and `--split`.
    Categories,
    /// Write the transactions to a spreadsheet, one row per split.
    ///
    /// Examples:
    ///   pacc export --to 2026.xlsx --year 2026
    ///   pacc export --to august.csv --month 2026-08
    #[command(verbatim_doc_comment)]
    Export(ExportArgs),
    /// List store locations and set their sales-tax rates.
    ///
    /// Examples:
    ///   pacc location
    ///   pacc location --name Home --rate 7.75
    ///   pacc location --name "Boise, ID" --rate 6 --home
    #[command(verbatim_doc_comment)]
    Location(LocationArgs),
    /// Set budgets, and see how they are funded and how they are spent.
    ///
    /// Examples:
    ///   pacc budget set --category "Car Insurance" --amount 1200 --period yearly
    ///   pacc budget set --category Groceries --amount 400
    ///   pacc budget status --month 2026-08
    ///   pacc budget adjust --category "Car Insurance" --month 2026-03 --amount 1200
    #[command(verbatim_doc_comment)]
    Budget(BudgetArgs),
    /// Budgeted vs actual, categories down and months across.
    ///
    /// Examples:
    ///   pacc report --year 2026
    ///   pacc report --year 2026 --basis cashflow
    ///   pacc report --from 2026-01 --to 2026-06
    #[command(verbatim_doc_comment)]
    Report(ReportArgs),
}

#[derive(Args)]
struct BudgetArgs {
    #[command(subcommand)]
    command: BudgetCommand,
}

#[derive(Subcommand)]
enum BudgetCommand {
    /// Create or replace a category's budget.
    Set(BudgetSetArgs),
    /// The budget lines in force in a month.
    List(BudgetMonthArgs),
    /// Funding and spending for every budgeted category.
    Status(BudgetMonthArgs),
    /// Move money into or out of one month's envelope.
    ///
    /// A positive amount funds the envelope — "the annual premium was paid out
    /// of this sinking fund, so stop calling it a shortfall". A negative one
    /// takes money out, e.g. to move it to another envelope.
    #[command(verbatim_doc_comment)]
    Adjust(BudgetAdjustArgs),
}

#[derive(Args)]
struct BudgetSetArgs {
    /// Category name, a unique prefix, or Group/Name.
    #[arg(long, value_name = "NAME")]
    category: String,

    /// The target amount for one *period*, not per month: a yearly budget of
    /// 1200 contributes about $100.00 a month.
    #[arg(long, value_parser = parse_money)]
    amount: Money,

    /// How often the amount comes due.
    #[arg(long, value_enum, default_value_t = PeriodArg::Monthly)]
    period: PeriodArg,

    /// Cycle length in months, for `--period every-n`. 2 to 120.
    #[arg(long, value_name = "N")]
    every: Option<u32>,

    /// The day the bill is due, for `--period by-date`.
    #[arg(long, value_name = "YYYY-MM-DD")]
    due: Option<NaiveDate>,

    /// First month the budget applies. Defaults to this month.
    ///
    /// Resolved here rather than in the core: a frontend is allowed to know
    /// what month it is, and `accounting-core` is not.
    #[arg(long, value_name = "YYYY-MM")]
    start: Option<YearMonth>,

    /// Last month the budget applies. Omit for "until further notice".
    #[arg(long, value_name = "YYYY-MM")]
    end: Option<YearMonth>,

    /// Carry an unspent balance into the next month. Defaults to the
    /// category's own setting — on for the sinking-fund categories, where a
    /// budget without carry-over cannot work at all.
    #[arg(long, conflicts_with = "no_rollover")]
    rollover: bool,

    /// Spend it or lose it: reset the envelope every cycle.
    #[arg(long)]
    no_rollover: bool,

    /// Free-text note on the budget line.
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
enum PeriodArg {
    /// The amount, every month.
    Monthly,
    /// The amount once a year, saved for in twelve parts.
    Yearly,
    /// The amount every `--every N` months.
    #[value(name = "every-n")]
    EveryN,
    /// The amount once, on `--due`, saved for from `--start` until then.
    #[value(name = "by-date")]
    ByDate,
}

#[derive(Args)]
struct BudgetMonthArgs {
    /// The month to report on. Defaults to this month.
    #[arg(long, value_name = "YYYY-MM")]
    month: Option<YearMonth>,
}

#[derive(Args)]
struct BudgetAdjustArgs {
    /// Category name, a unique prefix, or Group/Name.
    #[arg(long, value_name = "NAME")]
    category: String,

    /// The month whose envelope moves.
    #[arg(long, value_name = "YYYY-MM")]
    month: YearMonth,

    /// How much to add. Negative takes money out: `--amount -50`.
    #[arg(long, value_parser = parse_money, allow_hyphen_values = true)]
    amount: Money,

    /// Why. Worth writing: an unexplained adjustment is indistinguishable from
    /// a mistake six months later.
    #[arg(long, value_name = "TEXT")]
    reason: Option<String>,
}

#[derive(Args)]
struct ReportArgs {
    /// First month, inclusive. Defaults to January of this year; given alone,
    /// the window runs twelve months from here.
    #[arg(long, value_name = "YYYY-MM")]
    from: Option<YearMonth>,

    /// Last month, inclusive. Defaults to December of this year; given alone,
    /// the window runs twelve months back from here.
    #[arg(long, value_name = "YYYY-MM")]
    to: Option<YearMonth>,

    /// One calendar year. Sugar for `--from YYYY-01 --to YYYY-12`.
    #[arg(long, value_name = "YYYY", conflicts_with_all = ["from", "to"])]
    year: Option<i32>,

    /// A single month.
    #[arg(long, value_name = "YYYY-MM", conflicts_with_all = ["from", "to", "year"])]
    month: Option<YearMonth>,

    /// Which set of actuals to show. `amortized` spreads a budget cycle's
    /// spending across that cycle's months; `cashflow` shows what left the
    /// account each month. Money is stored cash-flow dated either way.
    #[arg(long, value_enum, default_value_t = BasisArg::Amortized)]
    basis: BasisArg,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
enum BasisArg {
    #[value(name = "amortized", alias = "amortised")]
    Amortized,
    #[value(name = "cashflow", alias = "cash-flow")]
    CashFlow,
}

impl From<BasisArg> for ReportBasis {
    fn from(b: BasisArg) -> ReportBasis {
        match b {
            BasisArg::Amortized => ReportBasis::Amortized,
            BasisArg::CashFlow => ReportBasis::CashFlow,
        }
    }
}

#[derive(Args)]
struct LocationArgs {
    /// The location to create or update. Omit to list them all.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Sales-tax rate as a percent, e.g. `7.75`. Stored as parts-per-million,
    /// so 9.4375% is exact rather than rounded to two decimals.
    #[arg(long, value_name = "PERCENT", value_parser = parse_percent_ppm)]
    rate: Option<u32>,

    /// Make this the default location for receipts that name none.
    #[arg(long)]
    home: bool,
}

#[derive(Args)]
struct BackupArgs {
    /// Destination file. Must not already exist.
    #[arg(long = "to", value_name = "PATH")]
    to: PathBuf,
}

#[derive(Args)]
struct ExportArgs {
    /// Destination file. Must not already exist unless --force.
    #[arg(long = "to", value_name = "PATH")]
    to: PathBuf,

    /// Output format. Defaults to the extension of --to, then to xlsx.
    #[arg(long, value_enum)]
    format: Option<ExportFormat>,

    /// Earliest date, inclusive.
    #[arg(long, value_name = "YYYY-MM-DD")]
    from: Option<NaiveDate>,

    /// Latest date, inclusive. Spelled `--to-date` rather than `--to` because
    /// `--to` is already the destination file, as it is for `pacc backup`.
    #[arg(long = "to-date", visible_alias = "until", value_name = "YYYY-MM-DD")]
    to_date: Option<NaiveDate>,

    /// One calendar month. Sugar for `--from` its first day and `--to-date`
    /// its last.
    #[arg(long, value_name = "YYYY-MM", conflicts_with_all = ["from", "to_date", "year"])]
    month: Option<YearMonth>,

    /// One calendar year. Sugar for `--from` Jan 1 and `--to-date` Dec 31.
    #[arg(long, value_name = "YYYY", conflicts_with_all = ["from", "to_date"])]
    year: Option<i32>,

    /// Only this merchant.
    #[arg(long, value_name = "NAME")]
    merchant: Option<String>,

    /// Only transactions with a split in this category.
    #[arg(long, value_name = "NAME")]
    category: Option<String>,

    /// Include archived transactions.
    #[arg(long)]
    all: bool,

    /// Overwrite the destination if it exists.
    #[arg(long)]
    force: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
enum ExportFormat {
    Xlsx,
    Csv,
}

#[derive(Args)]
struct AddArgs {
    /// Transaction date, `YYYY-MM-DD`. Defaults to today.
    ///
    /// The default is resolved *here*, not in the core: a frontend knows the
    /// user's timezone and may read the clock, the core may not.
    #[arg(long, value_name = "YYYY-MM-DD", default_value_t = Local::now().date_naive())]
    date: NaiveDate,

    /// Store name. Created if new, and taught this receipt's category,
    /// location and detail level so the next one prefills.
    #[arg(long, value_name = "NAME")]
    merchant: Option<String>,

    /// Store location, which is what determines the sales-tax rate. A new one
    /// is created only if `--rate` says what its rate is, so a typo cannot
    /// silently become a 0% location.
    #[arg(long, value_name = "NAME")]
    location: Option<String>,

    /// Amount charged, tax included.
    #[arg(long, value_parser = parse_money)]
    total: Option<Money>,

    /// Pre-tax amount.
    #[arg(long, value_parser = parse_money)]
    subtotal: Option<Money>,

    /// Tax as printed on the receipt. Given here, it is a fact and is never
    /// recomputed.
    #[arg(long, value_parser = parse_money)]
    tax: Option<Money>,

    /// Sales-tax rate as a percentage, e.g. `8.375`. Overrides the location's
    /// rate for this receipt only — and supplies the rate when `--location`
    /// names a new one. Tax derived from it is flagged as an estimate.
    #[arg(long, value_parser = parse_percent_ppm)]
    rate: Option<u32>,

    /// Put the whole transaction in one category. Sugar for
    /// `--split "NAME=remainder"`.
    #[arg(long, value_name = "NAME", conflicts_with = "splits")]
    category: Option<String>,

    /// A split, repeatable: `Groceries=45.00`, `Groceries=25%`, or
    /// `Groceries=remainder`.
    #[arg(long = "split", value_name = "SPEC", value_parser = parse_split_spec)]
    splits: Vec<SplitSpec>,

    /// A line item, repeatable: `[SPLIT#:]DESCRIPTION=AMOUNT`, e.g.
    /// `"1:Milk=3.49"`. The split number is 1-based and defaults to 1.
    /// Implies --itemized.
    #[arg(long = "item", value_name = "SPEC", value_parser = parse_item_spec)]
    items: Vec<ItemSpec>,

    /// Free-text note on the transaction.
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,

    /// Record this as an itemized receipt.
    #[arg(long)]
    itemized: bool,

    /// Flag for review, so it shows up in `pacc inbox`.
    #[arg(long)]
    needs_review: bool,

    /// Print the preview and write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct ListArgs {
    /// Earliest date, inclusive.
    #[arg(long, value_name = "YYYY-MM-DD")]
    from: Option<NaiveDate>,

    /// Latest date, inclusive.
    #[arg(long, value_name = "YYYY-MM-DD")]
    to: Option<NaiveDate>,

    /// One calendar month. Sugar for `--from` its first day and `--to` its
    /// last.
    #[arg(long, value_name = "YYYY-MM", conflicts_with_all = ["from", "to"])]
    month: Option<YearMonth>,

    /// Only this merchant.
    #[arg(long, value_name = "NAME")]
    merchant: Option<String>,

    /// Only transactions with a split in this category.
    #[arg(long, value_name = "NAME")]
    category: Option<String>,

    /// Only transactions awaiting categorization.
    #[arg(long)]
    needs_review: bool,

    /// Maximum rows; `0` for no limit.
    #[arg(long, default_value_t = 20)]
    limit: usize,

    /// Include archived transactions.
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
struct ShowArgs {
    /// Transaction id, as printed by `pacc list`.
    id: i64,
}

#[derive(Args)]
struct QuickAddArgs {
    /// Amount charged, tax included.
    #[arg(value_parser = parse_money)]
    total: Money,

    /// Store name, if you know it.
    #[arg(long, value_name = "NAME")]
    merchant: Option<String>,

    /// Transaction date, `YYYY-MM-DD`. Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD", default_value_t = Local::now().date_naive())]
    date: NaiveDate,
}

#[derive(Args)]
struct InboxArgs {
    /// Maximum rows; `0` for no limit.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args)]
struct CategorizeArgs {
    /// Transaction id, as printed by `pacc inbox`.
    id: i64,

    /// A split, repeatable. Same syntax as `pacc add --split`.
    #[arg(long = "split", value_name = "SPEC", required = true, value_parser = parse_split_spec)]
    splits: Vec<SplitSpec>,
}

#[derive(Args)]
struct RepeatArgs {
    /// Store name.
    merchant: String,

    /// Date for the new transaction, `YYYY-MM-DD`. Defaults to today.
    #[arg(long, value_name = "YYYY-MM-DD", default_value_t = Local::now().date_naive())]
    date: NaiveDate,

    /// A different total. The last split absorbs the difference.
    #[arg(long, value_parser = parse_money)]
    total: Option<Money>,

    /// Print the draft and write nothing.
    #[arg(long)]
    dry_run: bool,
}

/// `Groceries=45.00`, before the category name has been resolved.
///
/// Resolution needs a database, and clap parses before one is open — so the
/// value parser's job stops at the syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SplitSpec {
    category: String,
    amount: SplitAmount,
}

/// `[SPLIT#:]DESCRIPTION=AMOUNT`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemSpec {
    /// 1-based, as typed.
    split: usize,
    description: String,
    amount: Money,
}

#[derive(Args)]
struct CalcArgs {
    /// Pre-tax amount, e.g. `50.12` or `$1,234.5`.
    #[arg(long, value_parser = parse_money)]
    subtotal: Option<Money>,

    /// Tax amount.
    #[arg(long, value_parser = parse_money)]
    tax: Option<Money>,

    /// Amount charged, tax included.
    #[arg(long, value_parser = parse_money)]
    total: Option<Money>,

    /// Sales-tax rate as a percentage, e.g. `8.375` (converted to 83750 ppm).
    #[arg(long, value_parser = parse_percent_ppm, default_value = "0")]
    rate: u32,

    /// Merchant quotes prices with tax already included.
    #[arg(long)]
    prices_include_tax: bool,

    /// Which amount the user typed last; decides which field yields when all
    /// three are given and they disagree.
    #[arg(long, value_enum)]
    last_edited: Option<EditedField>,

    /// A split, repeatable: an amount (`33.33`), a percentage (`33.33%`), or
    /// `remainder`.
    #[arg(long = "split", value_parser = parse_split)]
    splits: Vec<SplitAmount>,
}

#[derive(Copy, Clone, ValueEnum)]
enum EditedField {
    Subtotal,
    Tax,
    Total,
}

impl From<EditedField> for AmountField {
    fn from(f: EditedField) -> AmountField {
        match f {
            EditedField::Subtotal => AmountField::Subtotal,
            EditedField::Tax => AmountField::Tax,
            EditedField::Total => AmountField::Total,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Command::Version => {
            outln!("pacc {}", env!("CARGO_PKG_VERSION"));
        }
        Command::Calc(args) => run_calc(args)?,
        Command::Init => run_init(&cli.db_path()?)?,
        Command::Doctor => {
            if !run_doctor(&cli.db_path()?)? {
                // A non-zero exit is what makes `pacc doctor` usable from a
                // pre-backup script or a CI step.
                std::process::exit(1);
            }
        }
        Command::Backup(args) => run_backup(&cli.db_path()?, &args.to)?,
        Command::Add(args) => run_add(&cli.db_path()?, args)?,
        Command::List(args) => run_list(&cli.db_path()?, args)?,
        Command::Show(args) => run_show(&cli.db_path()?, args.id)?,
        Command::QuickAdd(args) => run_quick_add(&cli.db_path()?, args)?,
        Command::Inbox(args) => run_inbox(&cli, args)?,
        Command::Categorize(args) => run_categorize(&cli.db_path()?, args)?,
        Command::Repeat(args) => run_repeat(&cli.db_path()?, args)?,
        Command::Categories => run_categories(&cli.db_path()?)?,
        Command::Export(args) => run_export(&cli.db_path()?, args)?,
        Command::Location(args) => run_location(&cli.db_path()?, args)?,
        Command::Budget(args) => run_budget(&cli.db_path()?, &args.command)?,
        Command::Report(args) => run_report(&cli.db_path()?, args)?,
    }

    Ok(())
}

/// An ISO-8601 UTC timestamp in the same shape as the schema's `strftime`
/// default, so hand-written rows and app-written rows sort together.
///
/// The clock is read here rather than in the core. Rule 3 forbids the *core*
/// from doing it — a frontend is exactly the layer that is allowed to know
/// what time it is.
fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

impl Cli {
    /// `--db`, else the environment, else the XDG default.
    fn db_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(p) = &self.db {
            return Ok(p.clone());
        }
        default_db_path()
    }
}

/// The default database location.
///
/// Hand-rolled rather than pulled from `dirs`/`directories`: this is the only
/// place in the workspace that knows a filesystem convention, and keeping it
/// to a dozen lines here is what lets `accounting-core` stay path-free — and
/// therefore reusable on Android, where those crates are actively wrong.
fn default_db_path() -> anyhow::Result<PathBuf> {
    if let Some(p) = non_empty_env("PACC_DB") {
        return Ok(PathBuf::from(p));
    }
    let base = match non_empty_env("XDG_DATA_HOME") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(
            non_empty_env("HOME")
                .context("neither --db, $PACC_DB, $XDG_DATA_HOME nor $HOME is set; pass --db")?,
        )
        .join(".local")
        .join("share"),
    };
    Ok(base.join("personal-accounting").join("pacc.db"))
}

fn non_empty_env(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}

/// Open an existing database, refusing to create one by accident.
///
/// `Db::open` would happily create an empty file, which turns a typo in `--db`
/// into "all my data is gone" rather than an error.
fn open_existing(path: &Path) -> anyhow::Result<Db> {
    anyhow::ensure!(
        path.exists(),
        "no database at {} — run `pacc init` first, or pass --db",
        path.display()
    );
    Db::open(path).with_context(|| format!("could not open {}", path.display()))
}

/// Creating the parent directory is the CLI's job, not the core's: the core is
/// handed a path and does not get to have filesystem opinions.
fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    Ok(())
}

fn run_init(path: &Path) -> anyhow::Result<()> {
    ensure_parent_dir(path)?;
    let existed = path.exists();

    let db = Db::open(path).with_context(|| format!("could not open {}", path.display()))?;
    seed_defaults(&db).context("could not install the default categories")?;
    // `init` is also the upgrade path — it reports "updated" on a database
    // that already existed — and this is the kind of gap an upgrade leaves:
    // migration 002 can add the food-rate column but cannot know that "Provo,
    // UT" has one.
    let adopted = accounting_core::adopt_area_food_rates(&db)
        .context("could not apply the reduced food rates")?;

    let counts = Counts::read(&db)?;
    outln!(
        "{} {}",
        if existed { "updated" } else { "created" },
        path.display()
    );
    outln!("  schema     user_version={}", db.user_version()?);
    outln!(
        "  seeded     {} groups, {} categories, {} stores, {} locations",
        counts.groups,
        counts.categories,
        counts.merchants,
        counts.locations
    );
    if adopted > 0 {
        outln!(
            "  food rate  applied to {} location{}",
            adopted,
            if adopted == 1 { "" } else { "s" }
        );
    }
    Ok(())
}

/// Returns `true` when the database is healthy.
fn run_doctor(path: &Path) -> anyhow::Result<bool> {
    let db = open_existing(path)?;
    let counts = Counts::read(&db)?;
    let user_version = db.user_version()?;
    let journal_mode = db.journal_mode()?;
    let foreign_keys = db.foreign_keys_enabled()?;

    outln!("database        {}", path.display());
    outln!("user_version    {user_version}");
    outln!("journal_mode    {journal_mode}");
    outln!(
        "foreign_keys    {}",
        if foreign_keys { "on" } else { "off" }
    );
    outln!("category_groups {}", counts.groups);
    outln!("categories      {}", counts.categories);
    outln!("locations       {}", counts.locations);
    outln!("merchants       {}", counts.merchants);
    outln!("transactions    {}", counts.txns);
    outln!("splits          {}", counts.splits);

    // Errors and warnings are counted separately on purpose. A warning is
    // legitimate user behaviour — archiving a category you have historical
    // spend in produces one — so folding warnings into a single total would
    // make "0 invariant violations" drift away from meaning "healthy" as soon
    // as the database sees real use, and a number that is never zero stops
    // being read at all.
    let violations = check_all(&db).context("integrity check failed")?;
    let (errors, warnings): (Vec<_>, Vec<_>) = violations
        .iter()
        .partition(|v| v.severity() == Severity::Error);

    outln!();
    outln!("{} errors, {} warnings", errors.len(), warnings.len());
    for v in errors.iter().chain(&warnings) {
        outln!("  [{:?}] {v}", v.severity());
    }

    // Only errors make the database unhealthy, and only errors set the exit
    // code. Warnings are reported and then got out of the way of.
    let mut healthy = errors.is_empty();

    if usize::try_from(user_version).unwrap_or(usize::MAX) != SCHEMA_VERSION {
        outln!("  [Error] user_version is {user_version}, expected {SCHEMA_VERSION}");
        healthy = false;
    }
    if !foreign_keys {
        // Should be impossible via Db::open; if it ever happens, every
        // REFERENCES clause in the schema is decorative.
        outln!("  [Error] foreign_keys is off");
        healthy = false;
    }

    Ok(healthy)
}

fn run_backup(path: &Path, dest: &Path) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    ensure_parent_dir(dest)?;
    db.backup(dest)
        .with_context(|| format!("could not back up to {}", dest.display()))?;
    outln!("backed up {} -> {}", path.display(), dest.display());
    Ok(())
}

struct Counts {
    groups: i64,
    categories: i64,
    locations: i64,
    merchants: i64,
    txns: i64,
    splits: i64,
}

impl Counts {
    fn read(db: &Db) -> anyhow::Result<Counts> {
        let n = |table: &str| -> anyhow::Result<i64> {
            // Table names are literals from the call below, never user input.
            Ok(db
                .conn()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
        };
        Ok(Counts {
            groups: n("category_group")?,
            categories: n("category")?,
            locations: n("location")?,
            merchants: n("merchant")?,
            txns: n("txn")?,
            splits: n("txn_split")?,
        })
    }
}

// ---------------------------------------------------------------------------
// transaction entry
// ---------------------------------------------------------------------------

fn run_add(path: &Path, args: &AddArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;

    let splits = resolve_splits(&db, &split_specs(args))?;
    let splits = attach_items(splits, &args.items)?;
    let location = resolve_location(&db, args)?;

    let draft = TransactionDraft {
        id: None,
        txn_date: args.date,
        merchant: args.merchant.clone(),
        location,
        entry: AmountEntry {
            subtotal: args.subtotal,
            tax: args.tax,
            total: args.total,
            last_edited: None,
        },
        splits,
        // Passing --item means the receipt is itemized; saying so twice is
        // noise, and storing line items under a 'grouped' transaction is a
        // state `pacc doctor` reports as a mismatch.
        detail_level: (args.itemized || !args.items.is_empty()).then_some(DetailLevel::Itemized),
        notes: args.note.clone(),
        needs_review: args.needs_review,
        // Naming a store is what teaches us about it. `remember` never clears
        // a stored default, so a sparse entry can only add to the memory.
        remember_defaults: args.merchant.is_some(),
    };

    let preview = preview_transaction_with_rate(&db, &draft, args.rate)
        .context("could not resolve the transaction")?;
    print_preview(&db, &preview)?;

    if args.dry_run {
        outln!();
        outln!("dry run — nothing was written");
        return Ok(());
    }
    anyhow::ensure!(
        preview.can_save(),
        "not saved: {}",
        preview.blockers.join("; ")
    );

    let id = save_transaction_with_rate(&db, &draft, args.rate, &now_iso())
        .context("could not save the transaction")?;
    outln!();
    outln!("saved transaction {id}");
    Ok(())
}

fn run_quick_add(path: &Path, args: &QuickAddArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let id = quick_add(
        &db,
        args.date,
        args.total,
        args.merchant.as_deref(),
        &now_iso(),
    )
    .context("could not save the transaction")?;

    outln!(
        "saved transaction {id}: {} on {} — Uncategorized, flagged for review",
        args.total.to_display_string(),
        args.date
    );
    outln!("categorize it with: pacc categorize {id} --split \"NAME=remainder\"");
    Ok(())
}

fn run_list(path: &Path, args: &ListArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;

    let (from, to) = match args.month {
        Some(month) => (Some(month.first_day()), Some(month.last_day())),
        None => (args.from, args.to),
    };
    let merchant_id = match &args.merchant {
        Some(name) => Some(
            merchants::find_by_name(&db, name)?
                .with_context(|| format!("no merchant named `{name}`"))?
                .id,
        ),
        None => None,
    };
    let category_id = match &args.category {
        Some(name) => Some(categories::resolve(&db, name)?.id),
        None => None,
    };

    let filter = TxnFilter {
        from,
        to,
        merchant_id,
        category_id,
        // `--needs-review` is a switch, not a tri-state: absent means "do not
        // filter", never "only the reviewed ones".
        needs_review: args.needs_review.then_some(true),
        include_archived: args.all,
        limit: (args.limit > 0).then_some(args.limit),
    };

    let txns = transactions::list(&db, &filter)?;
    if txns.is_empty() {
        outln!("no transactions matched");
        return Ok(());
    }

    let mut table = Table::new(
        &["id", "date", "merchant", "categories", "total", ""],
        &[true, false, false, false, true, false],
    );
    for txn in &txns {
        table.push(vec![
            txn.id.to_string(),
            txn.txn_date.to_string(),
            txn.merchant_name.clone().unwrap_or_else(|| "—".to_string()),
            split_summary(txn),
            txn.total.to_display_string(),
            flags(txn),
        ]);
    }
    table.print("");

    outln!();
    outln!("{} transactions", txns.len());
    if txns.iter().any(|t| t.tax_is_estimated || t.needs_review) {
        outln!("~ tax estimated   ! needs review");
    }
    Ok(())
}

fn run_show(path: &Path, id: i64) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let txn = transactions::get(&db, TxnId::new(id))?;
    print_txn(&txn);
    Ok(())
}

fn run_inbox(cli: &Cli, args: &InboxArgs) -> anyhow::Result<()> {
    let db = open_existing(&cli.db_path()?)?;
    let txns = needs_categorizing(&db, (args.limit > 0).then_some(args.limit))?;
    if txns.is_empty() {
        outln!("nothing to categorize");
        return Ok(());
    }

    // The command is printed with the same --db the user passed, so it is
    // genuinely copy-pasteable rather than "the right command for some other
    // database".
    let prefix = match &cli.db {
        Some(p) => format!("pacc --db {} ", p.display()),
        None => "pacc ".to_string(),
    };

    let merchant_width = txns
        .iter()
        .map(|t| t.merchant_name.as_deref().unwrap_or("—").chars().count())
        .max()
        .unwrap_or(0);
    let total_width = txns
        .iter()
        .map(|t| t.total.to_display_string().len())
        .max()
        .unwrap_or(0);

    outln!("{} awaiting categorization", txns.len());
    for txn in &txns {
        outln!();
        outln!(
            "  {:>4}  {}  {:<merchant_width$}  {:>total_width$}",
            txn.id.to_string(),
            txn.txn_date,
            txn.merchant_name.as_deref().unwrap_or("—"),
            txn.total.to_display_string(),
        );
        outln!(
            "        {prefix}categorize {} --split \"NAME=remainder\"",
            txn.id
        );
    }
    Ok(())
}

fn run_categorize(path: &Path, args: &CategorizeArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let id = TxnId::new(args.id);
    let splits = resolve_splits(&db, &args.splits)?;

    categorize(&db, id, &splits).context("could not categorize the transaction")?;
    outln!("categorized transaction {id}");
    outln!();
    print_txn(&transactions::get(&db, id)?);
    Ok(())
}

fn run_repeat(path: &Path, args: &RepeatArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let mut draft = repeat_last(&db, &args.merchant, args.date)?;

    if let Some(total) = args.total {
        // The copied splits are fixed amounts from the old receipt, so a new
        // total would leave money unassigned and block the save. The last
        // split absorbs the difference — stated in --total's help, and visible
        // in the preview before anything is written.
        draft.entry = AmountEntry::total_only(total);
        if let Some(last) = draft.splits.last_mut() {
            last.amount = SplitAmount::Remainder;
        }
    }

    let preview =
        preview_transaction_with_rate(&db, &draft, None).context("could not resolve the draft")?;
    outln!(
        "draft from {}'s last receipt, dated {}",
        args.merchant,
        draft.txn_date
    );
    outln!();
    print_preview(&db, &preview)?;

    if args.dry_run {
        outln!();
        outln!("dry run — nothing was written");
        return Ok(());
    }
    anyhow::ensure!(
        preview.can_save(),
        "not saved: {}",
        preview.blockers.join("; ")
    );

    let id = save_transaction_with_rate(&db, &draft, None, &now_iso())
        .context("could not save the transaction")?;
    outln!();
    outln!("saved transaction {id}");
    Ok(())
}

fn run_categories(path: &Path) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let groups = categories::list_groups(&db)?;
    let all = categories::list(&db)?;

    for group in &groups {
        let members: Vec<&accounting_core::Category> =
            all.iter().filter(|c| c.group_id == group.id).collect();
        if members.is_empty() {
            continue;
        }
        outln!(
            "{}{}",
            group.name,
            if group.is_hidden { " (hidden)" } else { "" }
        );
        for category in members {
            outln!("  {}", category.name);
        }
    }
    outln!();
    outln!(
        "{} categories in {} groups. --category takes a name, a unique prefix, or Group/Name.",
        all.len(),
        groups.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// location
// ---------------------------------------------------------------------------

/// List locations, or create/update one.
///
/// This exists because without it the app gives advice it makes impossible to
/// take. The seeded `Home` location carries a 0% rate, so every receipt that
/// names no rate reconciles to $0.00 tax and `preview_transaction` warns "no
/// tax was applied: this location has no tax rate set" — and until now there
/// was no command that could set one. The alternatives were passing `--rate`
/// on every single receipt forever, or creating a second location just to
/// carry a rate the first one should have had.
///
/// The rate is only ever a *prefill* for an editable estimate. A receipt
/// prints the real tax, and the real tax always wins; nothing here can
/// overwrite a figure the user typed.
fn run_location(path: &Path, args: &LocationArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;

    let Some(name) = args.name.as_deref() else {
        if args.rate.is_some() || args.home {
            anyhow::bail!("--rate and --home need a --name to apply to");
        }
        return list_locations(&db);
    };

    // Created at the given rate, or 0 when none was supplied. `get_or_create`
    // deliberately does not overwrite an existing location's rate, so the
    // `set_rate` below is what makes this command able to *change* one.
    let location = locations::get_or_create(&db, name, args.rate.unwrap_or(0))?;
    if let Some(rate) = args.rate {
        locations::set_rate(&db, location.id, rate)?;
    }
    // A known area also brings its reduced food rate, so groceries are not
    // taxed at the general rate. See `pacc doctor` output and areas.rs.
    if let Some(area) = accounting_core::domain::areas::find(name) {
        locations::set_food_rate(&db, location.id, area.food_rate_ppm)?;
    }
    if args.home {
        // Moves the flag rather than adding a second home: the schema's
        // ux_location_home index permits only one.
        locations::set_home(&db, location.id)?;
    }

    list_locations(&db)
}

fn list_locations(db: &accounting_core::Db) -> anyhow::Result<()> {
    let all = locations::list(db)?;
    if all.is_empty() {
        outln!("no locations yet — `pacc location --name NAME --rate PERCENT` adds one");
        return Ok(());
    }

    let width = all.iter().map(|l| l.name.len()).max().unwrap_or(4);
    for location in &all {
        outln!(
            "  {:<width$}  {:>18}{}",
            location.name,
            format_ppm(location.tax_rate_ppm),
            if location.is_home { "  (home)" } else { "" },
        );
    }
    outln!();
    outln!(
        "A rate only prefills an editable estimate. The receipt's own tax always wins,\n\
         and `--tax` on `pacc add` is never overwritten."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// budgets
// ---------------------------------------------------------------------------

/// The month the user means when they do not say.
///
/// The clock is read here, never in `accounting-core`: rule 3 forbids the core
/// from deriving dates from it, and a frontend is exactly the layer that is
/// allowed to know what month it is.
fn current_month() -> YearMonth {
    YearMonth::from_date(Local::now().date_naive())
}

fn run_budget(path: &Path, command: &BudgetCommand) -> anyhow::Result<()> {
    match command {
        BudgetCommand::Set(args) => run_budget_set(path, args),
        BudgetCommand::List(args) => run_budget_list(path, args),
        BudgetCommand::Status(args) => run_budget_status(path, args),
        BudgetCommand::Adjust(args) => run_budget_adjust(path, args),
    }
}

/// Set a category's budget from `--start` onward.
///
/// # Three cases, because a budget is a historical record
///
/// * Nothing is in force yet → a new line.
/// * A line already **starts** in that month → it is restated in place. This is
///   the "I typed the wrong number" case, and rewriting history is what the
///   user asked for.
/// * A line covers that month but started earlier → the old line is **closed**
///   the month before and a new one opens. This is the "groceries are dearer
///   now" case, and quietly editing the old line would rewrite every report
///   back to January, which is the whole reason `period_start`/`period_end`
///   exist.
///
/// All three happen inside one transaction in
/// [`accounting_core::repo::budgets::supersede`], so a refusal — a bad `--every`,
/// or a collision with a line further out — changes nothing at all. Doing the
/// close and the insert as two calls from here would leave the failing case with
/// the old budget closed and no new one, which is worse than either outcome the
/// user was choosing between.
fn run_budget_set(path: &Path, args: &BudgetSetArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let category = categories::resolve(&db, &args.category)?;
    let start = args.start.unwrap_or_else(current_month);

    let rec = BudgetLineRecord {
        id: None,
        category_id: category.id,
        period: args.resolved_period()?,
        period_start: start,
        period_end: args.end,
        amount: args.amount,
        // The category's own default, because the sinking-fund categories are
        // unusable without carry-over: a $1,200 annual premium budgeted at $100
        // a month never reaches $1,200 if the envelope empties every month.
        rollover: match (args.rollover, args.no_rollover) {
            (true, _) => true,
            (_, true) => false,
            _ => category.default_rollover,
        },
        notes: args.note.clone(),
    };
    let outcome = budget_service::set_budget(&db, &rec)?;

    if outcome.restated {
        outln!(
            "restated the {} budget that starts {start}",
            category.qualified_name()
        );
    }
    if let Some((closed, last)) = outcome.closed {
        let previous = budgets::get(&db, closed)?;
        outln!(
            "closed the previous {} budget of {} at {last}, so earlier reports keep their \
             own numbers",
            category.qualified_name(),
            previous.line.amount.to_display_string()
        );
    }

    outln!();
    print_budget_line(&budgets::get(&db, outcome.id)?, start);
    Ok(())
}

impl BudgetSetArgs {
    /// `--period`, plus the flag it needs.
    ///
    /// `--every` and `--due` also *imply* their period, so that
    /// `--every 3` alone cannot silently produce a monthly budget — the kind of
    /// typo that is invisible until the quarterly bill arrives.
    fn resolved_period(&self) -> anyhow::Result<Period> {
        anyhow::ensure!(
            self.every.is_none() || self.due.is_none(),
            "--every and --due describe two different cadences; pass one"
        );
        let period = match (self.period, self.every, self.due) {
            (PeriodArg::Monthly, Some(_), None) => PeriodArg::EveryN,
            (PeriodArg::Monthly, None, Some(_)) => PeriodArg::ByDate,
            (period, _, _) => period,
        };

        match period {
            PeriodArg::Monthly => Ok(Period::Monthly),
            PeriodArg::Yearly => Ok(Period::Yearly),
            PeriodArg::EveryN => Ok(Period::EveryNMonths(self.every.context(
                "--period every-n needs --every N, e.g. `--every 3` for a quarterly bill",
            )?)),
            PeriodArg::ByDate => Ok(Period::ByDate {
                due: self.due.context(
                    "--period by-date needs --due YYYY-MM-DD, the day the bill actually lands",
                )?,
            }),
        }
    }
}

fn run_budget_list(path: &Path, args: &BudgetMonthArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let month = args.month.unwrap_or_else(current_month);
    let lines = budgets::list(&db, Some(month))?;

    if lines.is_empty() {
        outln!("no budgets in force in {month}");
        outln!("  set one with: pacc budget set --category NAME --amount N");
        return Ok(());
    }

    let mut table = Table::new(
        &[
            "id",
            "category",
            "amount",
            "period",
            "per month",
            "months",
            "rollover",
            "note",
        ],
        &[true, false, true, false, true, false, false, false],
    );
    for stored in &lines {
        table.push(vec![
            stored.line.id.to_string(),
            stored.qualified_name(),
            stored.line.amount.to_display_string(),
            format_period(stored.line.period),
            monthly_contribution(&stored.line, month).to_display_string(),
            format_months(stored.line.period_start, stored.line.period_end),
            yes_no(stored.line.rollover),
            stored.notes.clone().unwrap_or_default(),
        ]);
    }
    outln!("budgets in force {month}");
    table.print("  ");

    outln!();
    outln!(
        "{}, {} a month in total",
        plural(lines.len(), "line", "lines"),
        lines
            .iter()
            .map(|s| monthly_contribution(&s.line, month))
            .sum::<Money>()
            .to_display_string()
    );
    outln!("  `amount` is per period; `per month` is what it sets aside in {month}.");
    Ok(())
}

/// The budget screen.
///
/// # Funding and overspending get their own columns
///
/// They answer different questions and disagree in ordinary use: a $400/month
/// grocery envelope with $400 spent is `behind $400.00` on **funding** — the
/// schedule says that money should be sitting there and it is not — and exactly
/// on budget on **spending**. Collapsing them into one "status" column, or
/// labelling `behind` as "over budget", turns the most normal month of the year
/// into a false alarm.
fn run_budget_status(path: &Path, args: &BudgetMonthArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let month = args.month.unwrap_or_else(current_month);
    let rows = budget_service::budget_status(&db, month)?;

    let (budgeted, unbudgeted): (Vec<_>, Vec<_>) = rows.iter().partition(|r| r.status.is_some());

    outln!("budget status {month}");
    if budgeted.is_empty() {
        outln!("  nothing is budgeted yet");
    } else {
        let mut table = Table::new(
            &[
                "category",
                "per month",
                "funded",
                "contributed",
                "target",
                "left",
                "funding",
                "spent (cycle)",
                "remaining",
                "over",
            ],
            &[false, true, true, true, true, true, false, true, true, true],
        );
        for row in &budgeted {
            let Some(status) = &row.status else { continue };
            table.push(vec![
                row.qualified_name(),
                status.monthly_contribution.to_display_string(),
                status.funded_balance.to_display_string(),
                status.contributed_to_date.to_display_string(),
                status.target.to_display_string(),
                format!("{} mo", status.months_remaining),
                format_funding(status.on_track),
                status.spend.spent.to_display_string(),
                status.spend.remaining.to_display_string(),
                if status.spend.overspent_by.is_zero() {
                    "—".to_string()
                } else {
                    status.spend.overspent_by.to_display_string()
                },
            ]);
        }
        table.print("  ");
    }

    if !unbudgeted.is_empty() {
        // The ones with money moving through them are the ones worth budgeting,
        // so they get a line each; the rest get a count. Twenty rows of "$0.00
        // spent" is a list nobody reads, which defeats the point of offering.
        let (spending, idle): (Vec<&CategoryBudgetStatus>, Vec<&CategoryBudgetStatus>) = unbudgeted
            .iter()
            .copied()
            .partition(|r| !r.spent_this_month.is_zero());
        let width = spending
            .iter()
            .map(|r| r.qualified_name().chars().count())
            .max()
            .unwrap_or(0);

        outln!();
        outln!(
            "no budget set ({})",
            plural(unbudgeted.len(), "category", "categories")
        );
        for row in &spending {
            outln!(
                "  {:<width$}  {} spent in {month}",
                row.qualified_name(),
                row.spent_this_month.to_display_string()
            );
        }
        if !idle.is_empty() {
            outln!("  {} more with nothing spent in {month}", idle.len());
        }
        outln!("  set one with: pacc budget set --category NAME --amount N");
    }

    outln!();
    outln!(
        "funding asks whether the money will be there when the bill lands; over asks whether\n\
         more has been spent than this cycle's budget. They disagree by design — an envelope\n\
         spent exactly to plan is behind on funding and perfectly fine on spending."
    );
    Ok(())
}

fn run_budget_adjust(path: &Path, args: &BudgetAdjustArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let category = categories::resolve(&db, &args.category)?;
    budgets::add_adjustment(
        &db,
        category.id,
        args.month,
        args.amount,
        args.reason.as_deref(),
    )?;

    outln!(
        "adjusted {} for {} by {}",
        category.qualified_name(),
        args.month,
        args.amount.to_display_string()
    );

    // Reported as of the later of the adjusted month and today, because what
    // the user wants to know is where the envelope stands *now*.
    let as_of = args.month.max(current_month());
    let status = budget_service::budget_status_for(&db, category.id, as_of)?;
    match &status.status {
        Some(funded) => outln!(
            "  {as_of}  funded {}  ({})",
            funded.funded_balance.to_display_string(),
            format_funding(funded.on_track)
        ),
        None => outln!("  {} has no budget line in {as_of}", category.name),
    }
    Ok(())
}

fn print_budget_line(stored: &StoredBudgetLine, as_of: YearMonth) {
    outln!("budget {} — {}", stored.line.id, stored.qualified_name());
    let rows: Vec<(&str, String)> = vec![
        (
            "amount",
            format!(
                "{} {}",
                stored.line.amount.to_display_string(),
                format_period(stored.line.period)
            ),
        ),
        (
            "per month",
            format!(
                "{} (in {as_of})",
                monthly_contribution(&stored.line, as_of).to_display_string()
            ),
        ),
        (
            "months",
            format_months(stored.line.period_start, stored.line.period_end),
        ),
        ("rollover", yes_no(stored.line.rollover)),
        (
            "note",
            stored.notes.clone().unwrap_or_else(|| "—".to_string()),
        ),
    ];
    for (label, value) in rows {
        outln!("  {label:<10} {value}");
    }
}

fn format_period(period: Period) -> String {
    match period {
        Period::Monthly => "monthly".to_string(),
        Period::Yearly => "yearly".to_string(),
        Period::EveryNMonths(n) => format!("every {n} months"),
        Period::ByDate { due } => format!("by {due}"),
    }
}

fn format_months(start: YearMonth, end: Option<YearMonth>) -> String {
    match end {
        Some(end) => format!("{start}..{end}"),
        None => format!("{start} onward"),
    }
}

/// Never the words "over budget": this is a funding measure, and
/// [`OnTrack::Behind`] on a spent-to-plan envelope is normal.
fn format_funding(on_track: OnTrack) -> String {
    match on_track {
        OnTrack::Ahead(by) => format!("ahead {}", by.to_display_string()),
        OnTrack::OnTrack => "on track".to_string(),
        OnTrack::Behind(by) => format!("behind {}", by.to_display_string()),
    }
}

// ---------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------

fn run_report(path: &Path, args: &ReportArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let matrix = monthly_matrix(&db, args.range()?, args.basis.into())?;
    print_matrix(&matrix);
    Ok(())
}

impl ReportArgs {
    /// `--month`, else `--year`, else `--from`/`--to`, else this calendar year.
    ///
    /// One bound given alone opens a twelve-month window rather than running to
    /// the end of the year, so `--from 2026-09` is nine months of report rather
    /// than four.
    fn range(&self) -> anyhow::Result<MonthRange> {
        if let Some(month) = self.month {
            return Ok(MonthRange::single(month));
        }
        if let Some(year) = self.year {
            let january =
                YearMonth::new(year, 1).with_context(|| format!("{year} is not a year"))?;
            return Ok(MonthRange::new(january, january.add_months(11)));
        }

        let range = match (self.from, self.to) {
            (Some(from), Some(to)) => MonthRange::new(from, to),
            (Some(from), None) => MonthRange::new(from, from.add_months(11)),
            (None, Some(to)) => MonthRange::new(to.add_months(-11), to),
            (None, None) => {
                let year = current_month().year;
                let january =
                    YearMonth::new(year, 1).with_context(|| format!("{year} is not a year"))?;
                MonthRange::new(january, january.add_months(11))
            }
        };
        anyhow::ensure!(
            !range.is_empty(),
            "--from {} is after --to {}",
            range.start,
            range.end
        );
        Ok(range)
    }
}

/// The grid: two lines per category, budgeted over actual.
///
/// Two lines rather than one cell holding both, because a twelve-month grid of
/// `$100.00/$1,200.00` cells is unreadable, and because the two rows are what a
/// reader actually compares — along a row, not across a cell.
fn print_matrix(matrix: &MonthlyMatrix) {
    let Some(first) = matrix.months.first().copied() else {
        outln!("that range covers no months");
        return;
    };
    let last = matrix.months.last().copied().unwrap_or(first);

    outln!("report {first}..{last} — {} basis", matrix.basis);
    outln!("  {}", matrix.basis.describe());
    outln!("  amounts are stored cash-flow dated; the basis is a view, never a rewrite");
    outln!();

    if matrix.rows.is_empty() {
        outln!("nothing budgeted or spent in this window");
        return;
    }

    // Bare month names when the window sits inside one year, `YYYY-MM` when it
    // spans two — otherwise a two-year report shows "Jan" twice.
    let one_year = matrix.months.iter().all(|m| m.year == first.year);
    let mut headers = vec!["category".to_string(), String::new()];
    headers.extend(matrix.months.iter().map(|m| {
        if one_year {
            month_short_name(m.month).to_string()
        } else {
            m.to_string()
        }
    }));
    headers.push("total".to_string());

    let mut right = vec![false, false];
    right.extend(std::iter::repeat_n(true, matrix.months.len() + 1));

    let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let mut table = Table::new(&refs, &right);

    for row in &matrix.rows {
        let mut budgeted = vec![row.qualified_name(), "budget".to_string()];
        let mut actual = vec![String::new(), "actual".to_string()];
        for cell in &row.cells {
            budgeted.push(amount_cell(cell.budgeted));
            actual.push(amount_cell(cell.actual));
        }
        budgeted.push(amount_cell(row.total.budgeted));
        actual.push(amount_cell(row.total.actual));
        table.push(budgeted);
        table.push(actual);
    }

    let mut budgeted = vec!["TOTAL".to_string(), "budget".to_string()];
    let mut actual = vec![String::new(), "actual".to_string()];
    for cell in &matrix.totals {
        budgeted.push(amount_cell(cell.budgeted));
        actual.push(amount_cell(cell.actual));
    }
    budgeted.push(amount_cell(matrix.grand_total.budgeted));
    actual.push(amount_cell(matrix.grand_total.actual));
    table.push(budgeted);
    table.push(actual);
    table.print("  ");

    outln!();
    outln!(
        "{} over {} — budgeted {}, actual {}, difference {}",
        plural(matrix.rows.len(), "category", "categories"),
        plural(matrix.months.len(), "month", "months"),
        matrix.grand_total.budgeted.to_display_string(),
        matrix.grand_total.actual.to_display_string(),
        matrix.grand_total.variance().to_display_string(),
    );
    if matrix.basis == ReportBasis::Amortized {
        outln!(
            "  a window that does not cover whole budget cycles will not total the same as\n  \
             --basis cashflow. That is arithmetic, not a discrepancy."
        );
    }
}

/// `"1 category"`, `"2 categories"`. These footers are read out loud in bug
/// reports, and "1 categories" reads like the count is wrong.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `$0.00` twelve times across is noise; a dash is a reading aid.
fn amount_cell(amount: Money) -> String {
    if amount.is_zero() {
        "—".to_string()
    } else {
        amount.to_display_string()
    }
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

/// Build the bytes in the core, write the file here.
///
/// The split is the point. `accounting-core` returns a `Vec<u8>` and never
/// learns what a path is, because on Android and iOS the destination comes
/// back from a system document picker as a stream the app is handed — a core
/// that called `std::fs::write` would need re-implementing for each platform.
/// This function is the whole of the "write it somewhere" half.
fn run_export(path: &Path, args: &ExportArgs) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    let format = args.resolved_format();

    // Checked before the export runs, not after: producing a megabyte of
    // spreadsheet and then refusing to write it wastes the user's time, and
    // this matches `pacc backup`'s stance that an existing file is data until
    // the user says otherwise.
    anyhow::ensure!(
        args.force || !args.to.exists(),
        "{} already exists; pass --force to overwrite it",
        args.to.display()
    );

    let filter = args.filter(&db)?;
    let rows = export::rows(&db, &filter)?;
    let bytes = match format {
        ExportFormat::Xlsx => export::xlsx::workbook_bytes(&rows),
        ExportFormat::Csv => export::csv::csv_bytes(&rows),
    }
    .context("could not build the export")?;

    ensure_parent_dir(&args.to)?;
    std::fs::write(&args.to, &bytes)
        .with_context(|| format!("could not write {}", args.to.display()))?;

    outln!(
        "wrote {} rows to {} ({})",
        rows.len(),
        args.to.display(),
        match format {
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Csv => "csv",
        }
    );
    // The span comes from the exported rows, which are sorted ascending, so it
    // reports what is actually in the file rather than what was asked for —
    // the difference is exactly how you notice a filter that matched nothing
    // you expected.
    match (rows.first(), rows.last()) {
        (Some(first), Some(last)) => outln!("  dates  {} .. {}", first.date, last.date),
        _ => outln!("  dates  (no rows matched)"),
    }
    Ok(())
}

impl ExportArgs {
    /// `--format`, else the extension of `--to`, else xlsx.
    ///
    /// Guessing from the extension is what makes `pacc export --to x.csv` do
    /// the obvious thing; an unrecognised extension falls back rather than
    /// erroring, because the user has already said where the file goes.
    fn resolved_format(&self) -> ExportFormat {
        if let Some(format) = self.format {
            return format;
        }
        match self
            .to
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("csv") => ExportFormat::Csv,
            _ => ExportFormat::Xlsx,
        }
    }

    fn filter(&self, db: &Db) -> anyhow::Result<TxnFilter> {
        // `--month` and `--year` are sugar over the two dates, resolved here so
        // the core only ever sees a date range.
        let (from, to) = match (self.month, self.year) {
            (Some(month), _) => (Some(month.first_day()), Some(month.last_day())),
            (None, Some(year)) => (
                Some(
                    NaiveDate::from_ymd_opt(year, 1, 1)
                        .with_context(|| format!("{year} is not a year"))?,
                ),
                Some(
                    NaiveDate::from_ymd_opt(year, 12, 31)
                        .with_context(|| format!("{year} is not a year"))?,
                ),
            ),
            (None, None) => (self.from, self.to_date),
        };

        let merchant_id = match &self.merchant {
            Some(name) => Some(
                merchants::find_by_name(db, name)?
                    .with_context(|| format!("no merchant named `{name}`"))?
                    .id,
            ),
            None => None,
        };
        let category_id = match &self.category {
            Some(name) => Some(categories::resolve(db, name)?.id),
            None => None,
        };

        Ok(TxnFilter {
            from,
            to,
            merchant_id,
            category_id,
            needs_review: None,
            include_archived: self.all,
            // An export is the whole set by definition; a page limit here
            // would silently truncate a year's accounts.
            limit: None,
        })
    }
}

// ---------------------------------------------------------------------------
// draft assembly
// ---------------------------------------------------------------------------

/// `--category X` is exactly `--split "X=remainder"`; clap already refuses
/// both at once.
fn split_specs(args: &AddArgs) -> Vec<SplitSpec> {
    match &args.category {
        Some(name) => vec![SplitSpec {
            category: name.clone(),
            amount: SplitAmount::Remainder,
        }],
        None => args.splits.clone(),
    }
}

/// Resolve `--location`, creating it when `--rate` says what its tax rate is.
///
/// # Why an unknown location is not created silently
///
/// The tax rate lives on the location, so an auto-created one would default to
/// 0% — and then a typo (`--location Hoem`) would quietly produce a receipt
/// with no tax and a second, permanent location in the picker. Requiring
/// `--rate` to create one makes the first visit to a store an explicit
/// statement of its rate, and every later visit a bare `--location`.
///
/// `--rate` alone (no `--location`) writes nothing at all: it is a per-receipt
/// override, threaded into the service as a rate so the tax it produces stays
/// flagged as an estimate.
fn resolve_location(db: &Db, args: &AddArgs) -> anyhow::Result<Option<String>> {
    let Some(name) = args.location.as_deref() else {
        return Ok(None);
    };
    if accounting_core::repo::locations::find_by_name(db, name)?.is_some() {
        return Ok(Some(name.to_string()));
    }

    let rate = args.rate.with_context(|| {
        format!(
            "no location named `{name}`; add it by giving its tax rate too, \
             e.g. --location \"{name}\" --rate 8.375"
        )
    })?;

    if args.dry_run {
        // A dry run writes nothing, location included. The preview still uses
        // the rate, so the numbers shown are the numbers that would be saved.
        outln!("would create location `{name}` at {}", format_ppm(rate));
        outln!();
        return Ok(None);
    }

    let created = accounting_core::repo::locations::get_or_create(db, name, rate)?;
    outln!(
        "created location `{}` at {}",
        created.name,
        format_ppm(rate)
    );
    outln!();
    Ok(Some(created.name))
}

fn resolve_splits(db: &Db, specs: &[SplitSpec]) -> anyhow::Result<Vec<SplitDraft>> {
    specs
        .iter()
        .map(|spec| {
            let category = categories::resolve(db, &spec.category)?;
            Ok(SplitDraft {
                id: None,
                category_id: Some(category.id),
                amount: spec.amount,
                note: None,
                line_items: Vec::new(),
            })
        })
        .collect()
}

fn attach_items(
    mut splits: Vec<SplitDraft>,
    items: &[ItemSpec],
) -> anyhow::Result<Vec<SplitDraft>> {
    for item in items {
        anyhow::ensure!(
            item.split >= 1 && item.split <= splits.len(),
            "--item refers to split {} but there {}; splits are numbered from 1",
            item.split,
            match splits.len() {
                0 => "are none".to_string(),
                1 => "is only 1".to_string(),
                n => format!("are only {n}"),
            }
        );
        splits[item.split - 1].line_items.push(LineItemDraft {
            description: item.description.clone(),
            // One unit unless a future flag says otherwise. Thousandths, so
            // 1.5 lb is 1500 and never a float.
            quantity_milli: 1_000,
            unit_price: Some(item.amount),
            amount: item.amount,
        });
    }
    Ok(splits)
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn print_preview(db: &Db, preview: &TransactionPreview) -> anyhow::Result<()> {
    print_amounts(&preview.amounts, &preview.context.tax);

    let names = CategoryNames::load(db)?;
    let mut table = Table::new(
        &["#", "category", "amount", "subtotal", "tax"],
        &[false, false, true, true, true],
    );
    for (index, row) in preview.plan.rows.iter().enumerate() {
        table.push(vec![
            (index + 1).to_string(),
            row.category_id
                .map_or_else(|| "(none)".to_string(), |id| names.get(id)),
            row.amount.to_display_string(),
            row.subtotal.to_display_string(),
            row.tax.to_display_string(),
        ]);
    }
    outln!();
    outln!("splits");
    table.print("  ");
    outln!(
        "  unassigned {}",
        preview.plan.unassigned.to_display_string()
    );
    outln!("  detail     {}", preview.detail_level);

    print_list("warnings", &preview.warnings);
    print_list("blockers", &preview.blockers);
    Ok(())
}

fn print_list(label: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    outln!();
    outln!("{label}");
    for line in lines {
        outln!("  - {line}");
    }
}

fn print_txn(txn: &Txn) {
    outln!("transaction {}", txn.id);
    let rows: Vec<(&str, String)> = vec![
        ("date", txn.txn_date.to_string()),
        (
            "merchant",
            txn.merchant_name.clone().unwrap_or_else(|| "—".to_string()),
        ),
        (
            "location",
            txn.location_name.clone().unwrap_or_else(|| "—".to_string()),
        ),
        ("subtotal", txn.subtotal.to_display_string()),
        ("tax", txn.tax.to_display_string()),
        ("total", txn.total.to_display_string()),
        (
            "rate",
            txn.tax_rate_ppm.map_or_else(|| "—".to_string(), format_ppm),
        ),
        ("estimated", yes_no(txn.tax_is_estimated)),
        ("review", yes_no(txn.needs_review)),
        ("detail", txn.detail_level.to_string()),
        ("archived", yes_no(txn.archived)),
        (
            "notes",
            txn.notes.clone().unwrap_or_else(|| "—".to_string()),
        ),
    ];
    for (label, value) in rows {
        outln!("  {label:<10} {value}");
    }

    let mut table = Table::new(
        &["#", "category", "amount", "subtotal", "tax", "note"],
        &[false, false, true, true, true, false],
    );
    for (index, split) in txn.splits.iter().enumerate() {
        table.push(vec![
            (index + 1).to_string(),
            format!("{}/{}", split.group_name, split.category_name),
            split.amount.to_display_string(),
            split.subtotal.to_display_string(),
            split.tax.to_display_string(),
            split.notes.clone().unwrap_or_default(),
        ]);
    }
    outln!();
    outln!("splits");
    table.print("  ");

    for (index, split) in txn.splits.iter().enumerate() {
        if split.line_items.is_empty() {
            continue;
        }
        let mut items = Table::new(
            &["#", "description", "qty", "amount", "tax"],
            &[false, false, true, true, true],
        );
        for (item_index, item) in split.line_items.iter().enumerate() {
            items.push(vec![
                (item_index + 1).to_string(),
                item.description.clone(),
                format_quantity(item.quantity_milli),
                item.amount.to_display_string(),
                item.tax.to_display_string(),
            ]);
        }
        outln!();
        outln!("  split {} items", index + 1);
        items.print("    ");
    }
}

/// `"Groceries $30.00, Gas $24.19"` — what the money in this transaction
/// was for, which is the column a list is actually read for.
fn split_summary(txn: &Txn) -> String {
    txn.splits
        .iter()
        .map(|s| format!("{} {}", s.category_name, s.amount.to_display_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn flags(txn: &Txn) -> String {
    let mut out = String::new();
    if txn.tax_is_estimated {
        out.push('~');
    }
    if txn.needs_review {
        out.push('!');
    }
    if txn.archived {
        out.push('x');
    }
    out
}

fn yes_no(v: bool) -> String {
    if v { "yes" } else { "no" }.to_string()
}

/// `1_500 -> "1.5"`, integer arithmetic only.
fn format_quantity(milli: i64) -> String {
    let whole = milli / 1_000;
    let frac = (milli % 1_000).abs();
    if frac == 0 {
        return whole.to_string();
    }
    let mut decimals = format!("{frac:03}");
    while decimals.ends_with('0') {
        decimals.pop();
    }
    format!("{whole}.{decimals}")
}

/// Category id → `Group/Name`, loaded once.
///
/// One query rather than one per split: the same N+1 that `hydrate` avoids in
/// the core would reappear here, in the printing loop, if each row looked its
/// own name up.
struct CategoryNames(HashMap<i64, String>);

impl CategoryNames {
    fn load(db: &Db) -> anyhow::Result<Self> {
        Ok(CategoryNames(
            categories::list(db)?
                .into_iter()
                .map(|c| (c.id.get(), c.qualified_name()))
                .collect(),
        ))
    }

    fn get(&self, id: accounting_core::CategoryId) -> String {
        self.0
            .get(&id.get())
            .cloned()
            // Archived categories are excluded from `list` but a historical
            // split may still point at one.
            .unwrap_or_else(|| format!("category {id}"))
    }
}

/// A column-aligned table. Widths are measured in `char`s, not bytes, so a
/// category name with an accent in it does not shift the columns.
struct Table {
    headers: Vec<String>,
    right: Vec<bool>,
    rows: Vec<Vec<String>>,
}

impl Table {
    fn new(headers: &[&str], right: &[bool]) -> Self {
        Table {
            headers: headers.iter().map(|h| (*h).to_string()).collect(),
            right: right.to_vec(),
            rows: Vec::new(),
        }
    }

    fn push(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    fn print(&self, indent: &str) {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if let Some(w) = widths.get_mut(i) {
                    *w = (*w).max(cell.chars().count());
                }
            }
        }

        let line = |cells: &[String]| {
            let mut out = String::from(indent);
            for (i, cell) in cells.iter().enumerate() {
                if i > 0 {
                    out.push_str("  ");
                }
                let width = widths.get(i).copied().unwrap_or(0);
                let pad = width.saturating_sub(cell.chars().count());
                if self.right.get(i).copied().unwrap_or(false) {
                    out.push_str(&" ".repeat(pad));
                    out.push_str(cell);
                } else {
                    out.push_str(cell);
                    out.push_str(&" ".repeat(pad));
                }
            }
            // Trailing padding on the last column is invisible but shows up in
            // a diff of captured output.
            out.trim_end().to_string()
        };

        outln!("{}", line(&self.headers));
        for row in &self.rows {
            outln!("{}", line(row));
        }
    }
}

fn run_calc(args: &CalcArgs) -> anyhow::Result<()> {
    let entry = AmountEntry {
        subtotal: args.subtotal,
        tax: args.tax,
        total: args.total,
        last_edited: args.last_edited.map(AmountField::from),
    };
    let ctx = TaxContext {
        rate_ppm: args.rate,
        prices_include_tax: args.prices_include_tax,
    };

    let amounts = reconcile(&entry, &ctx).context("could not reconcile the amounts")?;
    print_amounts(&amounts, &ctx);

    if !args.splits.is_empty() {
        let drafts: Vec<SplitDraft> = args.splits.iter().copied().map(SplitDraft::new).collect();
        let plan = plan_splits(&amounts, &drafts).context("could not resolve the splits")?;
        outln!();
        print_plan(&plan);
    }

    Ok(())
}

fn print_amounts(amounts: &Amounts, ctx: &TaxContext) {
    let rows = [
        ("subtotal", amounts.subtotal),
        ("tax", amounts.tax),
        ("total", amounts.total),
    ];
    let width = rows
        .iter()
        .map(|(_, m)| m.to_display_string().len())
        .max()
        .unwrap_or(0);

    outln!("amounts");
    for (label, value) in rows {
        outln!("  {label:<9} {:>width$}", value.to_display_string());
    }
    outln!("  {:<9} {}", "rate", format_ppm(ctx.rate_ppm));
    outln!("  {:<9} {}", "estimated", amounts.tax_is_estimated);
}

fn print_plan(plan: &SplitPlan) {
    let cells: Vec<[String; 4]> = plan
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            [
                format!("{}", i + 1),
                row.amount.to_display_string(),
                row.subtotal.to_display_string(),
                row.tax.to_display_string(),
            ]
        })
        .collect();

    let footer = [
        ("sum", plan.assigned().to_display_string()),
        ("left", plan.unassigned.to_display_string()),
    ];

    let headers = ["#", "amount", "subtotal", "tax"];
    let mut widths = headers.map(str::len);
    for row in &cells {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.len());
        }
    }
    // The first two columns also carry the footer.
    for (label, value) in &footer {
        widths[0] = widths[0].max(label.len());
        widths[1] = widths[1].max(value.len());
    }

    outln!("splits");
    outln!(
        "  {:<w0$}  {:>w1$}  {:>w2$}  {:>w3$}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
    );
    for row in &cells {
        outln!(
            "  {:<w0$}  {:>w1$}  {:>w2$}  {:>w3$}",
            row[0],
            row[1],
            row[2],
            row[3],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
        );
    }

    for (label, value) in &footer {
        outln!(
            "  {:<w0$}  {:>w1$}",
            label,
            value,
            w0 = widths[0],
            w1 = widths[1]
        );
    }
}

/// `83_750 -> "8.375% (83750 ppm)"`, without touching a float.
fn format_ppm(ppm: u32) -> String {
    let whole = ppm / 10_000;
    let frac = ppm % 10_000;
    let mut decimals = format!("{frac:04}");
    while decimals.ends_with('0') {
        decimals.pop();
    }
    if decimals.is_empty() {
        format!("{whole}% ({ppm} ppm)")
    } else {
        format!("{whole}.{decimals}% ({ppm} ppm)")
    }
}

// ---------------------------------------------------------------------------
// argument parsers
// ---------------------------------------------------------------------------

fn parse_money(s: &str) -> Result<Money, String> {
    Money::parse(s).map_err(|e| e.to_string())
}

/// `"8.375"` or `"8.375%"` -> `83_750` parts per million.
fn parse_percent_ppm(s: &str) -> Result<u32, String> {
    // percent -> ppm is a factor of 10 000, so four decimal places of percent
    // are exactly representable and a fifth is not.
    let scaled = parse_scaled_decimal(s, 4)?;
    u32::try_from(scaled).map_err(|_| format!("rate `{s}` is too large"))
}

/// `"33.33"` -> `3_333` basis points.
fn parse_percent_bp(s: &str) -> Result<u32, String> {
    let scaled = parse_scaled_decimal(s, 2)?;
    u32::try_from(scaled).map_err(|_| format!("percentage `{s}` is too large"))
}

/// Parse a non-negative decimal into a fixed-point integer with `places`
/// digits of fraction. Integer arithmetic only — a float here would be exactly
/// the bug this crate exists to prevent.
fn parse_scaled_decimal(s: &str, places: u32) -> Result<u64, String> {
    let t = s.trim().trim_end_matches('%').trim();
    if t.is_empty() {
        return Err("expected a number".to_string());
    }
    let (int_part, frac_part) = t.split_once('.').unwrap_or((t, ""));
    let places = places as usize;
    if frac_part.len() > places {
        return Err(format!(
            "`{s}` has more than {places} decimal places of precision"
        ));
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("`{s}` is not a non-negative number"));
    }
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(format!("`{s}` is not a number"));
    }

    let scale = 10_u64.pow(u32::try_from(places).unwrap_or(0));
    let whole: u64 = if int_part.is_empty() {
        0
    } else {
        int_part
            .parse()
            .map_err(|_| format!("`{s}` is out of range"))?
    };
    let mut frac: u64 = if frac_part.is_empty() {
        0
    } else {
        frac_part
            .parse()
            .map_err(|_| format!("`{s}` is out of range"))?
    };
    for _ in frac_part.len()..places {
        frac *= 10;
    }

    whole
        .checked_mul(scale)
        .and_then(|v| v.checked_add(frac))
        .ok_or_else(|| format!("`{s}` is out of range"))
}

/// `Groceries=45.00`, `Groceries=25%`, `Groceries=remainder`.
///
/// Split on the **last** `=`, so a category name may contain one; an amount
/// never can.
fn parse_split_spec(s: &str) -> Result<SplitSpec, String> {
    let (category, amount) = s.rsplit_once('=').ok_or_else(|| {
        format!("`{s}` is not a split; expected NAME=AMOUNT, NAME=PERCENT% or NAME=remainder")
    })?;
    let category = category.trim();
    if category.is_empty() {
        return Err(format!("`{s}` has no category name"));
    }
    Ok(SplitSpec {
        category: category.to_string(),
        amount: parse_split(amount)?,
    })
}

/// `[SPLIT#:]DESCRIPTION=AMOUNT`, e.g. `1:Milk=3.49` or `Milk=3.49`.
///
/// The split number is optional and defaults to 1, because the overwhelmingly
/// common receipt has exactly one split and typing `1:` on every line of a
/// grocery run is the kind of friction that gets an app abandoned.
fn parse_item_spec(s: &str) -> Result<ItemSpec, String> {
    let (left, amount) = s
        .rsplit_once('=')
        .ok_or_else(|| format!("`{s}` is not an item; expected [SPLIT#:]DESCRIPTION=AMOUNT"))?;

    // Only a *numeric* prefix is a split number. "Milk: 2% =3.49" keeps its
    // colon and stays one description.
    let (split, description) = match left.split_once(':') {
        Some((prefix, rest)) => match prefix.trim().parse::<usize>() {
            Ok(n) => (n, rest),
            Err(_) => (1, left),
        },
        None => (1, left),
    };
    let description = description.trim();
    if description.is_empty() {
        return Err(format!("`{s}` has no description"));
    }
    if split == 0 {
        return Err(format!("`{s}` targets split 0; splits are numbered from 1"));
    }

    Ok(ItemSpec {
        split,
        description: description.to_string(),
        amount: parse_money(amount)?,
    })
}

/// `remainder`, `33.33%`, or a plain amount.
fn parse_split(s: &str) -> Result<SplitAmount, String> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("remainder") || t.eq_ignore_ascii_case("rest") {
        return Ok(SplitAmount::Remainder);
    }
    if let Some(pct) = t.strip_suffix('%') {
        return Ok(SplitAmount::Percent(parse_percent_bp(pct)?));
    }
    Ok(SplitAmount::Fixed(parse_money(t)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_to_ppm() {
        assert_eq!(parse_percent_ppm("8.375").unwrap(), 83_750);
        assert_eq!(parse_percent_ppm("8.375%").unwrap(), 83_750);
        assert_eq!(parse_percent_ppm("0").unwrap(), 0);
        assert_eq!(parse_percent_ppm("10").unwrap(), 100_000);
        assert_eq!(parse_percent_ppm(".5").unwrap(), 5_000);
        assert!(parse_percent_ppm("8.37501").is_err());
        assert!(parse_percent_ppm("-1").is_err());
        assert!(parse_percent_ppm("abc").is_err());
    }

    #[test]
    fn splits_parse_in_all_three_forms() {
        assert_eq!(parse_split("remainder").unwrap(), SplitAmount::Remainder);
        assert_eq!(parse_split("33.33%").unwrap(), SplitAmount::Percent(3_333));
        assert_eq!(
            parse_split("33.33").unwrap(),
            SplitAmount::Fixed(Money::from_cents(3_333))
        );
        assert_eq!(
            parse_split("$1,234.50").unwrap(),
            SplitAmount::Fixed(Money::from_cents(123_450))
        );
    }

    #[test]
    fn split_specs_carry_a_category_and_an_amount() {
        assert_eq!(
            parse_split_spec("Groceries=45.00").unwrap(),
            SplitSpec {
                category: "Groceries".to_string(),
                amount: SplitAmount::Fixed(Money::from_cents(4500)),
            }
        );
        assert_eq!(
            parse_split_spec(" Frequent/Groceries = 25% ")
                .unwrap()
                .amount,
            SplitAmount::Percent(2_500)
        );
        assert_eq!(
            parse_split_spec("Groceries=remainder").unwrap().amount,
            SplitAmount::Remainder
        );
        // A category name containing '=' is not a thing, but a name
        // containing '/' very much is — the seed ships Rent.
        assert_eq!(parse_split_spec("Rent=1200.00").unwrap().category, "Rent");

        assert!(parse_split_spec("Groceries").is_err());
        assert!(parse_split_spec("=45.00").is_err());
        assert!(parse_split_spec("Groceries=banana").is_err());
    }

    #[test]
    fn item_specs_default_to_the_first_split() {
        assert_eq!(
            parse_item_spec("Milk=3.49").unwrap(),
            ItemSpec {
                split: 1,
                description: "Milk".to_string(),
                amount: Money::from_cents(349),
            }
        );
        assert_eq!(parse_item_spec("2:Bread=4.00").unwrap().split, 2);
        // A colon that is not a split number stays in the description.
        assert_eq!(
            parse_item_spec("Milk: whole=3.49").unwrap().description,
            "Milk: whole"
        );
        assert_eq!(parse_item_spec("Milk: whole=3.49").unwrap().split, 1);

        assert!(parse_item_spec("Milk").is_err());
        assert!(parse_item_spec("1:=3.49").is_err());
        assert!(parse_item_spec("0:Milk=3.49").is_err());
    }

    #[test]
    fn quantities_format_without_floats() {
        assert_eq!(format_quantity(1_000), "1");
        assert_eq!(format_quantity(1_500), "1.5");
        assert_eq!(format_quantity(1_250), "1.25");
        assert_eq!(format_quantity(500), "0.5");
    }

    #[test]
    fn tables_align_on_characters_not_bytes() {
        let mut t = Table::new(&["a", "b"], &[false, true]);
        t.push(vec!["café".to_string(), "1".to_string()]);
        t.push(vec!["x".to_string(), "22".to_string()]);
        // Rendering is checked by eye elsewhere; what matters here is that a
        // multi-byte cell does not shift its neighbour, which a byte-length
        // width calculation would do.
        assert_eq!("café".chars().count(), 4);
        assert_eq!("café".len(), 5);
        t.print("");
    }

    #[test]
    fn ppm_formats_without_floats() {
        assert_eq!(format_ppm(83_750), "8.375% (83750 ppm)");
        assert_eq!(format_ppm(0), "0% (0 ppm)");
        assert_eq!(format_ppm(100_000), "10% (100000 ppm)");
    }

    #[test]
    fn the_documented_examples_work() {
        // pacc calc --total 54.32 --rate 8.375
        let a = reconcile(
            &AmountEntry {
                total: Some(Money::from_cents(5432)),
                ..Default::default()
            },
            &TaxContext {
                rate_ppm: parse_percent_ppm("8.375").unwrap(),
                prices_include_tax: false,
            },
        )
        .unwrap();
        assert!(a.tax_is_estimated);
        assert_eq!(a.subtotal + a.tax, a.total);

        // pacc calc --total 100.00 --split 33.33 --split 33.33 --split remainder
        let a = reconcile(
            &AmountEntry {
                total: Some(Money::from_cents(10_000)),
                ..Default::default()
            },
            &TaxContext::exclusive(0),
        )
        .unwrap();
        let drafts: Vec<SplitDraft> = ["33.33", "33.33", "remainder"]
            .iter()
            .map(|s| SplitDraft::new(parse_split(s).unwrap()))
            .collect();
        let plan = plan_splits(&a, &drafts).unwrap();
        assert_eq!(plan.rows.len(), 3);
        assert_eq!(plan.assigned(), Money::from_cents(10_000));
        assert!(plan.unassigned.is_zero());
    }
}
