//! Rows that existed before the change log did, and joining from a file.
//!
//! 008 logs every write from the moment it runs, and a bundle is built from
//! that log alone — so everything a database held *before* 008 could never be
//! sent. Two copies of the same file never noticed, because both already had
//! those rows; a device joining one did, because it threw its seed away and
//! received a fraction of the ledger. These tests build databases the way the
//! real ones came to be — rows first, the log afterwards — and check both that
//! a joiner now gets everything and that a pair already syncing sees nothing
//! change.

mod common;

use std::path::Path;

use accounting_core::Db;
use accounting_core::domain::budget_calc::Period;
use accounting_core::repo::budgets::{self, BudgetLineRecord};
use accounting_core::repo::{categories, locations, transactions};
use accounting_core::sync::{Bundle, bundle, device, entity, merge};
use common::{
    amounts, assert_clean, cat, count, day, location, m, merchant, raw, seeded_db, split, txn, ym,
};

/// Every syncable table, and the tables that travel inside a receipt.
const TABLES: &[&str] = &[
    "category_group",
    "category",
    "location",
    "merchant",
    "budget_line",
    "budget_adjustment",
    "budget_contribution",
    "txn",
    "txn_split",
    "line_item",
];

/// Render and parse, so nothing passes by sharing an in-memory value.
fn over_the_wire(out: &Bundle) -> Bundle {
    bundle::parse(&bundle::render(out).expect("render")).expect("parse")
}

/// One ordinary exchange, `from` → `into`, marked as delivered.
fn sync(from: &Db, into: &Db, now: u64) -> (usize, merge::Applied) {
    let peer = device::id(into).expect("peer names itself");
    let out = merge::export(from, &peer, now).expect("export");
    let applied = merge::import(into, &over_the_wire(&out), now).expect("import");
    merge::mark_sent(from, &peer, &out).expect("mark sent");
    (out.changes.len(), applied)
}

/// A household's worth of data: receipts at a merchant and a place, a budget
/// line, a contribution, and a category that has since been archived — a
/// delete here is soft, and the archived row has to travel like any other.
fn fill(db: &Db) {
    let groceries = cat(db, "Groceries");
    let costco = merchant(db, "Costco");
    let lehi = location(db, "Lehi", 72_500);
    for (date, cents) in [
        ("2026-05-02", 4_210),
        ("2026-06-11", 1_359),
        ("2026-07-20", 8_800),
    ] {
        let mut new = txn(
            day(date),
            amounts(cents, 0),
            vec![split(groceries, cents, 0)],
        );
        new.merchant_id = Some(costco);
        new.location_id = Some(lehi);
        common::save(db, &new);
    }

    let pantry = categories::create_group(db, "Pantry").expect("group");
    let flour = categories::create(db, pantry.id, "Flour").expect("category");
    common::spend_on(db, "2026-04-01", flour.id, 650);
    let yeast = categories::create(db, pantry.id, "Yeast").expect("category");
    categories::archive(db, yeast.id).expect("archive");

    budgets::upsert(
        db,
        &BudgetLineRecord {
            id: None,
            category_id: groceries,
            period: Period::Monthly,
            period_start: ym(2026, 1),
            period_end: None,
            amount: m(60_000),
            rollover: true,
            notes: None,
        },
    )
    .expect("budget line");
    budgets::set_contribution(db, groceries, ym(2026, 6), m(5_000)).expect("contribution");
}

/// Turn a database into one whose rows all predate the change log, then open
/// it with this release — which is what happened to every database that was
/// in use before 008 and is upgraded now.
fn as_if_written_before_the_log(path: &Path) -> Db {
    {
        let db = Db::open(path).expect("open");
        raw(&db)
            .execute_batch("DELETE FROM sync_change; PRAGMA user_version = 8;")
            .expect("forget the log");
    }
    Db::open(path).expect("reopen, migrating")
}

/// Every row of every syncable kind, as a bundle would carry it, by uuid.
///
/// Compared between two databases, this is "the same data": the rowids are
/// each file's own business, and everything else is here.
fn contents(db: &Db) -> Vec<(String, String, serde_json::Value)> {
    let mut out = Vec::new();
    for spec in entity::ENTITIES {
        let mut stmt = raw(db)
            .prepare(&format!("SELECT uuid FROM {} ORDER BY uuid", spec.table))
            .expect("prepare");
        let uuids: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("collect");
        for uuid in uuids {
            let body = entity::read(db, spec, &uuid)
                .expect("read")
                .expect("present");
            out.push((spec.name.to_string(), uuid, body));
        }
    }
    out
}

fn table_counts(db: &Db) -> Vec<(&'static str, i64)> {
    TABLES
        .iter()
        .map(|t| (*t, count(raw(db), &format!("SELECT count(*) FROM {t}"))))
        .collect()
}

fn home_uuid(db: &Db) -> String {
    locations::home(db)
        .expect("home")
        .expect("a home location")
        .uuid
}

fn open_conflicts(db: &Db) -> i64 {
    count(raw(db), "SELECT count(*) FROM sync_conflict")
}

#[test]
fn every_row_from_before_the_log_is_logged_once_the_database_is_upgraded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("desk.db");
    {
        let db = Db::open(&path).expect("open");
        accounting_core::db::seed_defaults(&db).expect("seed");
        fill(&db);
    }
    let desk = as_if_written_before_the_log(&path);

    for spec in entity::ENTITIES {
        let rows = count(raw(&desk), &format!("SELECT count(*) FROM {}", spec.table));
        let logged = count(
            raw(&desk),
            &format!(
                "SELECT count(*) FROM {} WHERE uuid IN
                   (SELECT entity_uuid FROM sync_change WHERE entity = '{}')",
                spec.table, spec.name
            ),
        );
        assert_eq!(logged, rows, "every {} is logged", spec.name);
    }

    // Stamped before any real edit anywhere, and by no device: a row nobody
    // has touched since sync began must never outrank somebody's change to it.
    let backfilled = count(
        raw(&desk),
        "SELECT count(*) FROM sync_change
          WHERE hlc = '0000000000000000-00000000-(baseline)'",
    );
    assert!(backfilled > 0);
    let parsed: accounting_core::sync::Hlc = "0000000000000000-00000000-(baseline)"
        .parse()
        .expect("a baseline stamp is one a bundle can carry");
    assert_eq!(parsed.millis, 0);
}

#[test]
fn an_edit_made_after_the_log_existed_keeps_its_real_timestamp() {
    // The backfill fills gaps. Replacing an entry the triggers had already
    // written with time zero would stop that edit from ever being sent.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("desk.db");
    let before = {
        let db = Db::open(&path).expect("open");
        accounting_core::db::seed_defaults(&db).expect("seed");
        fill(&db);
        raw(&db)
            .execute_batch(
                "DELETE FROM sync_change WHERE entity <> 'merchant'; PRAGMA user_version = 8;",
            )
            .expect("forget most of the log");
        count(
            raw(&db),
            "SELECT count(*) FROM sync_change WHERE hlc NOT LIKE '0000000000000000-%'",
        )
    };
    assert!(before > 0);
    let desk = Db::open(&path).expect("reopen, migrating");
    let after = count(
        raw(&desk),
        "SELECT count(*) FROM sync_change WHERE hlc NOT LIKE '0000000000000000-%'",
    );
    assert_eq!(after, before);
}

#[test]
fn a_device_joining_an_upgraded_database_receives_the_whole_ledger() {
    // The failure this migration exists for. Before it, a device joining a
    // database that predated the log threw its own seed away and received only
    // the rows edited since — no receipts from before, no chart of accounts,
    // no home location.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("desk.db");
    {
        let db = Db::open(&path).expect("open");
        accounting_core::db::seed_defaults(&db).expect("seed");
        fill(&db);
    }
    let desk = as_if_written_before_the_log(&path);
    // And one edit made after the log existed, as a real desktop has.
    location(&desk, "Provo", 72_500);

    let browser = seeded_db();
    let file = over_the_wire(&merge::export_everything(&desk, 1_000).expect("export"));
    assert_eq!(file.complete, Some(true));
    let applied = merge::join(&browser, &file, 1_000).expect("join");

    assert!(applied.deferred.is_empty(), "{:?}", applied.deferred);
    assert_eq!(applied.conflicts, 0);
    assert_eq!(table_counts(&browser), table_counts(&desk));
    assert_eq!(contents(&browser), contents(&desk));
    assert_eq!(home_uuid(&browser), home_uuid(&desk));
    assert_eq!(
        transactions::count(
            &browser,
            &accounting_core::TxnFilter {
                include_archived: true,
                ..Default::default()
            }
        )
        .expect("count"),
        4
    );
    assert_clean(&browser);

    // Opening the app again seeds and refreshes as every launch does, and must
    // not start a second chart of accounts beside the one just adopted.
    accounting_core::db::seed_defaults(&browser).expect("relaunch seed");
    assert_eq!(table_counts(&browser), table_counts(&desk));
}

#[test]
fn a_pair_already_syncing_exchanges_nothing_new_and_disagrees_about_nothing() {
    // How the first real pair was made: the desktop's file, restored onto the
    // phone and renamed, then synced both ways. Both copies hold the same rows
    // that predate the log, and both have watermarks. Upgrading both must not
    // turn that history into traffic, conflicts, or changes.
    let dir = tempfile::tempdir().expect("tempdir");
    let desk_path = dir.path().join("desk.db");
    let phone_path = dir.path().join("phone.db");
    {
        let db = Db::open(&desk_path).expect("open");
        accounting_core::db::seed_defaults(&db).expect("seed");
        fill(&db);
        // Everything so far predates the log.
        raw(&db)
            .execute("DELETE FROM sync_change", [])
            .expect("forget");
        db.backup(&phone_path).expect("copy onto the phone");
    }
    {
        let desk = Db::open(&desk_path).expect("desk");
        let phone = Db::open(&phone_path).expect("phone");
        raw(&phone)
            .execute("UPDATE sync_clock SET device = 'phone-ca3c818f'", [])
            .expect("rename the restored copy");
        // A little work on each side after the log existed, exchanged both
        // ways, so each has a watermark for the other.
        location(&desk, "Provo", 72_500);
        merchant(&phone, "Neighbourhood garage sale");
        assert_eq!(sync(&phone, &desk, 1_000).0, 1);
        assert_eq!(sync(&desk, &phone, 2_000).0, 1);
        assert_eq!(contents(&desk), contents(&phone));
        raw(&desk)
            .execute_batch("PRAGMA user_version = 8")
            .expect("rewind");
        raw(&phone)
            .execute_batch("PRAGMA user_version = 8")
            .expect("rewind");
    }

    // Both upgraded.
    let desk = Db::open(&desk_path).expect("desk, migrating");
    let phone = Db::open(&phone_path).expect("phone, migrating");
    let before = contents(&desk);
    assert_eq!(before, contents(&phone));

    let (sent, applied) = sync(&desk, &phone, 3_000);
    assert_eq!(sent, 0, "nothing the phone does not already have");
    assert_eq!((applied.applied, applied.conflicts), (0, 0));
    let (sent, applied) = sync(&phone, &desk, 4_000);
    assert_eq!(sent, 0, "nothing the desktop does not already have");
    assert_eq!((applied.applied, applied.conflicts), (0, 0));
    assert_eq!(contents(&desk), before);
    assert_eq!(contents(&phone), before);

    // And ordinary sync carries on: an edit on each side to a row from before
    // the log arrives on the other, with nothing flagged.
    raw(&phone)
        .execute(
            "UPDATE txn SET notes = 'split with Sam' WHERE txn_date = '2026-06-11'",
            [],
        )
        .expect("edit on the phone");
    raw(&desk)
        .execute(
            "UPDATE category SET name = 'Food at home' WHERE name = 'Groceries'",
            [],
        )
        .expect("edit on the desktop");
    let (_, to_desk) = sync(&phone, &desk, 5_000);
    let (_, to_phone) = sync(&desk, &phone, 6_000);
    assert_eq!((to_desk.applied, to_desk.conflicts), (1, 0));
    assert_eq!((to_phone.applied, to_phone.conflicts), (1, 0));
    assert_eq!(contents(&desk), contents(&phone));
    assert_eq!(open_conflicts(&desk) + open_conflicts(&phone), 0);
    assert_clean(&desk);
    assert_clean(&phone);
}

#[test]
fn a_copy_that_never_sent_anything_offers_its_backfill_harmlessly() {
    // The same restored pair, but the phone has never exported: it has no
    // watermark for the desktop, so its first file after upgrading carries
    // every backfilled row. The desktop already has every one of them, under
    // the identical stamp — so it takes none, flags none, and changes nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    let desk_path = dir.path().join("desk.db");
    let phone_path = dir.path().join("phone.db");
    {
        let db = Db::open(&desk_path).expect("open");
        accounting_core::db::seed_defaults(&db).expect("seed");
        fill(&db);
        raw(&db)
            .execute_batch("DELETE FROM sync_change; PRAGMA user_version = 8;")
            .expect("forget");
        db.backup(&phone_path).expect("copy onto the phone");
    }
    {
        let phone = rusqlite::Connection::open(&phone_path).expect("phone");
        phone
            .execute("UPDATE sync_clock SET device = 'phone-ca3c818f'", [])
            .expect("rename the restored copy");
    }
    let desk = Db::open(&desk_path).expect("desk, migrating");
    let phone = Db::open(&phone_path).expect("phone, migrating");
    let before = contents(&desk);

    let (sent, applied) = sync(&phone, &desk, 1_000);
    assert!(sent > 0, "the backfill is offered");
    assert_eq!((applied.applied, applied.conflicts), (0, 0));
    assert!(applied.deferred.is_empty(), "{:?}", applied.deferred);
    assert_eq!(contents(&desk), before);
    assert_eq!(open_conflicts(&desk), 0);

    // And the other way, for the same reason.
    let (_, applied) = sync(&desk, &phone, 2_000);
    assert_eq!((applied.applied, applied.conflicts), (0, 0));
    assert_eq!(contents(&phone), before);
    assert_eq!(open_conflicts(&phone), 0);
}

#[test]
fn a_device_that_joined_from_a_file_corrects_what_it_was_given_without_a_disagreement() {
    // A file for a new device is carried by hand, so the device that wrote it
    // never learns who read it. The joiner's first file back has to say what
    // it was given, or every correction it made would be recorded as a
    // disagreement with a version it had in fact seen.
    let desk = seeded_db();
    fill(&desk);
    let browser = seeded_db();
    merge::join(
        &browser,
        &over_the_wire(&merge::export_everything(&desk, 1_000).expect("export")),
        1_000,
    )
    .expect("join");

    raw(&browser)
        .execute(
            "UPDATE txn SET notes = 'corrected in the browser' WHERE txn_date = '2026-06-11'",
            [],
        )
        .expect("edit");
    let (_, applied) = sync(&browser, &desk, 2_000);
    assert_eq!((applied.applied, applied.conflicts), (1, 0));
    assert_eq!(open_conflicts(&desk), 0);
    let notes: String = raw(&desk)
        .query_row(
            "SELECT notes FROM txn WHERE txn_date = '2026-06-11'",
            [],
            |r| r.get(0),
        )
        .expect("notes");
    assert_eq!(notes, "corrected in the browser");

    // And the acknowledgement stands in for the delivery nobody could record:
    // the desktop's next file does not hand the browser its copy again.
    let peer = device::id(&browser).expect("browser");
    let next = merge::export(&desk, &peer, 3_000).expect("export");
    assert!(next.changes.is_empty(), "{:?}", next.changes.len());
}

#[test]
fn joining_from_an_ordinary_exchange_file_is_refused_before_anything_is_lost() {
    // The natural file to reach for is the one the desktop writes for its
    // phone. It holds what changed since they last synced — a join keeps
    // nothing else, so it would trade this device's seed for a fraction of a
    // ledger.
    let desk = seeded_db();
    let phone = common::empty_db();
    sync(&desk, &phone, 1_000);
    fill(&desk);

    let phone_id = device::id(&phone).expect("phone");
    let for_the_phone = over_the_wire(&merge::export(&desk, &phone_id, 2_000).expect("export"));
    assert_eq!(for_the_phone.complete, Some(false));

    let browser = seeded_db();
    let seed = table_counts(&browser);
    let error = merge::join(&browser, &for_the_phone, 2_000).expect_err("refused");
    assert!(
        error.to_string().contains("file for a new device"),
        "{error}"
    );
    assert_eq!(table_counts(&browser), seed, "nothing was thrown away");
    assert!(merge::can_join(&browser).expect("check").is_ok());

    // The file made for the purpose is accepted.
    let whole = over_the_wire(&merge::export_everything(&desk, 3_000).expect("export"));
    merge::join(&browser, &whole, 3_000).expect("join");
    assert_eq!(contents(&browser), contents(&desk));
}

#[test]
fn a_join_that_fails_partway_leaves_the_device_as_it_was() {
    // A join wipes the device and then applies the file. If applying fails
    // after the wipe has been committed, the device has neither its own
    // records nor the other's, so both have to be one transaction.
    let desk = seeded_db();
    fill(&desk);
    let mut file = over_the_wire(&merge::export_everything(&desk, 1_000).expect("export"));
    // A place with no name, which the schema refuses outright. Last in the
    // dependency order that matters here, so plenty is applied before it.
    let hlc = file.changes[0].hlc.clone();
    file.changes.push(accounting_core::sync::Change {
        entity: "location".to_string(),
        uuid: "01ffffff-0000-7000-8000-000000000000".to_string(),
        hlc,
        body: serde_json::json!({
            "uuid": "01ffffff-0000-7000-8000-000000000000",
            "name": null,
        }),
    });

    let browser = seeded_db();
    let before = contents(&browser);
    merge::join(&browser, &file, 2_000).expect_err("the schema refuses the row");

    assert_eq!(
        contents(&browser),
        before,
        "its own records are all still here"
    );
    assert!(merge::peers(&browser).expect("peers").is_empty());
    assert!(
        merge::can_join(&browser).expect("check").is_ok(),
        "and it can try again"
    );
}

#[test]
fn a_change_sent_again_is_not_a_disagreement_with_what_was_done_after_it() {
    // A file imported, then the same history sent again because the sender
    // never heard it arrived — a browser download nobody confirmed. The
    // receiver edited the thing in between. Its edit was made *after* seeing
    // the other version, so the two are sequential, not rivals.
    let laptop = seeded_db();
    let phone = common::empty_db();
    sync(&laptop, &phone, 1_000);

    let phone_id = device::id(&phone).expect("phone");
    raw(&laptop)
        .execute(
            "UPDATE category SET name = 'Food' WHERE name = 'Groceries'",
            [],
        )
        .expect("edit on the laptop");
    let first = merge::export(&laptop, &phone_id, 2_000).expect("export");
    merge::import(&phone, &over_the_wire(&first), 2_000).expect("import");
    // Not marked as sent.

    raw(&phone)
        .execute(
            "UPDATE category SET name = 'Food at home' WHERE name = 'Food'",
            [],
        )
        .expect("edit on the phone, after seeing the laptop's");
    let again = merge::export(&laptop, &phone_id, 3_000).expect("export");
    let applied = merge::import(&phone, &over_the_wire(&again), 3_000).expect("import");
    assert_eq!(applied.conflicts, 0);
    assert_eq!(open_conflicts(&phone), 0);
    assert!(
        categories::find_by_name(&phone, "Food at home")
            .expect("find")
            .is_some()
    );
}
