//! Carrying a row between two databases that never agreed on a rowid.
//!
//! This is the part of sync that is easy to get wrong and impossible to notice
//! afterwards: a reference that resolves to *something* on the far side is
//! indistinguishable from one that resolved correctly, until a report groups
//! rent under groceries.

mod common;

use accounting_core::Db;
use accounting_core::repo::categories;
use accounting_core::sync::entity;
use common::seeded_db;

fn spec(name: &str) -> &'static entity::EntitySpec {
    entity::spec(name).unwrap_or_else(|| panic!("{name} is a known entity"))
}

fn group_of(db: &Db, category_uuid: &str) -> String {
    db.conn()
        .query_row(
            "SELECT g.name FROM category c
               JOIN category_group g ON g.id = c.group_id
              WHERE c.uuid = ?1",
            [category_uuid],
            |r| r.get(0),
        )
        .expect("the category exists")
}

#[test]
fn a_reference_travels_as_a_uuid_not_a_rowid() {
    // Two databases hand out id 7 to different things on their first day. A
    // body carrying `group_id: 7` would file the receipt under whatever
    // happened to be seventh over there.
    let db = seeded_db();
    let groceries = categories::resolve(&db, "Groceries").expect("resolve");

    let body = entity::read(&db, spec("category"), &groceries.uuid)
        .expect("read")
        .expect("the category exists");

    let map = body.as_object().expect("an object");
    assert!(
        !map.contains_key("id"),
        "the rowid is local and must not travel"
    );
    assert!(!map.contains_key("group_id"), "nor may a foreign rowid");
    let carried = map
        .get("group_uuid")
        .and_then(serde_json::Value::as_str)
        .expect("the group travels as a uuid");
    assert_eq!(carried.len(), 36, "and it is a uuid: {carried}");
}

#[test]
fn a_row_lands_in_the_right_group_on_a_device_with_different_rowids() {
    // The whole point. `into` gets extra groups first, so its rowids are
    // shifted relative to `from` — exactly the situation two real devices are
    // in, where each handed out id 7 to something different on its first day.
    let from = seeded_db();
    let into = seeded_db();
    for name in ["Shifted A", "Shifted B", "Shifted C"] {
        categories::create_group(&into, name).expect("shift the ids along");
    }

    // A group and a category in it, on the sending device only.
    let pantry = categories::create_group(&from, "Pantry").expect("create group");
    let flour = categories::create(&from, pantry.id, "Flour").expect("create category");

    let group_body = entity::read(&from, spec("category_group"), &pantry.uuid)
        .expect("read")
        .expect("exists");
    let category_body = entity::read(&from, spec("category"), &flour.uuid)
        .expect("read")
        .expect("exists");

    // The category cannot land before the group it points at: refused, rather
    // than filed under whatever happens to hold that rowid over here.
    let refused =
        entity::write(&into, spec("category"), &flour.uuid, &category_body).expect("no sql error");
    let missing = refused.expect_err("the group has not arrived yet");
    assert_eq!(
        missing.wanted,
        ("category_group".to_string(), pantry.uuid.clone())
    );

    entity::write(&into, spec("category_group"), &pantry.uuid, &group_body)
        .expect("no sql error")
        .expect("a group points at nothing");
    entity::write(&into, spec("category"), &flour.uuid, &category_body)
        .expect("no sql error")
        .expect("now the reference resolves");

    assert_eq!(
        group_of(&into, &flour.uuid),
        "Pantry",
        "the category must land in the group it actually belongs to"
    );

    // And the rowids really did differ, so this was not a coincidence.
    let there: i64 = from
        .conn()
        .query_row(
            "SELECT id FROM category_group WHERE uuid = ?1",
            [&pantry.uuid],
            |r| r.get(0),
        )
        .expect("exists");
    let here: i64 = into
        .conn()
        .query_row(
            "SELECT id FROM category_group WHERE uuid = ?1",
            [&pantry.uuid],
            |r| r.get(0),
        )
        .expect("exists");
    assert_ne!(here, there, "the two devices disagree about the rowid");
}

#[test]
fn a_column_this_release_does_not_have_is_dropped_rather_than_refused() {
    // A bundle from a newer app. Refusing the whole row would mean one
    // device's upgrade stopping the other from syncing at all.
    let db = seeded_db();
    let groceries = categories::resolve(&db, "Groceries").expect("resolve");
    let mut body = entity::read(&db, spec("category"), &groceries.uuid)
        .expect("read")
        .expect("exists");
    body.as_object_mut()
        .expect("object")
        .insert("invented_later".to_string(), serde_json::json!("hello"));

    entity::write(&db, spec("category"), &groceries.uuid, &body)
        .expect("no sql error")
        .expect("the known columns still apply");
}

#[test]
fn a_missing_row_reads_as_absent_rather_than_empty() {
    let db = seeded_db();
    assert!(
        entity::read(&db, spec("category"), "no-such-uuid")
            .expect("read")
            .is_none()
    );
}

#[test]
fn every_entity_comes_before_the_things_that_point_at_it() {
    // Bundles are ordered by time, but time is the wrong order to apply in: a
    // receipt written at noon may refer to a category created at one o'clock
    // on the other device. Walking kinds in dependency order is what makes a
    // foreign key satisfiable by the time it is needed.
    let mut seen: Vec<&str> = Vec::new();
    for spec in entity::ENTITIES {
        for (_, points_at) in spec.refs {
            assert!(
                seen.contains(points_at),
                "{} points at {points_at}, which must be applied first",
                spec.name
            );
        }
        seen.push(spec.name);
    }
}

#[test]
fn a_database_names_itself_once_and_keeps_the_name() {
    // The id is the last tiebreak in every timestamp and the thing peers track
    // watermarks against, so it must not change under a device that has
    // already told a peer how far along it is.
    use accounting_core::sync::device;

    let db = seeded_db();
    let first = device::id(&db).expect("mint");
    assert!(!first.is_empty());
    assert_eq!(
        device::id(&db).expect("again"),
        first,
        "minted once, not per call"
    );

    // And it is the name the clock reports.
    assert_eq!(device::clock(&db).expect("clock").device, first);
}

#[test]
fn observing_a_peer_drags_this_clock_past_it() {
    // What makes skew stop mattering: after a bundle lands, an edit made here
    // sorts after everything in it, so a peer three minutes fast cannot keep
    // out-ranking corrections made afterwards.
    use accounting_core::sync::device;
    use accounting_core::sync::hlc::Hlc;

    let db = seeded_db();
    let ahead = Hlc {
        millis: 9_999_999_999_999,
        counter: 5,
        device: "a-fast-phone".to_string(),
    };

    let merged = device::observe(&db, &ahead, 1_000).expect("observe");
    assert!(merged > ahead, "{merged} should sort after {ahead}");
    assert_eq!(
        merged.device,
        device::id(&db).expect("id"),
        "the clock stays ours"
    );

    // Survives a re-read, so the next edit here also outranks the peer.
    assert!(device::clock(&db).expect("clock") >= merged);
}
