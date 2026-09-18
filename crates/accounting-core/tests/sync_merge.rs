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

#[test]
fn a_receipt_arrives_with_its_splits_and_still_adds_up() {
    // The aggregate is the unit, and the module docs have said so from the
    // start — but reading only the `txn` row would send a receipt whose parts
    // are missing. It would satisfy the row-local CHECK (subtotal + tax =
    // total) and fail the aggregate invariant that no constraint can express,
    // so the receipt would look fine and `db::check_all` would find it later.
    use accounting_core::repo::{categories, transactions};
    use common::{amounts, day, split, txn};

    let laptop = seeded_db();
    let phone = empty_db();

    let groceries = categories::resolve(&laptop, "Groceries")
        .expect("resolve")
        .id;
    let household = categories::resolve(&laptop, "Household")
        .expect("resolve")
        .id;
    transactions::save(
        &laptop,
        &txn(
            day("2026-08-20"),
            amounts(5_000, 0),
            vec![split(groceries, 3_000, 0), split(household, 2_000, 0)],
        ),
    )
    .expect("save");

    let applied = sync(&laptop, &phone, 1_000);
    assert!(applied.deferred.is_empty(), "{:?}", applied.deferred);

    let there = transactions::list(&phone, &accounting_core::TxnFilter::default()).expect("list");
    assert_eq!(there.len(), 1, "the receipt should have arrived");
    let receipt = &there[0];
    assert_eq!(receipt.total.cents(), 5_000);
    assert_eq!(receipt.splits.len(), 2, "with both of its splits");

    let summed: i64 = receipt.splits.iter().map(|s| s.amount.cents()).sum();
    assert_eq!(summed, 5_000, "and the parts must still add up");

    // The invariant no SQLite constraint can express, checked the way the app
    // checks it.
    let violations = accounting_core::check_all(&phone).expect("check");
    assert!(violations.is_empty(), "{violations:?}");

    // And the splits point at the right categories, by uuid rather than by a
    // rowid that means nothing here.
    let names: Vec<&str> = receipt
        .splits
        .iter()
        .map(|s| s.category_name.as_str())
        .collect();
    assert!(
        names.contains(&"Groceries") && names.contains(&"Household"),
        "{names:?}"
    );
}

#[test]
fn re_splitting_a_receipt_replaces_its_parts_rather_than_adding_to_them() {
    // Children are replaced wholesale, as the write path does it. A merge that
    // appended would double a receipt's splits on every exchange.
    use accounting_core::repo::{categories, transactions};
    use common::{amounts, day, split, txn};

    let laptop = seeded_db();
    let phone = empty_db();
    let groceries = categories::resolve(&laptop, "Groceries")
        .expect("resolve")
        .id;
    let household = categories::resolve(&laptop, "Household")
        .expect("resolve")
        .id;

    let id = transactions::save(
        &laptop,
        &txn(
            day("2026-08-20"),
            amounts(5_000, 0),
            vec![split(groceries, 3_000, 0), split(household, 2_000, 0)],
        ),
    )
    .expect("save");
    sync(&laptop, &phone, 1_000);

    // Now it is all groceries.
    let mut replaced = txn(
        day("2026-08-20"),
        amounts(5_000, 0),
        vec![split(groceries, 5_000, 0)],
    );
    replaced.id = Some(id);
    transactions::save(&laptop, &replaced).expect("re-split");
    sync(&laptop, &phone, 2_000);

    let there = transactions::list(&phone, &accounting_core::TxnFilter::default()).expect("list");
    assert_eq!(there.len(), 1);
    assert_eq!(there[0].splits.len(), 1, "replaced, not appended");
    assert!(
        accounting_core::check_all(&phone)
            .expect("check")
            .is_empty()
    );
}

#[test]
fn re_importing_your_own_bundle_does_not_make_you_your_own_peer() {
    // An ordinary thing to do: check the file opened, or recover after a copy
    // that failed halfway. The damage would be silent and total — this device
    // would appear in its own peer list, `default_peer` would start returning
    // its own name, and export skips changes authored by the peer it is
    // addressing. Every later bundle would be empty and sync would look like
    // it was working.
    use accounting_core::sync::merge;

    let laptop = seeded_db();
    let mine = device::id(&laptop).expect("id");
    let out = merge::export(&laptop, "somebody-else", 1_000).expect("export");

    let applied = merge::import(&laptop, &out, 1_100).expect("import our own");
    assert_eq!(applied.applied, 0, "we already have all of it");

    let peers = merge::peers(&laptop).expect("peers");
    assert!(
        !peers.contains(&mine),
        "this device must not be its own peer: {peers:?}"
    );

    // And a bundle for a real peer still carries everything.
    let phone = empty_db();
    let applied = sync(&laptop, &phone, 2_000);
    assert!(
        applied.applied > 0,
        "sync must still carry the whole database"
    );
}

#[test]
fn re_importing_your_own_bundle_records_no_disagreements_and_touches_nothing() {
    // Every entry in our own file carries our own name and is newer than
    // anything we have sent anybody, so judged as a peer's it reads as a list
    // of edits made here that "they" had not seen — and each one used to be
    // recorded as a conflict, flagging receipts for review over a file that
    // came from this very device.
    use accounting_core::sync::merge;
    use common::{amounts, day, split, txn};

    let laptop = seeded_db();
    let groceries = categories::resolve(&laptop, "Groceries")
        .expect("resolve")
        .id;
    common::save(
        &laptop,
        &txn(
            day("2026-08-20"),
            amounts(1_359, 0),
            vec![split(groceries, 1_359, 0)],
        ),
    );
    let out = merge::export(&laptop, "somebody-else", 1_000).expect("export");
    assert!(!out.changes.is_empty());
    let log_before: Vec<(String, String)> = {
        let mut stmt = laptop
            .conn()
            .prepare("SELECT entity_uuid, hlc FROM sync_change ORDER BY entity_uuid")
            .expect("prepare");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("collect")
    };

    let parsed = bundle::parse(&bundle::render(&out).expect("render")).expect("parse");
    let applied = merge::import(&laptop, &parsed, 1_100).expect("import our own");

    assert_eq!(applied.conflicts, 0);
    assert_eq!(applied.applied, 0);
    assert_eq!(
        applied.ignored,
        out.changes.len(),
        "all of it is already here"
    );
    assert!(conflicts(&laptop).is_empty());
    let flagged: i64 = laptop
        .conn()
        .query_row("SELECT count(*) FROM txn WHERE needs_review = 1", [], |r| {
            r.get(0)
        })
        .expect("count");
    assert_eq!(flagged, 0, "no receipt was sent back to the Inbox");
    let log_after: Vec<(String, String)> = {
        let mut stmt = laptop
            .conn()
            .prepare("SELECT entity_uuid, hlc FROM sync_change ORDER BY entity_uuid")
            .expect("prepare");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("collect")
    };
    assert_eq!(log_after, log_before, "nothing was re-stamped");

    // Joining from it would throw this device's records away for nothing.
    let fresh = seeded_db();
    let own = merge::export_everything(&fresh, 1_000).expect("export");
    let error = merge::join(&fresh, &own, 1_000).expect_err("refused");
    assert!(
        error.to_string().contains("written by this device"),
        "{error}"
    );
    assert!(
        categories::resolve(&fresh, "Groceries").is_ok(),
        "still seeded"
    );
}

#[test]
fn a_freshly_seeded_device_can_join_and_adopts_the_other_chart_of_accounts() {
    // The case every real pairing starts from: both devices have been opened,
    // so both have seeded the same default categories under different uuids.
    // Merging would give two of everything; joining throws this device's seed
    // away and takes the other's.
    use accounting_core::repo::categories;
    use accounting_core::sync::merge;

    let laptop = seeded_db();
    let phone = seeded_db();

    let mine_before = categories::resolve(&phone, "Groceries")
        .expect("seeded")
        .uuid;
    let theirs = categories::resolve(&laptop, "Groceries")
        .expect("seeded")
        .uuid;
    assert_ne!(mine_before, theirs, "independent seeds differ by uuid");

    let bundle = merge::export(&laptop, "phone", 1_000).expect("export");
    merge::join(&phone, &bundle, 1_000).expect("join");

    // One Groceries, and it is the laptop's.
    let all = categories::list_all(&phone).expect("list");
    let groceries: Vec<&_> = all.iter().filter(|c| c.name == "Groceries").collect();
    assert_eq!(groceries.len(), 1, "not two of everything");
    assert_eq!(
        groceries[0].uuid, theirs,
        "and it is the other device's row"
    );

    // Only one location claims to be home, which the schema would refuse
    // outright if the merge had doubled it.
    let homes: i64 = phone
        .conn()
        .query_row(
            "SELECT count(*) FROM location WHERE is_home = 1 AND archived_at IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(homes, 1);
    assert!(
        accounting_core::check_all(&phone)
            .expect("check")
            .is_empty()
    );
}

#[test]
fn joining_refuses_to_destroy_work_that_is_actually_on_the_device() {
    // Joining is destructive on purpose, so the guard is about *work*, not
    // emptiness. A device that has only seeded itself has nothing worth
    // keeping; a device with one receipt on it is a different matter.
    use accounting_core::repo::{categories, transactions};
    use accounting_core::sync::merge;
    use common::{amounts, day, split, txn};

    let laptop = seeded_db();
    let phone = seeded_db();
    let groceries = categories::resolve(&phone, "Groceries")
        .expect("resolve")
        .id;
    transactions::save(
        &phone,
        &txn(
            day("2026-08-20"),
            amounts(1_359, 0),
            vec![split(groceries, 1_359, 0)],
        ),
    )
    .expect("a receipt entered on this device");

    let refused = merge::can_join(&phone)
        .expect("check")
        .expect_err("has work");
    assert!(matches!(refused, merge::JoinRefused::HasWork { .. }));
    // And the message tells the user what to do rather than just refusing.
    assert!(
        refused
            .to_string()
            .contains("export or merge this work first")
    );

    let bundle = merge::export(&laptop, "phone", 1_000).expect("export");
    assert!(
        merge::join(&phone, &bundle, 1_000).is_err(),
        "must not wipe it"
    );
    assert_eq!(
        transactions::count(&phone, &accounting_core::TxnFilter::default()).expect("count"),
        1,
        "the receipt is still here"
    );
}

#[test]
fn a_device_that_has_already_paired_does_not_join_again() {
    // Joining twice would throw away everything since the first join.
    use accounting_core::sync::merge;

    let laptop = seeded_db();
    let phone = empty_db();
    sync(&laptop, &phone, 1_000);

    let refused = merge::can_join(&phone).expect("check").expect_err("paired");
    assert!(matches!(refused, merge::JoinRefused::AlreadyPaired { .. }));
}

#[test]
fn a_joined_device_syncs_normally_afterwards() {
    // The point of joining is to reach the state ordinary sync assumes.
    use accounting_core::repo::categories;
    use accounting_core::sync::merge;

    let laptop = seeded_db();
    let phone = seeded_db();
    let bundle = merge::export(&laptop, "phone", 1_000).expect("export");
    merge::join(&phone, &bundle, 1_000).expect("join");

    // A later edit on the laptop reaches the phone by ordinary sync.
    categories::create_group(&laptop, "Pantry").expect("create");
    let applied = sync(&laptop, &phone, 2_000);
    assert!(applied.applied > 0);
    assert!(applied.deferred.is_empty(), "{:?}", applied.deferred);
    assert!(
        categories::list_groups(&phone)
            .expect("groups")
            .iter()
            .any(|g| g.name == "Pantry")
    );
}
