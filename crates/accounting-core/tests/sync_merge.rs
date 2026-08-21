//! Two databases, exchanging bundles.
//!
//! These are the tests that matter. Everything below happens between two
//! independent `Db`s, the way two devices would, with bundles rendered to
//! bytes and parsed back — so nothing here can accidentally pass by sharing a
//! connection, a clock, or a rowid.

mod common;

use accounting_core::Db;
use accounting_core::repo::{categories, locations};
use accounting_core::sync::{bundle, device, merge};
use common::{empty_db, seeded_db};

/// Hand everything `from` has to `into`, the long way round: render, parse,
/// apply. Returns what applying it did.
fn sync(from: &Db, into: &Db, now: u64) -> merge::Applied {
    let peer = device::id(into).expect("peer names itself");
    let out = merge::export(from, &peer, now).expect("export");
    let bytes = bundle::render(&out).expect("render");
    let parsed = bundle::parse(&bytes).expect("parse");
    let applied = merge::import(into, &parsed, now).expect("import");
    merge::mark_sent(from, &peer, &out).expect("mark sent");
    applied
}

fn rate_of(db: &Db, name: &str) -> u32 {
    locations::find_by_name(db, name)
        .expect("find")
        .expect("exists")
        .tax_rate_ppm
}

fn conflicts(db: &Db) -> Vec<(String, String)> {
    let mut stmt = db
        .conn()
        .prepare("SELECT entity, payload FROM sync_conflict ORDER BY id")
        .expect("prepare");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect")
}

#[test]
fn a_change_made_on_one_device_arrives_on_the_other() {
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();

    let pantry = categories::create_group(&laptop, "Pantry").expect("create");
    categories::create(&laptop, pantry.id, "Flour").expect("create");

    let applied = sync(&laptop, &phone, 1_000);
    assert!(applied.applied > 0);
    assert!(applied.deferred.is_empty(), "{:?}", applied.deferred);

    let there =
        categories::resolve(&phone, "Pantry/Flour").expect("the category should have arrived");
    assert_eq!(there.group_name, "Pantry");
}

#[test]
fn the_same_bundle_applied_twice_changes_nothing_the_second_time() {
    // The same file dropped into a synced folder twice, or a device that
    // re-reads what it already read. It must be cheap and it must be a no-op —
    // an import that re-applied would re-stamp everything and bounce it back.
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();
    categories::create_group(&laptop, "Pantry").expect("create");

    let peer = device::id(&phone).expect("id");
    let out = merge::export(&laptop, &peer, 1_000).expect("export");
    let first = merge::import(&phone, &out, 1_000).expect("first");
    let second = merge::import(&phone, &out, 1_100).expect("second");

    assert!(first.applied > 0);
    assert_eq!(second.applied, 0, "nothing new the second time");
    assert_eq!(second.conflicts, 0, "and re-reading is not a disagreement");
}

#[test]
fn an_applied_change_is_not_sent_back_as_our_own() {
    // Writing a row fires this device's change-log triggers, so a freshly
    // applied change looks locally authored unless the log entry is rewritten
    // with the timestamp it arrived carrying. Without that it bounces back and
    // outranks the original it came from.
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();
    categories::create_group(&laptop, "Pantry").expect("create");

    sync(&laptop, &phone, 1_000);

    // Now the phone sends to the laptop. It has authored nothing.
    let back = merge::export(&phone, &device::id(&laptop).expect("id"), 2_000).expect("export");
    let echoed = back.changes.iter().any(|c| {
        c.body
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|n| n == "Pantry")
    });
    assert!(
        !echoed,
        "the phone must not claim the laptop's edit as its own"
    );
}

#[test]
fn the_later_edit_wins_and_the_earlier_one_is_kept() {
    // Two people, one household, the same location. Somebody's figure has to
    // lose — but a ledger that drops one silently, with nothing to show it
    // ever existed, is one neither person trusts.
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();

    // Start them agreeing.
    sync(&laptop, &phone, 1_000);
    sync(&phone, &laptop, 1_100);

    let home_l = locations::find_by_name(&laptop, "Home")
        .expect("f")
        .expect("e");
    let home_p = locations::find_by_name(&phone, "Home")
        .expect("f")
        .expect("e");
    assert_eq!(home_l.uuid, home_p.uuid, "the same place after a sync");

    // Both edit it, neither having seen the other.
    locations::set_rate(&laptop, home_l.id, 70_000).expect("laptop edit");
    std::thread::sleep(std::time::Duration::from_millis(5));
    locations::set_rate(&phone, home_p.id, 84_500).expect("phone edit");

    // The phone's edit is later, so it wins on the laptop.
    let applied = sync(&phone, &laptop, 2_000);
    assert_eq!(rate_of(&laptop, "Home"), 84_500, "the later edit wins");
    assert_eq!(applied.conflicts, 1, "and the disagreement is recorded");

    let kept = conflicts(&laptop);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0, "location");
    assert!(
        kept[0].1.contains("70000"),
        "the losing figure is kept verbatim: {}",
        kept[0].1
    );
}

#[test]
fn a_peer_simply_being_behind_is_not_a_conflict() {
    // The other half of the rule. If they never saw our newer edit, their old
    // value losing is not a disagreement — it is them catching up, and
    // flagging it would train the user to ignore the flag.
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();
    sync(&laptop, &phone, 1_000);
    sync(&phone, &laptop, 1_100);

    let home_p = locations::find_by_name(&phone, "Home")
        .expect("f")
        .expect("e");
    locations::set_rate(&phone, home_p.id, 84_500).expect("edit");
    sync(&phone, &laptop, 2_000);

    // The laptop now re-sends what it has. The phone already has it.
    let applied = sync(&laptop, &phone, 2_100);
    assert_eq!(applied.conflicts, 0, "catching up is not disagreeing");
}

#[test]
fn a_row_whose_reference_has_not_arrived_is_deferred_not_forced() {
    // A bundle built from a partial range. Filing the category under whatever
    // holds that rowid locally would be indistinguishable afterwards from
    // having filed it correctly.
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();

    let pantry = categories::create_group(&laptop, "Pantry").expect("create");
    let flour = categories::create(&laptop, pantry.id, "Flour").expect("create");

    // Hand over only the category, not its group.
    let peer = device::id(&phone).expect("id");
    let mut out = merge::export(&laptop, &peer, 1_000).expect("export");
    out.changes
        .retain(|c| c.entity != "category_group" || c.uuid != pantry.uuid);

    let applied = merge::import(&phone, &out, 1_000).expect("import");
    assert!(
        applied.deferred.iter().any(|m| m.uuid == flour.uuid),
        "the category should be held back, not misfiled"
    );
    assert!(categories::resolve(&phone, "Pantry/Flour").is_err());
}

#[test]
fn a_kind_from_a_newer_release_is_noticed_rather_than_silently_dropped() {
    let laptop = seeded_db();
    // Empty, not separately seeded. Pairing hands a device the other's data;
    // two independent seeds are two different sets of uuids for the same
    // logical rows, and merging them would duplicate the whole chart of
    // accounts — including two locations both claiming to be home, which the
    // schema refuses outright.
    let phone = empty_db();
    let peer = device::id(&phone).expect("id");
    let mut out = merge::export(&laptop, &peer, 1_000).expect("export");
    out.changes.push(accounting_core::sync::Change {
        entity: "invented_later".to_string(),
        uuid: "u-1".to_string(),
        hlc: device::clock(&laptop).expect("clock"),
        body: serde_json::json!({ "name": "?" }),
    });

    let applied = merge::import(&phone, &out, 1_000).expect("import");
    assert_eq!(applied.unknown_kinds, vec!["invented_later".to_string()]);
}
