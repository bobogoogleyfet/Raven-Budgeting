//! The categories × months report, in two reporting bases.
//!
//! # Money is stored cash-flow dated. Always.
//!
//! A split dated 2026-03-14 is March's row in the database, in **every** basis.
//! Nothing in this module rewrites when money moved, and nothing writes at all:
//! a basis is a *presentation transform* applied on read. That is exactly why
//! the two bases reconcile — they are two renderings of one set of rows, not
//! two sets of numbers — and it is why the figures can still be checked against
//! a bank statement.
//!
//! # What the two bases answer
//!
//! * [`ReportBasis::CashFlow`] — "what left my account in March?" `$1,200.00`,
//!   the insurance premium, in the month it was paid. This is the basis that
//!   reconciles line by line with a statement.
//! * [`ReportBasis::Amortized`] — "what did March *cost* me?" `$100.00`, one
//!   twelfth of an annual premium. This is the basis that makes months
//!   comparable, and it is the default, because the alternative is a report in
//!   which one March looks like a catastrophe and eleven months look like a
//!   surplus.
//!
//! Amortization spreads the spending inside one **budget cycle** evenly across
//! that cycle's months with [`Money::allocate`], so the parts sum back to the
//! cycle's spending exactly — no cent is created or destroyed by the transform,
//! which is what makes "both bases total the same" a property rather than a
//! coincidence.
//!
//! A category with **no budget line**, or with a [`Period::Monthly`] one, has a
//! one-month cycle. Amortizing over one month is the identity, so for those
//! categories `Amortized == CashFlow`, exactly. Only non-monthly budgets — the
//! yearly premium, the quarterly water bill, the every-six-months car service —
//! actually spread.
//!
//! `budgeted` is [`monthly_contribution`] in **both** bases. It is already a
//! per-month schedule (`$1,200.00` a year is `$100.00` a month), so there is
//! nothing left to amortize, and showing the whole `$1,200.00` in January would
//! make the budget row disagree with the budget screen.
//!
//! # The trap: a cycle can hang off the end of the window
//!
//! Amortizing March requires the *whole* cycle March belongs to — January
//! through December — and most of it lies outside a request for
//! `2026-01..2026-06`. Computing the spread from only the in-range months
//! silently under-reports: the premium paid in November would vanish from a
//! January–June report that should show five twelfths of it. So spending is
//! loaded over the **union of the cycles overlapping the range**, amortized
//! there, and only then restricted to the requested months.
//!
//! The consequence is worth stating plainly, because it looks like a bug and is
//! not: **a window narrower than a cycle will not have
//! `Σ amortized == Σ cashflow`.** Six months of a twelve-month cycle should
//! show half of that cycle's spending, not all of it. The totals agree exactly
//! when the window covers whole cycles — a calendar year of yearly budgets
//! anchored in January, for instance.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::db::{Db, seed};
use crate::domain::budget_calc::{BudgetLine, Period, cycle_containing, monthly_contribution};
use crate::error::Result;
use crate::ids::CategoryId;
use crate::money::Money;
use crate::repo::{budgets, categories, transactions};
use crate::time::{MonthRange, YearMonth};

/// How a month's actual spending is presented.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportBasis {
    /// Spending inside a budget cycle, spread evenly across that cycle's
    /// months. The default: it is the view in which months are comparable.
    #[default]
    Amortized,
    /// Spending in the month its transactions fall in — what the bank
    /// statement says.
    CashFlow,
}

impl ReportBasis {
    /// The lowercase name used in settings, JSON and the CLI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ReportBasis::Amortized => "amortized",
            ReportBasis::CashFlow => "cashflow",
        }
    }

    /// A one-line description, for a report header that has to say which set of
    /// numbers the reader is looking at.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            ReportBasis::Amortized => {
                "spending inside a budget cycle is spread evenly across that cycle's months"
            }
            ReportBasis::CashFlow => "spending falls in the month its transactions are dated",
        }
    }
}

impl core::fmt::Display for ReportBasis {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::str::FromStr for ReportBasis {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self> {
        let t = s.trim().replace(['-', '_', ' '], "");
        if t.eq_ignore_ascii_case("amortized") || t.eq_ignore_ascii_case("amortised") {
            Ok(ReportBasis::Amortized)
        } else if t.eq_ignore_ascii_case("cashflow") || t.eq_ignore_ascii_case("cash") {
            Ok(ReportBasis::CashFlow)
        } else {
            Err(crate::error::CoreError::Invalid(format!(
                "`{s}` is not a reporting basis (expected `amortized` or `cashflow`)"
            )))
        }
    }
}

/// One cell: what was planned, and what happened.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct MatrixCell {
    pub budgeted: Money,
    pub actual: Money,
}

impl MatrixCell {
    /// An empty cell.
    pub const ZERO: MatrixCell = MatrixCell {
        budgeted: Money::ZERO,
        actual: Money::ZERO,
    };

    /// `budgeted - actual`: positive is under budget, negative is over.
    #[must_use]
    pub fn variance(self) -> Money {
        self.budgeted.saturating_sub(self.actual)
    }

    /// `true` when neither figure is present — the cells a renderer should show
    /// as blank rather than as `$0.00`.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.budgeted.is_zero() && self.actual.is_zero()
    }

    fn add(self, other: MatrixCell) -> MatrixCell {
        MatrixCell {
            budgeted: self.budgeted.saturating_add(other.budgeted),
            actual: self.actual.saturating_add(other.actual),
        }
    }
}

/// One category's row.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MatrixRow {
    pub category_id: CategoryId,
    pub category_name: String,
    pub group_name: String,
    /// One cell per month, in the same order as [`MonthlyMatrix::months`].
    pub cells: Vec<MatrixCell>,
    /// The row's own total, over the requested months only.
    pub total: MatrixCell,
}

impl MatrixRow {
    /// `"Group/Name"`.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.group_name, self.category_name)
    }
}

/// The whole grid.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MonthlyMatrix {
    pub months: Vec<YearMonth>,
    pub basis: ReportBasis,
    /// Rows in taxonomy order: group, then category.
    pub rows: Vec<MatrixRow>,
    /// Column totals, one per month.
    pub totals: Vec<MatrixCell>,
    pub grand_total: MatrixCell,
}

/// Build the categories × months grid.
///
/// Rows cover every category that has a budget line in force during the window
/// or any spending in it, in taxonomy order. Archived categories are included
/// when they carry history: retiring "Hobbies" in November does not delete
/// October's spending, and a report that renders it as `category 17` is worse
/// than one that names it.
///
/// # Query count
///
/// Four, regardless of range or category count: the categories, the budget
/// lines, all adjustments' owning categories (implicitly, via the lines) and
/// one aggregated spending query over the extended window. Amortization then
/// happens in memory, over a few dozen numbers.
pub fn monthly_matrix(db: &Db, range: MonthRange, basis: ReportBasis) -> Result<MonthlyMatrix> {
    let months: Vec<YearMonth> = range.iter().collect();
    if months.is_empty() {
        return Ok(MonthlyMatrix {
            months,
            basis,
            rows: Vec::new(),
            totals: Vec::new(),
            grand_total: MatrixCell::ZERO,
        });
    }

    // Every live line, not just the ones in force during the window: a cycle
    // that starts before the window still owns the months inside it.
    let mut lines: BTreeMap<CategoryId, Vec<BudgetLine>> = BTreeMap::new();
    for stored in budgets::list(db, None)? {
        lines
            .entry(stored.line.category_id)
            .or_default()
            .push(stored.line);
    }

    // The cycles each category's months belong to, and the window they span.
    // Computed before any spending is loaded, because cycles are a function of
    // the budget lines alone — which is what makes one bulk query possible.
    let cycles: BTreeMap<CategoryId, Vec<MonthRange>> = lines
        .iter()
        .map(|(id, lines)| (*id, cycles_over(lines, &months)))
        .collect();
    let load = extend(range, &cycles);

    let spend = transactions::spend_by_category_month(db, load, None)?;

    let mut rows: Vec<MatrixRow> = Vec::new();
    for category in categories::list_all(db)? {
        // Transfers are money moving between the user's own accounts, not
        // money leaving. Counting them would make a $500 move from checking to
        // savings read as $500 of April spending sitting next to the $60 of
        // groceries, and the report's total is the one number a user checks
        // against their own sense of the month — inflate it once and they stop
        // trusting the whole screen. This is the entire reason the `transfer`
        // system category exists; the schema says so where it is declared.
        //
        // Keyed on the system key rather than on `is_budgeted`, which is 0 for
        // all three system categories and so cannot tell them apart.
        // Uncategorized and Reimbursable both stay in: that money really did
        // leave, and dropping Uncategorized would make every `quick_add`
        // receipt vanish from reports until someone got around to filing it,
        // which is a far worse failure than the one being fixed here.
        if category.system_key.as_deref() == Some(seed::SYSTEM_TRANSFER) {
            continue;
        }

        let category_lines = lines.get(&category.id);
        let category_spend = spend.get(&category.id);

        let in_force = category_lines
            .is_some_and(|lines| months.iter().any(|ym| line_for(lines, *ym).is_some()));
        let spent_in_range = category_spend.is_some_and(|by_month| {
            months
                .iter()
                .any(|ym| by_month.get(ym).is_some_and(|m| !m.is_zero()))
        });
        if !in_force && !spent_in_range {
            continue;
        }

        let empty = BTreeMap::new();
        let raw = category_spend.unwrap_or(&empty);
        let actuals = match basis {
            ReportBasis::CashFlow => raw.clone(),
            ReportBasis::Amortized => {
                amortize(raw, cycles.get(&category.id).map_or(&[][..], Vec::as_slice))
            }
        };

        let cells: Vec<MatrixCell> = months
            .iter()
            .map(|ym| MatrixCell {
                budgeted: category_lines.map_or(Money::ZERO, |lines| {
                    line_for(lines, *ym).map_or(Money::ZERO, |line| monthly_contribution(line, *ym))
                }),
                actual: actuals.get(ym).copied().unwrap_or(Money::ZERO),
            })
            .collect();
        let total = cells.iter().fold(MatrixCell::ZERO, |acc, c| acc.add(*c));

        rows.push(MatrixRow {
            category_id: category.id,
            category_name: category.name,
            group_name: category.group_name,
            cells,
            total,
        });
    }

    let mut totals = vec![MatrixCell::ZERO; months.len()];
    for row in &rows {
        for (total, cell) in totals.iter_mut().zip(&row.cells) {
            *total = total.add(*cell);
        }
    }
    let grand_total = totals.iter().fold(MatrixCell::ZERO, |acc, c| acc.add(*c));

    Ok(MonthlyMatrix {
        months,
        basis,
        rows,
        totals,
        grand_total,
    })
}

/// The line in force in `ym`, if any.
///
/// At most one can be — [`budgets::upsert`] refuses overlapping live lines —
/// and taking the first match keeps this total rather than panicking if a
/// hand-edited database ever manages two.
fn line_for(lines: &[BudgetLine], ym: YearMonth) -> Option<&BudgetLine> {
    lines
        .iter()
        .find(|l| l.period_start <= ym && l.period_end.is_none_or(|end| end >= ym))
}

/// The distinct budget cycles that the requested months fall into.
///
/// Each month belongs to exactly one cycle: the clamped
/// [`cycle_containing`] of whichever line is in force, or — for a month no line
/// covers — nothing at all, which the caller treats as a one-month cycle. Walk
/// the months in order and skip the ones an already-collected cycle swallowed,
/// so a twelve-month range of a yearly budget yields one cycle rather than
/// twelve copies of it.
fn cycles_over(lines: &[BudgetLine], months: &[YearMonth]) -> Vec<MonthRange> {
    let mut out: Vec<MonthRange> = Vec::new();
    for ym in months {
        if out.iter().any(|c| c.contains(*ym)) {
            continue;
        }
        let Some(line) = line_for(lines, *ym) else {
            continue;
        };
        // A monthly cycle is the identity under amortization; skipping it keeps
        // the common case free of pointless work.
        if line.period == Period::Monthly {
            continue;
        }
        if let Some(cycle) = cycle_containing(line, *ym) {
            out.push(cycle);
        }
    }
    out
}

/// Widen `range` to cover every cycle that overlaps it.
///
/// This is the fix for the trap in the module docs: amortizing a month needs
/// the whole cycle containing it, and the cycle routinely hangs off both ends
/// of the requested window.
fn extend(range: MonthRange, cycles: &BTreeMap<CategoryId, Vec<MonthRange>>) -> MonthRange {
    let mut out = range;
    for cycle in cycles.values().flatten() {
        out.start = out.start.min(cycle.start);
        out.end = out.end.max(cycle.end);
    }
    out
}

/// Spread each cycle's spending evenly across that cycle's months.
///
/// [`Money::allocate`] over equal weights, so the months of a cycle sum back to
/// the cycle's spending to the cent — twelve independently rounded `$100.00`s
/// would be a rounding argument waiting to happen, and over a $1,000 premium
/// they would not even come to $1,000.
///
/// Months outside every cycle keep their own figure: they are their own
/// one-month cycle, and amortizing one month over one month is the identity.
fn amortize(
    spend: &BTreeMap<YearMonth, Money>,
    cycles: &[MonthRange],
) -> BTreeMap<YearMonth, Money> {
    let mut out: BTreeMap<YearMonth, Money> = BTreeMap::new();
    let mut covered: BTreeSet<YearMonth> = BTreeSet::new();

    for cycle in cycles {
        let len = cycle.len();
        if len == 0 {
            continue;
        }
        let total: Money = cycle
            .iter()
            .map(|ym| spend.get(&ym).copied().unwrap_or(Money::ZERO))
            .sum();
        for (ym, part) in cycle.iter().zip(total.allocate(&vec![1_i64; len])) {
            out.insert(ym, part);
            covered.insert(ym);
        }
    }

    for (ym, amount) in spend {
        if !covered.contains(ym) {
            out.insert(*ym, *amount);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;
    use crate::domain::reconcile::Amounts;
    use crate::repo::budgets::BudgetLineRecord;
    use crate::repo::transactions::{NewTransaction, SplitRecord};

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    fn ym(y: i32, m: u32) -> YearMonth {
        YearMonth::new(y, m).expect("valid month")
    }

    fn category(db: &Db, name: &str) -> CategoryId {
        categories::resolve(db, name).expect("category").id
    }

    fn spend_on(db: &Db, category_id: CategoryId, date: &str, amount: Money) {
        let new = NewTransaction {
            id: None,
            txn_date: date.parse().expect("valid date"),
            merchant_id: None,
            location_id: None,
            amounts: Amounts {
                subtotal: amount,
                tax: Money::ZERO,
                total: amount,
                tax_is_estimated: false,
            },
            tax_rate_ppm: None,
            detail_level: crate::repo::DetailLevel::Grouped,
            needs_review: false,
            notes: None,
            splits: vec![SplitRecord {
                id: None,
                uuid: None,
                category_id,
                subtotal: amount,
                tax: Money::ZERO,
                amount,
                sort_order: 0,
                notes: None,
                line_items: Vec::new(),
            }],
        };
        transactions::save(db, &new).expect("save");
    }

    fn budget(db: &Db, category_id: CategoryId, period: Period, amount: i64, start: YearMonth) {
        budgets::upsert(
            db,
            &BudgetLineRecord {
                id: None,
                category_id,
                period,
                period_start: start,
                period_end: None,
                amount: Money::from_dollars(amount),
                rollover: true,
                notes: None,
            },
        )
        .expect("budget line");
    }

    fn row<'a>(matrix: &'a MonthlyMatrix, name: &str) -> &'a MatrixRow {
        matrix
            .rows
            .iter()
            .find(|r| r.category_name == name)
            .unwrap_or_else(|| panic!("no row for {name}: {:?}", matrix.rows))
    }

    fn year_2026() -> MonthRange {
        MonthRange::new(ym(2026, 1), ym(2026, 12))
    }

    #[test]
    fn the_stated_property_a_yearly_premium_paid_in_march() {
        // $1,200/yr from January, $1,200 paid on 2026-03-14.
        let db = seeded();
        let insurance = category(&db, "Insurance (Auto)");
        budget(&db, insurance, Period::Yearly, 1_200, ym(2026, 1));
        spend_on(&db, insurance, "2026-03-14", Money::from_dollars(1_200));

        let amortized = monthly_matrix(&db, year_2026(), ReportBasis::Amortized).unwrap();
        let cash = monthly_matrix(&db, year_2026(), ReportBasis::CashFlow).unwrap();
        let march = 2_usize; // zero-based index of 2026-03

        assert_eq!(
            row(&amortized, "Insurance (Auto)").cells[march].actual,
            Money::from_dollars(100)
        );
        assert_eq!(
            row(&cash, "Insurance (Auto)").cells[march].actual,
            Money::from_dollars(1_200)
        );
        // Every other month of the cycle carries its twelfth too.
        assert_eq!(
            row(&amortized, "Insurance (Auto)").cells[0].actual,
            Money::from_dollars(100)
        );
        assert_eq!(row(&cash, "Insurance (Auto)").cells[0].actual, Money::ZERO);

        // …and the transform creates and destroys nothing.
        assert_eq!(
            row(&amortized, "Insurance (Auto)").total.actual,
            Money::from_dollars(1_200)
        );
        assert_eq!(
            row(&cash, "Insurance (Auto)").total.actual,
            Money::from_dollars(1_200)
        );
        assert_eq!(amortized.grand_total.actual, cash.grand_total.actual);

        // The budget row is the per-month schedule in both bases.
        for basis in [&amortized, &cash] {
            assert_eq!(
                row(basis, "Insurance (Auto)").cells[march].budgeted,
                Money::from_dollars(100)
            );
            assert_eq!(
                row(basis, "Insurance (Auto)").total.budgeted,
                Money::from_dollars(1_200)
            );
        }
    }

    #[test]
    fn a_monthly_budget_and_an_unbudgeted_category_never_move() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let fuel = category(&db, "Gas/Fuel");
        budget(&db, groceries, Period::Monthly, 400, ym(2026, 1));
        spend_on(&db, groceries, "2026-02-10", Money::from_dollars(380));
        spend_on(&db, fuel, "2026-05-02", Money::from_dollars(60));

        let amortized = monthly_matrix(&db, year_2026(), ReportBasis::Amortized).unwrap();
        let cash = monthly_matrix(&db, year_2026(), ReportBasis::CashFlow).unwrap();
        assert_eq!(
            row(&amortized, "Groceries").cells,
            row(&cash, "Groceries").cells
        );
        assert_eq!(
            row(&amortized, "Gas/Fuel").cells,
            row(&cash, "Gas/Fuel").cells
        );
        assert_eq!(
            row(&amortized, "Groceries").cells[1].actual,
            Money::from_dollars(380)
        );
        assert_eq!(
            row(&amortized, "Groceries").cells[1].budgeted,
            Money::from_dollars(400)
        );
        assert_eq!(
            row(&amortized, "Gas/Fuel").cells[4].actual,
            Money::from_dollars(60)
        );
    }

    #[test]
    fn a_cycle_hanging_off_the_window_is_still_loaded_whole() {
        // The trap: November's premium must show as five twelfths of itself
        // across a January–June report. Computing the spread from the in-range
        // months alone would show nothing at all.
        let db = seeded();
        let insurance = category(&db, "Insurance (Auto)");
        budget(&db, insurance, Period::Yearly, 1_200, ym(2026, 1));
        spend_on(&db, insurance, "2026-11-20", Money::from_dollars(1_200));

        let half = MonthRange::new(ym(2026, 1), ym(2026, 6));
        let amortized = monthly_matrix(&db, half, ReportBasis::Amortized).unwrap();
        let cash = monthly_matrix(&db, half, ReportBasis::CashFlow).unwrap();

        assert_eq!(
            row(&amortized, "Insurance (Auto)").total.actual,
            Money::from_dollars(600),
            "six twelfths of the annual premium"
        );
        assert_eq!(
            row(&cash, "Insurance (Auto)").total.actual,
            Money::ZERO,
            "nothing was paid in the first half"
        );
        // Documented, and correct: a window narrower than a cycle does not
        // reconcile between the bases.
        assert_ne!(amortized.grand_total.actual, cash.grand_total.actual);
    }

    #[test]
    fn odd_amounts_are_apportioned_without_leaking_a_cent() {
        let db = seeded();
        let travel = category(&db, "Travel");
        budget(&db, travel, Period::Yearly, 0, ym(2026, 1));
        spend_on(&db, travel, "2026-07-04", Money::from_cents(100_001));

        let matrix = monthly_matrix(&db, year_2026(), ReportBasis::Amortized).unwrap();
        let cells = &row(&matrix, "Travel").cells;
        assert_eq!(
            cells.iter().map(|c| c.actual).sum::<Money>(),
            Money::from_cents(100_001)
        );
        // No month is more than a cent from the even share.
        for cell in cells {
            assert!((cell.actual - Money::from_cents(8_333)).abs() <= Money::from_cents(1));
        }
    }

    #[test]
    fn a_quarterly_cycle_spreads_within_its_own_quarter_only() {
        let db = seeded();
        let auto = category(&db, "Auto Maintenance");
        budget(&db, auto, Period::EveryNMonths(3), 300, ym(2026, 1));
        spend_on(&db, auto, "2026-02-10", Money::from_dollars(300));
        spend_on(&db, auto, "2026-08-01", Money::from_dollars(600));

        let matrix = monthly_matrix(&db, year_2026(), ReportBasis::Amortized).unwrap();
        let cells = &row(&matrix, "Auto Maintenance").cells;
        // Q1 spreads $300 over Jan–Mar; Q3 spreads $600 over Jul–Sep; Q2 and Q4
        // saw nothing and stay empty, which is the point of a *cycle* boundary.
        let by_quarter: Vec<Money> = cells.iter().map(|c| c.actual).collect();
        assert_eq!(by_quarter[0..3], [Money::from_dollars(100); 3]);
        assert_eq!(by_quarter[3..6], [Money::ZERO; 3]);
        assert_eq!(by_quarter[6..9], [Money::from_dollars(200); 3]);
        assert_eq!(by_quarter[9..12], [Money::ZERO; 3]);
        assert_eq!(
            cells.iter().map(|c| c.actual).sum::<Money>(),
            Money::from_dollars(900)
        );
    }

    #[test]
    fn rows_totals_and_ordering_hold_together() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let insurance = category(&db, "Insurance (Auto)");
        budget(&db, groceries, Period::Monthly, 400, ym(2026, 1));
        budget(&db, insurance, Period::Yearly, 1_200, ym(2026, 1));
        spend_on(&db, groceries, "2026-02-10", Money::from_dollars(380));
        spend_on(&db, insurance, "2026-03-14", Money::from_dollars(1_200));

        let matrix = monthly_matrix(&db, year_2026(), ReportBasis::Amortized).unwrap();
        assert_eq!(matrix.months.len(), 12);
        assert_eq!(matrix.totals.len(), 12);
        assert_eq!(matrix.basis, ReportBasis::Amortized);

        // Bills before Frequent.
        assert_eq!(matrix.rows[0].category_name, "Insurance (Auto)");
        assert_eq!(matrix.rows[0].qualified_name(), "Bills/Insurance (Auto)");
        assert_eq!(matrix.rows[1].category_name, "Groceries");
        assert_eq!(matrix.rows.len(), 2, "only rows with a budget or spending");

        // Column totals are the sum of the rows, and the grand total the sum of
        // the columns — in both directions.
        for (index, column) in matrix.totals.iter().enumerate() {
            let by_row: Money = matrix.rows.iter().map(|r| r.cells[index].actual).sum();
            assert_eq!(column.actual, by_row, "month {index}");
        }
        assert_eq!(
            matrix.grand_total.actual,
            matrix.rows.iter().map(|r| r.total.actual).sum::<Money>()
        );
        assert_eq!(
            matrix.grand_total.budgeted,
            Money::from_dollars(400 * 12 + 1_200)
        );
        assert_eq!(matrix.grand_total.actual, Money::from_dollars(380 + 1_200));
        assert_eq!(
            matrix.grand_total.variance(),
            Money::from_dollars(400 * 12 + 1_200 - 380 - 1_200)
        );
    }

    #[test]
    fn an_archived_category_keeps_its_name_in_the_history_it_owns() {
        let db = seeded();
        let hobbies = category(&db, "Hobbies");
        spend_on(&db, hobbies, "2026-04-02", Money::from_dollars(45));
        categories::archive(&db, hobbies).unwrap();

        let matrix = monthly_matrix(&db, year_2026(), ReportBasis::CashFlow).unwrap();
        assert_eq!(
            row(&matrix, "Hobbies").cells[3].actual,
            Money::from_dollars(45)
        );
    }

    #[test]
    fn an_empty_range_is_an_empty_matrix_rather_than_an_error() {
        let db = seeded();
        let matrix = monthly_matrix(
            &db,
            MonthRange::new(ym(2026, 6), ym(2026, 1)),
            ReportBasis::Amortized,
        )
        .unwrap();
        assert!(matrix.months.is_empty());
        assert!(matrix.rows.is_empty());
        assert!(matrix.totals.is_empty());
        assert_eq!(matrix.grand_total, MatrixCell::ZERO);
    }

    #[test]
    fn a_basis_round_trips_through_its_name() {
        assert_eq!(ReportBasis::default(), ReportBasis::Amortized);
        assert_eq!(ReportBasis::Amortized.to_string(), "amortized");
        assert_eq!(ReportBasis::CashFlow.to_string(), "cashflow");
        for (text, expected) in [
            ("amortized", ReportBasis::Amortized),
            ("AMORTISED", ReportBasis::Amortized),
            ("cashflow", ReportBasis::CashFlow),
            (" cash-flow ", ReportBasis::CashFlow),
            ("cash_flow", ReportBasis::CashFlow),
        ] {
            assert_eq!(text.parse::<ReportBasis>().unwrap(), expected, "{text}");
        }
        assert!("accrual".parse::<ReportBasis>().is_err());
    }
}
