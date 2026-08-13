//! Subtotal-first entry: a subtotal per category, tax derived, no total typed.
//!
//! This is how a receipt is actually read. The other mode starts from a total
//! and divides it; this one starts from the parts, so nobody types a number
//! the app can work out — and therefore nobody can type one that disagrees
//! with it.
//!
//! The property under test throughout is that the rows sum to the transaction
//! exactly, in all three columns. Tax is apportioned by subtotal through
//! `Money::allocate`, so an odd cent is handed to a row rather than dropped.

mod common;

use accounting_core::Money;
use accounting_core::repo::transactions::TxnFilter;
use accounting_core::repo::{locations, transactions};
use accounting_core::service::entry::{
    SubtotalDraft, SubtotalRow, preview_subtotals, save_subtotals,
};
use common::{assert_clean, cat, day, m, seeded_db};

const NOW: &str = "2026-03-14T10:00:00.000Z";

/// 7.75%, as parts-per-million.
const RATE: u32 = 77_500;

fn with_rate(db: &accounting_core::Db, ppm: u32) {
    let home = locations::find_by_name(db, "Home")
        .expect("find home")
        .expect("home is seeded");
    locations::set_rate(db, home.id, ppm).expect("set rate");
}

fn draft(db: &accounting_core::Db, rows: &[(&str, i64)]) -> SubtotalDraft {
    SubtotalDraft {
        id: None,
        txn_date: day("2026-03-14"),
        merchant: Some("Costco".to_string()),
        location: None,
        rows: rows
            .iter()
            .map(|(name, cents)| SubtotalRow {
                category_id: Some(cat(db, name)),
                subtotal: m(*cents),
                note: None,
            })
            .collect(),
        notes: None,
        needs_review: false,
        remember_defaults: true,
    }
}

#[test]
fn tax_is_derived_from_the_location_and_never_asked_for() {
    let db = seeded_db();
    with_rate(&db, RATE);

    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 10_000)])).expect("preview");

    // $100.00 at 7.75% is $7.75 of tax and a $107.75 total.
    assert_eq!(p.amounts.subtotal, m(10_000));
    assert_eq!(p.amounts.tax, m(775));
    assert_eq!(p.amounts.total, m(10_775));
    // Computed, not read off the paper — and the flag must say so.
    assert!(p.amounts.tax_is_estimated);
    assert_eq!(p.tax_rate_ppm, RATE);
}

#[test]
fn the_rows_sum_to_the_transaction_in_every_column() {
    let db = seeded_db();
    with_rate(&db, RATE);

    // Two identical $33.33 rows force the odd cent: $166.66 at 7.75% is
    // $12.9161…, which rounds to $12.92, and 12.92 does not divide evenly by
    // the three subtotals.
    let p = preview_subtotals(
        &db,
        &draft(
            &db,
            &[
                ("Groceries", 10_000),
                ("Household", 3_333),
                ("Toiletries", 3_333),
            ],
        ),
    )
    .expect("preview");

    let sum = |f: fn(&accounting_core::domain::splits::ResolvedSplit) -> Money| {
        p.plan
            .rows
            .iter()
            .map(f)
            .fold(Money::ZERO, |a, b| a.checked_add(b).expect("no overflow"))
    };
    assert_eq!(sum(|r| r.subtotal), p.amounts.subtotal, "subtotals");
    assert_eq!(sum(|r| r.tax), p.amounts.tax, "taxes");
    assert_eq!(sum(|r| r.amount), p.amounts.total, "amounts");

    // The odd cent is handed to a row, not dropped: the two equal subtotals
    // get $2.59 and $2.58 rather than $2.58 and $2.58.
    let taxes: Vec<Money> = p.plan.rows.iter().map(|r| r.tax).collect();
    assert_eq!(taxes, vec![m(775), m(259), m(258)]);
}

#[test]
fn nothing_is_ever_unassigned_because_the_rows_are_the_transaction() {
    let db = seeded_db();
    with_rate(&db, RATE);
    for rows in [
        vec![("Groceries", 1)],
        vec![("Groceries", 99_999), ("Gas", 1)],
        vec![
            ("Groceries", 3_333),
            ("Household", 3_333),
            ("Toiletries", 3_334),
        ],
    ] {
        let p = preview_subtotals(&db, &draft(&db, &rows)).expect("preview");
        assert_eq!(
            p.plan.unassigned,
            Money::ZERO,
            "there is no total to leave a remainder against: {rows:?}"
        );
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
    }
}

#[test]
fn a_location_with_no_rate_adds_no_tax_and_says_so() {
    let db = seeded_db();
    // The seeded Home starts at 0%: a wrong prefilled rate is worse than none.
    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 10_000)])).expect("preview");

    assert_eq!(p.amounts.tax, Money::ZERO);
    assert_eq!(p.amounts.total, m(10_000));
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("no sales-tax rate set")),
        "the user must be told why there is no tax: {:?}",
        p.warnings
    );
}

#[test]
fn a_row_without_a_category_blocks_saving() {
    let db = seeded_db();
    with_rate(&db, RATE);
    let mut d = draft(&db, &[("Groceries", 10_000)]);
    d.rows.push(SubtotalRow {
        category_id: None,
        subtotal: m(500),
        note: None,
    });

    let p = preview_subtotals(&db, &d).expect("preview");
    assert!(!p.can_save());
    assert!(
        p.blockers.iter().any(|b| b.contains("no category")),
        "{:?}",
        p.blockers
    );
    assert!(save_subtotals(&db, &d, NOW).is_err());
}

#[test]
fn an_empty_receipt_blocks_rather_than_saving_nothing() {
    let db = seeded_db();
    let mut d = draft(&db, &[]);
    d.rows.clear();
    let p = preview_subtotals(&db, &d).expect("preview");
    assert!(!p.can_save());
}

#[test]
fn previewing_writes_nothing() {
    let db = seeded_db();
    with_rate(&db, RATE);
    let before = common::row_counts(&db);
    preview_subtotals(&db, &draft(&db, &[("Groceries", 10_000)])).expect("preview");
    assert_eq!(common::row_counts(&db), before, "preview must not write");
}

#[test]
fn a_saved_receipt_round_trips_with_its_derived_tax() {
    let db = seeded_db();
    with_rate(&db, RATE);
    let d = draft(&db, &[("Groceries", 10_000), ("Gas", 5_000)]);

    let id = save_subtotals(&db, &d, NOW).expect("save");
    let txn = transactions::get(&db, id).expect("get");

    // $150.00 at 7.75% is $11.625, which rounds to $11.63.
    assert_eq!(txn.subtotal, m(15_000));
    assert_eq!(txn.tax, m(1_163));
    assert_eq!(txn.total, m(16_163));
    assert!(txn.tax_is_estimated);
    assert_eq!(txn.tax_rate_ppm, Some(RATE));
    assert_eq!(txn.merchant_name.as_deref(), Some("Costco"));
    assert_eq!(txn.splits.len(), 2);

    let stored: Money = txn
        .splits
        .iter()
        .map(|s| s.amount)
        .fold(Money::ZERO, |a, b| a.checked_add(b).expect("no overflow"));
    assert_eq!(
        stored, txn.total,
        "stored splits must sum to the stored total"
    );

    assert_eq!(
        transactions::list(&db, &TxnFilter::default())
            .expect("list")
            .len(),
        1
    );
    assert_clean(&db);
}

#[test]
fn a_single_row_receipt_teaches_the_store_its_category() {
    let db = seeded_db();
    with_rate(&db, RATE);
    // Target ships with no default precisely because it spans categories, so
    // it is the honest subject for "does one row teach it anything".
    let mut d = draft(&db, &[("Groceries", 4_200)]);
    d.merchant = Some("Target".to_string());

    save_subtotals(&db, &d, NOW).expect("save");

    let target = accounting_core::repo::merchants::find_by_name(&db, "Target")
        .expect("find")
        .expect("exists");
    assert_eq!(target.default_category_id, Some(cat(&db, "Groceries")));
    assert_eq!(target.last_used_at.as_deref(), Some(NOW));
    assert_clean(&db);
}
