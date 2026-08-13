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

use std::path::{Path, PathBuf};

use accounting_core::Money;
use accounting_core::db::integrity::Severity;
use accounting_core::db::{Db, SCHEMA_VERSION, check_all, seed_defaults};
use accounting_core::domain::reconcile::{
    AmountEntry, AmountField, Amounts, TaxContext, reconcile,
};
use accounting_core::domain::splits::{SplitAmount, SplitDraft, SplitPlan, plan_splits};
use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};

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
}

#[derive(Args)]
struct BackupArgs {
    /// Destination file. Must not already exist.
    #[arg(long = "to", value_name = "PATH")]
    to: PathBuf,
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
            println!("pacc {}", env!("CARGO_PKG_VERSION"));
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
    }

    Ok(())
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

    let counts = Counts::read(&db)?;
    println!(
        "{} {}",
        if existed { "updated" } else { "created" },
        path.display()
    );
    println!("  schema     user_version={}", db.user_version()?);
    println!(
        "  seeded     {} groups, {} categories, {} locations",
        counts.groups, counts.categories, counts.locations
    );
    Ok(())
}

/// Returns `true` when the database is healthy.
fn run_doctor(path: &Path) -> anyhow::Result<bool> {
    let db = open_existing(path)?;
    let counts = Counts::read(&db)?;
    let user_version = db.user_version()?;
    let journal_mode = db.journal_mode()?;
    let foreign_keys = db.foreign_keys_enabled()?;

    println!("database        {}", path.display());
    println!("user_version    {user_version}");
    println!("journal_mode    {journal_mode}");
    println!(
        "foreign_keys    {}",
        if foreign_keys { "on" } else { "off" }
    );
    println!("category_groups {}", counts.groups);
    println!("categories      {}", counts.categories);
    println!("locations       {}", counts.locations);
    println!("merchants       {}", counts.merchants);
    println!("transactions    {}", counts.txns);
    println!("splits          {}", counts.splits);

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

    println!();
    println!("{} errors, {} warnings", errors.len(), warnings.len());
    for v in errors.iter().chain(&warnings) {
        println!("  [{:?}] {v}", v.severity());
    }

    // Only errors make the database unhealthy, and only errors set the exit
    // code. Warnings are reported and then got out of the way of.
    let mut healthy = errors.is_empty();

    if usize::try_from(user_version).unwrap_or(usize::MAX) != SCHEMA_VERSION {
        println!("  [Error] user_version is {user_version}, expected {SCHEMA_VERSION}");
        healthy = false;
    }
    if !foreign_keys {
        // Should be impossible via Db::open; if it ever happens, every
        // REFERENCES clause in the schema is decorative.
        println!("  [Error] foreign_keys is off");
        healthy = false;
    }

    Ok(healthy)
}

fn run_backup(path: &Path, dest: &Path) -> anyhow::Result<()> {
    let db = open_existing(path)?;
    ensure_parent_dir(dest)?;
    db.backup(dest)
        .with_context(|| format!("could not back up to {}", dest.display()))?;
    println!("backed up {} -> {}", path.display(), dest.display());
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
        println!();
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

    println!("amounts");
    for (label, value) in rows {
        println!("  {label:<9} {:>width$}", value.to_display_string());
    }
    println!("  {:<9} {}", "rate", format_ppm(ctx.rate_ppm));
    println!("  {:<9} {}", "estimated", amounts.tax_is_estimated);
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

    println!("splits");
    println!(
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
        println!(
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
        println!(
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
