//! The change log, as maintained by the schema's own triggers.
//!
//! These are worth testing directly rather than through the sync engine: a
//! trigger that quietly fails to fire produces a device that syncs
//! successfully and is missing data, which is the worst shape a bug can take
//! here. Every assertion below is about what the *database* records when
//! ordinary application code writes a row.

mod common;

use accounting_core::Db;
use accounting_core::repo::{categories, locations, merchants, transactions};
use common::{amounts, day, seeded_db, split, txn};

fn device(db: &Db, name: &str) {
    db.conn()
        .execute("UPDATE sync_clock SET device = ?1 WHERE id = 1", [name])
        .expect("name this device");
}

/// Every logged change, oldest first.
fn changes(db: &Db) -> Vec<(String, String, String)> {
    let mut stmt = db
        .conn()
        .prepare("SELECT entity, entity_uuid, hlc FROM sync_change ORDER BY hlc")
        .expect("prepare");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect")
}

fn changes_to(db: &Db, entity: &str) -> Vec<String> {
    changes(db)
        .into_iter()
        .filter(|(e, _, _)| e == entity)
        .map(|(_, uuid, _)| uuid)
        .collect()
}

#[test]
fn writing_a_row_records_it_without_the_write_path_knowing() {
    // The whole reason this is a trigger. `merchants::get_or_create` says
    // nothing about syncing, and does not have to.
    let db = seeded_db();
    device(&db, "laptop");
    let before = changes(&db).len();

    let made = merchants::get_or_create(&db, "Leatherby's").expect("create");

    let logged = changes_to(&db, "merchant");
    assert!(
        logged.contains(&made.uuid),
        "the new merchant should be in the log"
    );
    assert!(changes(&db).len() > before);
}

#[test]
fn editing_the_same_row_twice_leaves_one_row_to_send() {
    // State-based, not a replay: a bundle carries what a thing *is* now, so a
    // category renamed five times is still one thing to send.
    let db = seeded_db();
    device(&db, "laptop");
    let home = locations::find_by_name(&db, "Home")
        .expect("find")
        .expect("seeded");

    for rate in [70_000, 72_500, 74_500] {
        locations::set_rate(&db, home.id, rate).expect("set rate");
    }

    let logged: Vec<String> = changes(&db)
        .into_iter()
        .filter(|(e, u, _)| e == "location" && u == &home.uuid)
        .map(|(_, _, hlc)| hlc)
        .collect();
    assert_eq!(logged.len(), 1, "one row per entity, not one per edit");
}

#[test]
fn a_later_edit_gets_a_later_timestamp() {
    let db = seeded_db();
    device(&db, "laptop");

    let first = merchants::get_or_create(&db, "Novel One").expect("create");
    let hlc_first = hlc_of(&db, "merchant", &first.uuid);
    let second = merchants::get_or_create(&db, "Novel Two").expect("create");
    let hlc_second = hlc_of(&db, "merchant", &second.uuid);

    // Text order is clock order, which is what the fixed-width hex is for.
    assert!(
        hlc_second > hlc_first,
        "{hlc_second} should sort after {hlc_first}"
    );
}

#[test]
fn writes_inside_one_millisecond_still_order() {
    // Several rows written in one SQLite transaction land in the same
    // millisecond routinely. Without the counter they would be indistinguishable
    // and a merge would have to guess.
    let db = seeded_db();
    device(&db, "laptop");

    let mut stamps = Vec::new();
    for name in ["Novel A", "Novel B", "Novel C", "Novel D", "Novel E"] {
        let made = merchants::get_or_create(&db, name).expect("create");
        stamps.push(hlc_of(&db, "merchant", &made.uuid));
    }

    let mut sorted = stamps.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), stamps.len(), "every stamp is distinct");
    assert_eq!(
        sorted, stamps,
        "and they are in the order they were written"
    );
}

#[test]
fn the_device_is_part_of_every_stamp() {
    // It is what breaks the last tie between two devices editing in the same
    // millisecond, so both sides pick the same winner.
    let db = seeded_db();
    device(&db, "tanner-phone");
    // A name the seed does not already carry: `get_or_create` finding an
    // existing row writes nothing, and so records nothing.
    let made = merchants::get_or_create(&db, "Bruges Waffles").expect("create");

    assert!(
        hlc_of(&db, "merchant", &made.uuid).ends_with("-tanner-phone"),
        "the stamp should name the device that wrote it"
    );
}

#[test]
fn a_published_rate_table_is_not_a_change_to_send() {
    // 389 jurisdictions, identical on every device that has refreshed, and
    // replaced wholesale every quarter. Syncing them would be the largest thing
    // in every bundle and would fix nothing.
    let db = seeded_db();
    device(&db, "laptop");
    let before = changes(&db).len();

    db.conn()
        .execute(
            "UPDATE tax_area SET combined_rate_ppm = combined_rate_ppm + 1",
            [],
        )
        .expect("revise the published table");

    assert_eq!(
        changes(&db).len(),
        before,
        "the published rate table is not user data"
    );
}

#[test]
fn a_location_edit_is_recorded() {
    let db = seeded_db();
    device(&db, "laptop");
    let home = locations::find_by_name(&db, "Home")
        .expect("find")
        .expect("seeded");

    locations::set_rate(&db, home.id, 74_500).expect("set rate");

    let logged = changes_to(&db, "location");
    assert!(logged.contains(&home.uuid), "the rate change should travel");
}

fn hlc_of(db: &Db, entity: &str, uuid: &str) -> String {
    db.conn()
        .query_row(
            "SELECT hlc FROM sync_change WHERE entity = ?1 AND entity_uuid = ?2",
            rusqlite::params![entity, uuid],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("no change logged for {entity} {uuid}: {e}"))
}

#[test]
fn a_receipt_travels_as_one_thing_however_many_splits_it_has() {
    // The aggregate is the unit. Logging splits separately would record a dozen
    // changes for one edited receipt, and would invite a merge that took the
    // splits from one device and the transaction from the other — a receipt
    // whose splits do not sum to its total, which the schema's own CHECKs would
    // refuse.
    let db = seeded_db();
    device(&db, "laptop");
    let groceries = categories::resolve(&db, "Groceries").expect("resolve").id;
    let household = categories::resolve(&db, "Household").expect("resolve").id;

    let id = transactions::save(
        &db,
        &txn(
            day("2026-08-20"),
            amounts(5_000, 0),
            vec![split(groceries, 3_000, 0), split(household, 2_000, 0)],
        ),
    )
    .expect("save");

    let logged = changes(&db);
    assert!(
        logged.iter().all(|(entity, _, _)| entity != "txn_split"),
        "splits are not entities of their own"
    );
    assert!(
        logged.iter().all(|(entity, _, _)| entity != "line_item"),
        "nor are line items"
    );

    let uuid = txn_uuid(&db, id);
    let receipts: Vec<&(String, String, String)> = logged
        .iter()
        .filter(|(entity, u, _)| entity == "txn" && u == &uuid)
        .collect();
    assert_eq!(receipts.len(), 1, "one receipt, one thing to send");
}

#[test]
fn re_splitting_a_receipt_is_a_change_to_the_receipt() {
    // The write path replaces a whole aggregate by deleting the children and
    // re-inserting them, so removing a split is a DELETE and nothing else.
    // Without a trigger on that, dropping a category from a receipt would be
    // the one edit that never travelled.
    let db = seeded_db();
    device(&db, "laptop");
    let groceries = categories::resolve(&db, "Groceries").expect("resolve").id;
    let household = categories::resolve(&db, "Household").expect("resolve").id;

    let id = transactions::save(
        &db,
        &txn(
            day("2026-08-20"),
            amounts(5_000, 0),
            vec![split(groceries, 3_000, 0), split(household, 2_000, 0)],
        ),
    )
    .expect("save");
    let uuid = txn_uuid(&db, id);
    let before = hlc_of(&db, "txn", &uuid);

    // The same receipt, now entirely groceries.
    let mut replaced = txn(
        day("2026-08-20"),
        amounts(5_000, 0),
        vec![split(groceries, 5_000, 0)],
    );
    replaced.id = Some(id);
    transactions::save(&db, &replaced).expect("re-split");

    let after = hlc_of(&db, "txn", &uuid);
    assert!(
        after > before,
        "re-splitting must mark the receipt as changed: {after} !> {before}"
    );
}

fn txn_uuid(db: &Db, id: accounting_core::TxnId) -> String {
    db.conn()
        .query_row(
            "SELECT uuid FROM txn WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("the receipt exists")
}

#[test]
fn re_opening_the_app_is_not_an_edit() {
    // `seed_defaults` runs on every open and re-applies the category flags,
    // which is deliberate: they are facts about what a category *is*, and a
    // database seeded before a flag existed must pick it up. But SQLite fires
    // AFTER UPDATE whether or not a value actually changed, so a naive
    // re-apply stamps every seeded category into the change log on every
    // launch.
    //
    // Under last-writer-wins that is data loss with no error anywhere: merely
    // opening the app on the laptop carries a newer timestamp than a genuine
    // rename made on the phone an hour earlier, so the launch wins and the
    // rename is silently reverted. A write that changes nothing must not look
    // like a write.
    let db = seeded_db();
    device(&db, "laptop");

    let before = changes(&db);
    accounting_core::seed_defaults(&db).expect("re-open");
    accounting_core::seed_defaults(&db).expect("and again");
    let after = changes(&db);

    assert_eq!(
        before, after,
        "re-running the seed must not touch the change log"
    );
}

#[test]
fn a_seed_flag_that_is_genuinely_new_still_lands() {
    // The other half: skipping no-ops must not skip real ones. A database
    // seeded before `is_obligation` existed has to pick it up, or an upgraded
    // install keeps calling an unspent grocery envelope "behind" forever.
    let db = seeded_db();
    device(&db, "laptop");
    db.conn()
        .execute("UPDATE category_group SET is_obligation = 0", [])
        .expect("rewind to a pre-007 database");

    accounting_core::seed_defaults(&db).expect("re-open");

    let bills: i64 = db
        .conn()
        .query_row(
            "SELECT is_obligation FROM category_group WHERE name = 'Bills'",
            [],
            |r| r.get(0),
        )
        .expect("Bills is seeded");
    assert_eq!(bills, 1, "a genuinely missing flag must still be applied");
}
