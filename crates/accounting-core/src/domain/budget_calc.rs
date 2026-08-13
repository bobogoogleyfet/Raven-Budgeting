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
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum OnTrack {
    /// Funded ahead of schedule by this much.
    Ahead(Money),
    /// Exactly on schedule.
    OnTrack,
    /// Short of schedule by this much.
    Behind(Money),
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
    pub on_track: OnTrack,
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
/// `carried[m]` is `balance[m-1]` when `rollover` is set and [`Money::ZERO`]
/// otherwise — the reset is **monthly**, not per period. A non-rollover
/// grocery envelope therefore resets every month (February's overspend is
/// February's problem), and a `Yearly` line with `rollover == false` never
/// accumulates toward its bill at all: annual lines want `rollover == true`.
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

        // `rollover == false` zeroes the carry *every month*, not at the
        // cycle boundary: an envelope that does not roll over is spent or
        // lost within its month. Note the consequence — a `Yearly` line with
        // `rollover == false` never accumulates toward its bill and is a
        // degenerate configuration; annual lines want `rollover == true`.
        let carried = if line.rollover { balance } else { Money::ZERO };
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
    fn a_non_rollover_line_resets_every_month() {
        // Even on a yearly period: without rollover nothing accumulates.
        let l = line(Period::Yearly, 120_000, false);
        let empty = BTreeMap::new();
        for month in [1, 6, 12] {
            let s = funded_balance(&l, &empty, &empty, ym(2026, month));
            assert_eq!(s.funded_balance, m(10_000), "{month}");
        }
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
    fn odd_amounts_never_leak_a_cent() {
        for amount in [1_i64, 7, 99, 100_001, -100_001] {
            let l = line(Period::Yearly, amount, true);
            let c = contributions(&l, MonthRange::new(ym(2026, 1), ym(2026, 12)));
            assert_eq!(c.values().copied().sum::<Money>(), m(amount), "{amount}");
        }
    }
}
