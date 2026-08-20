//! The budget, for one month.

use accounting_core::YearMonth;
use accounting_core::domain::budget_calc::OnTrack;
use accounting_core::service::budgets;

use crate::api::db;
use crate::error::to_message;

/// One category's budget in the month asked about.
///
/// Mirrors the Tauri shell's `BudgetRowDto` with one field added: `remaining`.
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
    /// `"ahead" | "on track" | "behind" | "none"`.
    pub funding: String,
    /// The magnitude of ahead or behind. Never negative.
    pub funding_by: i64,
    /// Months left in the cycle, counting this one: 12 in the first month of a
    /// yearly line, 1 in its last.
    pub months_remaining: Option<u32>,
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
                funding: funding.to_string(),
                funding_by,
            }
        })
        .collect())
}
