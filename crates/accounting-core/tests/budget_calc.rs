//! Tests for budget contribution schedules and envelope balances.
//!
//! The premise of the whole budget feature: an annually-paid bill must not
//! wreck the month it lands in, so every line is turned into a per-month
//! contribution that sums back to the exact amount budgeted.

use std::collections::BTreeMap;

use accounting_core::domain::budget_calc::{
    BudgetLine, OnTrack, Period, contributions, funded_balance, monthly_contribution,
};
use accounting_core::ids::{BudgetLineId, CategoryId};
use accounting_core::money::Money;
use accounting_core::time::{MonthRange, YearMonth};
use chrono::NaiveDate;
use proptest::prelude::*;

const GROCERIES: CategoryId = CategoryId::new(1);
const INSURANCE: CategoryId = CategoryId::new(2);

fn ym(year: i32, month: u32) -> YearMonth {
    YearMonth::new(year, month).expect("valid year-month")
}

fn range(start: (i32, u32), end: (i32, u32)) -> MonthRange {
    MonthRange::new(ym(start.0, start.1), ym(end.0, end.1))
}

fn line(period: Period, start: (i32, u32), amount_cents: i64, rollover: bool) -> BudgetLine {
    BudgetLine {
        id: BudgetLineId::new(1),
        category_id: INSURANCE,
        period,
        period_start: ym(start.0, start.1),
        period_end: None,
        amount: Money::from_cents(amount_cents),
        rollover,
    }
}

fn total(map: &BTreeMap<YearMonth, Money>) -> i64 {
    map.values().map(|m| m.cents()).sum()
}

fn month_cents(map: &BTreeMap<YearMonth, Money>, y: i32, m: u32) -> i64 {
    map.get(&ym(y, m)).copied().unwrap_or(Money::ZERO).cents()
}

fn spend(entries: &[((i32, u32), i64)]) -> BTreeMap<YearMonth, Money> {
    entries
        .iter()
        .map(|((y, m), c)| (ym(*y, *m), Money::from_cents(*c)))
        .collect()
}

// ---------------------------------------------------------------------------
// Contribution schedules
// ---------------------------------------------------------------------------

#[test]
fn a_yearly_line_contributes_exactly_its_amount_over_twelve_months() {
    // $1000/yr does not divide evenly by 12; the schedule must still sum to
    // exactly $1000 rather than $999.96.
    let l = line(Period::Yearly, (2026, 1), 100_000, true);
    let c = contributions(&l, range((2026, 1), (2026, 12)));
    assert_eq!(c.len(), 12);
    assert_eq!(total(&c), 100_000);
    for (m, v) in &c {
        assert!(
            v.cents() == 8_333 || v.cents() == 8_334,
            "{m} contributed {v:?}"
        );
    }
}

#[test]
fn a_twelve_hundred_dollar_yearly_line_is_exactly_one_hundred_a_month() {
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let c = contributions(&l, range((2026, 1), (2026, 12)));
    assert_eq!(c.len(), 12);
    for (m, v) in &c {
        assert_eq!(v.cents(), 10_000, "{m}");
    }
}

#[test]
fn a_monthly_line_contributes_its_full_amount_every_month() {
    let l = line(Period::Monthly, (2026, 1), 60_000, false);
    let c = contributions(&l, range((2026, 1), (2026, 6)));
    assert_eq!(c.len(), 6);
    for (m, v) in &c {
        assert_eq!(v.cents(), 60_000, "{m}");
    }
    assert_eq!(monthly_contribution(&l, ym(2026, 4)).cents(), 60_000);
}

#[test]
fn february_contributes_the_same_as_a_thirty_one_day_month() {
    // Budget math is per-month, never per-day: a short month must not shrink a
    // contribution and a leap day must not grow one.
    let l = line(Period::Yearly, (2028, 1), 120_000, true);
    let c = contributions(&l, range((2028, 1), (2028, 12)));
    assert_eq!(month_cents(&c, 2028, 2), month_cents(&c, 2028, 3));
    assert_eq!(month_cents(&c, 2028, 2), month_cents(&c, 2028, 1));
    assert_eq!(
        monthly_contribution(&l, ym(2028, 2)),
        monthly_contribution(&l, ym(2028, 3))
    );
}

#[test]
fn a_leap_year_schedule_matches_a_common_year_schedule() {
    let leap = contributions(
        &line(Period::Yearly, (2028, 1), 100_000, true),
        range((2028, 1), (2028, 12)),
    );
    let common = contributions(
        &line(Period::Yearly, (2027, 1), 100_000, true),
        range((2027, 1), (2027, 12)),
    );
    let leap_values: Vec<i64> = leap.values().map(|m| m.cents()).collect();
    let common_values: Vec<i64> = common.values().map(|m| m.cents()).collect();
    assert_eq!(leap_values, common_values);
}

#[test]
fn every_n_months_spreads_the_amount_across_its_own_period() {
    // A quarterly $300 bill is $100/month of budget, not a $300 spike.
    let l = line(Period::EveryNMonths(3), (2026, 1), 30_000, true);
    let c = contributions(&l, range((2026, 1), (2026, 3)));
    assert_eq!(c.len(), 3);
    assert_eq!(total(&c), 30_000);
    for (m, v) in &c {
        assert_eq!(v.cents(), 10_000, "{m}");
    }
}

#[test]
fn every_n_months_repeats_for_the_next_period() {
    let l = line(Period::EveryNMonths(3), (2026, 1), 30_000, true);
    let c = contributions(&l, range((2026, 1), (2026, 12)));
    assert_eq!(total(&c), 120_000, "four quarters of $300");
}

#[test]
fn a_by_date_line_funds_itself_by_the_month_it_is_due() {
    // $600 due 2026-06-15, starting 2026-01: six months of $100.
    let due = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let l = line(Period::ByDate { due }, (2026, 1), 60_000, true);
    let c = contributions(&l, range((2026, 1), (2026, 6)));
    assert_eq!(total(&c), 60_000);
    assert_eq!(c.len(), 6);
    for (m, v) in &c {
        assert_eq!(v.cents(), 10_000, "{m}");
    }
}

#[test]
fn a_by_date_line_contributes_nothing_after_its_due_month() {
    let due = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let l = line(Period::ByDate { due }, (2026, 1), 60_000, true);
    let c = contributions(&l, range((2026, 7), (2026, 12)));
    assert_eq!(total(&c), 0, "a one-off goal must not keep charging");
}

#[test]
fn a_by_date_line_due_outside_the_horizon_contributes_only_its_window_share() {
    // Asking for the first three months of a twelve-month goal yields three
    // months of contributions, not the whole goal.
    let due = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
    let l = line(Period::ByDate { due }, (2026, 1), 120_000, true);
    let window = contributions(&l, range((2026, 1), (2026, 3)));
    assert_eq!(window.len(), 3);
    assert_eq!(total(&window), 30_000);
    let full = contributions(&l, range((2026, 1), (2026, 12)));
    assert_eq!(total(&full), 120_000);
}

#[test]
fn months_before_the_line_starts_contribute_nothing() {
    let l = line(Period::Monthly, (2026, 6), 10_000, false);
    let c = contributions(&l, range((2026, 1), (2026, 12)));
    assert_eq!(month_cents(&c, 2026, 5), 0);
    assert_eq!(month_cents(&c, 2026, 6), 10_000);
    assert_eq!(total(&c), 70_000, "June through December");
}

#[test]
fn months_after_period_end_contribute_nothing() {
    let mut l = line(Period::Monthly, (2026, 1), 10_000, false);
    l.period_end = Some(ym(2026, 3));
    let c = contributions(&l, range((2026, 1), (2026, 12)));
    assert_eq!(total(&c), 30_000);
    assert_eq!(month_cents(&c, 2026, 4), 0);
}

#[test]
fn monthly_contribution_agrees_with_the_schedule_month_by_month() {
    // The two entry points must never disagree, or a UI that shows one and
    // totals the other will not add up.
    let l = line(Period::Yearly, (2026, 1), 100_000, true);
    let c = contributions(&l, range((2026, 1), (2027, 12)));
    for m in range((2026, 1), (2027, 12)).iter() {
        assert_eq!(
            monthly_contribution(&l, m).cents(),
            c.get(&m).copied().unwrap_or(Money::ZERO).cents(),
            "{m}"
        );
    }
}

proptest! {
    /// The core budgeting promise: a full period of contributions is exactly
    /// the budgeted amount, for any amount and any starting month.
    #[test]
    fn a_full_period_of_contributions_is_exactly_the_budgeted_amount(
        amount in -10_000_000i64..=10_000_000,
        year in 2000i32..=2100,
        month in 1u32..=12,
        months in prop::sample::select(vec![1u32, 2, 3, 4, 6, 12]),
    ) {
        let period = match months {
            1 => Period::Monthly,
            12 => Period::Yearly,
            n => Period::EveryNMonths(n),
        };
        let l = line(period, (year, month), amount, true);
        let start = ym(year, month);
        let end = start.add_months(i32::try_from(months).unwrap() - 1);
        let c = contributions(&l, MonthRange::new(start, end));
        prop_assert_eq!(
            total(&c),
            amount,
            "{} months from {} did not sum to {}", months, start, amount
        );
    }
}

// ---------------------------------------------------------------------------
// Funded balance
// ---------------------------------------------------------------------------

#[test]
fn a_rollover_envelope_accumulates_unspent_contributions() {
    // $100/month with no spending is $500 saved after five months — this is
    // what makes an annual bill payable.
    let l = line(Period::Monthly, (2026, 1), 10_000, true);
    let empty = BTreeMap::new();
    let status = funded_balance(&l, &empty, &empty, ym(2026, 5));
    assert_eq!(status.contributed_to_date, Money::from_cents(50_000));
    assert_eq!(status.spent_to_date, Money::ZERO);
    assert_eq!(status.funded_balance, Money::from_cents(50_000));
    assert_eq!(status.category_id, INSURANCE);
    assert_eq!(status.monthly_contribution, Money::from_cents(10_000));
}

#[test]
fn a_rollover_balance_series_tracks_spend_and_adjustments_for_two_years() {
    let l = line(Period::Monthly, (2026, 1), 10_000, true);
    let spends = spend(&[((2026, 1), 8_000), ((2026, 2), 15_000), ((2026, 6), 5_000)]);
    let adjustments = spend(&[((2026, 3), -2_000)]);

    // Month-by-month: previous balance + contribution - spend + adjustment.
    let expected = [
        ((2026, 1), 2_000),
        ((2026, 2), -3_000),
        ((2026, 3), 5_000),
        ((2026, 4), 15_000),
        ((2026, 5), 25_000),
        ((2026, 6), 30_000),
        ((2026, 12), 90_000),
        ((2027, 12), 210_000),
    ];
    for ((y, m), want) in expected {
        let status = funded_balance(&l, &spends, &adjustments, ym(y, m));
        assert_eq!(
            status.funded_balance.cents(),
            want,
            "balance at {y}-{m:02} (spent {:?})",
            status.spent_to_date
        );
    }

    let end = funded_balance(&l, &spends, &adjustments, ym(2027, 12));
    assert_eq!(end.spent_to_date, Money::from_cents(28_000));
}

#[test]
fn a_non_rollover_envelope_resets_every_month() {
    // Groceries: overspending in February is February's problem and must not
    // follow you into March.
    let l = BudgetLine {
        category_id: GROCERIES,
        ..line(Period::Monthly, (2026, 1), 10_000, false)
    };
    let spends = spend(&[((2026, 1), 8_000), ((2026, 2), 15_000)]);
    let empty = BTreeMap::new();

    let jan = funded_balance(&l, &spends, &empty, ym(2026, 1));
    assert_eq!(jan.funded_balance, Money::from_cents(2_000));

    let feb = funded_balance(&l, &spends, &empty, ym(2026, 2));
    assert_eq!(
        feb.funded_balance,
        Money::from_cents(-5_000),
        "January's $20 surplus must not soften February"
    );

    let mar = funded_balance(&l, &spends, &empty, ym(2026, 3));
    assert_eq!(
        mar.funded_balance,
        Money::from_cents(10_000),
        "March starts fresh"
    );
}

#[test]
fn a_rollover_yearly_line_carries_its_balance_into_the_next_period() {
    // Yearly $1200 starting 2026-03: the period runs 2026-03..2027-02, so
    // 2027-03 is month one of a brand new period.
    let l = line(Period::Yearly, (2026, 3), 120_000, true);
    let empty = BTreeMap::new();

    let end_of_period = funded_balance(&l, &empty, &empty, ym(2027, 2));
    assert_eq!(end_of_period.funded_balance, Money::from_cents(120_000));

    let new_period = funded_balance(&l, &empty, &empty, ym(2027, 3));
    assert_eq!(
        new_period.funded_balance,
        Money::from_cents(130_000),
        "an unspent envelope carries across the period boundary"
    );

    // ...and the new period's schedule restarts from scratch.
    let next = contributions(&l, range((2027, 3), (2028, 2)));
    assert_eq!(total(&next), 120_000);
}

#[test]
fn a_non_rollover_line_resets_every_month_even_on_a_yearly_period() {
    // Documented behaviour: `rollover = false` zeroes the carry *every month*,
    // not at the cycle boundary, so a yearly line without rollover never
    // accumulates toward its bill — it is a degenerate configuration by
    // construction. Pinned here because the M1 brief describes the reset as
    // happening at the period boundary; see the report.
    let l = line(Period::Yearly, (2026, 3), 120_000, false);
    let empty = BTreeMap::new();

    let mid_period = funded_balance(&l, &empty, &empty, ym(2026, 9));
    assert_eq!(mid_period.funded_balance, Money::from_cents(10_000));

    let end_of_period = funded_balance(&l, &empty, &empty, ym(2027, 2));
    assert_eq!(end_of_period.funded_balance, Money::from_cents(10_000));

    let new_period = funded_balance(&l, &empty, &empty, ym(2027, 3));
    assert_eq!(new_period.funded_balance, Money::from_cents(10_000));

    // Contributions themselves still restart cleanly for the new cycle.
    assert_eq!(
        total(&contributions(&l, range((2027, 3), (2028, 2)))),
        120_000
    );
}

#[test]
fn a_rollover_line_keeps_a_deficit_across_the_period_boundary() {
    // The mirror image of carrying a surplus: an overspent envelope must not be
    // forgiven by the calendar rolling over.
    let l = line(Period::Yearly, (2026, 3), 120_000, true);
    let spends = spend(&[((2026, 4), 500_000)]);
    let empty = BTreeMap::new();

    let before = funded_balance(&l, &spends, &empty, ym(2027, 2));
    assert_eq!(before.funded_balance, Money::from_cents(120_000 - 500_000));

    let after = funded_balance(&l, &spends, &empty, ym(2027, 3));
    assert_eq!(
        after.funded_balance,
        Money::from_cents(120_000 - 500_000 + 10_000)
    );
}

#[test]
fn the_target_is_the_periods_budgeted_amount() {
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let empty = BTreeMap::new();
    let status = funded_balance(&l, &empty, &empty, ym(2026, 5));
    assert_eq!(status.target, Money::from_cents(120_000));
}

#[test]
fn months_remaining_counts_the_current_month_as_documented() {
    // FundedStatus::months_remaining is documented as "counting `as_of`
    // itself: 12 in the first month of a yearly line, 1 in its last". The
    // implementation computes `len - idx - 1`, which is the exclusive count.
    // The doc and the code disagree; this asserts the doc.
    let l = line(Period::Yearly, (2026, 3), 120_000, true);
    let empty = BTreeMap::new();
    let first = funded_balance(&l, &empty, &empty, ym(2026, 3));
    let last = funded_balance(&l, &empty, &empty, ym(2027, 2));
    assert_eq!(first.months_remaining, 12, "the whole period is ahead");
    assert_eq!(last.months_remaining, 1, "only the current month is left");
}

#[test]
fn months_remaining_falls_by_exactly_one_per_month() {
    let l = line(Period::Yearly, (2026, 3), 120_000, true);
    let empty = BTreeMap::new();
    let mut prev: Option<u32> = None;
    for m in range((2026, 3), (2027, 2)).iter() {
        let status = funded_balance(&l, &empty, &empty, m);
        if let Some(p) = prev {
            assert_eq!(
                status.months_remaining + 1,
                p,
                "months_remaining must fall by exactly one per month at {m}"
            );
        }
        prev = Some(status.months_remaining);
    }
    // Whatever the off-by-one convention, the countdown spans the cycle.
    let first = funded_balance(&l, &empty, &empty, ym(2026, 3));
    let last = funded_balance(&l, &empty, &empty, ym(2027, 2));
    assert_eq!(first.months_remaining - last.months_remaining, 11);
}

// ---------------------------------------------------------------------------
// on_track
// ---------------------------------------------------------------------------

// `on_track` compares the envelope balance against the *schedule so far* — the
// contributions the line should have banked by this month — not against zero.

#[test]
fn an_unspent_envelope_is_exactly_on_schedule() {
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let empty = BTreeMap::new();
    let status = funded_balance(&l, &empty, &empty, ym(2026, 6));
    assert_eq!(status.funded_balance, Money::from_cents(60_000));
    assert_eq!(status.on_track, OnTrack::OnTrack);
}

#[test]
fn spending_out_of_a_sinking_fund_puts_it_behind_by_the_amount_spent() {
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let spends = spend(&[((2026, 3), 1_000)]);
    let empty = BTreeMap::new();
    let status = funded_balance(&l, &spends, &empty, ym(2026, 6));
    assert_eq!(status.on_track, OnTrack::Behind(Money::from_cents(1_000)));
}

#[test]
fn an_extra_contribution_puts_a_sinking_fund_ahead_by_the_surplus() {
    // Adjustments are the "I threw an extra $50 at it" lever.
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let adjustments = spend(&[((2026, 2), 5_000)]);
    let empty = BTreeMap::new();
    let status = funded_balance(&l, &empty, &adjustments, ym(2026, 6));
    assert_eq!(status.on_track, OnTrack::Ahead(Money::from_cents(5_000)));
}

#[test]
fn the_ahead_behind_boundary_is_one_cent_wide() {
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let empty = BTreeMap::new();
    for (adjustment, want) in [
        (-1i64, OnTrack::Behind(Money::from_cents(1))),
        (0, OnTrack::OnTrack),
        (1, OnTrack::Ahead(Money::from_cents(1))),
    ] {
        let adjustments = spend(&[((2026, 2), adjustment)]);
        let status = funded_balance(&l, &empty, &adjustments, ym(2026, 6));
        assert_eq!(status.on_track, want, "adjustment of {adjustment}c");
    }
}

#[test]
fn on_track_always_measures_the_gap_between_the_balance_and_the_schedule() {
    // Within a line's first cycle the schedule so far is exactly
    // `contributed_to_date`, so the invariant can be checked without
    // re-deriving the schedule.
    let l = line(Period::Yearly, (2026, 1), 120_000, true);
    let empty = BTreeMap::new();
    for spent in [0i64, 5_000, 9_999, 10_000, 10_001, 50_000, -2_500] {
        let spends = spend(&[((2026, 2), spent)]);
        let status = funded_balance(&l, &spends, &empty, ym(2026, 6));
        let gap = status.funded_balance - status.contributed_to_date;
        match status.on_track {
            OnTrack::Ahead(by) => assert_eq!(by, gap, "spent {spent}c"),
            OnTrack::OnTrack => assert!(gap.is_zero(), "spent {spent}c: gap {gap:?}"),
            OnTrack::Behind(by) => assert_eq!(by, -gap, "spent {spent}c"),
        }
    }
}

#[test]
fn a_fully_spent_monthly_envelope_reports_behind_by_the_whole_budget() {
    // Consequence of the schedule-relative definition worth having in writing:
    // Groceries spent exactly to plan reports `Behind($100)`, because the
    // envelope is empty rather than funded. A UI must not render this as
    // overspending. See the report.
    let l = BudgetLine {
        category_id: GROCERIES,
        ..line(Period::Monthly, (2026, 1), 10_000, false)
    };
    let spends = spend(&[((2026, 1), 10_000)]);
    let empty = BTreeMap::new();
    let status = funded_balance(&l, &spends, &empty, ym(2026, 1));
    assert!(status.funded_balance.is_zero());
    assert_eq!(status.on_track, OnTrack::Behind(Money::from_cents(10_000)));
}
