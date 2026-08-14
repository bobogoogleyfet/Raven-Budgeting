//! The reduced rate on unprepared food.
//!
//! One rate per location is wrong in most US states. Utah taxes unprepared
//! food at a combined 3% and everything else at about 7.25%, so a real $7.28
//! grocery receipt owes $0.22 — not the $0.53 a single rate predicts. That is
//! a 140% overestimate on an ordinary trip, and it compounds every week.
//!
//! The case that matters most is the mixed basket: one Walmart run holding
//! groceries and shampoo needs both rates applied within the same receipt.

mod common;

use accounting_core::Money;
use accounting_core::repo::{categories, locations};
use accounting_core::service::entry::{SubtotalDraft, SubtotalRow, preview_subtotals};
use common::{cat, day, m, seeded_db};

/// Highland, UT: 7.25% general, 3% on unprepared food.
const GENERAL: u32 = 72_500;
const FOOD: u32 = 30_000;

fn utah_home(db: &accounting_core::Db) {
    let home = locations::find_by_name(db, "Home")
        .expect("find home")
        .expect("home is seeded");
    locations::set_rate(db, home.id, GENERAL).expect("set rate");
    locations::set_food_rate(db, home.id, Some(FOOD)).expect("set food rate");
}

fn draft(db: &accounting_core::Db, rows: &[(&str, i64)]) -> SubtotalDraft {
    SubtotalDraft {
        id: None,
        txn_date: day("2026-03-14"),
        merchant: Some("Macey's".to_string()),
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
        remember_defaults: false,
    }
}

#[test]
fn a_real_grocery_receipt_matches_to_the_cent() {
    let db = seeded_db();
    utah_home(&db);

    // An actual receipt: subtotal $7.28, tax $0.22, total $7.50.
    // At 3%: 7.28 * 0.03 = $0.2184, which rounds to $0.22.
    // At 7.25% it would have been $0.53 and a $7.81 total — off by $0.31.
    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 728)])).expect("preview");

    assert_eq!(p.amounts.subtotal, m(728));
    assert_eq!(
        p.amounts.tax,
        m(22),
        "groceries are taxed at the reduced rate"
    );
    assert_eq!(p.amounts.total, m(750));
}

#[test]
fn a_mixed_basket_applies_both_rates_within_one_receipt() {
    let db = seeded_db();
    utah_home(&db);

    // $50.00 of groceries at 3% is $1.50; $20.00 of shampoo at 7.25% is $1.45.
    let p = preview_subtotals(
        &db,
        &draft(&db, &[("Groceries", 5_000), ("Toiletries", 2_000)]),
    )
    .expect("preview");

    assert_eq!(p.plan.rows[0].tax, m(150), "groceries at 3%");
    assert_eq!(p.plan.rows[1].tax, m(145), "toiletries at 7.25%");
    assert_eq!(p.amounts.tax, m(295), "and the receipt's tax is their sum");
    assert_eq!(p.amounts.total, m(7_295));
}

#[test]
fn prepared_food_is_not_grocery_food() {
    let db = seeded_db();
    utah_home(&db);

    // Restaurants are prepared food and taxed in full — a distinction a single
    // "food" flag would get wrong in the user's favour and then under-report.
    let p = preview_subtotals(&db, &draft(&db, &[("Restaurants", 1_000)])).expect("preview");
    assert_eq!(
        p.amounts.tax,
        m(73),
        "$10.00 at 7.25% is $0.725, rounding to $0.73"
    );
}

#[test]
fn a_location_without_a_food_rate_taxes_groceries_in_full() {
    let db = seeded_db();
    let home = locations::find_by_name(&db, "Home").unwrap().unwrap();
    locations::set_rate(&db, home.id, GENERAL).unwrap();
    // No food rate: the correct reading for a state that has no reduced one.
    locations::set_food_rate(&db, home.id, None).unwrap();

    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 728)])).expect("preview");
    assert_eq!(p.amounts.tax, m(53), "$7.28 at 7.25% is $0.5278");
}

#[test]
fn the_seed_marks_groceries_as_food_and_restaurants_as_not() {
    let db = seeded_db();
    let by_name = |n: &str| {
        categories::list(&db)
            .expect("list")
            .into_iter()
            .find(|c| c.name == n)
            .unwrap_or_else(|| panic!("{n} is seeded"))
    };
    assert!(by_name("Groceries").uses_food_rate);
    assert!(!by_name("Restaurants").uses_food_rate);
    assert!(!by_name("Toiletries").uses_food_rate);
    assert!(!by_name("Gas").uses_food_rate);
}

#[test]
fn tax_is_computed_per_rate_bucket_not_per_line() {
    let db = seeded_db();
    utah_home(&db);

    // Three $33.33 toiletry rows, all at 7.25%. Rounding each line alone gives
    // $2.42 x 3 = $7.26; the register charges 7.25% of $99.99, which is
    // $7.2493 -> $7.25. The bucket is taxed as a whole and then apportioned,
    // so the total matches the till and the parts still sum to it exactly.
    let p = preview_subtotals(
        &db,
        &draft(
            &db,
            &[("Toiletries", 3_333), ("Clothing", 3_333), ("Gifts", 3_333)],
        ),
    )
    .expect("preview");

    assert_eq!(p.amounts.subtotal, m(9_999));
    assert_eq!(
        p.amounts.tax,
        m(725),
        "7.25% of $99.99, not three separate roundings"
    );

    let summed = p
        .plan
        .rows
        .iter()
        .map(|r| r.tax)
        .fold(Money::ZERO, |a, b| a.checked_add(b).expect("no overflow"));
    assert_eq!(
        summed, p.amounts.tax,
        "the parts must still sum to the whole"
    );
}

#[test]
fn each_category_reports_the_rate_it_will_be_taxed_at() {
    let db = seeded_db();
    utah_home(&db);

    let context = accounting_core::service::entry::entry_context(&db, None).expect("context");
    let rates = accounting_core::service::entry::category_rates(&db, &context).expect("rates");
    let rate_of = |name: &str| {
        let id = cat(&db, name);
        rates
            .iter()
            .find(|r| r.category_id == id)
            .map(|r| r.rate_ppm)
            .expect("every category has a rate")
    };

    // What the entry form labels each row with. The rate is a property of the
    // category, not of the store, so one figure shown against the shop is
    // wrong for at least one line of any mixed receipt.
    assert_eq!(rate_of("Groceries"), FOOD);
    assert_eq!(rate_of("Toiletries"), GENERAL);
    // Prepared food is taxed in full, which is the distinction a single "food"
    // flag would get wrong in the user's favour.
    assert_eq!(rate_of("Restaurants"), GENERAL);
}

#[test]
fn the_displayed_rate_is_the_rate_that_gets_charged() {
    let db = seeded_db();
    utah_home(&db);

    let context = accounting_core::service::entry::entry_context(&db, None).expect("context");
    let rates = accounting_core::service::entry::category_rates(&db, &context).expect("rates");
    let groceries = rates
        .iter()
        .find(|r| r.category_id == cat(&db, "Groceries"))
        .expect("groceries");

    // The screen and the arithmetic must not be two opinions. $7.28 taxed at
    // the rate the form advertises has to be the $0.22 the receipt shows.
    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 728)])).expect("preview");
    assert_eq!(p.applied_rate_ppm, Some(groceries.rate_ppm));
    assert_eq!(p.amounts.tax, m(22));
}

#[test]
fn the_preview_reports_the_rate_it_actually_applied() {
    let db = seeded_db();
    utah_home(&db);

    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 728)])).expect("preview");

    // The bug this closes: the preview reported the location's *general* rate
    // whatever the receipt was charged, so the app printed "Tax (7.25%) $0.22"
    // — a label contradicting the money beside it. It reads as broken
    // arithmetic, and was reported as the food rate not being applied at all.
    assert_eq!(p.amounts.tax, m(22));
    assert_eq!(p.applied_rate_ppm, Some(FOOD));
    assert_eq!(
        p.tax_rate_ppm, GENERAL,
        "the store's general rate is still reported, for the store line"
    );
}

#[test]
fn a_receipt_straddling_two_rates_claims_neither() {
    let db = seeded_db();
    utah_home(&db);

    let p = preview_subtotals(
        &db,
        &draft(&db, &[("Groceries", 5_000), ("Toiletries", 2_000)]),
    )
    .expect("preview");

    // There is no single rate here, and printing either would label the whole
    // tax figure with a percentage true of only part of the receipt.
    assert_eq!(p.applied_rate_ppm, None);
}

#[test]
fn an_untaxed_row_does_not_count_as_a_second_rate() {
    let db = seeded_db();
    utah_home(&db);

    // A half-filled form: one real grocery line and an empty second row. That
    // is one rate, not two — the entry form must not flip to "mixed rates"
    // while somebody is still typing.
    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 728), ("Toiletries", 0)]))
        .expect("preview");

    assert_eq!(p.applied_rate_ppm, Some(FOOD));
}
