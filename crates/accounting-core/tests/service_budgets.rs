//! The budget service: stored lines and stored spending, met.
//!
//! Everything below this layer is either pure arithmetic (`budget_calc`) or a
//! single table (`repo::budgets`). The service is where they meet, and three
//! things can only go wrong here:
//!
//! * **Spending has to be attributed to the month it was dated in**, from the
//!   transaction rather than from the split, and archived receipts must not
//!   count. A budget drawn against money that is not really spent is worse than
//!   no budget.
//! * **Every budgeted category has to appear**, including the ones with no
//!   budget line at all. That absence is what lets a UI offer "set a budget"
//!   for them; a status list built by joining *from* `budget_line` silently
//!   hides every category the user has not budgeted yet — which is exactly the
//!   set they most need to see.
//! * **Funding and overspending must stay separate.** `OnTrack` answers "will
//!   the money be there when the bill lands?"; `SpendStatus` answers "have I
//!   spent more than I budgeted?". They disagree in normal use, and conflating
//!   them makes every non-rollover monthly envelope read as over budget.

mod common;

use accounting_core::domain::budget_calc::{BudgetLine, OnTrack, Period, monthly_contribution};
use accounting_core::repo::budgets::{self, BudgetLineRecord};
use accounting_core::repo::{categories, transactions};
use accounting_core::service::budgets as service;
use accounting_core::service::budgets::CategoryBudgetStatus;
use accounting_core::{CategoryId, Db, Money, YearMonth};
use common::{
    amounts, assert_clean, cat, day, m, months, save, seeded_db, spend_on, split, txn, ym,
};

// ---------------------------------------------------------------------------
// Builders
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
    service::upsert_budget_line(db, rec).unwrap_or_else(|e| panic!("upsert_budget_line: {e}"));
}

#[track_caller]
fn status_for(db: &Db, category_id: CategoryId, as_of: YearMonth) -> CategoryBudgetStatus {
    service::budget_status_for(db, category_id, as_of)
        .unwrap_or_else(|e| panic!("budget_status_for {category_id} at {as_of}: {e}"))
}

#[track_caller]
fn row<'a>(rows: &'a [CategoryBudgetStatus], name: &str) -> &'a CategoryBudgetStatus {
    rows.iter()
        .find(|r| r.category_name == name)
        .unwrap_or_else(|| {
            let present: Vec<&str> = rows.iter().map(|r| r.category_name.as_str()).collect();
            panic!("no status row for `{name}`; the list held {present:?}")
        })
}

#[track_caller]
fn line_of(status: &CategoryBudgetStatus) -> &BudgetLine {
    status
        .line
        .as_ref()
        .unwrap_or_else(|| panic!("`{}` must have a budget line", status.category_name))
}

// ---------------------------------------------------------------------------
// spend_by_month
// ---------------------------------------------------------------------------

#[test]
fn spend_by_month_files_each_transaction_under_the_month_it_is_dated() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let restaurants = cat(&db, "Restaurants");

    spend_on(&db, "2026-01-31", groceries, 10_000);
    spend_on(&db, "2026-02-01", groceries, 25_000);
    spend_on(&db, "2026-02-28", groceries, 5_000);
    // Another category's money, and a month outside the range asked for.
    spend_on(&db, "2026-02-14", restaurants, 8_000);
    spend_on(&db, "2026-05-04", groceries, 99_900);

    let by_month = service::spend_by_month(&db, groceries, months((2026, 1), (2026, 3)))
        .expect("spend_by_month");

    assert_eq!(by_month.get(&ym(2026, 1)).copied(), Some(m(10_000)));
    // $250.00 on the 1st plus $50.00 on the 28th.
    assert_eq!(by_month.get(&ym(2026, 2)).copied(), Some(m(30_000)));
    assert_eq!(
        by_month.get(&ym(2026, 5)).copied(),
        None,
        "May is outside the range"
    );
    assert_eq!(
        by_month.values().copied().sum::<Money>(),
        m(40_000),
        "Restaurants' $80.00 must not be counted against Groceries"
    );
    assert_clean(&db);
}

#[test]
fn spend_by_month_sums_several_splits_landing_in_the_same_month() {
    // Two receipts, one of which is split across categories: the category's
    // share of a mixed receipt counts, and the rest of it does not.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");

    save(
        &db,
        &txn(
            day("2026-04-02"),
            amounts(5_000, 0),
            vec![split(groceries, 3_000, 0), split(household, 2_000, 0)],
        ),
    );
    spend_on(&db, "2026-04-20", groceries, 4_550);

    let by_month =
        service::spend_by_month(&db, groceries, months((2026, 4), (2026, 4))).expect("spend");

    // $30.00 of the split receipt plus $45.50 on its own = $75.50.
    assert_eq!(by_month.get(&ym(2026, 4)).copied(), Some(m(7_550)));
    assert_clean(&db);
}

#[test]
fn spend_by_month_ignores_archived_transactions() {
    // An archived receipt is one the user has taken back. Counting it holds an
    // envelope down against money that was never really spent, and no report
    // shows the receipt that is doing it.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    spend_on(&db, "2026-03-05", groceries, 6_000);
    let mistake = spend_on(&db, "2026-03-06", groceries, 40_000);
    transactions::archive(&db, mistake).expect("archive");

    let by_month =
        service::spend_by_month(&db, groceries, months((2026, 3), (2026, 3))).expect("spend");
    assert_eq!(
        by_month.get(&ym(2026, 3)).copied().unwrap_or(Money::ZERO),
        m(6_000),
        "only the receipt that still stands may count"
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// budget_status
// ---------------------------------------------------------------------------

#[test]
fn budget_status_covers_budgeted_categories_that_have_no_line_yet() {
    // The list is the budget *screen*, not a list of budgets. A category with
    // no line is exactly the row a UI turns into "set a budget", so it must be
    // present, and it must be recognisable as unbudgeted rather than as a $0
    // budget.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let travel = cat(&db, "Travel");
    set_line(&db, &record(travel, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-08-03", groceries, 7_500);

    let rows = service::budget_status(&db, ym(2026, 8)).expect("budget_status");

    let unbudgeted = row(&rows, "Groceries");
    assert!(
        unbudgeted.line.is_none(),
        "Groceries has no budget line; got {:?}",
        unbudgeted.line
    );
    assert!(
        unbudgeted.status.is_none(),
        "with no line there is no funded balance to report"
    );
    assert_eq!(
        unbudgeted.spent_this_month,
        m(7_500),
        "spending is still reported for an unbudgeted category"
    );
    assert_eq!(unbudgeted.group_name, "Frequent");

    let budgeted = row(&rows, "Travel");
    assert!(budgeted.line.is_some());
    assert!(budgeted.status.is_some());

    // The system categories are `is_budgeted = 0`: budgeting "Transfer" is
    // meaningless and budgeting "Uncategorized" rewards leaving things there.
    for hidden in ["Uncategorized", "Transfer", "Reimbursable"] {
        assert!(
            !rows.iter().any(|r| r.category_name == hidden),
            "`{hidden}` is not a budgeted category and must not be on the screen"
        );
    }
    assert_clean(&db);
}

#[test]
fn budget_status_drops_a_category_once_it_is_archived() {
    let db = seeded_db();
    let hobbies = cat(&db, "Hobbies");
    assert!(
        service::budget_status(&db, ym(2026, 8))
            .expect("budget_status")
            .iter()
            .any(|r| r.category_name == "Hobbies")
    );

    categories::archive(&db, hobbies).expect("archive");
    assert!(
        !service::budget_status(&db, ym(2026, 8))
            .expect("budget_status")
            .iter()
            .any(|r| r.category_name == "Hobbies"),
        "a retired category must leave the budget screen"
    );
}

#[test]
fn funding_and_overspending_are_reported_separately_and_disagree() {
    // The bug this guards: a $400/month grocery envelope with exactly $400
    // spent is `Behind($400)` on *funding* — the schedule says $400 should be
    // banked and the envelope is empty — while being exactly on budget on
    // *spending*. Only `spend.overspent_by` may ever be rendered as "over
    // budget"; rendering `OnTrack::Behind` that way marks every healthy monthly
    // envelope red.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let mut line = record(groceries, Period::Monthly, (2026, 8), 40_000);
    line.rollover = false;
    set_line(&db, &line);
    spend_on(&db, "2026-08-12", groceries, 40_000);

    let status = status_for(&db, groceries, ym(2026, 8));
    let funded = status.status.as_ref().expect("a line means a status");

    assert_eq!(funded.on_track, OnTrack::Behind(m(40_000)));
    assert_eq!(funded.funded_balance, Money::ZERO);
    assert_eq!(funded.spend.budgeted, m(40_000));
    assert_eq!(funded.spend.spent, m(40_000));
    assert_eq!(funded.spend.remaining, Money::ZERO);
    assert_eq!(
        funded.spend.overspent_by,
        Money::ZERO,
        "spending exactly to plan is not overspending, however `on_track` reads"
    );
    assert_eq!(status.spent_this_month, m(40_000));
    assert_clean(&db);
}

#[test]
fn overspending_shows_up_only_in_the_spend_status() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let mut line = record(groceries, Period::Monthly, (2026, 8), 40_000);
    line.rollover = false;
    set_line(&db, &line);
    spend_on(&db, "2026-08-12", groceries, 52_500);

    let status = status_for(&db, groceries, ym(2026, 8));
    let funded = status.status.as_ref().expect("status");
    // $400.00 budgeted, $525.00 spent: $125.00 over.
    assert_eq!(funded.spend.remaining, m(-12_500));
    assert_eq!(funded.spend.overspent_by, m(12_500));
    assert_clean(&db);
}

#[test]
fn a_twelve_hundred_a_year_line_is_a_hundred_a_month_and_eight_hundred_funded_by_august() {
    // The milestone's acceptance criterion, end to end through storage:
    // $1200/yr from 2026-01 with nothing spent reads, in August, as $100.00
    // this month, 8 x $100.00 = $800.00 contributed, and exactly on schedule.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));

    let status = status_for(&db, insurance, ym(2026, 8));
    let funded = status.status.as_ref().expect("status");

    assert_eq!(funded.monthly_contribution, m(10_000));
    assert_eq!(funded.contributed_to_date, m(80_000));
    assert_eq!(funded.spent_to_date, Money::ZERO);
    assert_eq!(funded.funded_balance, m(80_000));
    assert_eq!(funded.target, m(120_000));
    assert_eq!(funded.on_track, OnTrack::OnTrack);
    assert_eq!(status.spent_this_month, Money::ZERO);

    // …and the same row read through the whole-screen call agrees.
    let rows = service::budget_status(&db, ym(2026, 8)).expect("budget_status");
    let listed = row(&rows, "Car Insurance");
    assert_eq!(
        listed.status.as_ref().map(|s| s.funded_balance),
        Some(m(80_000))
    );
    assert_eq!(listed.line.as_ref().map(|l| l.amount), Some(m(120_000)));
    assert_clean(&db);
}

#[test]
fn an_adjustment_moves_the_funded_balance_by_exactly_its_amount() {
    // The "I threw an extra $50 at it" lever, and the "write this overspend
    // off so it stops propagating through every later rollover" lever, are the
    // same lever.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));

    let before = status_for(&db, insurance, ym(2026, 8));
    let before_balance = before.status.as_ref().expect("status").funded_balance;

    budgets::add_adjustment(&db, insurance, ym(2026, 5), m(5_000), Some("tax refund"))
        .expect("add_adjustment");

    let after = status_for(&db, insurance, ym(2026, 8));
    let after_status = after.status.as_ref().expect("status");
    assert_eq!(
        after_status.funded_balance - before_balance,
        m(5_000),
        "an adjustment moves the balance by its own amount and nothing else"
    );
    assert_eq!(
        after_status.contributed_to_date,
        before.status.as_ref().expect("status").contributed_to_date,
        "an adjustment is not a contribution; the schedule is unchanged"
    );
    assert_eq!(after_status.on_track, OnTrack::Ahead(m(5_000)));
    assert_clean(&db);
}

#[test]
fn spending_shows_up_against_the_line_it_belongs_to() {
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    spend_on(&db, "2026-03-14", insurance, 120_000);

    let status = status_for(&db, insurance, ym(2026, 8));
    let funded = status.status.as_ref().expect("status");
    // Eight contributions of $100.00 with the whole $1200.00 bill already paid
    // leaves the fund $400.00 short of its schedule.
    assert_eq!(funded.spent_to_date, m(120_000));
    assert_eq!(funded.funded_balance, m(80_000 - 120_000));
    assert_eq!(funded.on_track, OnTrack::Behind(m(120_000)));
    assert_eq!(
        status.spent_this_month,
        Money::ZERO,
        "the bill was paid in March, not in August"
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Rollover
// ---------------------------------------------------------------------------

#[test]
fn a_non_rollover_monthly_envelope_starts_each_month_from_zero() {
    // Groceries: overspending in one month is that month's problem and must not
    // follow the user into the next.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let mut line = record(groceries, Period::Monthly, (2026, 1), 20_000);
    line.rollover = false;
    set_line(&db, &line);
    spend_on(&db, "2026-01-15", groceries, 5_000);

    // January: $200.00 budgeted less $50.00 spent.
    let january = status_for(&db, groceries, ym(2026, 1));
    assert_eq!(
        january.status.as_ref().expect("status").funded_balance,
        m(15_000)
    );

    // February starts fresh at the month's full $200.00 — January's leftover
    // $150.00 does not come with it.
    let february = status_for(&db, groceries, ym(2026, 2));
    assert_eq!(
        february.status.as_ref().expect("status").funded_balance,
        m(20_000)
    );
    assert_clean(&db);
}

#[test]
fn a_non_rollover_yearly_line_still_accumulates_inside_its_year() {
    // "No rollover" resets at the *cycle* boundary, not every month. A yearly
    // line that reset monthly could never reach its own target, which would
    // make the yearly period useless.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    let mut line = record(insurance, Period::Yearly, (2026, 1), 120_000);
    line.rollover = false;
    set_line(&db, &line);

    // Eight months of $100.00 have accumulated toward the bill.
    assert_eq!(
        status_for(&db, insurance, ym(2026, 8))
            .status
            .expect("status")
            .funded_balance,
        m(80_000)
    );
    assert_eq!(
        status_for(&db, insurance, ym(2026, 12))
            .status
            .expect("status")
            .funded_balance,
        m(120_000),
        "the whole target is funded by the last month of the cycle"
    );
    // The anniversary starts the next cycle from zero, so January is one
    // contribution and not thirteen.
    assert_eq!(
        status_for(&db, insurance, ym(2027, 1))
            .status
            .expect("status")
            .funded_balance,
        m(10_000)
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// The whole screen
// ---------------------------------------------------------------------------

#[test]
fn budget_status_stays_correct_across_a_full_chart_of_accounts() {
    // Thirty-odd categories, some budgeted and some not, some spent against and
    // some not. The interesting failure at this size is not slowness but a
    // per-category query that quietly pairs a line with the wrong row, or a
    // join that drops the categories with no line — both of which look fine on
    // a two-category fixture.
    let db = seeded_db();
    let groups = categories::list_groups(&db).expect("list_groups");
    let quality = groups
        .iter()
        .find(|g| g.name == "Quality of Life")
        .expect("the seed ships a Quality of Life group");
    for i in 0..10 {
        categories::create(&db, quality.id, &format!("Extra {i}")).expect("create category");
    }

    let as_of = ym(2026, 8);
    let budgeted: Vec<_> = categories::list(&db)
        .expect("list")
        .into_iter()
        .filter(|c| c.is_budgeted)
        .collect();
    assert!(
        budgeted.len() >= 30,
        "this test needs a chart of accounts worth the name; got {}",
        budgeted.len()
    );

    // Every third category gets a line, every fifth gets some spending, so the
    // four combinations of (line, spend) all occur.
    let mut expected_lines: Vec<CategoryId> = Vec::new();
    let mut expected_spend: Vec<(CategoryId, Money)> = Vec::new();
    for (i, category) in budgeted.iter().enumerate() {
        if i % 3 == 0 {
            let cents = 10_000 + i64::try_from(i).expect("small index") * 100;
            set_line(&db, &record(category.id, Period::Monthly, (2026, 1), cents));
            expected_lines.push(category.id);
        }
        if i % 5 == 0 {
            let cents = 2_500 + i64::try_from(i).expect("small index") * 10;
            spend_on(&db, "2026-08-09", category.id, cents);
            expected_spend.push((category.id, m(cents)));
        }
    }

    let rows = service::budget_status(&db, as_of).expect("budget_status");

    let mut got: Vec<CategoryId> = rows.iter().map(|r| r.category_id).collect();
    let mut want: Vec<CategoryId> = budgeted.iter().map(|c| c.id).collect();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(
        got, want,
        "every budgeted, live category appears exactly once"
    );

    for r in &rows {
        match (&r.line, &r.status) {
            (Some(line), Some(status)) => {
                assert_eq!(
                    line.category_id, r.category_id,
                    "`{}` was paired with another category's line",
                    r.category_name
                );
                assert_eq!(status.category_id, r.category_id);
                assert_eq!(
                    status.monthly_contribution,
                    monthly_contribution(line, as_of),
                    "`{}`'s contribution must be its own line's",
                    r.category_name
                );
                assert!(expected_lines.contains(&r.category_id));
            }
            (None, None) => assert!(!expected_lines.contains(&r.category_id)),
            (line, status) => panic!(
                "`{}` has a line without a status or the reverse: {line:?} / {status:?}",
                r.category_name
            ),
        }

        let want_spend = expected_spend
            .iter()
            .find(|(id, _)| *id == r.category_id)
            .map_or(Money::ZERO, |(_, amount)| *amount);
        assert_eq!(
            r.spent_this_month, want_spend,
            "`{}`'s August spending",
            r.category_name
        );
    }

    // And the single-category call agrees with the screen, row for row.
    for r in &rows {
        let single = status_for(&db, r.category_id, as_of);
        assert_eq!(single.category_id, r.category_id);
        assert_eq!(single.spent_this_month, r.spent_this_month);
        assert_eq!(
            single.line.as_ref().map(|l| l.id),
            r.line.as_ref().map(|l| l.id),
            "`{}` reads differently one at a time than in the list",
            r.category_name
        );
    }

    assert_clean(&db);
}

#[test]
fn a_lines_own_contribution_schedule_is_what_the_status_reports() {
    // Guards the join, not the arithmetic: two categories with different
    // cadences must not be handed each other's schedule.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    let travel = cat(&db, "Travel");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));
    set_line(
        &db,
        &record(travel, Period::EveryNMonths(3), (2026, 1), 30_000),
    );

    let rows = service::budget_status(&db, ym(2026, 8)).expect("budget_status");
    for name in ["Car Insurance", "Travel"] {
        let r = row(&rows, name);
        let line = line_of(r);
        assert_eq!(
            r.status.as_ref().expect("status").monthly_contribution,
            monthly_contribution(line, ym(2026, 8)),
            "{name}"
        );
    }

    // $1200/yr is $100/month; $300/quarter is also $100/month, and they must
    // arrive at it through their own cadence rather than by coincidence.
    assert_eq!(line_of(row(&rows, "Car Insurance")).period, Period::Yearly);
    assert_eq!(
        line_of(row(&rows, "Travel")).period,
        Period::EveryNMonths(3)
    );
    assert_clean(&db);
}

#[test]
fn a_month_before_the_line_starts_reports_no_contribution_yet() {
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 120_000));

    // The status is *as of* a month, and in December 2025 the line is not in
    // force: it is reported as unbudgeted, exactly like a category that has no
    // line at all. Reporting next year's line here would show December a
    // contribution it never made.
    let status = status_for(&db, insurance, ym(2025, 12));
    assert!(
        status.line.is_none(),
        "a line that has not started yet is not in force; got {:?}",
        status.line
    );
    assert!(
        status.status.is_none(),
        "no line in force means no funded status"
    );
    assert_eq!(status.spent_this_month, Money::ZERO);

    // …and it appears the moment it starts.
    let january = status_for(&db, insurance, ym(2026, 1));
    assert_eq!(
        january
            .status
            .as_ref()
            .expect("status")
            .monthly_contribution,
        m(10_000)
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// month_totals
// ---------------------------------------------------------------------------

#[test]
fn month_totals_add_up_the_month_rather_than_the_cycle() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let gas = cat(&db, "Gas");

    set_line(&db, &record(groceries, Period::Monthly, (2026, 1), 20_000));
    set_line(&db, &record(gas, Period::Monthly, (2026, 1), 30_000));

    spend_on(&db, "2026-08-04", groceries, 7_550);
    spend_on(&db, "2026-08-19", gas, 4_200);
    // Another month's money, which this month's total must not claim.
    spend_on(&db, "2026-07-30", groceries, 99_900);

    let totals = service::month_totals(&db, ym(2026, 8)).expect("month_totals");

    assert_eq!(totals.budgeted, m(50_000), "$200.00 plus $300.00");
    assert_eq!(totals.spent, m(11_750), "$75.50 plus $42.00");
    assert_clean(&db);
}

#[test]
fn a_yearly_line_contributes_its_monthly_share_to_the_total() {
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    let groceries = cat(&db, "Groceries");

    // $780.00 a year is $65.00 a month. Putting the whole bill into a month's
    // budget total would swamp every other envelope and answer a question
    // nobody asked.
    set_line(&db, &record(insurance, Period::Yearly, (2026, 1), 78_000));
    set_line(&db, &record(groceries, Period::Monthly, (2026, 1), 20_000));

    let totals = service::month_totals(&db, ym(2026, 8)).expect("month_totals");

    assert_eq!(totals.budgeted, m(26_500), "$65.00 plus $200.00");
    assert_clean(&db);
}

#[test]
fn spending_with_no_budget_still_counts_as_spent() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let hobbies = cat(&db, "Hobbies");

    set_line(&db, &record(groceries, Period::Monthly, (2026, 1), 20_000));
    // Nobody budgeted for this, but the money still left the account. Hiding
    // it would make the total disagree with the expense list.
    spend_on(&db, "2026-08-11", hobbies, 3_000);

    let totals = service::month_totals(&db, ym(2026, 8)).expect("month_totals");

    assert_eq!(totals.budgeted, m(20_000));
    assert_eq!(totals.spent, m(3_000));
    assert_clean(&db);
}

#[test]
fn a_month_with_nothing_budgeted_totals_zero_rather_than_failing() {
    let db = seeded_db();

    let totals = service::month_totals(&db, ym(2026, 8)).expect("month_totals");

    assert_eq!(totals.budgeted, Money::ZERO);
    assert_eq!(totals.spent, Money::ZERO);
    assert_clean(&db);
}
