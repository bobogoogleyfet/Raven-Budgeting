//! The shape of an export, before any file format gets involved.
//!
//! An export row is **one split**, not one transaction. That is the whole
//! design: a $100 receipt divided between Groceries and Household is
//! two facts about where money went, and a per-transaction export can only
//! answer "how much did I spend at Costco", never "how much did I spend on
//! groceries" — which is the question the app exists to answer. So the claims
//! worth testing here are:
//!
//! 1. **The split columns still add up to the transaction.** Fanning one row
//!    out into three is exactly where a cent goes missing, and a per-category
//!    report that disagrees with the bank statement is worse than no report.
//! 2. **The derived columns are derived correctly.** Year/Quarter/Month/
//!    MonthName/YearMonth exist so a pivot table needs no formulas; they are
//!    written as literal values, so nothing recomputes them if they are wrong.
//! 3. **The order is chronological.** `repo::transactions::list` returns
//!    newest-first, because that is what a "recent receipts" screen wants. A
//!    spreadsheet wants the opposite, and the conversion is the exporter's job.
//! 4. **Absent facts stay absent.** A receipt with no merchant must export a
//!    blank, not the empty string masquerading as a store called "".

mod common;

use accounting_core::export::{self, ExportRow};
use accounting_core::repo::transactions::TxnFilter;
use accounting_core::{Db, Money, TxnId, YearMonth};
use chrono::NaiveDate;
use common::{
    amounts, assert_clean, cat, day, estimated_amounts, location, m, merchant, save, seeded_db,
    split, txn,
};

#[track_caller]
fn rows(db: &Db, filter: &TxnFilter) -> Vec<ExportRow> {
    export::rows(db, filter).expect("export::rows")
}

#[track_caller]
fn all_rows(db: &Db) -> Vec<ExportRow> {
    rows(db, &TxnFilter::default())
}

/// The rows belonging to one transaction, in the order the export produced
/// them.
fn of_txn(rows: &[ExportRow], id: TxnId) -> Vec<&ExportRow> {
    rows.iter().filter(|r| r.txn_id == id).collect()
}

// ---------------------------------------------------------------------------
// One row per split
// ---------------------------------------------------------------------------

#[test]
fn a_three_way_split_becomes_three_rows_whose_columns_sum_back_to_the_transaction() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");
    let personal = cat(&db, "Toiletries");

    // $90.00 + $10.00 tax = $100.00, divided so that every column adds up on
    // its own:
    //   subtotal  45.00 + 30.00 + 15.00 =  90.00
    //   tax        5.00 +  3.33 +  1.67 =  10.00
    //   total     50.00 + 33.33 + 16.67 = 100.00
    //
    // The tax column is split unevenly on purpose. An exporter that recomputed
    // each row's tax from the transaction's rate, or that pro-rated it by
    // subtotal, would produce 5.00 / 3.33 / 1.67 for *this* receipt by luck;
    // it must instead carry each split's own stored number, and the assertion
    // below is on the stored values.
    let id = save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(9000, 1000),
            vec![
                split(groceries, 4500, 500),
                split(household, 3000, 333),
                split(personal, 1500, 167),
            ],
        ),
    );

    let exported = all_rows(&db);
    let mine = of_txn(&exported, id);
    assert_eq!(
        mine.len(),
        3,
        "a three-way split is three facts about where the money went, and must \
         export as three rows; got {mine:?}"
    );

    assert_eq!(
        mine.iter().map(|r| r.subtotal).sum::<Money>(),
        m(9000),
        "the subtotal column must still add up to the receipt's subtotal"
    );
    assert_eq!(
        mine.iter().map(|r| r.tax).sum::<Money>(),
        m(1000),
        "and the tax column on its own — a row that offsets an overstated \
         subtotal against an understated tax still sums correctly in Total"
    );
    assert_eq!(
        mine.iter().map(|r| r.total).sum::<Money>(),
        m(10_000),
        "and the total column, which is the number the bank statement shows"
    );

    for r in &mine {
        assert_eq!(
            r.subtotal + r.tax,
            r.total,
            "every exported row must balance on its own; {r:?}"
        );
    }

    // Each split's own numbers, not an even third of anything.
    let household_row = mine
        .iter()
        .find(|r| r.category == "Household")
        .expect("the Household split must be exported");
    assert_eq!(household_row.subtotal, m(3000));
    assert_eq!(household_row.tax, m(333));
    assert_eq!(household_row.total, m(3333));
    assert_eq!(
        household_row.group, "Frequent",
        "the group is joined in so a pivot can roll categories up without a lookup table"
    );

    assert_clean(&db);
}

#[test]
fn every_row_of_a_receipt_repeats_the_transactions_own_facts() {
    // The flip side of one-row-per-split: the columns that belong to the
    // *transaction* have to be copied onto each of its rows, or a spreadsheet
    // filtered to one category loses the date and the store.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let fuel = cat(&db, "Gas");
    let costco = merchant(&db, "Costco");
    let sunnyvale = location(&db, "Costco Sunnyvale", 91_250);

    let mut new = txn(
        day("2026-03-14"),
        amounts(5000, 0),
        vec![split(groceries, 3000, 0), split(fuel, 2000, 0)],
    );
    new.merchant_id = Some(costco);
    new.location_id = Some(sunnyvale);
    new.notes = Some("weekly shop".to_string());
    let id = save(&db, &new);

    let exported = all_rows(&db);
    let mine = of_txn(&exported, id);
    assert_eq!(mine.len(), 2);
    for r in &mine {
        assert_eq!(r.date, day("2026-03-14"));
        assert_eq!(r.merchant.as_deref(), Some("Costco"));
        assert_eq!(r.location.as_deref(), Some("Costco Sunnyvale"));
        assert_eq!(r.notes.as_deref(), Some("weekly shop"));
        assert_eq!(r.txn_id, id);
    }

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Derived columns
// ---------------------------------------------------------------------------

/// `(the date, its expected year, quarter, month, month name, year-month)`.
///
/// These are written into the sheet as literal values rather than formulas, so
/// nothing downstream recomputes them: whatever is wrong here stays wrong in
/// every pivot table built on the export.
const CALENDAR: &[(&str, i32, u32, u32, &str, &str)] = &[
    // The ordinary case.
    ("2026-03-14", 2026, 1, 3, "Mar", "2026-03"),
    // The Q4 boundary, and the last day of a year: an off-by-one in the
    // quarter table puts December in Q3, and an off-by-one in a
    // days-since-epoch month calculation rolls 31 December into January of the
    // following year.
    ("2026-12-31", 2026, 4, 12, "Dec", "2026-12"),
    // A leap day, which only exists at all if the date is handled as a
    // calendar date rather than as a day count.
    ("2028-02-29", 2028, 1, 2, "Feb", "2028-02"),
    // The first day of a quarter, the other side of the boundary above.
    ("2026-10-01", 2026, 4, 10, "Oct", "2026-10"),
];

#[test]
fn the_derived_calendar_columns_match_the_date_they_come_from() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    for (date, ..) in CALENDAR {
        save(
            &db,
            &txn(day(date), amounts(1000, 0), vec![split(groceries, 1000, 0)]),
        );
    }

    let exported = all_rows(&db);
    for (date, year, quarter, month, month_name, year_month) in CALENDAR {
        let d: NaiveDate = day(date);
        let r = exported
            .iter()
            .find(|r| r.date == d)
            .unwrap_or_else(|| panic!("no exported row for {date}"));

        assert_eq!(r.year, *year, "Year for {date}");
        assert_eq!(r.quarter, *quarter, "Quarter for {date}");
        assert_eq!(r.month, *month, "Month for {date}");
        assert_eq!(r.month_name, *month_name, "MonthName for {date}");
        assert_eq!(
            r.year_month,
            year_month
                .parse::<YearMonth>()
                .expect("the expectation itself must be a valid YYYY-MM"),
            "YearMonth for {date}"
        );
        assert_eq!(
            r.year_month.to_string(),
            *year_month,
            "YearMonth renders as YYYY-MM, zero-padded, so it sorts as text"
        );
    }

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Ordering
// ---------------------------------------------------------------------------

#[test]
fn rows_come_back_oldest_first_even_though_the_repository_lists_newest_first() {
    // `repo::transactions::list` is newest-first, because that is what a
    // recent-receipts screen wants. A spreadsheet wants the opposite: a
    // reverse-chronological export makes every running total read backwards
    // and every chart run right to left. Converting between the two is the
    // exporter's job, and this is the test that says so.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let fuel = cat(&db, "Gas");

    // Saved deliberately out of order, so passing cannot be an accident of
    // insertion order either.
    let august = save(
        &db,
        &txn(
            day("2026-08-20"),
            amounts(2000, 0),
            vec![split(fuel, 2000, 0)],
        ),
    );
    let june = save(
        &db,
        &txn(
            day("2026-06-01"),
            amounts(1000, 0),
            vec![split(groceries, 1000, 0)],
        ),
    );
    // A second receipt on the same day as `august`, saved after it, and split
    // two ways with explicit sort orders that run *against* the category
    // alphabet — so a sort that fell back to the category name would be
    // visible here.
    let mut same_day = txn(
        day("2026-08-20"),
        amounts(3000, 0),
        vec![split(fuel, 1000, 0), split(groceries, 2000, 0)],
    );
    same_day.splits[0].sort_order = 200;
    same_day.splits[1].sort_order = 100;
    let also_august = save(&db, &same_day);

    let exported = all_rows(&db);

    let dates: Vec<NaiveDate> = exported.iter().map(|r| r.date).collect();
    assert!(
        dates.windows(2).all(|w| w[0] <= w[1]),
        "export rows must run date-ascending; got {dates:?}"
    );

    let position = |id: TxnId| -> usize {
        exported
            .iter()
            .position(|r| r.txn_id == id)
            .unwrap_or_else(|| panic!("{id:?} missing from the export"))
    };
    assert!(
        position(june) < position(august),
        "June must precede August"
    );
    assert!(
        position(august) < position(also_august),
        "two receipts on the same day break the tie by transaction id, so the \
         one entered first comes first"
    );

    // Within a transaction, the rows follow the split's own sort_order — the
    // order the user arranged them in, which is what `pacc show` displays.
    let within: Vec<&str> = of_txn(&exported, also_august)
        .iter()
        .map(|r| r.category.as_str())
        .collect();
    assert_eq!(
        within,
        vec!["Groceries", "Gas"],
        "the splits of one receipt must export in sort_order (100 before 200), \
         not in category or insertion order"
    );

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// Three months of receipts across two merchants and two categories.
struct Fixture {
    db: Db,
    costco: accounting_core::MerchantId,
    groceries: accounting_core::CategoryId,
    fuel: accounting_core::CategoryId,
    june: TxnId,
    july: TxnId,
    august: TxnId,
}

fn fixture() -> Fixture {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let fuel = cat(&db, "Gas");
    let costco = merchant(&db, "Costco");
    let shell = merchant(&db, "Shell");

    let mut june = txn(
        day("2026-06-15"),
        amounts(1000, 0),
        vec![split(groceries, 1000, 0)],
    );
    june.merchant_id = Some(costco);
    let june = save(&db, &june);

    // Two categories, so a category filter has something to be selective
    // about within a single transaction.
    let mut july = txn(
        day("2026-07-04"),
        amounts(5000, 0),
        vec![split(groceries, 3000, 0), split(fuel, 2000, 0)],
    );
    july.merchant_id = Some(shell);
    let july = save(&db, &july);

    let mut august = txn(
        day("2026-08-01"),
        amounts(2000, 0),
        vec![split(fuel, 2000, 0)],
    );
    august.merchant_id = Some(costco);
    let august = save(&db, &august);

    let f = Fixture {
        db,
        costco,
        groceries,
        fuel,
        june,
        july,
        august,
    };
    assert_clean(&f.db);
    f
}

#[test]
fn a_month_filter_keeps_only_that_months_rows() {
    let f = fixture();
    let july = YearMonth::new(2026, 7).expect("July exists");

    let exported = rows(
        &f.db,
        &TxnFilter {
            from: Some(july.first_day()),
            to: Some(july.last_day()),
            ..TxnFilter::default()
        },
    );

    assert!(
        exported.iter().all(|r| r.year_month == july),
        "every row must fall in the requested month; got {:?}",
        exported.iter().map(|r| r.year_month).collect::<Vec<_>>()
    );
    let ids: Vec<TxnId> = exported.iter().map(|r| r.txn_id).collect();
    assert!(ids.contains(&f.july), "July's receipt must be there");
    assert!(!ids.contains(&f.june));
    assert!(!ids.contains(&f.august));

    // The negative control: the filter cannot be passing by returning nothing.
    assert!(!exported.is_empty());
    assert!(
        all_rows(&f.db).iter().any(|r| r.txn_id == f.june),
        "an unfiltered export must still contain June"
    );
}

#[test]
fn a_category_filter_narrows_the_export_to_receipts_touching_that_category() {
    // `TxnFilter::category_id` selects *transactions* that have a split in the
    // category (`EXISTS`, so a receipt touching it twice is not doubled). What
    // the exporter then does with a matched transaction's *other* splits is
    // genuinely underspecified by the contract — it may keep them for context
    // or drop them — so this asserts the property that holds either way: every
    // exported row belongs to a receipt that touches the category, the
    // category's own split is present, and receipts that never touch it are
    // gone.
    let f = fixture();

    let exported = rows(
        &f.db,
        &TxnFilter {
            category_id: Some(f.fuel),
            ..TxnFilter::default()
        },
    );

    let ids: Vec<TxnId> = exported.iter().map(|r| r.txn_id).collect();
    assert!(
        ids.iter().all(|id| *id == f.july || *id == f.august),
        "only the two receipts with a fuel split may appear; got {ids:?}"
    );
    assert!(
        exported
            .iter()
            .any(|r| r.txn_id == f.july && r.category == "Gas"),
        "July's fuel split is the row the filter was asked for"
    );
    assert!(
        !ids.contains(&f.june),
        "June's groceries-only receipt must not appear under a fuel filter"
    );

    // And the other category, so the filter is not simply passing everything
    // with a fuel row in it through.
    let groceries = rows(
        &f.db,
        &TxnFilter {
            category_id: Some(f.groceries),
            ..TxnFilter::default()
        },
    );
    let groceries_ids: Vec<TxnId> = groceries.iter().map(|r| r.txn_id).collect();
    assert!(groceries_ids.contains(&f.june));
    assert!(
        !groceries_ids.contains(&f.august),
        "August is fuel only and must not appear under Groceries"
    );
}

#[test]
fn a_merchant_filter_keeps_only_that_merchants_rows() {
    let f = fixture();

    let exported = rows(
        &f.db,
        &TxnFilter {
            merchant_id: Some(f.costco),
            ..TxnFilter::default()
        },
    );

    assert!(
        exported
            .iter()
            .all(|r| r.merchant.as_deref() == Some("Costco")),
        "every row must belong to the requested merchant; got {:?}",
        exported
            .iter()
            .map(|r| r.merchant.clone())
            .collect::<Vec<_>>()
    );
    let ids: Vec<TxnId> = exported.iter().map(|r| r.txn_id).collect();
    assert!(ids.contains(&f.june));
    assert!(ids.contains(&f.august));
    assert!(!ids.contains(&f.july), "that one is Shell's");
}

#[test]
fn an_archived_receipt_stays_out_of_the_export_unless_it_is_asked_for() {
    let f = fixture();
    accounting_core::repo::transactions::archive(&f.db, f.july).expect("archive");

    let live: Vec<TxnId> = all_rows(&f.db).iter().map(|r| r.txn_id).collect();
    assert!(
        !live.contains(&f.july),
        "an archived receipt is deleted as far as the user is concerned, and a \
         tax export that still counts it is wrong"
    );

    let everything: Vec<TxnId> = rows(
        &f.db,
        &TxnFilter {
            include_archived: true,
            ..TxnFilter::default()
        },
    )
    .iter()
    .map(|r| r.txn_id)
    .collect();
    assert!(everything.contains(&f.july));
    assert!(
        everything.contains(&f.june),
        "include_archived widens the export, it does not replace it"
    );

    assert_clean(&f.db);
}

// ---------------------------------------------------------------------------
// Absent facts
// ---------------------------------------------------------------------------

#[test]
fn a_receipt_with_no_merchant_and_no_location_exports_nothing_rather_than_an_empty_string() {
    // A cash receipt from somewhere the user did not name. The distinction
    // matters downstream: `None` becomes a blank cell that a pivot table
    // ignores, whereas `Some("")` becomes a store whose name is the empty
    // string, sitting in the merchant list forever.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    let id = save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(1000, 0),
            vec![split(groceries, 1000, 0)],
        ),
    );

    let exported = all_rows(&db);
    let only = match of_txn(&exported, id).as_slice() {
        [one] => *one,
        other => panic!("a single-category receipt exports as one row, got {other:?}"),
    };

    assert_eq!(only.merchant, None, "no merchant means no merchant");
    assert_eq!(only.location, None);
    assert_eq!(only.notes, None);
    // And the columns that are never absent are still filled in.
    assert_eq!(only.category, "Groceries");
    assert_eq!(only.group, "Frequent");

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// tax_estimated
// ---------------------------------------------------------------------------

#[test]
fn tax_estimated_is_carried_per_transaction_and_is_false_for_a_typed_in_tax() {
    // An estimate that silently looks like a fact is worse than no estimate:
    // the user reconciling against a receipt needs to know which of the two
    // numbers in front of them this app made up.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");

    // $50.12 + $4.20 tax, read straight off the paper.
    let typed = save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(5012, 420),
            vec![split(groceries, 5012, 420)],
        ),
    );
    // $50.00 at 8.375%: 50.00 * 0.08375 = $4.1875, which rounds to $4.19.
    // Nobody typed that; the app worked it out.
    let mut computed = txn(
        day("2026-03-15"),
        estimated_amounts(5000, 419),
        vec![split(groceries, 3000, 251), split(household, 2000, 168)],
    );
    computed.tax_rate_ppm = Some(83_750);
    let computed = save(&db, &computed);

    let exported = all_rows(&db);

    for r in of_txn(&exported, typed) {
        assert!(
            !r.tax_estimated,
            "a tax read off the receipt is a fact; {r:?}"
        );
    }
    let computed_rows = of_txn(&exported, computed);
    assert_eq!(
        computed_rows.len(),
        2,
        "the estimated receipt is split two ways"
    );
    for r in &computed_rows {
        assert!(
            r.tax_estimated,
            "the flag belongs to the transaction, so *every* row of a receipt \
             whose tax was estimated must carry it; {r:?}"
        );
    }

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Nothing at all
// ---------------------------------------------------------------------------

#[test]
fn an_empty_database_exports_no_rows_rather_than_failing() {
    let db = seeded_db();
    assert!(
        all_rows(&db).is_empty(),
        "a database with a taxonomy but no receipts has nothing to export, \
         which is not an error"
    );
}
