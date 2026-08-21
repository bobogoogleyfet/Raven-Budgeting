//! The budget, for one month.

use accounting_core::domain::budget_calc::OnTrack;
use accounting_core::domain::budget_calc::Period;
use accounting_core::repo::budgets::BudgetLineRecord;
use accounting_core::service::budgets;
use accounting_core::{CategoryId, Money, YearMonth};

use crate::api::db;
use crate::error::to_message;

/// One category's budget in the month asked about.
///
/// Carries `remaining` alongside the budgeted and actual figures, so the screen
/// never subtracts two money values of its own.
/// The home screen answers "how much is left?", and that number is
/// `SpendStatus::remaining` in the core. Subtracting `spent_cycle` from
/// `budgeted` in Dart would be the frontend computing money, which is the one
/// thing this layer must never do — so the core's own figure crosses the
/// bridge instead.
///
/// `funding` and `overspent_by` answer different questions and disagree in
/// normal use: an envelope spent exactly to plan is behind on *funding* — the
/// money was meant to be banked and is not — while being perfectly fine on
/// spending. Only a non-zero `overspent_by` means over budget. Rendering
/// `behind` as over budget would make the screen cry wolf on every well-run
/// category.
pub struct BudgetRowDto {
    pub category_id: i64,
    pub category_name: String,
    pub group_name: String,
    /// The line's budget for one full cycle. `None` throughout when the
    /// category has no budget line.
    pub budgeted: Option<i64>,
    /// The contribution for this month alone — a yearly line shows a twelfth
    /// here and the whole bill in `budgeted`.
    pub per_month: Option<i64>,
    pub funded: Option<i64>,
    /// Spending within the current cycle, through this month.
    pub spent_cycle: Option<i64>,
    /// `budgeted - spent_cycle`. Goes negative once spending passes the
    /// budget, which is the same news `overspent_by` carries as a magnitude.
    pub remaining: Option<i64>,
    /// Zero while within budget. Any non-zero value is the over-budget signal.
    pub overspent_by: Option<i64>,
    /// Spending in this month alone. Never null.
    pub spent_this_month: i64,
    /// Whether this category's group holds money that is owed rather than
    /// merely available — rent versus groceries. Decides whether a shortfall
    /// is worded as being behind or as having money left.
    pub group_is_obligation: bool,
    /// What was actually set aside this month, in cents. `None` means nothing
    /// has been recorded — which a sinking fund counts as nothing saved, and
    /// which the screen must tell apart from a recorded zero.
    pub contributed_this_month: Option<i64>,
    /// Whether this line is saved into rather than paid out of: a yearly
    /// premium, a car-repair fund. Only these can take a contribution.
    pub is_sinking_fund: bool,
    /// `"ahead" | "on track" | "behind" | "none"`.
    pub funding: String,
    /// The magnitude of ahead or behind. Never negative.
    pub funding_by: i64,
    /// Months left in the cycle, counting this one: 12 in the first month of a
    /// yearly line, 1 in its last.
    pub months_remaining: Option<u32>,
}

/// What is budgeted this month, and what has been spent against it.
///
/// Summed by the core, not here and not in Dart: adding money is arithmetic,
/// and a total computed per frontend is a total that drifts per frontend.
pub struct MonthTotalsDto {
    pub budgeted: i64,
    pub spent: i64,
}

/// The month's totals, for the figure the overview leads with.
pub fn month_totals(month: String) -> Result<MonthTotalsDto, String> {
    let as_of: YearMonth = month.parse().map_err(to_message)?;
    let db = db::locked()?;
    let totals = budgets::month_totals(&db, as_of).map_err(to_message)?;
    Ok(MonthTotalsDto {
        budgeted: totals.budgeted.cents(),
        spent: totals.spent.cents(),
    })
}

/// Every budgeted category, for the month given as `YYYY-MM`.
///
/// Includes categories with no budget line — the screen's most useful job is
/// offering to budget something that is not budgeted yet, and a row that is
/// missing cannot make that offer.
pub fn budget_status(month: String) -> Result<Vec<BudgetRowDto>, String> {
    let as_of: YearMonth = month.parse().map_err(to_message)?;
    let db = db::locked()?;
    let rows = budgets::budget_status(&db, as_of).map_err(to_message)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let (funding, funding_by) = match row.status.as_ref().map(|s| s.on_track) {
                Some(OnTrack::Ahead(by)) => ("ahead", by.cents()),
                Some(OnTrack::OnTrack) => ("on track", 0),
                Some(OnTrack::Behind(by)) => ("behind", by.cents()),
                None => ("none", 0),
            };
            BudgetRowDto {
                category_id: row.category_id.get(),
                category_name: row.category_name,
                group_name: row.group_name,
                budgeted: row.status.as_ref().map(|s| s.spend.budgeted.cents()),
                per_month: row.status.as_ref().map(|s| s.monthly_contribution.cents()),
                funded: row.status.as_ref().map(|s| s.funded_balance.cents()),
                spent_cycle: row.status.as_ref().map(|s| s.spend.spent.cents()),
                remaining: row.status.as_ref().map(|s| s.spend.remaining.cents()),
                overspent_by: row.status.as_ref().map(|s| s.spend.overspent_by.cents()),
                months_remaining: row.status.as_ref().map(|s| s.months_remaining),
                spent_this_month: row.spent_this_month.cents(),
                group_is_obligation: row.group_is_obligation,
                contributed_this_month: row.contributed_this_month.map(|m| m.cents()),
                is_sinking_fund: row
                    .line
                    .as_ref()
                    .is_some_and(|l| !matches!(l.period, Period::Monthly)),
                funding: funding.to_string(),
                funding_by,
            }
        })
        .collect())
}

/// Record what was actually set aside for a category in one month.
///
/// A sinking fund is funded by what this records and by nothing else. Without
/// it every yearly line reads as behind by its whole schedule, which is true
/// and useless — the app would be reporting that you have not saved because it
/// has never been told that you have.
///
/// Never a transaction: money moved into savings has not been spent, and filing
/// it as spending would count it against the budget now and again when the bill
/// is finally paid.
pub fn record_contribution(
    category_id: i64,
    month: String,
    amount_cents: i64,
) -> Result<(), String> {
    let month: YearMonth = month.parse().map_err(to_message)?;
    let db = db::locked()?;
    budgets::record_contribution(
        &db,
        CategoryId::new(category_id),
        month,
        Money::from_cents(amount_cents),
    )
    .map_err(to_message)
}

/// Forget a recorded contribution, returning the month to "nothing recorded".
pub fn forget_contribution(category_id: i64, month: String) -> Result<(), String> {
    let month: YearMonth = month.parse().map_err(to_message)?;
    let db = db::locked()?;
    budgets::forget_contribution(&db, CategoryId::new(category_id), month).map_err(to_message)
}

/// What setting a budget did, so the screen can say so out loud.
pub struct SupersededDto {
    /// True when a line already starting that month was restated in place
    /// rather than a new one being opened.
    pub restated: bool,
    /// The last month the previous line now applies to, when one was closed to
    /// make room.
    pub closed_after: Option<String>,
}

/// Set a category's budget from `month` onward.
///
/// Three cases, all handled by the core in one transaction: nothing in force
/// opens a new line; a line already starting that month is restated in place;
/// a line that started earlier is closed the month before and the new one
/// opened. A budget is a historical record rather than a setting, which is why
/// last year's figures do not move when this year's changes.
///
/// `every_n_months` is read only when `period` is `everyN`. An unknown period
/// is refused rather than defaulted: a budget silently landing on the wrong
/// schedule is a number nobody checks again.
pub fn set_budget(
    category_id: i64,
    amount_cents: i64,
    period: String,
    every_n_months: Option<u32>,
    month: String,
    rollover: bool,
) -> Result<SupersededDto, String> {
    let period = match period.as_str() {
        "monthly" => Period::Monthly,
        "yearly" => Period::Yearly,
        "everyN" => Period::EveryNMonths(
            every_n_months.ok_or_else(|| "every-n needs a number of months".to_string())?,
        ),
        other => return Err(format!("{other} is not a budget period")),
    };
    let period_start: YearMonth = month.parse().map_err(to_message)?;

    let record = BudgetLineRecord {
        id: None,
        category_id: CategoryId::new(category_id),
        period,
        period_start,
        // Still in force: a budget set today runs until something supersedes
        // it, which is what the next call to this does.
        period_end: None,
        amount: Money::from_cents(amount_cents),
        rollover,
        notes: None,
    };

    let db = db::locked()?;
    let outcome = budgets::set_budget(&db, &record).map_err(to_message)?;
    Ok(SupersededDto {
        restated: outcome.restated,
        closed_after: outcome.closed.map(|(_, last)| last.to_string()),
    })
}
