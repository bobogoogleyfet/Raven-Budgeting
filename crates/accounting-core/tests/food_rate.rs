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
                tax: None,
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
fn a_bill_is_not_a_subtotal_and_gets_no_tax() {
    let db = seeded_db();
    utah_home(&db);

    // $1,500 of rent used to become $1,608.75 — a tax nobody charged, on a
    // payment that matched no statement anywhere. Residential rent is outside
    // Utah's tax base entirely, and a rent cheque has no subtotal to tax.
    let p = preview_subtotals(&db, &draft(&db, &[("Rent", 150_000)])).expect("preview");

    assert_eq!(p.amounts.tax, Money::ZERO);
    assert_eq!(p.amounts.total, m(150_000), "you paid what you paid");
}

#[test]
fn an_untaxed_receipt_does_not_blame_the_location() {
    let db = seeded_db();
    utah_home(&db);

    // Highland charges 7.25%. Rent is simply not sales-taxed, so "this
    // location has no tax rate set" would send the user off to fix a setting
    // that is already correct.
    let p = preview_subtotals(&db, &draft(&db, &[("Rent", 150_000)])).expect("preview");

    assert_eq!(p.amounts.tax, Money::ZERO);
    assert!(
        !p.warnings.iter().any(|w| w.contains("rate set")),
        "nothing is misconfigured here: {:?}",
        p.warnings
    );
}

#[test]
fn a_missing_rate_is_still_reported_when_it_would_have_mattered() {
    let db = seeded_db();
    // The seeded Home starts at 0%, and groceries *are* taxable, so the
    // missing rate is the reason this receipt has no tax — and worth saying.
    let p = preview_subtotals(&db, &draft(&db, &[("Groceries", 10_000)])).expect("preview");

    assert!(
        p.warnings.iter().any(|w| w.contains("no sales-tax rate set")),
        "a real setup problem must still surface: {:?}",
        p.warnings
    );
}

#[test]
fn the_seed_knows_which_categories_are_billed_rather_than_shopped() {
    let db = seeded_db();
    let taxable = |name: &str| {
        categories::list(&db)
            .expect("list")
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} is seeded"))
            .taxable
    };

    // Not taxed by Utah at all: rent is outside the base, broadband is barred
    // from state taxation by federal law, insurance carries a premium tax the
    // insurer pays, and motor fuel pays a per-gallon excise instead.
    assert!(!taxable("Rent"));
    assert!(!taxable("Internet"));
    assert!(!taxable("Car Insurance"));
    assert!(!taxable("Health Insurance"));
    assert!(!taxable("Gas"));
    // Taxed, but the figure on the bill already includes it — along with a
    // municipal energy tax, 911 fees, or transient room taxes that no sales
    // rate could reconstruct.
    assert!(!taxable("Electric"));
    assert!(!taxable("Phone"));
    assert!(!taxable("Travel"));
    // Moving money is not buying anything.
    assert!(!taxable("Transfer"));
    assert!(!taxable("Emergency Fund"));
    // The fallback stands in for an ordinary purchase — it is whatever the
    // receipt turns out to be, and most receipts are taxed.
    assert!(taxable("Uncategorized"));
    // Ordinary retail: a subtotal with tax added at the till.
    assert!(taxable("Groceries"));
    assert!(taxable("Restaurants"));
    assert!(taxable("Household"));
    assert!(taxable("Clothing"));
    // Utah taxes repair labour as well as parts, unlike most states.
    assert!(taxable("Car Repair"));
}

#[test]
fn a_mixed_receipt_taxes_only_the_part_that_is_taxed() {
    let db = seeded_db();
    utah_home(&db);

    // One trip to a big-box store: groceries at 3%, shampoo at 7.25%, and a
    // tank of fuel that is taxed at neither.
    let p = preview_subtotals(
        &db,
        &draft(
            &db,
            &[("Groceries", 5_000), ("Toiletries", 2_000), ("Gas", 4_000)],
        ),
    )
    .expect("preview");

    assert_eq!(p.plan.rows[0].tax, m(150), "groceries at 3%");
    assert_eq!(p.plan.rows[1].tax, m(145), "toiletries at 7.25%");
    assert_eq!(p.plan.rows[2].tax, Money::ZERO, "fuel is not sales-taxed");
    assert_eq!(p.amounts.tax, m(295));
    assert_eq!(p.amounts.total, m(11_295));
}

#[test]
fn a_tax_read_off_the_receipt_beats_the_rate() {
    let db = seeded_db();
    utah_home(&db);

    // The register charged $0.25 where the published rate predicts $0.22. The
    // receipt is evidence and the rate is an estimate, so the receipt wins —
    // and the total moves with it rather than being recomputed behind it.
    let mut d = draft(&db, &[("Groceries", 728)]);
    d.rows[0].tax = Some(m(25));

    let p = preview_subtotals(&db, &d).expect("preview");

    assert_eq!(p.amounts.tax, m(25));
    assert_eq!(p.amounts.total, m(753));
    assert_eq!(p.plan.rows[0].tax, m(25));
    assert!(
        !p.amounts.tax_is_estimated,
        "a figure the user supplied is not an estimate"
    );
}

#[test]
fn one_corrected_row_does_not_disturb_the_others() {
    let db = seeded_db();
    utah_home(&db);

    // Groceries corrected by hand, toiletries left to the rate. The corrected
    // row must not join the bucket it no longer belongs to — apportioning a
    // bucket total across a row whose tax is already known would round the
    // other rows against money that is not theirs.
    let mut d = draft(&db, &[("Groceries", 5_000), ("Toiletries", 2_000)]);
    d.rows[0].tax = Some(m(160));

    let p = preview_subtotals(&db, &d).expect("preview");

    assert_eq!(p.plan.rows[0].tax, m(160), "the correction stands");
    assert_eq!(p.plan.rows[1].tax, m(145), "$20.00 at 7.25%, untouched");
    assert_eq!(p.amounts.tax, m(305));
    assert!(
        p.amounts.tax_is_estimated,
        "one row is still the app's arithmetic, so the total is still an estimate"
    );
}

#[test]
fn one_category_cannot_take_two_rows() {
    let db = seeded_db();
    utah_home(&db);

    let p = preview_subtotals(
        &db,
        &draft(&db, &[("Groceries", 1_000), ("Groceries", 2_000)]),
    )
    .expect("preview");

    // Not a receipt with two lines — one line entered twice. Every report adds
    // them back together, so the split list would just hold two rows nobody
    // could tell apart.
    assert!(!p.can_save(), "a duplicated category must block the save");
    assert!(
        p.blockers.iter().any(|b| b.contains("Groceries")),
        "the blocker must name the category; got {:?}",
        p.blockers
    );
}

#[test]
fn different_categories_are_fine_on_one_receipt() {
    let db = seeded_db();
    utah_home(&db);

    let p = preview_subtotals(
        &db,
        &draft(&db, &[("Groceries", 1_000), ("Toiletries", 2_000)]),
    )
    .expect("preview");

    assert!(p.can_save(), "unexpected blockers: {:?}", p.blockers);
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
