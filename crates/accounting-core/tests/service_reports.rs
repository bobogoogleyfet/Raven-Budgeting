//! The monthly budget-vs-actual matrix, in both reporting bases.
//!
//! # The one idea this milestone rests on
//!
//! Money is **stored cash-flow dated, always** — the `txn_date` is the day the
//! money moved, and every figure can be walked back to a bank statement. The
//! *basis* is a presentation transform applied on read, never a second way of
//! storing things. That is what makes the two views reconcile:
//!
//! * `CashFlow` shows the $1200 insurance premium as $1200 in March.
//! * `Amortized` shows it as $100 in each of the twelve months of the budget
//!   cycle it falls in, so March stops looking like a catastrophe.
//!
//! Over a **cycle-aligned** range the two must total to the same cent, because
//! they are the same money arranged differently. Over a range narrower than a
//! cycle they must not, and that is a documented consequence rather than a
//! bug: half a cycle of amortized spending is, correctly, half the money.
//!
//! A category with no budget line, or with a `Monthly` one, has a one-month
//! cycle — so the two bases are identical for it. Only non-monthly budgets
//! amortize.

mod common;

use accounting_core::domain::budget_calc::{Period, monthly_contribution};
use accounting_core::repo::budgets::{self, BudgetLineRecord};
use accounting_core::service::reports::{self, MatrixRow, MonthlyMatrix, ReportBasis};
use accounting_core::time::MonthRange;
use accounting_core::{CategoryId, Db, Money, YearMonth};
use common::{assert_clean, cat, empty_db, m, months, seeded_db, spend_on, ym};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Builders and accessors
// ---------------------------------------------------------------------------

fn record(
    category_id: CategoryId,
    period: Period,
    start: (i32, u32),
    cents: i64,
) -> BudgetLineRecord {
    BudgetLineRecord {
        id: None,
        category_id,
        period,
        period_start: ym(start.0, start.1),
        period_end: None,
        amount: m(cents),
        rollover: true,
        notes: None,
    }
}

#[track_caller]
fn set_line(db: &Db, rec: &BudgetLineRecord) {
    budgets::upsert(db, rec).unwrap_or_else(|e| panic!("upsert budget line: {e}"));
}

#[track_caller]
fn matrix(db: &Db, range: MonthRange, basis: ReportBasis) -> MonthlyMatrix {
    reports::monthly_matrix(db, range, basis)
        .unwrap_or_else(|e| panic!("monthly_matrix {}..{}: {e}", range.start, range.end))
}

#[track_caller]
fn month_index(mx: &MonthlyMatrix, month: YearMonth) -> usize {
    mx.months
        .iter()
        .position(|m| *m == month)
        .unwrap_or_else(|| panic!("{month} is not one of the matrix's months: {:?}", mx.months))
}

/// The actual spending shown for one category in one month.
#[track_caller]
fn actual(mx: &MonthlyMatrix, category: &str, month: YearMonth) -> Money {
    let i = month_index(mx, month);
    row(mx, category).cells[i].actual
}

/// The budgeted contribution shown for one category in one month.
#[track_caller]
fn budgeted(mx: &MonthlyMatrix, category: &str, month: YearMonth) -> Money {
    let i = month_index(mx, month);
    row(mx, category).cells[i].budgeted
}

#[track_caller]
fn row<'a>(mx: &'a MonthlyMatrix, category: &str) -> &'a MatrixRow {
    mx.rows
        .iter()
        .find(|r| r.category_name == category)
        .unwrap_or_else(|| {
            let present: Vec<&str> = mx.rows.iter().map(|r| r.category_name.as_str()).collect();
            panic!("no row for `{category}`; the matrix held {present:?}")
        })
}

/// Every actual in the matrix, however it is arranged.
fn total_actual(mx: &MonthlyMatrix) -> Money {
    mx.rows
        .iter()
        .flat_map(|r| r.cells.iter())
        .map(|c| c.actual)
        .sum()
}

// ---------------------------------------------------------------------------
// The agreement property
// ---------------------------------------------------------------------------

#[test]
fn the_two_bases_disagree_month_by_month_and_agree_over_the_cycle() {
    // The plan's headline property. A $1200/yr line from 2026-01 and a $1200
    // premium paid on 2026-03-14, read over the whole of 2026:
    //
    //   cash flow: March = $1,200.00, every other month $0.00
    //   amortized: $1200.00 / 12 months of the cycle = $100.00 in each
    //
    // Both add to $1,200.00, because they are the same money arranged
    // differently — not two different stored numbers.
    let db = seeded_db();
    let insurance = cat(&db, "Insurance (Auto)");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-03-14", insurance, 120_000);

    let year = months((2026, 1), (2026, 12));
    let amortized = matrix(&db, year, ReportBasis::Amortized);
    let cashflow = matrix(&db, year, ReportBasis::CashFlow);

    assert_eq!(
        actual(&amortized, "Insurance (Auto)", ym(2026, 3)),
        m(10_000)
    );
    assert_eq!(
        actual(&cashflow, "Insurance (Auto)", ym(2026, 3)),
        m(120_000)
    );
    assert_eq!(
        actual(&amortized, "Insurance (Auto)", ym(2026, 1)),
        m(10_000),
        "January carries its share even though no money moved in January"
    );
    assert_eq!(
        actual(&cashflow, "Insurance (Auto)", ym(2026, 1)),
        Money::ZERO
    );

    assert_eq!(total_actual(&amortized), m(120_000));
    assert_eq!(total_actual(&cashflow), m(120_000));
    assert_eq!(
        total_actual(&amortized),
        total_actual(&cashflow),
        "the basis is a presentation transform; it may not create or destroy money"
    );
    assert_eq!(row(&amortized, "Insurance (Auto)").total.actual, m(120_000));
    assert_eq!(row(&cashflow, "Insurance (Auto)").total.actual, m(120_000));

    // The matrix says which view it is, so a UI cannot label one as the other.
    assert!(matches!(amortized.basis, ReportBasis::Amortized));
    assert!(matches!(cashflow.basis, ReportBasis::CashFlow));
    assert_clean(&db);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// The general form of the property above: for any cycle length and any
    /// amount, the two bases total the same over a cycle-aligned range, and the
    /// amortized parts sum back to the spend exactly — `Money::allocate`'s
    /// guarantee, exercised all the way through the report.
    #[test]
    fn the_two_bases_agree_over_any_cycle_aligned_range(
        cycle in prop::sample::select(vec![1_u32, 2, 3, 4, 6, 12]),
        cents in 1_i64..=1_000_000,
        month_of_cycle in 0_u32..12,
    ) {
        let period = match cycle {
            1 => Period::Monthly,
            12 => Period::Yearly,
            n => Period::EveryNMonths(n),
        };
        let spend_month = month_of_cycle % cycle;

        let db = seeded_db();
        let travel = cat(&db, "Travel");
        budgets::upsert(&db, &record(travel, period, (2026, 1), 120_000))
            .expect("upsert budget line");
        spend_on(
            &db,
            &format!("2026-{:02}-15", spend_month + 1),
            travel,
            cents,
        );

        // One whole cycle, starting where the line starts.
        let end = ym(2026, 1).add_months(i32::try_from(cycle).expect("small cycle") - 1);
        let range = MonthRange::new(ym(2026, 1), end);

        let amortized = matrix(&db, range, ReportBasis::Amortized);
        let cashflow = matrix(&db, range, ReportBasis::CashFlow);

        prop_assert_eq!(total_actual(&cashflow), m(cents));
        prop_assert_eq!(
            total_actual(&amortized),
            m(cents),
            "amortizing must not lose or invent a cent"
        );

        // …and it really is spread, not parked in one month: the shares of a
        // cycle differ by at most the one cent that cannot be divided.
        let shares: Vec<i64> = row(&amortized, "Travel")
            .cells
            .iter()
            .map(|c| c.actual.cents())
            .collect();
        let spread = shares.iter().max().copied().unwrap_or(0)
            - shares.iter().min().copied().unwrap_or(0);
        prop_assert!(
            spread <= 1,
            "a cycle's months must differ by at most a cent; got {:?}",
            shares
        );
    }
}

#[test]
fn an_amount_that_does_not_divide_evenly_still_sums_to_the_cent() {
    // $1000.00 over a twelve-month cycle is $83.333…, which is exactly the
    // shape that loses money to independent rounding: twelve times $83.33 is
    // $999.96. `Money::allocate` hands the four leftover cents to the first
    // four months instead, so the year reads
    //
    //   Jan-Apr  $83.34   (4 x 8334 = 33,336c)
    //   May-Dec  $83.33   (8 x 8333 = 66,664c)
    //   total   $1000.00  (100,000c)
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    set_line(&db, &record(travel, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-07-04", travel, 100_000);

    let year = months((2026, 1), (2026, 12));
    let mx = matrix(&db, year, ReportBasis::Amortized);
    let shares: Vec<i64> = row(&mx, "Travel")
        .cells
        .iter()
        .map(|c| c.actual.cents())
        .collect();

    assert_eq!(
        shares.iter().sum::<i64>(),
        100_000,
        "the remainder must be distributed, not dropped; got {shares:?}"
    );
    assert!(
        shares.iter().all(|c| *c == 8_333 || *c == 8_334),
        "every month is one of the two possible shares; got {shares:?}"
    );
    assert_eq!(
        shares.iter().filter(|c| **c == 8_334).count(),
        4,
        "exactly four cents are left over to distribute; got {shares:?}"
    );
    assert_eq!(
        actual(&mx, "Travel", ym(2026, 1)).cents(),
        8_334,
        "ties go to the lowest index, as `Money::allocate` documents"
    );
    assert_clean(&db);
}

#[test]
fn only_non_monthly_budgets_amortize() {
    // A `Monthly` line's cycle is one month long, and a category with no line
    // has nothing to amortize over, so for both of them the transform is the
    // identity and the two bases must agree cell for cell. If they ever differ
    // here, the report is spreading money across a window it invented.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let restaurants = cat(&db, "Restaurants & Takeout");
    set_line(&db, &record(groceries, Period::Monthly, (2026, 1), 40_000));

    spend_on(&db, "2026-02-10", groceries, 37_500);
    spend_on(&db, "2026-05-20", groceries, 41_000);
    spend_on(&db, "2026-02-11", restaurants, 6_400);

    let year = months((2026, 1), (2026, 12));
    let amortized = matrix(&db, year, ReportBasis::Amortized);
    let cashflow = matrix(&db, year, ReportBasis::CashFlow);

    for name in ["Groceries", "Restaurants & Takeout"] {
        for month in year.iter() {
            assert_eq!(
                actual(&amortized, name, month),
                actual(&cashflow, name, month),
                "`{name}` must read the same in both bases in {month}"
            );
            assert_eq!(
                budgeted(&amortized, name, month),
                budgeted(&cashflow, name, month),
                "`{name}`'s budget in {month} does not depend on the basis"
            );
        }
    }

    // And the spending really is where it was dated, in both.
    assert_eq!(actual(&amortized, "Groceries", ym(2026, 2)), m(37_500));
    assert_eq!(actual(&amortized, "Groceries", ym(2026, 3)), Money::ZERO);
    assert_eq!(
        actual(&cashflow, "Restaurants & Takeout", ym(2026, 2)),
        m(6_400)
    );
    assert_clean(&db);
}

#[test]
fn the_default_basis_is_amortized() {
    // The default has to be the comparable view. Defaulting to cash flow would
    // mean the first report a user ever sees is the one in which March looks
    // like a catastrophe and the other eleven months look like a surplus.
    assert_eq!(ReportBasis::default(), ReportBasis::Amortized);
}

#[test]
fn spending_in_a_month_no_line_covers_is_not_spread() {
    // The category has a budget, but not yet in February. A month no line
    // covers is its own one-month cycle, so February's spending stays in
    // February — spreading it across the cycle of a line that had not started
    // would put money in months the user had no budget in at all.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    set_line(&db, &record(travel, Period::Yearly, (2026, 6), 120_000));
    spend_on(&db, "2026-02-10", travel, 60_000);

    let year = months((2026, 1), (2026, 12));
    let amortized = matrix(&db, year, ReportBasis::Amortized);
    let cashflow = matrix(&db, year, ReportBasis::CashFlow);

    assert_eq!(actual(&amortized, "Travel", ym(2026, 2)), m(60_000));
    assert_eq!(actual(&cashflow, "Travel", ym(2026, 2)), m(60_000));
    assert_eq!(
        actual(&amortized, "Travel", ym(2026, 7)),
        Money::ZERO,
        "February's money must not drift into the line's own cycle"
    );
    assert_eq!(total_actual(&amortized), total_actual(&cashflow));
    // …and February is unbudgeted while July is not.
    assert_eq!(budgeted(&amortized, "Travel", ym(2026, 2)), Money::ZERO);
    assert_eq!(budgeted(&amortized, "Travel", ym(2026, 7)), m(10_000));
    assert_clean(&db);
}

#[test]
fn a_cycle_cut_short_by_the_lines_end_amortizes_over_what_is_left_of_it() {
    // A yearly line that stops in June has a six-month cycle, not a
    // twelve-month one: there is no line in force for July onwards to spread
    // anything into. $600 spent in March therefore reads as $100.00 in each of
    // January through June — and both bases still total $600.00, because the
    // clamped cycle is still a whole cycle.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    let mut rec = record(travel, Period::Yearly, (2026, 1), 120_000);
    rec.period_end = Some(ym(2026, 6));
    set_line(&db, &rec);
    spend_on(&db, "2026-03-14", travel, 60_000);

    let year = months((2026, 1), (2026, 12));
    let amortized = matrix(&db, year, ReportBasis::Amortized);
    let cashflow = matrix(&db, year, ReportBasis::CashFlow);

    for month in months((2026, 1), (2026, 6)).iter() {
        assert_eq!(
            actual(&amortized, "Travel", month),
            m(10_000),
            "{month} carries a sixth of the $600.00"
        );
    }
    assert_eq!(
        actual(&amortized, "Travel", ym(2026, 7)),
        Money::ZERO,
        "the line had ended; nothing may be spread past it"
    );
    assert_eq!(total_actual(&amortized), m(60_000));
    assert_eq!(total_actual(&cashflow), m(60_000));
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Budgeted figures
// ---------------------------------------------------------------------------

#[test]
fn the_budgeted_column_is_the_monthly_contribution_in_both_bases() {
    // The basis transforms *actuals*. The budget is already a per-month figure
    // — that is what a contribution schedule is — so it is identical in both
    // views, and it must be the line's own schedule rather than
    // `amount / months_in_range`.
    let db = seeded_db();
    let insurance = cat(&db, "Insurance (Auto)");
    let groceries = cat(&db, "Groceries");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 100_000));
    spend_on(&db, "2026-03-01", groceries, 5_000);

    let stored = budgets::for_category(&db, insurance, ym(2026, 3))
        .expect("for_category")
        .expect("the line is in force in March");

    let year = months((2026, 1), (2026, 12));
    for basis in [ReportBasis::Amortized, ReportBasis::CashFlow] {
        let mx = matrix(&db, year, basis);
        for month in year.iter() {
            assert_eq!(
                budgeted(&mx, "Insurance (Auto)", month),
                monthly_contribution(&stored.line, month),
                "{month} in {:?}",
                mx.basis
            );
        }
        // $1000.00 a year is $1000.00 a year however it is sliced.
        assert_eq!(
            row(&mx, "Insurance (Auto)").total.budgeted,
            m(100_000),
            "the year's contributions must sum to the year's budget"
        );
        assert_eq!(
            budgeted(&mx, "Groceries", ym(2026, 3)),
            Money::ZERO,
            "a category with no line has no budget, not a zero-dollar one"
        );
        assert_eq!(row(&mx, "Groceries").total.budgeted, Money::ZERO);
    }
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Cycles that straddle the range
// ---------------------------------------------------------------------------

#[test]
fn amortization_reaches_into_the_range_from_a_month_outside_it() {
    // Amortizing needs the whole cycle a month belongs to, and the rest of that
    // cycle may lie outside the range asked for. An implementation that loads
    // only the range's own transactions shows $0.00 here — the premium was
    // paid in March and the report starts in May — which is precisely the
    // number the amortized view exists to avoid.
    let db = seeded_db();
    let insurance = cat(&db, "Insurance (Auto)");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-03-14", insurance, 120_000);

    let summer = months((2026, 5), (2026, 8));
    let amortized = matrix(&db, summer, ReportBasis::Amortized);
    let cashflow = matrix(&db, summer, ReportBasis::CashFlow);

    for month in summer.iter() {
        assert_eq!(
            actual(&amortized, "Insurance (Auto)", month),
            m(10_000),
            "{month} carries its $100.00 share of the March premium"
        );
    }
    // 4 months x $100.00 = $400.00 of a $1200.00 premium.
    assert_eq!(total_actual(&amortized), m(40_000));
    assert_eq!(
        total_actual(&cashflow),
        Money::ZERO,
        "no money moved between May and August"
    );
    assert_eq!(
        amortized.months, cashflow.months,
        "the basis changes the numbers, never the columns"
    );
    assert_clean(&db);
}

#[test]
fn a_range_narrower_than_a_cycle_deliberately_does_not_reconcile() {
    // The documented consequence, asserted so it stays a decision rather than
    // becoming a surprise: over half a yearly cycle the cash-flow view shows
    // the whole $1,200.00 premium (it was paid in March) while the amortized
    // view shows six months x $100.00 = $600.00 of it. The totals-agree
    // property holds over cycle-aligned ranges and nowhere else.
    let db = seeded_db();
    let insurance = cat(&db, "Insurance (Auto)");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-03-14", insurance, 120_000);

    let half = months((2026, 1), (2026, 6));
    let amortized = matrix(&db, half, ReportBasis::Amortized);
    let cashflow = matrix(&db, half, ReportBasis::CashFlow);

    assert_eq!(total_actual(&cashflow), m(120_000));
    assert_eq!(total_actual(&amortized), m(60_000));
    assert_ne!(
        total_actual(&amortized),
        total_actual(&cashflow),
        "a range narrower than a cycle must not be expected to reconcile"
    );

    // …and over the whole cycle they agree again, which is what makes the
    // difference above a property of the *range* rather than of the data.
    let year = months((2026, 1), (2026, 12));
    assert_eq!(
        total_actual(&matrix(&db, year, ReportBasis::Amortized)),
        total_actual(&matrix(&db, year, ReportBasis::CashFlow))
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Shape of the matrix
// ---------------------------------------------------------------------------

#[test]
fn column_totals_and_the_grand_total_are_the_sums_of_the_cells_they_cover() {
    // A total is not an independently computed figure; it is the sum of what is
    // above it. If it is not, the report disagrees with itself on screen.
    let db = seeded_db();
    let insurance = cat(&db, "Insurance (Auto)");
    let groceries = cat(&db, "Groceries");
    let travel = cat(&db, "Travel");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    set_line(&db, &record(groceries, Period::Monthly, (2026, 1), 40_000));
    set_line(
        &db,
        &record(travel, Period::EveryNMonths(3), (2026, 1), 30_000),
    );
    spend_on(&db, "2026-03-14", insurance, 120_000);
    spend_on(&db, "2026-02-10", groceries, 37_500);
    spend_on(&db, "2026-04-02", groceries, 41_000);
    spend_on(&db, "2026-05-30", travel, 27_000);

    let year = months((2026, 1), (2026, 12));
    for basis in [ReportBasis::Amortized, ReportBasis::CashFlow] {
        let mx = matrix(&db, year, basis);
        assert_eq!(mx.totals.len(), mx.months.len());

        for (i, month) in mx.months.iter().enumerate() {
            let column_actual: Money = mx.rows.iter().map(|r| r.cells[i].actual).sum();
            let column_budgeted: Money = mx.rows.iter().map(|r| r.cells[i].budgeted).sum();
            assert_eq!(mx.totals[i].actual, column_actual, "{month} actual");
            assert_eq!(mx.totals[i].budgeted, column_budgeted, "{month} budgeted");
        }

        for r in &mx.rows {
            assert_eq!(
                r.total.actual,
                r.cells.iter().map(|c| c.actual).sum::<Money>(),
                "`{}` row total",
                r.category_name
            );
            assert_eq!(
                r.total.budgeted,
                r.cells.iter().map(|c| c.budgeted).sum::<Money>(),
                "`{}` row total",
                r.category_name
            );
            assert_eq!(
                r.cells.len(),
                mx.months.len(),
                "`{}` must have a cell per month",
                r.category_name
            );
        }

        assert_eq!(
            mx.grand_total.actual,
            mx.totals.iter().map(|c| c.actual).sum::<Money>()
        );
        assert_eq!(
            mx.grand_total.actual,
            mx.rows.iter().map(|r| r.total.actual).sum::<Money>(),
            "the grand total must agree whichever way it is reached"
        );
        assert_eq!(
            mx.grand_total.budgeted,
            mx.rows.iter().map(|r| r.total.budgeted).sum::<Money>()
        );
    }
    assert_clean(&db);
}

#[test]
fn rows_cover_a_budget_with_no_spending_and_spending_with_no_budget() {
    // Both halves are the interesting ones. A line with no spending is an
    // envelope quietly filling up; spending with no line is money the user
    // never planned for. A matrix built by joining budgets to actuals one way
    // or the other loses one of them.
    let db = seeded_db();
    let insurance = cat(&db, "Insurance (Auto)");
    let restaurants = cat(&db, "Restaurants & Takeout");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-04-18", restaurants, 8_800);

    let year = months((2026, 1), (2026, 12));
    for basis in [ReportBasis::Amortized, ReportBasis::CashFlow] {
        let mx = matrix(&db, year, basis);

        let budgeted_only = row(&mx, "Insurance (Auto)");
        assert_eq!(budgeted_only.total.budgeted, m(120_000));
        assert_eq!(budgeted_only.total.actual, Money::ZERO);
        assert_eq!(budgeted_only.group_name, "Bills");

        let spent_only = row(&mx, "Restaurants & Takeout");
        assert_eq!(spent_only.total.budgeted, Money::ZERO);
        assert_eq!(spent_only.total.actual, m(8_800));
        assert_eq!(actual(&mx, "Restaurants & Takeout", ym(2026, 4)), m(8_800));
        assert_eq!(spent_only.category_id, restaurants);
    }
    assert_clean(&db);
}

#[test]
fn an_empty_database_yields_the_months_and_no_rows() {
    // Reporting on a database with nothing in it is what the first launch does.
    // It must produce an empty report, not an error and not a panic on an empty
    // `months` vector.
    let db = empty_db();
    let year = months((2026, 1), (2026, 12));
    for basis in [ReportBasis::Amortized, ReportBasis::CashFlow] {
        let mx = matrix(&db, year, basis);
        assert_eq!(mx.months, year.iter().collect::<Vec<_>>());
        assert!(
            mx.rows.is_empty(),
            "nothing has been budgeted or spent; got {:?}",
            mx.rows
        );
        assert_eq!(mx.grand_total.actual, Money::ZERO);
        assert_eq!(mx.grand_total.budgeted, Money::ZERO);
        assert!(
            mx.totals
                .iter()
                .all(|c| c.actual.is_zero() && c.budgeted.is_zero())
        );
    }
}

#[test]
fn a_seeded_database_with_no_activity_reports_no_rows() {
    // The default chart of accounts exists, but nothing has been budgeted or
    // spent. The report is not the budget screen: `budget_status` lists every
    // budgeted category precisely so a UI can offer "set a budget" for the
    // empty ones, while a *report* of two dozen all-zero rows is noise around
    // the numbers the reader came for.
    let db = seeded_db();
    let mx = matrix(&db, months((2026, 1), (2026, 3)), ReportBasis::Amortized);
    assert!(
        mx.rows.is_empty(),
        "a category with neither a budget nor spending has nothing to report; \
         got {:?}",
        mx.rows
            .iter()
            .map(|r| r.category_name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(mx.grand_total.actual, Money::ZERO);
    assert_eq!(mx.grand_total.budgeted, Money::ZERO);
}

#[test]
fn a_single_month_range_is_a_one_column_matrix() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    spend_on(&db, "2026-08-09", groceries, 12_345);

    let mx = matrix(&db, months((2026, 8), (2026, 8)), ReportBasis::Amortized);
    assert_eq!(mx.months, vec![ym(2026, 8)]);
    assert_eq!(actual(&mx, "Groceries", ym(2026, 8)), m(12_345));
    assert_eq!(mx.grand_total.actual, m(12_345));
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Transfers are not spending
// ---------------------------------------------------------------------------

/// A move between the user's own accounts must not appear as spending.
///
/// This is what the `transfer` system category is *for* — the schema says so
/// where it is declared — and the report is the place it matters most. The
/// grand total is the one number a user checks against their own sense of the
/// month; count a $500 checking-to-savings move as April spending and the
/// figure is nearly ten times the truth, which costs trust in the whole
/// screen rather than in one row.
#[test]
fn a_transfer_between_accounts_is_not_counted_as_spending() {
    let db = seeded_db();

    spend_on(&db, "2026-04-02", cat(&db, "Transfer"), 50_000);
    spend_on(&db, "2026-04-05", cat(&db, "Groceries"), 6_000);

    for basis in [ReportBasis::CashFlow, ReportBasis::Amortized] {
        let mx = matrix(&db, months((2026, 4), (2026, 4)), basis);

        // $60.00 of groceries, and none of the $500.00 that merely moved.
        assert_eq!(
            total_actual(&mx),
            m(6_000),
            "{basis:?} basis counted the transfer; rows were {:?}",
            mx.rows
                .iter()
                .map(|r| r.category_name.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            !mx.rows.iter().any(|r| r.category_name == "Transfer"),
            "the Transfer row must not be in the matrix at all"
        );
        assert_eq!(actual(&mx, "Groceries", ym(2026, 4)), m(6_000));
    }

    assert_clean(&db);
}

/// The two sibling system categories stay in, and for good reason.
///
/// Uncategorized is real money that has merely not been filed yet — dropping
/// it would make every `quick_add` receipt vanish from the report until
/// someone got around to sorting it, which is a worse and much quieter
/// failure than the transfer one. Reimbursable money has genuinely left too;
/// it is just expected back.
#[test]
fn uncategorized_and_reimbursable_spending_still_appears() {
    let db = seeded_db();

    spend_on(&db, "2026-04-02", cat(&db, "Uncategorized"), 4_200);
    spend_on(&db, "2026-04-03", cat(&db, "Reimbursable"), 1_800);

    let mx = matrix(&db, months((2026, 4), (2026, 4)), ReportBasis::CashFlow);

    // $42.00 + $18.00 = $60.00, all of it still visible.
    assert_eq!(total_actual(&mx), m(6_000));
    assert_eq!(actual(&mx, "Uncategorized", ym(2026, 4)), m(4_200));
    assert_eq!(actual(&mx, "Reimbursable", ym(2026, 4)), m(1_800));

    assert_clean(&db);
}
