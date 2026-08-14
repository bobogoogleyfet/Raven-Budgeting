//! Installing the published rate table, and what it does to the user's rows.
//!
//! Two things have to be true at once for a quarterly refresh to be worth
//! having: the table has to update, and the *receipt* has to change with it. A
//! table that refreshes while every location stays frozen at the rate it was
//! created with is a download that does nothing, which is the failure these
//! tests are mostly about.

mod common;

use accounting_core::repo::{locations, tax_areas};
use accounting_core::service::rates as rates_service;
use accounting_core::{Db, adopt_published_areas};
use common::seeded_db;

/// A seeded database with the bundled publication installed, which is the
/// state every real install reaches on its first launch.
fn db_with_rates() -> Db {
    let db = seeded_db();
    rates_service::ensure_installed(&db).expect("install bundled rates");
    db
}

/// The bundled publication, relabelled as the next quarter — the shape a real
/// refresh arrives in. The stated effective date moves with the quarter,
/// because a file whose header disagrees with its label is exactly what
/// `install` refuses.
fn next_quarters_publication() -> accounting_core::rates::RateTable {
    let mut table = accounting_core::rates::bundled();
    table.quarter = accounting_core::Quarter::new(2026, 4).expect("valid");
    table.effective_on = Some(table.quarter.effective_on());
    table
}

/// A location as it looked before the rate table existed: a name, a rate
/// somebody's hand-written list supplied, and nothing saying where it came
/// from.
fn legacy_location(db: &Db, name: &str, rate: u32) -> accounting_core::LocationId {
    let home = locations::find_by_name(db, "Home")
        .expect("find home")
        .expect("home is seeded");
    locations::rename(db, home.id, name).expect("rename");
    locations::set_rate(db, home.id, rate).expect("set rate");
    home.id
}

#[test]
fn the_bundled_table_installs_on_a_fresh_database() {
    let db = seeded_db();
    assert_eq!(tax_areas::count(&db).expect("count"), 0);

    assert!(rates_service::ensure_installed(&db).expect("install"));

    // A first run may well be in a checkout queue on a phone that has never
    // reached the network, and it still has to tax a receipt.
    assert!(tax_areas::count(&db).expect("count") > 200);
    let release = tax_areas::release(&db).expect("release").expect("installed");
    assert_eq!(release.quarter, "2026Q3");
}

#[test]
fn installing_again_does_not_undo_a_newer_publication() {
    let db = db_with_rates();
    // Pretend a download has moved this database ahead of the binary.
    tax_areas::record_release(&db, "2027Q1", "2027-01-01", "https://example.test/27q1")
        .expect("record");

    assert!(!rates_service::ensure_installed(&db).expect("install"));
    assert_eq!(
        tax_areas::release(&db)
            .expect("release")
            .expect("installed")
            .quarter,
        "2027Q1",
        "the bundled table dragged a newer one backwards"
    );
}

#[test]
fn special_districts_are_stored_but_not_offered_as_places() {
    let db = db_with_rates();
    let all = tax_areas::list(&db).expect("list");
    let places = tax_areas::list_places(&db).expect("places");

    assert!(
        all.iter().any(|a| a.name.starts_with("UIPA ")),
        "the stored table should be a faithful copy of the publication"
    );
    assert!(
        !places.iter().any(|a| a.name.starts_with("UIPA ")),
        "an inland-port district is not an answer to `where do you shop`"
    );
    assert!(places.iter().any(|a| a.name == "Provo"));
}

#[test]
fn a_legacy_location_is_linked_to_its_published_jurisdiction() {
    let db = db_with_rates();
    // 7.25% is what the old hand-written list said Highland charged. The state
    // publishes 7.45%: the list was right when it was written and the rate
    // moved afterwards, which is the whole reason for downloading them.
    let id = legacy_location(&db, "Highland, UT", 72_500);

    assert_eq!(adopt_published_areas(&db).expect("adopt"), 1);

    let after = locations::get(&db, id).expect("get");
    assert_eq!(after.area_code.as_deref(), Some("25-099"));
    assert_eq!(after.tax_rate_ppm, 74_500);
    assert_eq!(after.food_rate_ppm, Some(30_000));
}

#[test]
fn a_location_outside_the_publication_is_left_alone() {
    let db = db_with_rates();
    let id = legacy_location(&db, "Boise, ID", 60_000);

    assert_eq!(adopt_published_areas(&db).expect("adopt"), 0);

    let after = locations::get(&db, id).expect("get");
    assert_eq!(after.area_code, None, "there is nothing to link it to");
    assert_eq!(after.tax_rate_ppm, 60_000, "the typed rate stands");
}

#[test]
fn linking_happens_once_and_never_undoes_a_correction() {
    let db = db_with_rates();
    let id = legacy_location(&db, "Provo, UT", 72_500);
    assert_eq!(adopt_published_areas(&db).expect("first"), 1);

    // Somebody reads the real figure off a receipt and types it. `set_rate`
    // clears the link, which is what says "stop following the publication".
    locations::set_rate(&db, id, 76_000).expect("correct the rate");

    // A second pass must not helpfully re-link it. If it did, the correction
    // would be undone on every launch with nothing on screen to explain it.
    assert_eq!(adopt_published_areas(&db).expect("second"), 0);
    let after = locations::get(&db, id).expect("get");
    assert_eq!(after.tax_rate_ppm, 76_000, "the correction was overwritten");
    assert_eq!(after.area_code, None);
}

#[test]
fn a_new_publication_moves_the_locations_that_follow_one() {
    let db = db_with_rates();
    let id = legacy_location(&db, "Provo, UT", 72_500);
    adopt_published_areas(&db).expect("adopt");
    let followed = locations::get(&db, id).expect("get").tax_rate_ppm;

    // The quarter turns and Provo goes up a quarter of a percent.
    let mut table = next_quarters_publication();
    for area in &mut table.areas {
        if area.code == "25-090" {
            area.combined_rate_ppm = followed + 2_500;
        }
    }
    let installed = rates_service::install(&db, &table).expect("install");

    assert_eq!(installed.locations_updated, 1);
    assert_eq!(
        locations::get(&db, id).expect("get").tax_rate_ppm,
        followed + 2_500,
        "the refresh never reached the location, so it changed nothing"
    );
}

#[test]
fn a_new_publication_leaves_a_hand_typed_rate_alone() {
    let db = db_with_rates();
    let id = legacy_location(&db, "Cheyenne, WY", 60_000);
    adopt_published_areas(&db).expect("adopt");

    let mut table = next_quarters_publication();
    for area in &mut table.areas {
        area.combined_rate_ppm = 99_000;
    }
    let installed = rates_service::install(&db, &table).expect("install");

    assert_eq!(installed.locations_updated, 0);
    assert_eq!(locations::get(&db, id).expect("get").tax_rate_ppm, 60_000);
}

#[test]
fn a_dissolved_jurisdiction_leaves_the_last_known_rate_in_place() {
    let db = db_with_rates();
    let id = legacy_location(&db, "Provo, UT", 72_500);
    adopt_published_areas(&db).expect("adopt");
    let known = locations::get(&db, id).expect("get").tax_rate_ppm;

    // The table is replaced wholesale every quarter, and this one no longer
    // carries Provo. The location must keep charging what it last knew rather
    // than falling to zero — a receipt taxed at 0% is wrong in a way nothing
    // downstream would flag.
    let mut table = next_quarters_publication();
    table.areas.retain(|a| a.code != "25-090");
    rates_service::install(&db, &table).expect("install");

    let after = locations::get(&db, id).expect("get");
    assert_eq!(after.tax_rate_ppm, known);
    assert_eq!(
        after.area_code.as_deref(),
        Some("25-090"),
        "the link stays, so the jurisdiction reappearing picks up again"
    );
}

#[test]
fn every_location_is_reached_not_just_home() {
    let db = db_with_rates();
    legacy_location(&db, "Provo, UT", 72_500);
    // A receipt entered against a second location is taxed by that row, so
    // converting only the home would leave the trip to Moab frozen.
    let away = locations::get_or_create(&db, "Moab", 88_500).expect("create");

    assert_eq!(adopt_published_areas(&db).expect("adopt"), 2);
    assert_eq!(
        locations::get(&db, away.id).expect("get").tax_rate_ppm,
        93_500,
        "Moab's published Q3 2026 rate"
    );
}
