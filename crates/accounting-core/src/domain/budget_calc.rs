//! Budget contributions and funded balances.
//!
//! # Two rules that shape everything here
//!
//! 1. **A month is an indivisible bucket.** Contributions are never pro-rated
//!    by days. February contributes exactly what March contributes; leap
//!    years change nothing. This keeps the model comprehensible and keeps
//!    every figure reproducible without a clock.
//! 2. **A period's contributions sum to the period's amount, exactly.** A
//!    `$1000.00 / year` line is apportioned with [`Money::allocate`] over its
//!    twelve months, so the twelve monthly figures add back up to
//!    `$1000.00` — not `$999.96`, which is what twelve independently rounded
//!    `$83.33`s would give.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::ids::{BudgetLineId, CategoryId};
use crate::money::Money;
use crate::time::{MonthRange, YearMonth};

/// How often a budget line's target amount comes due.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Period {
    /// `amount` every month.
    Monthly,
    /// `amount` per twelve-month cycle, saved for in twelve equal-ish parts.
    Yearly,
    /// `amount` per `n`-month cycle. `n == 0` is inert (no contributions).
    EveryNMonths(u32),
    /// `amount` due once, on `due`; saved for from `period_start` through the
    /// month `due` falls in.
    ByDate { due: NaiveDate },
}

/// One budgeted line: a category, a target amount, and a cadence.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BudgetLine {
    pub id: BudgetLineId,
    pub category_id: CategoryId,
    pub period: Period,
    /// First month this line is active, and the anchor for every cycle.
    pub period_start: YearMonth,
    /// Last month this line is active; `None` means open-ended.
    pub period_end: Option<YearMonth>,
    /// The target amount per period.
    pub amount: Money,
    /// Whether an unspent balance carries into the next month.
    pub rollover: bool,
}

/// How a funded balance compares with where it should be by now.
///
/// This is a **funding** measure — "will the money be there when the bill
/// arrives?" — and it is the right question for a sinking fund. It is *not* an
/// overspending warning: see [`SpendStatus`], which answers the separate
/// question "have I spent more than I budgeted?". A UI must not render
/// [`OnTrack::Behind`] as "over budget"; a grocery envelope spent exactly to
/// plan is legitimately `Behind` on funding and perfectly fine on spending.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum OnTrack {
    /// Funded ahead of schedule by this much.
    Ahead(Money),
    /// Exactly on schedule.
    OnTrack,
    /// Short of schedule by this much.
    Behind(Money),
}

/// Spending against the budget for the **current cycle**.
///
/// Deliberately separate from [`OnTrack`]. The two answer different questions
/// and disagree in normal use: a $400/mo grocery envelope with $400 spent is
/// `OnTrack::Behind($400)` on funding (the schedule says that money should be
/// banked, and it is not) while being exactly on budget here.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SpendStatus {
    /// The line's budget for one full cycle.
    pub budgeted: Money,
    /// Spending within the current cycle, through `as_of`.
    pub spent: Money,
    /// `budgeted - spent`. Negative once spending passes the budget.
    pub remaining: Money,
    /// How far past the cycle budget spending has gone; [`Money::ZERO`] when
    /// still within it. A UI can treat any non-zero value here as the "over
    /// budget" signal.
    pub overspent_by: Money,
}

/// The state of one budget line as of a given month.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FundedStatus {
    pub category_id: CategoryId,
    /// The contribution for `as_of` itself.
    pub monthly_contribution: Money,
    /// Contributions from `period_start` through `as_of`.
    pub contributed_to_date: Money,
    /// Spending from `period_start` through `as_of`.
    pub spent_to_date: Money,
    /// Contributions minus spending plus adjustments, honouring `rollover`.
    pub funded_balance: Money,
    /// The line's target amount for one period.
    pub target: Money,
    /// Months left in the current cycle, **counting `as_of` itself**: `12` in
    /// the first month of a yearly line, `1` in its last. Together with the
    /// elapsed months it always adds up to the cycle length.
    pub months_remaining: u32,
    /// Funding progress against the contribution schedule.
    pub on_track: OnTrack,
    /// Spending against this cycle's budget. Separate from `on_track` — see
    /// [`SpendStatus`].
    pub spend: SpendStatus,
}

/// Length of the cycle containing `ym`, and `ym`'s zero-based index within it.
///
/// `None` when the line is not active in `ym` (before its start, after its
/// end, past a `ByDate` due month, or a degenerate `EveryNMonths(0)`).
fn cycle_position(line: &BudgetLine, ym: YearMonth) -> Option<(usize, usize)> {
    if ym < line.period_start {
        return None;
    }
    if let Some(end) = line.period_end
        && ym > end
    {
        return None;
    }
    let offset = ym.months_since(line.period_start);
    let offset = usize::try_from(offset).ok()?;

    match line.period {
        Period::Monthly => Some((1, 0)),
        Period::Yearly => Some((12, offset % 12)),
        Period::EveryNMonths(0) => None,
        Period::EveryNMonths(n) => {
            let n = usize::try_from(n).ok()?;
            Some((n, offset % n))
        }
        Period::ByDate { due } => {
            let due_ym = YearMonth::from_date(due);
            let span = due_ym.months_since(line.period_start) + 1;
            let span = usize::try_from(span).ok()?;
            if span == 0 || offset >= span {
                None
            } else {
                Some((span, offset))
            }
        }
    }
}

/// The months of the budget cycle that `ym` belongs to, inclusive.
///
/// `None` when the line is not active in `ym` — before it starts, after it
/// ends, past a `ByDate` due month, or a degenerate `EveryNMonths(0)`. A caller
/// that needs a cycle for *every* month (the amortized report does) treats
/// `None` as a one-month cycle, which is exactly what a month with no budget
/// deserves.
///
/// # Why this is public
///
/// Amortizing spending across a cycle needs the cycle's boundaries, and the
/// only correct source for them is the same [`cycle_position`] the
/// contributions use. A second implementation in the reporting layer would
/// drift from this one the first time a period type is added, and the symptom
/// would be a report that quietly disagrees with the budget screen.
///
/// # Clamping
///
/// The range never extends past `period_end`. Without that clamp a `Yearly`
/// line ending in June would claim a January–December cycle for its own months
/// *and* leave July onward as separate one-month cycles — months would belong
/// to two cycles at once, and amortized spending would be counted twice.
#[must_use]
pub fn cycle_containing(line: &BudgetLine, ym: YearMonth) -> Option<MonthRange> {
    let (len, idx) = cycle_position(line, ym)?;
    let idx = i32::try_from(idx).ok()?;
    let len = i32::try_from(len).ok()?;
    let start = ym.add_months(-idx);
    let mut end = start.add_months(len - 1);
    if let Some(period_end) = line.period_end
        && period_end < end
    {
        end = period_end;
    }
    Some(MonthRange::new(start, end))
}

/// This month's contribution toward `line`.
///
/// The period's amount is apportioned across the months of the cycle with
/// [`Money::allocate`] over equal weights, so summing the cycle's months
/// reproduces `line.amount` to the cent. Returns [`Money::ZERO`] for months
/// in which the line is not active.
#[must_use]
pub fn monthly_contribution(line: &BudgetLine, ym: YearMonth) -> Money {
    let Some((len, idx)) = cycle_position(line, ym) else {
        return Money::ZERO;
    };
    cycle_schedule(line.amount, len)
        .get(idx)
        .copied()
        .unwrap_or(Money::ZERO)
}

/// The per-month contributions for one cycle of `len` months.
fn cycle_schedule(amount: Money, len: usize) -> Vec<Money> {
    amount.allocate(&vec![1_i64; len])
}

/// Contributions for every month of `range`.
///
/// Months in which the line is inactive are present with a [`Money::ZERO`]
/// value, so callers can chart the range without re-deriving its months.
#[must_use]
pub fn contributions(line: &BudgetLine, range: MonthRange) -> BTreeMap<YearMonth, Money> {
    range
        .iter()
        .map(|ym| (ym, monthly_contribution(line, ym)))
        .collect()
}

/// Roll the balance forward month by month and report where the line stands.
///
/// The recurrence, for every month from `period_start` through `as_of`:
///
/// ```text
/// balance[m] = carried[m] + contribution[m] - spend[m] + adjustment[m]
/// balance[start - 1] = 0
/// ```
///
/// `carried[m]` is `balance[m-1]`, except that a line with `rollover == false`
/// starts each **cycle** from zero. The reset is per period, not per month, so
/// both shapes behave sensibly: a monthly grocery envelope resets every month
/// (its cycle *is* one month, so February's overspend is February's problem),
/// while a `Yearly` line still accumulates across its twelve months toward the
/// bill and starts fresh on the anniversary.
///
/// `spend_by_month` and `adjustments` are read only for months inside the
/// `period_start..=as_of` window; entries outside it are ignored.
#[must_use]
pub fn funded_balance(
    line: &BudgetLine,
    spend_by_month: &BTreeMap<YearMonth, Money>,
    adjustments: &BTreeMap<YearMonth, Money>,
    as_of: YearMonth,
) -> FundedStatus {
    let range = MonthRange::new(line.period_start, as_of);

    let mut balance = Money::ZERO;
    let mut contributed = Money::ZERO;
    let mut spent = Money::ZERO;

    for ym in range.iter() {
        let contribution = monthly_contribution(line, ym);
        let spend = spend_by_month.get(&ym).copied().unwrap_or(Money::ZERO);
        let adjustment = adjustments.get(&ym).copied().unwrap_or(Money::ZERO);

        // Without rollover the balance restarts at each *cycle* boundary, not
        // each month. For a `Monthly` line the cycle is one month, so this is
        // the familiar "use it or lose it" envelope. For a `Yearly` line it
        // still accumulates across the twelve months toward the bill and
        // resets on the anniversary, which is what "yearly budget, no
        // rollover" should mean.
        let starts_new_cycle = cycle_position(line, ym).is_some_and(|(_, idx)| idx == 0);
        let carried = if line.rollover || !starts_new_cycle {
            balance
        } else {
            Money::ZERO
        };
        balance = carried
            .saturating_add(contribution)
            .saturating_sub(spend)
            .saturating_add(adjustment);

        contributed = contributed.saturating_add(contribution);
        spent = spent.saturating_add(spend);
    }

    // Where the line *should* be by now: `target * months_elapsed /
    // months_in_period`, computed as the sum of the cycle schedule through
    // `as_of` so that it is exact integer math rather than a float ratio.
    // `as_of` counts as elapsed (its contribution has landed) and also counts
    // as remaining (its money is still there to spend), so within a line's
    // first cycle `expected == contributed_to_date`.
    //
    // Consequence worth stating: an envelope spent exactly to plan reports
    // `Behind(target)`, because the schedule says the money should be banked
    // and it is not. That is a funding measure, not an overspending warning.
    let (expected, months_remaining) = match cycle_position(line, as_of) {
        Some((len, idx)) => {
            let schedule = cycle_schedule(line.amount, len);
            let expected: Money = schedule.iter().take(idx + 1).copied().sum();
            let remaining = u32::try_from(len - idx).unwrap_or(0);
            (expected, remaining)
        }
        // Inactive: before the line starts nothing is expected yet; after it
        // ends the whole target should have been funded.
        None if as_of < line.period_start => (Money::ZERO, 0),
        None => (line.amount, 0),
    };

    // Spending against *this cycle's* budget. Deliberately a separate measure
    // from `on_track`: that one asks whether the money will be there when the
    // bill lands, this one asks whether more has been spent than was budgeted.
    // For a monthly envelope the two routinely disagree, and only this one
    // should ever be surfaced as "over budget".
    let cycle_start = match cycle_position(line, as_of) {
        Some((_, idx)) => as_of.add_months(-i32::try_from(idx).unwrap_or(0)),
        None => line.period_start,
    };
    let spent_this_cycle: Money = MonthRange::new(cycle_start, as_of)
        .iter()
        .map(|ym| spend_by_month.get(&ym).copied().unwrap_or(Money::ZERO))
        .sum();
    let remaining = line.amount.saturating_sub(spent_this_cycle);
    let spend = SpendStatus {
        budgeted: line.amount,
        spent: spent_this_cycle,
        remaining,
        overspent_by: if remaining.is_negative() {
            remaining.abs()
        } else {
            Money::ZERO
        },
    };

    let on_track = match balance.cmp(&expected) {
        std::cmp::Ordering::Greater => OnTrack::Ahead(balance.saturating_sub(expected)),
        std::cmp::Ordering::Equal => OnTrack::OnTrack,
        std::cmp::Ordering::Less => OnTrack::Behind(expected.saturating_sub(balance)),
    };

    FundedStatus {
        category_id: line.category_id,
        monthly_contribution: monthly_contribution(line, as_of),
        contributed_to_date: contributed,
        spent_to_date: spent,
        funded_balance: balance,
        target: line.amount,
        months_remaining,
        on_track,
        spend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ym(y: i32, m: u32) -> YearMonth {
        YearMonth::new(y, m).expect("valid month")
    }

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn line(period: Period, amount: i64, rollover: bool) -> BudgetLine {
        BudgetLine {
            id: BudgetLineId::new(1),
            category_id: CategoryId::new(1),
            period,
            period_start: ym(2026, 1),
            period_end: None,
            amount: m(amount),
            rollover,
        }
    }

    #[test]
    fn a_yearly_line_contributes_its_full_amount_over_twelve_months() {
        let l = line(Period::Yearly, 100_000, true);
        let year = MonthRange::new(ym(2026, 1), ym(2026, 12));
        let c = contributions(&l, year);
        assert_eq!(c.len(), 12);
        assert_eq!(c.values().copied().sum::<Money>(), m(100_000));
        // No month is more than a cent from the even share.
        for v in c.values() {
            assert!((*v - m(8333)).abs() <= m(1));
        }
    }

    #[test]
    fn yearly_cycles_repeat_after_twelve_months() {
        let l = line(Period::Yearly, 100_000, true);
        let two_years = MonthRange::new(ym(2026, 1), ym(2027, 12));
        let c = contributions(&l, two_years);
        assert_eq!(c.values().copied().sum::<Money>(), m(200_000));
        assert_eq!(c[&ym(2026, 1)], c[&ym(2027, 1)]);
    }

    #[test]
    fn february_equals_march() {
        // Leap year, short month: the calendar must not matter.
        let l = line(Period::Yearly, 100_000, true);
        let leap = BudgetLine {
            period_start: ym(2024, 1),
            ..l.clone()
        };
        assert_eq!(
            monthly_contribution(&leap, ym(2024, 2)),
            monthly_contribution(&leap, ym(2024, 3))
        );
    }

    #[test]
    fn monthly_lines_contribute_every_month() {
        let l = line(Period::Monthly, 25_000, false);
        let c = contributions(&l, MonthRange::new(ym(2026, 1), ym(2026, 3)));
        assert_eq!(c.values().copied().collect::<Vec<_>>(), vec![m(25_000); 3]);
    }

    #[test]
    fn every_n_months_apportions_over_the_cycle() {
        let l = line(Period::EveryNMonths(3), 10_000, true);
        let q = contributions(&l, MonthRange::new(ym(2026, 1), ym(2026, 3)));
        assert_eq!(q.values().copied().sum::<Money>(), m(10_000));
        // Next cycle repeats.
        let q2 = contributions(&l, MonthRange::new(ym(2026, 4), ym(2026, 6)));
        assert_eq!(q2.values().copied().sum::<Money>(), m(10_000));
        // A zero-month cycle is inert rather than a panic.
        let inert = line(Period::EveryNMonths(0), 10_000, true);
        assert_eq!(monthly_contribution(&inert, ym(2026, 1)), Money::ZERO);
    }

    #[test]
    fn by_date_saves_up_through_the_due_month_then_stops() {
        let l = line(
            Period::ByDate {
                due: NaiveDate::from_ymd_opt(2026, 4, 15).unwrap(),
            },
            40_000,
            true,
        );
        let c = contributions(&l, MonthRange::new(ym(2026, 1), ym(2026, 6)));
        assert_eq!(c.values().copied().sum::<Money>(), m(40_000));
        assert_eq!(c[&ym(2026, 4)], m(10_000));
        assert_eq!(c[&ym(2026, 5)], Money::ZERO);
        assert_eq!(c[&ym(2026, 6)], Money::ZERO);
    }

    #[test]
    fn contributions_are_zero_outside_the_active_window() {
        let mut l = line(Period::Monthly, 5_000, false);
        l.period_end = Some(ym(2026, 2));
        let c = contributions(&l, MonthRange::new(ym(2025, 12), ym(2026, 4)));
        assert_eq!(c[&ym(2025, 12)], Money::ZERO);
        assert_eq!(c[&ym(2026, 1)], m(5_000));
        assert_eq!(c[&ym(2026, 2)], m(5_000));
        assert_eq!(c[&ym(2026, 3)], Money::ZERO);
    }

    #[test]
    fn rollover_accumulates_and_non_rollover_does_not() {
        let spend = BTreeMap::new();
        let adj = BTreeMap::new();

        let rolling = line(Period::Monthly, 10_000, true);
        let s = funded_balance(&rolling, &spend, &adj, ym(2026, 3));
        assert_eq!(s.funded_balance, m(30_000));
        assert_eq!(s.contributed_to_date, m(30_000));

        let spending = line(Period::Monthly, 10_000, false);
        let s = funded_balance(&spending, &spend, &adj, ym(2026, 3));
        assert_eq!(s.funded_balance, m(10_000));
        assert_eq!(s.contributed_to_date, m(30_000));
    }

    #[test]
    fn spending_and_adjustments_apply_in_their_month() {
        let l = line(Period::Monthly, 10_000, true);
        let spend = BTreeMap::from([(ym(2026, 2), m(4_000))]);
        let adj = BTreeMap::from([(ym(2026, 3), m(500))]);
        let s = funded_balance(&l, &spend, &adj, ym(2026, 3));
        assert_eq!(s.spent_to_date, m(4_000));
        assert_eq!(s.funded_balance, m(30_000 - 4_000 + 500));
    }

    #[test]
    fn on_track_measures_the_balance_against_the_schedule_so_far() {
        let l = line(Period::Yearly, 120_000, true);
        let empty = BTreeMap::new();

        // Six months in, six months banked: exactly on schedule.
        let s = funded_balance(&l, &empty, &empty, ym(2026, 6));
        assert_eq!(s.funded_balance, m(60_000));
        assert_eq!(s.on_track, OnTrack::OnTrack);
        assert_eq!(s.months_remaining, 7);
        assert_eq!(s.target, m(120_000));

        // Anything spent out of the sinking fund is a shortfall...
        let spend = BTreeMap::from([(ym(2026, 3), m(1_000))]);
        let s = funded_balance(&l, &spend, &empty, ym(2026, 6));
        assert_eq!(s.on_track, OnTrack::Behind(m(1_000)));

        // ...and an extra contribution is a surplus.
        let adj = BTreeMap::from([(ym(2026, 3), m(5_000))]);
        let s = funded_balance(&l, &empty, &adj, ym(2026, 6));
        assert_eq!(s.on_track, OnTrack::Ahead(m(5_000)));

        // The countdown always spans the cycle.
        for month in 1..=12 {
            let s = funded_balance(&l, &empty, &empty, ym(2026, month));
            assert_eq!(month + s.months_remaining, 13);
        }
    }

    #[test]
    fn an_envelope_spent_to_plan_reads_as_behind() {
        // Documented consequence of the schedule-relative definition: a
        // monthly envelope spent exactly to plan is empty, so it is behind by
        // the whole budget. A UI must not render this as overspending.
        let l = line(Period::Monthly, 10_000, true);
        let spend = BTreeMap::from([(ym(2026, 1), m(10_000))]);
        let s = funded_balance(&l, &spend, &BTreeMap::new(), ym(2026, 1));
        assert!(s.funded_balance.is_zero());
        assert_eq!(s.on_track, OnTrack::Behind(m(10_000)));
    }

    #[test]
    fn a_non_rollover_line_resets_at_the_cycle_boundary_not_monthly() {
        // A yearly line still accumulates toward its bill across the twelve
        // months; "no rollover" resets it on the anniversary, not every month.
        // Resetting monthly would make the yearly period useless, since the
        // line could never reach its target.
        let l = line(Period::Yearly, 120_000, false);
        let empty = BTreeMap::new();
        for (month, expected) in [(1, 10_000), (6, 60_000), (12, 120_000)] {
            let s = funded_balance(&l, &empty, &empty, ym(2026, month));
            assert_eq!(s.funded_balance, m(expected), "month {month}");
        }
        // The anniversary starts the next cycle from zero.
        let s = funded_balance(&l, &empty, &empty, ym(2027, 1));
        assert_eq!(s.funded_balance, m(10_000));
    }

    #[test]
    fn a_monthly_non_rollover_envelope_still_resets_every_month() {
        // A monthly line's cycle *is* one month, so the familiar use-it-or-
        // lose-it envelope falls out of the same rule.
        let l = line(Period::Monthly, 40_000, false);
        let spend = BTreeMap::from([(ym(2026, 1), m(40_000))]);
        let s = funded_balance(&l, &spend, &BTreeMap::new(), ym(2026, 2));
        assert_eq!(s.funded_balance, m(40_000));
    }

    #[test]
    fn spending_status_is_reported_separately_from_funding_status() {
        // The case that drove splitting these apart: a grocery envelope spent
        // exactly to plan is `Behind` on *funding* while being exactly on
        // budget on *spending*. Only the latter may be shown as "over budget".
        let l = line(Period::Monthly, 40_000, false);
        let spend = BTreeMap::from([(ym(2026, 1), m(40_000))]);
        let s = funded_balance(&l, &spend, &BTreeMap::new(), ym(2026, 1));

        assert_eq!(s.on_track, OnTrack::Behind(m(40_000)));
        assert_eq!(s.spend.budgeted, m(40_000));
        assert_eq!(s.spend.spent, m(40_000));
        assert_eq!(s.spend.remaining, Money::ZERO);
        assert_eq!(
            s.spend.overspent_by,
            Money::ZERO,
            "spending to plan is not overspending"
        );
    }

    #[test]
    fn overspending_is_reported_only_by_spend_status() {
        let l = line(Period::Monthly, 40_000, false);
        let spend = BTreeMap::from([(ym(2026, 1), m(45_000))]);
        let s = funded_balance(&l, &spend, &BTreeMap::new(), ym(2026, 1));

        assert_eq!(s.spend.spent, m(45_000));
        assert_eq!(s.spend.remaining, m(-5_000));
        assert_eq!(s.spend.overspent_by, m(5_000));
    }

    #[test]
    fn spend_status_counts_the_current_cycle_only() {
        // A yearly line's spending resets with the cycle, so last year's bill
        // does not count against this year's budget.
        let l = line(Period::Yearly, 120_000, true);
        let spend = BTreeMap::from([(ym(2026, 3), m(120_000)), (ym(2027, 3), m(30_000))]);
        let s = funded_balance(&l, &spend, &BTreeMap::new(), ym(2027, 6));
        assert_eq!(s.spend.spent, m(30_000));
        assert_eq!(s.spend.overspent_by, Money::ZERO);
    }

    #[test]
    fn as_of_before_the_start_is_all_zeroes() {
        let l = line(Period::Monthly, 10_000, true);
        let s = funded_balance(&l, &BTreeMap::new(), &BTreeMap::new(), ym(2025, 12));
        assert_eq!(s.funded_balance, Money::ZERO);
        assert_eq!(s.contributed_to_date, Money::ZERO);
        assert_eq!(s.on_track, OnTrack::OnTrack);
    }

    #[test]
    fn cycles_partition_the_months_they_cover() {
        let yearly = line(Period::Yearly, 120_000, true);
        let cycle = cycle_containing(&yearly, ym(2026, 3)).expect("active");
        assert_eq!(cycle, MonthRange::new(ym(2026, 1), ym(2026, 12)));
        // Every month of a cycle reports the same cycle — the property the
        // amortized report relies on to assign each month to exactly one.
        for month in 1..=12 {
            assert_eq!(cycle_containing(&yearly, ym(2026, month)), Some(cycle));
        }
        assert_eq!(
            cycle_containing(&yearly, ym(2027, 1)),
            Some(MonthRange::new(ym(2027, 1), ym(2027, 12)))
        );

        // A monthly line's cycle is the month itself, which is why a monthly
        // budget amortizes to exactly its own cash flow.
        let monthly = line(Period::Monthly, 40_000, false);
        assert_eq!(
            cycle_containing(&monthly, ym(2026, 5)),
            Some(MonthRange::single(ym(2026, 5)))
        );

        // Inactive months have no cycle at all.
        assert_eq!(cycle_containing(&yearly, ym(2025, 12)), None);
        let inert = line(Period::EveryNMonths(0), 10_000, true);
        assert_eq!(cycle_containing(&inert, ym(2026, 1)), None);
    }

    #[test]
    fn a_cycle_never_runs_past_the_lines_last_month() {
        // Unclamped, June–December would belong both to January's twelve-month
        // cycle and to their own one-month ones, and amortized spending would
        // be counted twice.
        let mut l = line(Period::Yearly, 120_000, true);
        l.period_end = Some(ym(2026, 6));
        assert_eq!(
            cycle_containing(&l, ym(2026, 3)),
            Some(MonthRange::new(ym(2026, 1), ym(2026, 6)))
        );
        assert_eq!(cycle_containing(&l, ym(2026, 7)), None);

        // A ByDate line's cycle ends in the due month, and nothing follows it.
        let by_date = line(
            Period::ByDate {
                due: NaiveDate::from_ymd_opt(2026, 4, 15).unwrap(),
            },
            40_000,
            true,
        );
        assert_eq!(
            cycle_containing(&by_date, ym(2026, 2)),
            Some(MonthRange::new(ym(2026, 1), ym(2026, 4)))
        );
        assert_eq!(cycle_containing(&by_date, ym(2026, 5)), None);
    }

    #[test]
    fn odd_amounts_never_leak_a_cent() {
        for amount in [1_i64, 7, 99, 100_001, -100_001] {
            let l = line(Period::Yearly, amount, true);
            let c = contributions(&l, MonthRange::new(ym(2026, 1), ym(2026, 12)));
            assert_eq!(c.values().copied().sum::<Money>(), m(amount), "{amount}");
        }
    }
}
