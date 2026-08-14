//! Giving an existing database the food rate its area implies.
//!
//! Migration 002 can add `location.food_rate_ppm`, but it cannot know that
//! "Provo, UT" has one — the area table is Rust, not SQL. Every install that
//! predates 002 therefore comes through the migration with the column NULL and
//! the first-run prompt already answered, so nothing would ever fill it in.
//! That user keeps overestimating grocery tax by 140% with nothing to notice
//! it by, which is the whole point of the reduced rate.

mod common;

use accounting_core::adopt_area_food_rates;
use accounting_core::repo::locations;
use common::seeded_db;

/// The state migration 002 leaves behind: a named home, a general rate, and no
/// food rate, because nothing existed that could supply one.
fn pre_migration_home(db: &accounting_core::Db, name: &str, rate: u32) -> accounting_core::Location {
    let home = locations::find_by_name(db, "Home")
        .expect("find home")
        .expect("home is seeded");
    locations::rename(db, home.id, name).expect("rename");
    locations::set_rate(db, home.id, rate).expect("set rate");
    locations::get(db, home.id).expect("get")
}

#[test]
fn a_known_area_gets_its_reduced_rate() {
    let db = seeded_db();
    let home = pre_migration_home(&db, "Provo, UT", 72_500);
    assert_eq!(home.food_rate_ppm, None, "the gap this exists to close");

    assert_eq!(adopt_area_food_rates(&db).expect("adopt"), 1);

    let after = locations::get(&db, home.id).expect("get");
    assert_eq!(after.food_rate_ppm, Some(30_000), "Utah's combined 3%");
    assert_eq!(after.tax_rate_ppm, 72_500, "the general rate is untouched");
}

#[test]
fn an_area_nobody_has_heard_of_is_left_alone() {
    let db = seeded_db();
    // Guessing here would invent a reduced rate for a state that may not have
    // one. NULL already says "food is taxed like everything else".
    let home = pre_migration_home(&db, "Boise, ID", 60_000);

    assert_eq!(adopt_area_food_rates(&db).expect("adopt"), 0);
    assert_eq!(
        locations::get(&db, home.id).expect("get").food_rate_ppm,
        None
    );
}

#[test]
fn a_rate_already_set_is_never_overwritten() {
    let db = seeded_db();
    let home = pre_migration_home(&db, "Provo, UT", 72_500);
    // Someone who read their receipt and corrected the figure outranks the
    // table: the receipt always wins, including over a later release of it.
    locations::set_food_rate(&db, home.id, Some(17_500)).expect("set food rate");

    assert_eq!(adopt_area_food_rates(&db).expect("adopt"), 0);
    assert_eq!(
        locations::get(&db, home.id).expect("get").food_rate_ppm,
        Some(17_500),
        "the user's own figure survives"
    );
}

#[test]
fn running_it_twice_changes_nothing_the_second_time() {
    let db = seeded_db();
    pre_migration_home(&db, "Provo, UT", 72_500);

    // It runs on every launch, so a second pass reporting work would mean the
    // first pass did not stick.
    assert_eq!(adopt_area_food_rates(&db).expect("first"), 1);
    assert_eq!(adopt_area_food_rates(&db).expect("second"), 0);
}

#[test]
fn every_stale_location_is_reached_not_just_home() {
    let db = seeded_db();
    pre_migration_home(&db, "Provo, UT", 72_500);
    // A receipt entered against a second location is taxed by *that* row, so
    // fixing only the home would leave the trip to Moab still overestimating.
    let away = locations::get_or_create(&db, "Moab, UT", 88_500).expect("create");

    assert_eq!(adopt_area_food_rates(&db).expect("adopt"), 2);
    assert_eq!(
        locations::get(&db, away.id).expect("get").food_rate_ppm,
        Some(30_000)
    );
}

#[test]
fn the_area_name_is_matched_the_way_a_user_typed_it() {
    let db = seeded_db();
    // The picker commits whatever is in the box, and a location can also be
    // named by hand from the CLI. Case and stray space are not a different
    // city, and treating them as one would silently skip the backfill.
    let home = pre_migration_home(&db, "  provo, ut  ", 72_500);

    assert_eq!(adopt_area_food_rates(&db).expect("adopt"), 1);
    assert_eq!(
        locations::get(&db, home.id).expect("get").food_rate_ppm,
        Some(30_000)
    );
}
