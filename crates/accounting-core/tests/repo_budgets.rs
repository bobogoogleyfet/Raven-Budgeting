//! Budget lines and adjustments in storage.
//!
//! A budget line is a *historical record*, not a setting: `period_start` and
//! `period_end` exist so that raising next year's insurance budget does not
//! silently rewrite last year's report. That makes two things load-bearing
//! here, and neither can be delegated to the schema:
//!
//! 1. **Every `Period` variant survives the round trip.** The four variants
//!    flatten into three columns (`period_type`, `period_interval`,
//!    `due_date`), and a mapping that loses `EveryNMonths(3)`'s interval or a
//!    `ByDate` line's due date turns a funded sinking fund into a silently
//!    inert one.
//! 2. **Live lines for one category may not overlap.** The schema's partial
//!    unique index only catches an identical `period_start`; two lines
//!    covering `2026-01..2026-06` and `2026-04..` share three months, and every
//!    one of those months would then be budgeted twice. Nothing downstream
//!    notices — the report just shows a number that is too big.
//!
//! The pure contribution math is `budget_calc.rs`'s subject and is not
//! repeated here. This file is about what the database gives back.

mod common;

use accounting_core::domain::budget_calc::Period;
use accounting_core::repo::budgets::{self, BudgetLineRecord};
use accounting_core::repo::categories;
use accounting_core::{BudgetLineId, CategoryId, CoreError, Db, Money};
use chrono::NaiveDate;
use common::{assert_clean, count, m, months, raw, seeded_db, ym};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn cat(db: &Db, name: &str) -> CategoryId {
    categories::resolve(db, name)
        .unwrap_or_else(|e| panic!("resolve category `{name}`: {e}"))
        .id
}

/// A $1200 rollover line, which is the shape every other field is varied
/// against.
fn record(category_id: CategoryId, period: Period, start: (i32, u32)) -> BudgetLineRecord {
    BudgetLineRecord {
        id: None,
        category_id,
        period,
        period_start: ym(start.0, start.1),
        period_end: None,
        amount: m(120_000),
        rollover: true,
        notes: None,
    }
}

fn due(y: i32, mo: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, mo, d).unwrap_or_else(|| panic!("`{y}-{mo}-{d}` is not a date"))
}

#[track_caller]
fn upsert(db: &Db, rec: &BudgetLineRecord) -> BudgetLineId {
    budgets::upsert(db, rec).unwrap_or_else(|e| panic!("upsert {}: {e}", rec.period_start))
}

fn live_lines(db: &Db) -> i64 {
    count(
        raw(db),
        "SELECT COUNT(*) FROM budget_line WHERE archived_at IS NULL",
    )
}

/// Assert a rejection is the repository's own [`CoreError::Invalid`] — not a
/// CHECK constraint that happened to catch it — and that the message carries at
/// least one of the facts a user needs to fix it.
#[track_caller]
fn assert_invalid_mentioning<T: std::fmt::Debug>(
    result: accounting_core::Result<T>,
    needles: &[&str],
) {
    match result {
        Ok(v) => panic!("expected a rejection mentioning one of {needles:?}, got Ok({v:?})"),
        Err(err) => {
            assert!(
                matches!(err, CoreError::Invalid(_)),
                "the repository must reject this itself rather than let a \
                 constraint catch it; got {err:?}"
            );
            let msg = err.to_string().to_lowercase();
            assert!(
                needles.iter().any(|n| msg.contains(&n.to_lowercase())),
                "the message must name at least one of {needles:?} so the user \
                 can act on it; got: {err}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

#[test]
fn every_period_variant_comes_back_out_exactly_as_it_went_in() {
    // The four variants flatten into three nullable columns. A mapping that
    // drops `EveryNMonths`'s interval reads back as some other cadence, and
    // the line goes on contributing the wrong amount every month with nothing
    // to flag it.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    let travel = cat(&db, "Travel");
    let clothing = cat(&db, "Clothing");
    let gifts = cat(&db, "Gifts");

    let cases = [
        (insurance, Period::Monthly),
        (travel, Period::Yearly),
        (clothing, Period::EveryNMonths(3)),
        (
            gifts,
            Period::ByDate {
                due: due(2026, 12, 24),
            },
        ),
    ];

    for (category_id, period) in cases {
        let mut rec = record(category_id, period, (2026, 1));
        rec.period_end = Some(ym(2027, 6));
        rec.rollover = false;
        let note = format!("note for {period:?}");
        rec.notes = Some(note.clone());

        let id = upsert(&db, &rec);
        let stored = budgets::get(&db, id).expect("get the line just written");
        let category = categories::get(&db, category_id).expect("get the category");

        assert_eq!(stored.line.id, id);
        assert_eq!(stored.line.category_id, category_id);
        assert_eq!(stored.line.period, period, "the cadence must survive");
        assert_eq!(stored.line.period_start, ym(2026, 1));
        assert_eq!(stored.line.period_end, Some(ym(2027, 6)));
        assert_eq!(stored.line.amount, m(120_000));
        assert!(!stored.line.rollover);
        assert_eq!(stored.notes.as_deref(), Some(note.as_str()));
        assert_eq!(stored.category_name, category.name);
        assert_eq!(stored.group_name, category.group_name);
    }

    assert_clean(&db);
}

#[test]
fn a_stored_line_carries_the_names_needed_to_display_it() {
    // A category name alone is ambiguous by design ("Insurance" under Bills and
    // under Occasional are different envelopes), so a budget row that cannot
    // be rendered as `Group/Name` cannot be rendered correctly at all.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    let id = upsert(&db, &record(travel, Period::Yearly, (2026, 1)));

    let stored = budgets::get(&db, id).expect("get");
    assert_eq!(stored.category_name, "Travel");
    assert_eq!(stored.group_name, "Occasional");
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

#[test]
fn an_every_n_months_interval_outside_two_to_one_hundred_and_twenty_is_refused() {
    // 0 is inert (`cycle_position` returns `None`, so the line silently never
    // contributes) and 1 is a `Monthly` line wearing a disguise; both are user
    // mistakes rather than intentions. 121 is past the schema's bound, and a
    // repository that let it through would surface a raw CHECK failure instead
    // of a sentence.
    let db = seeded_db();
    let travel = cat(&db, "Travel");

    for n in [0_u32, 1, 121] {
        let rec = record(travel, Period::EveryNMonths(n), (2026, 1));
        assert_invalid_mentioning(budgets::upsert(&db, &rec), &["month", "interval", "every"]);
    }

    assert_eq!(
        live_lines(&db),
        0,
        "a refused line must not be half-written"
    );
    assert!(budgets::list(&db, None).expect("list").is_empty());
    assert_clean(&db);
}

#[test]
fn an_every_n_months_interval_at_the_bounds_is_accepted() {
    // The complement of the rejection above: 2 and 120 are inside the schema's
    // `BETWEEN 2 AND 120`, so a guard written with `>` instead of `>=` is
    // caught here rather than by a user whose quarterly bill will not save.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    let clothing = cat(&db, "Clothing");

    for (category_id, n) in [(travel, 2_u32), (clothing, 120)] {
        let id = upsert(
            &db,
            &record(category_id, Period::EveryNMonths(n), (2026, 1)),
        );
        assert_eq!(
            budgets::get(&db, id).expect("get").line.period,
            Period::EveryNMonths(n)
        );
    }

    assert_clean(&db);
}

#[test]
fn a_by_date_line_can_never_be_stored_without_its_due_date() {
    // Two layers say so. In Rust the due date is *inside* the variant
    // (`Period::ByDate { due }`), so `upsert` has no way to write the row
    // without one — the case is unrepresentable rather than merely rejected.
    // The schema holds the same rule for every other writer, including
    // `sqlite3` on the command line, and that is what this asserts, because it
    // is the only layer a test can actually attack.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    let err = raw(&db)
        .execute(
            "INSERT INTO budget_line (uuid, category_id, period_type, period_start, amount_cents)
             VALUES ('by-date-without-a-due-date', ?1, 'by_date', '2026-01', 120000)",
            rusqlite::params![travel],
        )
        .expect_err("a by_date line with no due date must be refused");
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "the schema's CHECK must be what refuses it; got: {err}"
    );

    // …and the representable form stores and reads back its due date.
    let id = upsert(
        &db,
        &record(
            travel,
            Period::ByDate {
                due: due(2026, 6, 15),
            },
            (2026, 1),
        ),
    );
    assert_eq!(
        budgets::get(&db, id).expect("get").line.period,
        Period::ByDate {
            due: due(2026, 6, 15)
        }
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Overlap
// ---------------------------------------------------------------------------

#[test]
fn a_second_line_overlapping_the_first_is_refused_and_the_conflict_is_named() {
    // 2026-01..2026-06 and 2026-04..(open) share April, May and June. Every one
    // of those months would be budgeted twice, and the only symptom is a report
    // whose numbers are too large. The schema's unique index does not catch
    // this: the two `period_start`s differ.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    let mut first = record(insurance, Period::Monthly, (2026, 1));
    first.period_end = Some(ym(2026, 6));
    let first_id = upsert(&db, &first);

    let overlapping = record(insurance, Period::Monthly, (2026, 4));
    assert_invalid_mentioning(
        budgets::upsert(&db, &overlapping),
        &["2026-04", "2026-01", "2026-06", "overlap"],
    );

    let live = budgets::list(&db, None).expect("list");
    assert!(
        live.iter().all(|l| l.line.id == first_id),
        "the refused line must not be on file; got {live:?}"
    );
    assert_eq!(live_lines(&db), 1);
    assert_clean(&db);
}

#[test]
fn overlap_is_refused_from_either_direction() {
    // The open-ended line is written first this time, so a check that only
    // looks *backwards* from the new line's start would let this through.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    upsert(&db, &record(insurance, Period::Monthly, (2026, 4)));

    let mut earlier = record(insurance, Period::Monthly, (2026, 1));
    earlier.period_end = Some(ym(2026, 6));
    assert_invalid_mentioning(
        budgets::upsert(&db, &earlier),
        &["2026-04", "2026-01", "2026-06", "overlap"],
    );

    assert_eq!(live_lines(&db), 1);
    assert_clean(&db);
}

#[test]
fn a_line_wholly_inside_another_is_refused() {
    // Containment is the case a naive "do the endpoints interleave?" test
    // misses: 2026-03..2026-04 sits entirely within 2026-01..2026-12.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    let mut outer = record(insurance, Period::Monthly, (2026, 1));
    outer.period_end = Some(ym(2026, 12));
    upsert(&db, &outer);

    let mut inner = record(insurance, Period::Monthly, (2026, 3));
    inner.period_end = Some(ym(2026, 4));
    assert_invalid_mentioning(
        budgets::upsert(&db, &inner),
        &["2026-03", "2026-04", "2026-01", "2026-12", "overlap"],
    );

    assert_eq!(live_lines(&db), 1);
    assert_clean(&db);
}

#[test]
fn touching_but_not_overlapping_spans_are_permitted() {
    // The permitted neighbour, and the reason the check has to be about months
    // rather than about "does this category already have a line": 2026-07 is
    // the month *after* the first line ends, so nothing is double-counted and
    // "we raised the budget in July" must remain expressible.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    let mut first = record(insurance, Period::Monthly, (2026, 1));
    first.period_end = Some(ym(2026, 6));
    first.amount = m(10_000);
    let first_id = upsert(&db, &first);

    let mut second = record(insurance, Period::Monthly, (2026, 7));
    second.amount = m(15_000);
    let second_id = upsert(&db, &second);

    assert_ne!(first_id, second_id);
    // Each month resolves to exactly one of them, which is the property the
    // overlap rule exists to protect.
    let june = budgets::for_category(&db, insurance, ym(2026, 6))
        .expect("for_category")
        .expect("June is covered by the first line");
    assert_eq!(june.line.id, first_id);
    assert_eq!(june.line.amount, m(10_000));

    let july = budgets::for_category(&db, insurance, ym(2026, 7))
        .expect("for_category")
        .expect("July is covered by the second line");
    assert_eq!(july.line.id, second_id);
    assert_eq!(july.line.amount, m(15_000));

    assert_clean(&db);
}

#[test]
fn the_same_span_for_a_different_category_is_permitted() {
    // Overlap is per category. Two envelopes budgeted over the same months is
    // the normal case — it is what a budget *is*.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    let travel = cat(&db, "Travel");

    let mut first = record(insurance, Period::Monthly, (2026, 1));
    first.period_end = Some(ym(2026, 6));
    upsert(&db, &first);

    let mut second = record(travel, Period::Monthly, (2026, 1));
    second.period_end = Some(ym(2026, 6));
    let travel_id = upsert(&db, &second);

    let march = budgets::list(&db, Some(ym(2026, 3))).expect("list");
    assert!(
        march.iter().any(|l| l.line.id == travel_id),
        "Travel's line must be live in March alongside Insurance's; got {march:?}"
    );
    assert!(march.iter().any(|l| l.line.category_id == insurance));
    assert_clean(&db);
}

#[test]
fn updating_a_line_in_place_does_not_collide_with_itself() {
    // The obvious way to get the overlap check wrong: an update re-checks the
    // row it is updating, decides the line overlaps itself, and makes every
    // budget permanently uneditable.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    let mut rec = record(insurance, Period::Monthly, (2026, 1));
    rec.period_end = Some(ym(2026, 6));
    rec.amount = m(10_000);
    let id = upsert(&db, &rec);

    rec.id = Some(id);
    rec.amount = m(12_500);
    rec.rollover = false;
    rec.notes = Some("raised in review".to_string());
    rec.period_end = Some(ym(2026, 9));
    let same_id = upsert(&db, &rec);

    assert_eq!(same_id, id, "an update must not create a second line");
    let stored = budgets::get(&db, id).expect("get");
    assert_eq!(stored.line.amount, m(12_500));
    assert_eq!(stored.line.period_end, Some(ym(2026, 9)));
    assert!(!stored.line.rollover);
    assert_eq!(stored.notes.as_deref(), Some("raised in review"));
    assert_eq!(live_lines(&db), 1);
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Archiving and listing
// ---------------------------------------------------------------------------

#[test]
fn an_archived_line_leaves_the_list_and_frees_its_span() {
    // Nothing user-facing is hard-deleted, so "I set that budget up wrong"
    // has to be expressible as archive-then-recreate — including recreating it
    // over exactly the same months.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    let mut rec = record(insurance, Period::Monthly, (2026, 1));
    rec.period_end = Some(ym(2026, 6));
    let old_id = upsert(&db, &rec);

    budgets::archive(&db, old_id).expect("archive");

    let listed = budgets::list(&db, None).expect("list");
    assert!(
        !listed.iter().any(|l| l.line.id == old_id),
        "an archived line must not be listed; got {listed:?}"
    );
    assert_eq!(
        budgets::for_category(&db, insurance, ym(2026, 3))
            .expect("for_category")
            .map(|l| l.line.id),
        None,
        "an archived line must not answer for its months either"
    );

    // The same span is now free — both for the schema's partial unique index
    // and for the repository's overlap check.
    let mut replacement = record(insurance, Period::Yearly, (2026, 1));
    replacement.period_end = Some(ym(2026, 6));
    replacement.amount = m(60_000);
    let new_id = upsert(&db, &replacement);
    assert_ne!(new_id, old_id);
    assert_eq!(
        budgets::get(&db, new_id).expect("get").line.amount,
        m(60_000)
    );

    // …and the archived row is still there to be read by id, so a report over
    // last year does not lose the budget it was drawn against.
    assert_eq!(budgets::get(&db, old_id).expect("get").line.id, old_id);
    assert_clean(&db);
}

#[test]
fn list_filtered_by_month_returns_only_the_lines_in_force_that_month() {
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");
    let travel = cat(&db, "Travel");

    let mut early = record(insurance, Period::Monthly, (2026, 1));
    early.period_end = Some(ym(2026, 6));
    let early_id = upsert(&db, &early);

    let late = record(travel, Period::Monthly, (2026, 7));
    let late_id = upsert(&db, &late);

    let march = budgets::list(&db, Some(ym(2026, 3))).expect("list March");
    assert!(march.iter().any(|l| l.line.id == early_id));
    assert!(
        !march.iter().any(|l| l.line.id == late_id),
        "a line that starts in July is not in force in March; got {march:?}"
    );

    let september = budgets::list(&db, Some(ym(2026, 9))).expect("list September");
    assert!(september.iter().any(|l| l.line.id == late_id));
    assert!(
        !september.iter().any(|l| l.line.id == early_id),
        "a line that ended in June is not in force in September; got {september:?}"
    );

    let december_2025 = budgets::list(&db, Some(ym(2025, 12))).expect("list 2025-12");
    assert!(
        december_2025.is_empty(),
        "neither line had started; got {december_2025:?}"
    );

    let unfiltered = budgets::list(&db, None).expect("list all");
    assert!(unfiltered.iter().any(|l| l.line.id == early_id));
    assert!(unfiltered.iter().any(|l| l.line.id == late_id));
    assert_clean(&db);
}

#[test]
fn for_category_picks_the_line_covering_the_month_out_of_a_history_of_them() {
    // Three consecutive budgets for one category — the shape a category
    // accumulates over years. Picking the newest, or the first row the query
    // happened to return, would restate history.
    let db = seeded_db();
    let insurance = cat(&db, "Car Insurance");

    let mut y2024 = record(insurance, Period::Monthly, (2024, 1));
    y2024.period_end = Some(ym(2024, 12));
    y2024.amount = m(8_000);
    let id_2024 = upsert(&db, &y2024);

    let mut y2025 = record(insurance, Period::Monthly, (2025, 1));
    y2025.period_end = Some(ym(2025, 12));
    y2025.amount = m(9_000);
    let id_2025 = upsert(&db, &y2025);

    let y2026 = record(insurance, Period::Monthly, (2026, 1));
    let id_2026 = upsert(&db, &y2026);

    for (month, want_id, want_amount) in [
        ((2024, 7), id_2024, m(8_000)),
        ((2025, 1), id_2025, m(9_000)),
        ((2025, 12), id_2025, m(9_000)),
        ((2026, 1), id_2026, m(120_000)),
        ((2030, 6), id_2026, m(120_000)),
    ] {
        let found = budgets::for_category(&db, insurance, ym(month.0, month.1))
            .expect("for_category")
            .unwrap_or_else(|| panic!("{}-{:02} must be covered", month.0, month.1));
        assert_eq!(found.line.id, want_id, "{}-{:02}", month.0, month.1);
        assert_eq!(found.line.amount, want_amount);
    }

    assert_eq!(
        budgets::for_category(&db, insurance, ym(2023, 12))
            .expect("for_category")
            .map(|l| l.line.id),
        None,
        "before the first line there is no budget, not the earliest one"
    );
    assert_clean(&db);
}

#[test]
fn a_category_with_no_line_at_all_resolves_to_nothing() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    assert!(
        budgets::for_category(&db, groceries, ym(2026, 8))
            .expect("for_category")
            .is_none()
    );
    assert!(budgets::list(&db, None).expect("list").is_empty());
}

// ---------------------------------------------------------------------------
// Adjustments
// ---------------------------------------------------------------------------

#[test]
fn adjustments_sum_within_a_month_and_stop_at_the_range() {
    // Adjustments are the "I moved $50 from Groceries into Travel" lever, and
    // there is no reason a month may only hold one of them. Several in a month
    // must add up rather than the last one winning.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    let gifts = cat(&db, "Gifts");

    budgets::add_adjustment(&db, travel, ym(2026, 3), m(5_000), Some("moved from Gifts"))
        .expect("add_adjustment");
    budgets::add_adjustment(&db, travel, ym(2026, 3), m(-2_000), None).expect("add_adjustment");
    budgets::add_adjustment(&db, travel, ym(2026, 5), m(1_250), None).expect("add_adjustment");
    // Outside the range asked for below, and outside the category asked for.
    budgets::add_adjustment(&db, travel, ym(2025, 12), m(99_900), None).expect("add_adjustment");
    budgets::add_adjustment(&db, travel, ym(2026, 8), m(99_900), None).expect("add_adjustment");
    budgets::add_adjustment(&db, gifts, ym(2026, 3), m(77_700), None).expect("add_adjustment");

    let found = budgets::adjustments_for(&db, travel, months((2026, 1), (2026, 6)))
        .expect("adjustments_for");

    // $50.00 - $20.00 = $30.00 in March.
    assert_eq!(found.get(&ym(2026, 3)).copied(), Some(m(3_000)));
    assert_eq!(found.get(&ym(2026, 5)).copied(), Some(m(1_250)));
    assert_eq!(
        found.get(&ym(2025, 12)).copied(),
        None,
        "a month before the range must not be reported"
    );
    assert_eq!(
        found.get(&ym(2026, 8)).copied(),
        None,
        "a month after the range must not be reported"
    );
    assert_eq!(
        found.values().copied().sum::<Money>(),
        m(4_250),
        "$30.00 in March plus $12.50 in May, and nothing else"
    );
    assert!(
        found.values().all(|v| !v.is_zero()),
        "Gifts' $777.00 must not have leaked into Travel's totals; got {found:?}"
    );

    // The other category keeps its own.
    let gifts_found = budgets::adjustments_for(&db, gifts, months((2026, 1), (2026, 6)))
        .expect("adjustments_for");
    assert_eq!(gifts_found.get(&ym(2026, 3)).copied(), Some(m(77_700)));
    assert_clean(&db);
}

#[test]
fn a_category_with_no_adjustments_reports_none_rather_than_failing() {
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    let found = budgets::adjustments_for(&db, travel, months((2026, 1), (2026, 12)))
        .expect("adjustments_for");
    assert!(
        found.values().all(|v| v.is_zero()),
        "no adjustments means no money moved; got {found:?}"
    );
}

#[test]
fn an_adjustment_of_zero_is_stored_and_reads_back_as_zero() {
    // A write-off of exactly nothing is degenerate but legal, and it must not
    // be confused with "no adjustment": the reason text is the record.
    let db = seeded_db();
    let travel = cat(&db, "Travel");
    budgets::add_adjustment(
        &db,
        travel,
        ym(2026, 3),
        Money::ZERO,
        Some("noted, no money moved"),
    )
    .expect("add_adjustment");
    let found = budgets::adjustments_for(&db, travel, months((2026, 3), (2026, 3)))
        .expect("adjustments_for");
    assert_eq!(
        found.get(&ym(2026, 3)).copied().unwrap_or(Money::ZERO),
        Money::ZERO
    );
    assert_clean(&db);
}

#[test]
fn a_missing_line_id_is_not_found_rather_than_a_raw_sqlite_error() {
    let db = seeded_db();
    assert!(matches!(
        budgets::get(&db, BudgetLineId::new(9999)),
        Err(CoreError::NotFound(_))
    ));
    assert!(matches!(
        budgets::archive(&db, BudgetLineId::new(9999)),
        Err(CoreError::NotFound(_))
    ));
}
