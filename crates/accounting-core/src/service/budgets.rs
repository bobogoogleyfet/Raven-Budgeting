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
//! use — though which way they disagree depends on the cadence. Funding asks
//! whether the money will be there when the bill lands: a $900 rent line paid
//! in full is [`OnTrack`], nothing left owing, and the same line untouched is
//! [`OnTrack::Behind`]($900); a $1,200 yearly premium with no contribution
//! recorded is behind by the whole schedule to date, because a month nobody
//! recorded anything for saved nothing. Spending asks the other
//! question entirely, and a $400/month grocery envelope with $500 spent is
//! settled on funding while being $100 over on spending. A UI must render them
//! as separate columns and must never label `Behind` as "over budget"; only
//! [`SpendStatus::overspent_by`] means that.
//!
//! Whether "behind" is the right *word* is a third question, and
//! [`CategoryBudgetStatus::group_is_obligation`] answers it: a shortfall is
//! only a shortfall where the money is owed.
//!
//! [`OnTrack`]: crate::domain::budget_calc::OnTrack
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
    /// What was actually set aside for this category in the month asked
    /// about, and whether anybody said so.
    ///
    /// `None` means no contribution has been recorded for the month — which a
    /// sinking fund treats as nothing saved, and which the screen needs to tell
    /// apart from a recorded zero so it can offer to record one.
    pub contributed_this_month: Option<Money>,
    /// Whether this category's group holds money that is owed.
    ///
    /// Decides how a shortfall is worded, and it is the difference between
    /// sense and nonsense: rent budgeted and unpaid is behind, groceries
    /// budgeted and unspent is money left. Carried here so the answer is
    /// decided once, in the data, rather than by each screen guessing from a
    /// group's name.
    pub group_is_obligation: bool,
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
    let mut contributions = budgets::all_contributions(db, window)?;

    Ok(budgeted
        .into_iter()
        .map(|category| {
            let spent = spend.remove(&category.id).unwrap_or_default();
            let adjusted = adjustments.remove(&category.id).unwrap_or_default();
            let saved = contributions.remove(&category.id).unwrap_or_default();
            let contributed_this_month = saved.get(&as_of).copied();
            let spent_this_month = spent.get(&as_of).copied().unwrap_or(Money::ZERO);
            let line = by_category.get(&category.id).cloned();
            let status = line
                .as_ref()
                .map(|line| funded_balance(line, &saved, &spent, &adjusted, as_of));

            CategoryBudgetStatus {
                category_id: category.id,
                category_name: category.name,
                group_name: category.group_name,
                contributed_this_month,
                group_is_obligation: category.group_is_obligation,
                line,
                status,
                spent_this_month,
            }
        })
        .collect())
}

/// What is budgeted this month, and what has been spent against it.
///
/// A screen that leads with one figure needs that figure computed here rather
/// than by summing rows in a frontend: adding money is arithmetic, and rule 4
/// keeps arithmetic on this side of the boundary so it is written once instead
/// of once per platform.
///
/// Both figures are for the **month**, not the cycle. A yearly line
/// contributes its monthly share to `budgeted`, which is what makes the pair
/// comparable — a $780 insurance bill in `budgeted` would swamp a month whose
/// other envelopes total a few hundred, and answer a question nobody asked.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthTotals {
    /// The sum of every budgeted category's contribution for this month.
    pub budgeted: Money,
    /// The sum of what was spent in this month against those categories.
    pub spent: Money,
}

/// [`MonthTotals`] for `as_of`.
///
/// Categories with no line in force contribute nothing to `budgeted`, but
/// their spending still counts towards `spent`: money that went out is money
/// that went out, and hiding it because nobody budgeted for it would make the
/// total disagree with the expense list.
pub fn month_totals(db: &Db, as_of: YearMonth) -> Result<MonthTotals> {
    let rows = budget_status(db, as_of)?;
    Ok(MonthTotals {
        budgeted: rows
            .iter()
            .filter_map(|r| r.status.as_ref())
            .map(|s| s.monthly_contribution)
            .sum(),
        spent: rows.iter().map(|r| r.spent_this_month).sum(),
    })
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
    let saved = budgets::contributions_for(db, category_id, window)?;

    Ok(CategoryBudgetStatus {
        category_id,
        category_name: category.name,
        group_name: category.group_name,
        contributed_this_month: saved.get(&as_of).copied(),
        group_is_obligation: category.group_is_obligation,
        spent_this_month: spent.get(&as_of).copied().unwrap_or(Money::ZERO),
        status: line
            .as_ref()
            .map(|line| funded_balance(line, &saved, &spent, &adjusted, as_of)),
        line,
    })
}

/// The budget screen's bottom line: every column of the table, added up.
///
/// Derived from the rows the screen is already showing rather than from a
/// second query, so the total and the lines above it cannot disagree — a
/// footer that re-reads the database can be a month behind the table it sits
/// under and there is nothing on screen to say so.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BudgetTotals {
    /// How much is committed per month across every budgeted category.
    pub per_month: Money,
    /// What is banked across every envelope right now.
    pub funded: Money,
    /// Spending within each line's current cycle.
    ///
    /// A sum across windows of different lengths, deliberately: a yearly line's
    /// cycle-to-date can span eleven months while a monthly line's spans days.
    /// It is the counterpart of [`Self::remaining`] and only means anything
    /// beside it — "of what these envelopes hold, this much has gone".
    pub spent_cycle: Money,
    /// What is left across every envelope. Current balances, so this one adds
    /// up cleanly however the cycles differ.
    pub remaining: Money,
    /// Total overspend, counting only the lines actually over. Overspending in
    /// one envelope is not cancelled by another having room.
    pub overspent_by: Money,
    /// How many budgeted categories are in the total.
    pub categories: usize,
}

/// Add up a set of budget rows.
///
/// Unbudgeted categories are skipped: they have no figure in any column, and
/// counting them would make the row count disagree with the table.
#[must_use]
pub fn totals(rows: &[CategoryBudgetStatus]) -> BudgetTotals {
    let mut out = BudgetTotals {
        per_month: Money::ZERO,
        funded: Money::ZERO,
        spent_cycle: Money::ZERO,
        remaining: Money::ZERO,
        overspent_by: Money::ZERO,
        categories: 0,
    };
    for row in rows {
        let Some(status) = row.status.as_ref() else {
            continue;
        };
        out.categories += 1;
        out.per_month = out.per_month.saturating_add(status.monthly_contribution);
        out.funded = out.funded.saturating_add(status.funded_balance);
        out.spent_cycle = out.spent_cycle.saturating_add(status.spend.spent);
        out.remaining = out.remaining.saturating_add(status.spend.remaining);
        out.overspent_by = out.overspent_by.saturating_add(status.spend.overspent_by);
    }
    out
}

/// Record what was actually set aside for a category in one month.
///
/// The write half of the sinking-fund change. A non-monthly line is funded by
/// what this records and by nothing else, so without a way to call it every
/// such line reads `Behind` by its whole schedule forever — which is true, and
/// useless.
///
/// Replaces rather than adds: a second figure for a month already recorded is a
/// correction, not a second deposit. See [`budgets::set_contribution`].
///
/// Deliberately not a transaction. Moving $100 into savings for a yearly
/// premium is not spending $100 — filing it as one would count it against the
/// category's budget, inflate every spending report, and then count it a second
/// time when the premium is actually paid.
pub fn record_contribution(
    db: &Db,
    category_id: CategoryId,
    month: YearMonth,
    amount: Money,
) -> Result<()> {
    budgets::set_contribution(db, category_id, month, amount)
}

/// Forget a recorded contribution, returning that month to "nothing recorded".
pub fn forget_contribution(db: &Db, category_id: CategoryId, month: YearMonth) -> Result<()> {
    budgets::clear_contribution(db, category_id, month)
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
    use crate::domain::budget_calc::{OnTrack, Period, monthly_contribution};
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

    /// Set aside, for real, every month the schedule asks for from the line's
    /// start through `through`.
    ///
    /// A stored line is a *plan*; a sinking fund's balance is built from what
    /// was actually banked. A test about cycle arithmetic has to state the fact
    /// as well as the plan, or it stops asking what the arithmetic does with
    /// the money and starts asking whether anybody ever transferred any.
    #[track_caller]
    fn fund_on_plan(db: &Db, category_id: CategoryId, through: YearMonth) {
        let stored = budgets::for_category(db, category_id, through)
            .expect("for_category")
            .expect("a budget line to fund");
        for month in MonthRange::new(stored.line.period_start, through).iter() {
            budgets::set_contribution(
                db,
                category_id,
                month,
                monthly_contribution(&stored.line, month),
            )
            .expect("set_contribution");
        }
    }

    #[test]
    fn the_acceptance_case_a_yearly_sinking_fund_in_august() {
        // $1,200/yr insurance from January, $1,200 paid in March. By August the
        // envelope has taken in eight $100 contributions and paid out $1,200,
        // so it is $400 in hand short of the $800 the schedule expects.
        //
        // The eight deposits are recorded rather than assumed: a sinking fund
        // holds what was actually set aside, so leaving them out would change
        // the question from "where does this arithmetic land" to "did anybody
        // ever pay in", which is pinned elsewhere.
        let db = seeded();
        let insurance = category(&db, "Car Insurance");
        budget(&db, insurance, Period::Yearly, 1_200, true);
        fund_on_plan(&db, insurance, ym(2026, 8));
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
        // The two measures answer different questions, and only spending is
        // entitled to say "over budget". A monthly envelope spent exactly to
        // plan satisfies both at once, so that case cannot tell them apart —
        // the cases that can are the envelope nobody has paid yet (behind on
        // funding, nothing overspent) and the overspent one (settled on
        // funding, over on spending).
        let db = seeded();
        let groceries = category(&db, "Groceries");
        budget(&db, groceries, Period::Monthly, 400, false);

        let untouched = budget_status_for(&db, groceries, ym(2026, 8))
            .unwrap()
            .status
            .unwrap();
        assert_eq!(
            untouched.on_track,
            OnTrack::Behind(Money::from_dollars(400)),
            "nothing paid is the case that is genuinely behind"
        );
        assert_eq!(untouched.spend.overspent_by, Money::ZERO);

        spend(&db, groceries, "2026-08-03", Money::from_dollars(400));
        let row = budget_status_for(&db, groceries, ym(2026, 8)).unwrap();
        let status = row.status.unwrap();
        // Spent exactly to plan: the month's budget is settled, nothing is over.
        assert_eq!(status.on_track, OnTrack::OnTrack);
        assert_eq!(status.spend.spent, Money::from_dollars(400));
        assert_eq!(status.spend.overspent_by, Money::ZERO);
        assert_eq!(row.spent_this_month, Money::from_dollars(400));

        spend(&db, groceries, "2026-08-20", Money::from_dollars(50));
        let over = budget_status_for(&db, groceries, ym(2026, 8))
            .unwrap()
            .status
            .unwrap();
        assert_eq!(
            over.on_track,
            OnTrack::OnTrack,
            "an overspend is not a funding shortfall; only `overspent_by` says over budget"
        );
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
            category(&db, "Gas"),
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

        let fuel_row = rows.iter().find(|r| r.category_name == "Gas").unwrap();
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
        let insurance = category(&db, "Car Insurance");
        budget(&db, insurance, Period::Yearly, 1_200, true);
        // Funded on plan, so the balance the two views must agree on is a real
        // one — $800 banked, $1,200 paid out, $60 adjusted in.
        fund_on_plan(&db, insurance, ym(2026, 8));
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
