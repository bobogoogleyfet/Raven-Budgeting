//! The budget screen: what each category is funded to, and what it has spent.
//!
//! # This layer combines; it does not calculate
//!
//! Every number here comes out of [`crate::domain::budget_calc`]. This module's
//! job is to fetch the two maps that calculation needs — spending per month and
//! adjustments per month — and hand them over. Re-deriving a contribution or a
//! funded balance in SQL would give the budget screen its own arithmetic, and
//! the first rounding disagreement with the report would be unattributable.
//!
//! # Funding and overspending are different questions
//!
//! [`FundedStatus`] carries both, deliberately, and they disagree in normal
//! use: a $400/month grocery envelope with $400 spent is
//! [`OnTrack::Behind`]($400) on funding — the schedule says that money should
//! be banked and it is not — and exactly on budget on spending. A UI must
//! render them as separate columns and must never label `Behind` as "over
//! budget"; only [`SpendStatus::overspent_by`] means that.
//!
//! [`OnTrack::Behind`]: crate::domain::budget_calc::OnTrack::Behind
//! [`SpendStatus::overspent_by`]: crate::domain::budget_calc::SpendStatus::overspent_by

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::domain::budget_calc::{BudgetLine, FundedStatus, funded_balance};
use crate::error::Result;
use crate::ids::{BudgetLineId, CategoryId};
use crate::money::Money;
use crate::repo::budgets::{self, BudgetLineRecord, Superseded};
use crate::repo::{categories, transactions};
use crate::time::{MonthRange, YearMonth};

/// One row of the budget screen.
///
/// Present for **every** budgeted category, including ones with no line at all:
/// the screen's most useful job is offering to budget something that is not
/// budgeted yet, and a row that is missing cannot make that offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryBudgetStatus {
    pub category_id: CategoryId,
    pub category_name: String,
    pub group_name: String,
    /// The line in force in the month asked about, if any.
    pub line: Option<BudgetLine>,
    /// `None` exactly when `line` is `None` — there is nothing to be on track
    /// against.
    pub status: Option<FundedStatus>,
    /// Spending in the month asked about. Distinct from
    /// `status.spend.spent`, which covers the whole current *cycle*: for a
    /// yearly line those are wildly different numbers and both are wanted.
    pub spent_this_month: Money,
}

impl CategoryBudgetStatus {
    /// `"Group/Name"`.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.group_name, self.category_name)
    }
}

/// Live spending for one category, month by month.
///
/// Months with no spending are absent rather than zero, which is the shape
/// [`funded_balance`] reads.
pub fn spend_by_month(
    db: &Db,
    category_id: CategoryId,
    range: MonthRange,
) -> Result<BTreeMap<YearMonth, Money>> {
    Ok(
        transactions::spend_by_category_month(db, range, Some(category_id))?
            .remove(&category_id)
            .unwrap_or_default(),
    )
}

/// The whole budget screen as of one month.
///
/// # Four queries, whatever the category count
///
/// The categories, the lines in force, **all** spending and **all** adjustments
/// — then the partitioning happens in memory. The obvious shape (loop over
/// categories, fetch each one's spend and adjustments) is 2N+2 round trips for
/// a screen that opens on every launch, and it gets slower exactly as the
/// user's history becomes worth looking at.
///
/// The window loaded starts at the earliest `period_start` of any line in
/// force, because [`funded_balance`] rolls the balance forward from there: a
/// sinking fund's answer for August depends on every month since January.
pub fn budget_status(db: &Db, as_of: YearMonth) -> Result<Vec<CategoryBudgetStatus>> {
    // `list` excludes archived categories; `is_budgeted = 0` excludes the
    // system ones, where budgeting "Transfer" is meaningless and budgeting
    // "Uncategorized" would encourage leaving things there.
    let budgeted: Vec<_> = categories::list(db)?
        .into_iter()
        .filter(|c| c.is_budgeted)
        .collect();

    let lines = budgets::list(db, Some(as_of))?;
    let by_category: BTreeMap<CategoryId, BudgetLine> = lines
        .into_iter()
        .map(|stored| (stored.line.category_id, stored.line))
        .collect();

    let earliest = by_category
        .values()
        .map(|l| l.period_start)
        .min()
        .unwrap_or(as_of)
        .min(as_of);
    let window = MonthRange::new(earliest, as_of);

    let mut spend = transactions::spend_by_category_month(db, window, None)?;
    let mut adjustments = budgets::all_adjustments(db, window)?;

    Ok(budgeted
        .into_iter()
        .map(|category| {
            let spent = spend.remove(&category.id).unwrap_or_default();
            let adjusted = adjustments.remove(&category.id).unwrap_or_default();
            let spent_this_month = spent.get(&as_of).copied().unwrap_or(Money::ZERO);
            let line = by_category.get(&category.id).cloned();
            let status = line
                .as_ref()
                .map(|line| funded_balance(line, &spent, &adjusted, as_of));

            CategoryBudgetStatus {
                category_id: category.id,
                category_name: category.name,
                group_name: category.group_name,
                line,
                status,
                spent_this_month,
            }
        })
        .collect())
}

/// One category's budget status, for a detail screen.
///
/// Works for an unbudgeted or archived category too — it reports a row with no
/// line rather than refusing — because "this has no budget" is a legitimate,
/// useful answer. An id that names no category at all is
/// [`crate::CoreError::NotFound`].
pub fn budget_status_for(
    db: &Db,
    category_id: CategoryId,
    as_of: YearMonth,
) -> Result<CategoryBudgetStatus> {
    let category = categories::get(db, category_id)?;
    let line = budgets::for_category(db, category_id, as_of)?.map(|stored| stored.line);

    let window = MonthRange::new(
        line.as_ref().map_or(as_of, |l| l.period_start).min(as_of),
        as_of,
    );
    let spent = spend_by_month(db, category_id, window)?;
    let adjusted = budgets::adjustments_for(db, category_id, window)?;

    Ok(CategoryBudgetStatus {
        category_id,
        category_name: category.name,
        group_name: category.group_name,
        spent_this_month: spent.get(&as_of).copied().unwrap_or(Money::ZERO),
        status: line
            .as_ref()
            .map(|line| funded_balance(line, &spent, &adjusted, as_of)),
        line,
    })
}

/// Create or replace one specific budget line.
///
/// Exact: it writes the record it is given and refuses to overlap anything
/// live. That is what an editor screen wants, where the user is holding one
/// line and changing its fields. What "change my grocery budget from July" is
/// asking for is [`set_budget`].
///
/// A thin pass-through today. It exists so that frontends call the service
/// layer for every write, and so that the day a budget change needs to do
/// something else as well — notify, re-plan, record an audit row — there is one
/// place to add it rather than three call sites in three UIs.
pub fn upsert_budget_line(db: &Db, rec: &BudgetLineRecord) -> Result<BudgetLineId> {
    budgets::upsert(db, rec)
}

/// Set a category's budget from `rec.period_start` onward.
///
/// The workflow behind a "save" button: whatever line already covers that month
/// is closed the month before (or restated, if it starts there), and the new
/// one opens — in one transaction, so a refusal changes nothing. See
/// [`budgets::supersede`] for why the two writes cannot be split.
pub fn set_budget(db: &Db, rec: &BudgetLineRecord) -> Result<Superseded> {
    budgets::supersede(db, rec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;
    use crate::domain::budget_calc::{OnTrack, Period};
    use crate::domain::reconcile::Amounts;
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

    fn spend(db: &Db, category_id: CategoryId, date: &str, amount: Money) {
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

    fn budget(db: &Db, category_id: CategoryId, period: Period, amount: i64, rollover: bool) {
        budgets::upsert(
            db,
            &BudgetLineRecord {
                id: None,
                category_id,
                period,
                period_start: ym(2026, 1),
                period_end: None,
                amount: Money::from_dollars(amount),
                rollover,
                notes: None,
            },
        )
        .expect("budget line");
    }

    #[test]
    fn the_acceptance_case_a_yearly_sinking_fund_in_august() {
        // $1,200/yr insurance from January, $1,200 paid in March. By August the
        // envelope has taken in eight $100 contributions and paid out $1,200,
        // so it is $400 in hand short of the $800 the schedule expects.
        let db = seeded();
        let insurance = category(&db, "Insurance (Auto)");
        budget(&db, insurance, Period::Yearly, 1_200, true);
        spend(&db, insurance, "2026-03-14", Money::from_dollars(1_200));

        let row = budget_status_for(&db, insurance, ym(2026, 8)).unwrap();
        let status = row.status.expect("a line was set");
        assert_eq!(status.monthly_contribution, Money::from_dollars(100));
        assert_eq!(status.contributed_to_date, Money::from_dollars(800));
        assert_eq!(status.spent_to_date, Money::from_dollars(1_200));
        assert_eq!(status.funded_balance, Money::from_dollars(-400));
        assert_eq!(status.target, Money::from_dollars(1_200));
        assert_eq!(status.months_remaining, 5);
        assert_eq!(status.on_track, OnTrack::Behind(Money::from_dollars(1_200)));
        assert_eq!(row.spent_this_month, Money::ZERO);

        // The same line, with the bill written off as a planned withdrawal:
        // funded $800, exactly on track. This is the plan's stated criterion,
        // and it is what an adjustment is for.
        budgets::add_adjustment(
            &db,
            insurance,
            ym(2026, 3),
            Money::from_dollars(1_200),
            Some("paid the annual premium out of the sinking fund"),
        )
        .unwrap();
        let after = budget_status_for(&db, insurance, ym(2026, 8))
            .unwrap()
            .status
            .expect("a line was set");
        assert_eq!(after.funded_balance, Money::from_dollars(800));
        assert_eq!(after.on_track, OnTrack::OnTrack);
    }

    #[test]
    fn funding_and_overspending_are_reported_separately() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        budget(&db, groceries, Period::Monthly, 400, false);
        spend(&db, groceries, "2026-08-03", Money::from_dollars(400));

        let row = budget_status_for(&db, groceries, ym(2026, 8)).unwrap();
        let status = row.status.unwrap();
        // Spent exactly to plan: behind on funding, not over on spending.
        assert_eq!(status.on_track, OnTrack::Behind(Money::from_dollars(400)));
        assert_eq!(status.spend.spent, Money::from_dollars(400));
        assert_eq!(status.spend.overspent_by, Money::ZERO);
        assert_eq!(row.spent_this_month, Money::from_dollars(400));

        spend(&db, groceries, "2026-08-20", Money::from_dollars(50));
        let over = budget_status_for(&db, groceries, ym(2026, 8))
            .unwrap()
            .status
            .unwrap();
        assert_eq!(over.spend.overspent_by, Money::from_dollars(50));
    }

    #[test]
    fn every_budgeted_category_gets_a_row_even_without_a_line() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        budget(&db, groceries, Period::Monthly, 400, false);
        spend(&db, groceries, "2026-08-03", Money::from_dollars(120));
        spend(
            &db,
            category(&db, "Gas/Fuel"),
            "2026-08-04",
            Money::from_dollars(60),
        );

        let rows = budget_status(&db, ym(2026, 8)).unwrap();
        // The three system categories are is_budgeted = 0 and must not appear.
        assert_eq!(rows.len(), 21);
        assert!(rows.iter().all(|r| r.category_name != "Uncategorized"));

        let groceries_row = rows.iter().find(|r| r.category_id == groceries).unwrap();
        assert!(groceries_row.line.is_some());
        assert_eq!(groceries_row.spent_this_month, Money::from_dollars(120));
        assert_eq!(groceries_row.qualified_name(), "Frequent/Groceries");

        let fuel_row = rows.iter().find(|r| r.category_name == "Gas/Fuel").unwrap();
        assert!(fuel_row.line.is_none(), "no budget set for fuel");
        assert!(
            fuel_row.status.is_none(),
            "and so nothing to be on track to"
        );
        assert_eq!(
            fuel_row.spent_this_month,
            Money::from_dollars(60),
            "spending is still reported, which is how the screen offers to budget it"
        );

        // Bills before Frequent: display order comes from the taxonomy.
        let names: Vec<&str> = rows.iter().map(|r| r.category_name.as_str()).collect();
        assert!(
            names.iter().position(|n| *n == "Electric")
                < names.iter().position(|n| *n == "Groceries")
        );
    }

    #[test]
    fn the_screen_agrees_with_the_single_category_view() {
        let db = seeded();
        let insurance = category(&db, "Insurance (Auto)");
        budget(&db, insurance, Period::Yearly, 1_200, true);
        spend(&db, insurance, "2026-03-14", Money::from_dollars(1_200));
        budgets::add_adjustment(&db, insurance, ym(2026, 4), Money::from_dollars(60), None)
            .unwrap();

        let from_screen = budget_status(&db, ym(2026, 8))
            .unwrap()
            .into_iter()
            .find(|r| r.category_id == insurance)
            .unwrap();
        let single = budget_status_for(&db, insurance, ym(2026, 8)).unwrap();
        assert_eq!(from_screen, single);
        // The adjustment landed in both.
        assert_eq!(
            single.status.unwrap().funded_balance,
            Money::from_dollars(-340)
        );
    }

    #[test]
    fn spend_by_month_is_empty_rather_than_zero_filled() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        spend(&db, groceries, "2026-03-02", Money::from_dollars(30));

        let got =
            spend_by_month(&db, groceries, MonthRange::new(ym(2026, 1), ym(2026, 12))).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[&ym(2026, 3)], Money::from_dollars(30));
    }
}
