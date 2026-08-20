//! The elevated rate on prepared food.
//!
//! Utah counties may levy a Restaurant Tax of up to 1% on prepared food (Utah
//! Code 59-12-603) on top of the combined sales rate. The app modelled food
//! taxed *below* the general rate from M2 and had no way to express the
//! opposite, so `uses_food_rate = 0` on Restaurants was read as "the general
//! rate" when it only ever meant "not the reduced rate".
//!
//! The consequence was arithmetic, not philosophy: every restaurant receipt
//! came out 1% light and reconciled against nothing.

mod common;

use accounting_core::repo::{categories, locations};
use accounting_core::service::entry::{SubtotalDraft, SubtotalRow, preview_subtotals};
use common::{cat, day, m, seeded_db};

/// Highland, UT, as the state publishes it for 2026Q3.
const GENERAL: u32 = 74_500;
const FOOD: u32 = 30_000;
const RESTAURANT: u32 = 84_500;

fn highland(db: &accounting_core::Db) {
    let home = locations::find_by_name(db, "Home")
        .expect("find home")
        .expect("home is seeded");
    locations::set_rate(db, home.id, GENERAL).expect("set rate");
    locations::set_food_rate(db, home.id, Some(FOOD)).expect("set food rate");
    locations::set_restaurant_rate(db, home.id, Some(RESTAURANT)).expect("set restaurant rate");
}

fn draft(db: &accounting_core::Db, rows: &[(&str, i64)]) -> SubtotalDraft {
    SubtotalDraft {
        id: None,
        txn_date: day("2026-08-20"),
        merchant: Some("Cubby's".to_string()),
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
fn a_real_restaurant_receipt_matches_to_the_cent() {
    let db = seeded_db();
    highland(&db);

    // The receipt that found the bug: a $13.59 subtotal in Highland.
    // At 8.45%: 13.59 * 0.0845 = $1.148355, which rounds to $1.15.
    // At the general 7.45% it came out $1.01 — fourteen cents light, and a
    // total that matched no receipt.
    let p = preview_subtotals(&db, &draft(&db, &[("Restaurants", 1_359)])).expect("preview");

    assert_eq!(p.amounts.subtotal, m(1_359));
    assert_eq!(
        p.amounts.tax,
        m(115),
        "prepared food is taxed at the county's restaurant rate"
    );
    assert_eq!(p.amounts.total, m(1_474));
}

#[test]
fn all_three_rates_can_apply_within_one_receipt() {
    let db = seeded_db();
    highland(&db);

    // A single stop that buys groceries, a shirt and a sandwich. $100 of
    // groceries at 3% is $3.00; $100 of clothing at 7.45% is $7.45; $100 of
    // prepared food at 8.45% is $8.45. One rate for the receipt cannot be
    // right for more than one of them.
    let p = preview_subtotals(
        &db,
        &draft(
            &db,
            &[
                ("Groceries", 10_000),
                ("Clothing", 10_000),
                ("Restaurants", 10_000),
            ],
        ),
    )
    .expect("preview");

    assert_eq!(p.plan.rows[0].tax, m(300), "groceries at 3%");
    assert_eq!(p.plan.rows[1].tax, m(745), "clothing at 7.45%");
    assert_eq!(p.plan.rows[2].tax, m(845), "prepared food at 8.45%");
    assert_eq!(
        p.amounts.tax,
        m(1_890),
        "and the receipt's tax is their sum"
    );
}

#[test]
fn a_county_with_no_restaurant_tax_falls_back_to_the_general_rate() {
    let db = seeded_db();
    let home = locations::find_by_name(&db, "Home")
        .expect("find home")
        .expect("home is seeded");
    locations::set_rate(&db, home.id, GENERAL).expect("set rate");
    // No restaurant rate published here. That must mean "taxed like everything
    // else" and never "untaxed" — a NULL read as zero would silently stop
    // charging tax on every meal.
    locations::set_restaurant_rate(&db, home.id, None).expect("clear restaurant rate");

    let p = preview_subtotals(&db, &draft(&db, &[("Restaurants", 1_359)])).expect("preview");

    // 13.59 * 0.0745 = $1.012455 -> $1.01.
    assert_eq!(p.amounts.tax, m(101), "the general rate, not zero");
}

#[test]
fn the_seed_marks_restaurants_as_prepared_food_and_groceries_as_not() {
    let db = seeded_db();
    let all = categories::list_all(&db).expect("list categories");
    let by_name = |name: &str| {
        all.iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} is seeded"))
    };

    let restaurants = by_name("Restaurants");
    assert!(restaurants.uses_restaurant_rate);
    // Mutually exclusive: prepared food is not unprepared food, and a category
    // claiming both would have no meaningful rate.
    assert!(!restaurants.uses_food_rate);

    let groceries = by_name("Groceries");
    assert!(groceries.uses_food_rate);
    assert!(!groceries.uses_restaurant_rate);
}

#[test]
fn upgrading_an_install_backfills_the_prepared_food_rates() {
    // The path that actually reaches an existing user. Migrating adds the
    // column but cannot fill it, and `refresh_due` is false for the rest of
    // the quarter — so without a backfill an upgraded install would hold a
    // complete-looking rate table that taxes every restaurant receipt at the
    // general rate, forever.
    use accounting_core::repo::tax_areas;
    use accounting_core::service::rates as rates_service;

    let db = seeded_db();
    rates_service::ensure_installed(&db).expect("install bundled");
    assert!(tax_areas::has_restaurant_rates(&db).expect("check"));

    // Rewind to what a pre-upgrade database looks like: every other column
    // populated, this one NULL on all 389 rows — and no backfill marker,
    // because the release that writes it had not shipped yet.
    db.conn()
        .execute("UPDATE tax_area SET restaurant_rate_ppm = NULL", [])
        .expect("simulate a pre-upgrade table");
    db.conn()
        .execute(
            "DELETE FROM app_setting WHERE key = 'rates.prepared_food_backfill_generation'",
            [],
        )
        .expect("simulate a pre-upgrade marker");
    assert!(!tax_areas::has_restaurant_rates(&db).expect("check"));

    assert!(
        rates_service::ensure_installed(&db).expect("backfill"),
        "a table with no prepared-food rates must be replaced, not kept"
    );
    assert!(tax_areas::has_restaurant_rates(&db).expect("check"));

    // And it is idempotent: a table that already has them is left alone.
    assert!(
        !rates_service::ensure_installed(&db).expect("second call"),
        "a healthy table must not be reinstalled on every launch"
    );
}

#[test]
fn a_table_that_genuinely_has_no_restaurant_tax_is_not_reinstalled_every_launch() {
    // The backfill must key off a one-time marker, not off the data. "No row
    // carries a prepared-food rate" is also true of a state that levies no
    // restaurant tax anywhere, and of a publication whose prepared-food column
    // failed to parse. Deciding from the data would reinstall on every launch
    // — discarding a mid-quarter revision the user had downloaded and resetting
    // their followed locations to compiled-in figures, indefinitely.
    use accounting_core::repo::tax_areas;
    use accounting_core::service::rates as rates_service;

    let db = seeded_db();
    rates_service::ensure_installed(&db).expect("install bundled");

    // A published table for this quarter in which no jurisdiction levies one.
    db.conn()
        .execute("UPDATE tax_area SET restaurant_rate_ppm = NULL", [])
        .expect("clear the rates");
    db.conn()
        .execute(
            "UPDATE tax_area SET name = 'Locally Revised' WHERE code = '25-099'",
            [],
        )
        .expect("mark the table as the user's own");

    for launch in 1..=3 {
        assert!(
            !rates_service::ensure_installed(&db).expect("launch"),
            "launch {launch} reinstalled a table it should have left alone"
        );
    }

    let name: String = db
        .conn()
        .query_row("SELECT name FROM tax_area WHERE code = '25-099'", [], |r| {
            r.get(0)
        })
        .expect("read back");
    assert_eq!(
        name, "Locally Revised",
        "the downloaded table was clobbered by the bundled one"
    );
    assert!(!tax_areas::has_restaurant_rates(&db).expect("check"));
}

#[test]
fn a_revised_restaurant_rate_reaches_a_location_that_follows_its_jurisdiction() {
    // The leg that makes a download matter. Without it the published table
    // could carry a new restaurant rate every quarter and every receipt would
    // still be taxed at whatever was copied into `location` the day it was
    // created — the refresh would be decoration.
    use accounting_core::repo::{locations, tax_areas};
    use accounting_core::service::rates as rates_service;

    let db = seeded_db();
    rates_service::ensure_installed(&db).expect("install bundled");

    let home = locations::find_by_name(&db, "Home")
        .expect("find home")
        .expect("home is seeded");
    let highland = tax_areas::find_by_name(&db, "Highland")
        .expect("find area")
        .expect("Highland is published");
    assert_eq!(highland.restaurant_rate_ppm, Some(RESTAURANT));

    locations::adopt_area(
        &db,
        home.id,
        &highland.code,
        highland.combined_rate_ppm,
        highland.food_rate_ppm,
        highland.restaurant_rate_ppm,
    )
    .expect("adopt");
    let followed = locations::get(&db, home.id).expect("reload");
    assert_eq!(followed.restaurant_rate_ppm, Some(RESTAURANT));

    // The county drops its restaurant tax to 0.5%, and the state republishes.
    db.conn()
        .execute(
            "UPDATE tax_area SET restaurant_rate_ppm = 79500 WHERE code = ?1",
            [&highland.code],
        )
        .expect("revise the publication");
    let moved = locations::refresh_followed_rates(&db).expect("refresh");
    assert_eq!(moved, 1, "the location follows the jurisdiction");

    let after = locations::get(&db, home.id).expect("reload");
    assert_eq!(after.restaurant_rate_ppm, Some(79_500));

    // And a hand-typed location is left alone, however many quarters pass: a
    // figure read off a receipt outranks a published estimate.
    let typed = locations::get_or_create(&db, "Somewhere Else", GENERAL).expect("create");
    locations::set_restaurant_rate(&db, typed.id, Some(90_000)).expect("type a rate");
    locations::refresh_followed_rates(&db).expect("refresh again");
    assert_eq!(
        locations::get(&db, typed.id)
            .expect("reload")
            .restaurant_rate_ppm,
        Some(90_000),
        "a hand-typed rate must not be revised by a download"
    );
}
