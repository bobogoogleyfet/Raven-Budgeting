//! Seed tests: the default taxonomy a first run gets, and the guarantees that
//! keep it safe to re-run.

mod common;

use accounting_core::db::seed::seed_defaults;
use common::{TRIGGER, assert_constraint, assert_err_mentions, count, empty_db, raw, seeded_db};

/// The three categories the code itself references by name. They must exist,
/// and must be impossible to remove — an "Uncategorized" that can be deleted
/// turns the needs-categorizing inbox into a dangling foreign key.
const SYSTEM_KEYS: [&str; 3] = ["uncategorized", "transfer", "reimbursable"];

fn system_keys_present(c: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = c
        .prepare("SELECT LOWER(system_key) FROM category WHERE system_key IS NOT NULL ORDER BY 1")
        .expect("prepare system_key query");
    let mut keys: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .expect("query system keys")
        .map(|r| r.expect("system_key row"))
        .collect();
    keys.sort();
    keys
}

// ---------------------------------------------------------------------------
// Contents
// ---------------------------------------------------------------------------

#[test]
fn seeding_creates_the_default_groups_and_categories() {
    let db = seeded_db();
    let c = raw(&db);

    let groups = count(c, "SELECT COUNT(*) FROM category_group");
    assert!(
        groups >= 2,
        "expected at least a Monthly and a Non-Monthly group, found {groups}"
    );

    let categories = count(c, "SELECT COUNT(*) FROM category");
    assert!(
        (15..=30).contains(&categories),
        "expected roughly 20 seeded categories, found {categories}"
    );

    assert_eq!(
        count(c, "SELECT COUNT(*) FROM category WHERE group_id IS NULL"),
        0,
        "every category belongs to a group — the taxonomy is exactly one level deep"
    );
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM category WHERE archived_at IS NOT NULL"
        ),
        0,
        "nothing seeds pre-archived"
    );
}

#[test]
fn seeding_creates_a_non_monthly_group_for_sinking_funds() {
    // Yearly items (insurance, registration) live here. Without a non-monthly
    // group with rollover, a $1200/yr line wrecks whichever month it lands in.
    let db = seeded_db();
    let c = raw(&db);
    assert!(
        count(
            c,
            "SELECT COUNT(*) FROM category_group WHERE LOWER(REPLACE(name, '-', '')) LIKE '%nonmonthly%'",
        ) >= 1,
        "expected a Non-Monthly category group"
    );
}

#[test]
fn the_three_system_categories_exist_with_their_keys_set() {
    let db = seeded_db();
    let c = raw(&db);

    let keys = system_keys_present(c);
    let mut expected: Vec<String> = SYSTEM_KEYS.iter().map(|k| (*k).to_string()).collect();
    expected.sort();

    assert_eq!(
        keys, expected,
        "exactly the three system categories must carry a system_key"
    );
}

#[test]
fn exactly_one_seeded_location_is_home() {
    let db = seeded_db();
    let c = raw(&db);
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM location WHERE is_home = 1"),
        1,
        "the home location is what prefills the tax-rate estimate"
    );
}

// ---------------------------------------------------------------------------
// Idempotence
// ---------------------------------------------------------------------------

#[test]
fn seeding_twice_does_not_duplicate_anything() {
    // seed_defaults runs on every open in the CLI, so "already seeded" is the
    // common case, not the exceptional one.
    let db = empty_db();
    seed_defaults(&db).expect("first seed");

    let c = raw(&db);
    let before = (
        count(c, "SELECT COUNT(*) FROM category_group"),
        count(c, "SELECT COUNT(*) FROM category"),
        count(c, "SELECT COUNT(*) FROM location"),
        count(c, "SELECT COUNT(*) FROM budget_line"),
    );

    seed_defaults(&db).expect("second seed must succeed, not conflict");

    let after = (
        count(c, "SELECT COUNT(*) FROM category_group"),
        count(c, "SELECT COUNT(*) FROM category"),
        count(c, "SELECT COUNT(*) FROM location"),
        count(c, "SELECT COUNT(*) FROM budget_line"),
    );

    assert_eq!(before, after, "seed_defaults must be idempotent");
}

#[test]
fn reseeding_does_not_resurrect_an_archived_category() {
    // A user who archives "Dining Out" must not find it back tomorrow.
    let db = seeded_db();
    let c = raw(&db);

    let victim: i64 = c
        .query_row(
            "SELECT id FROM category WHERE system_key IS NULL ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("pick a non-system category");
    c.execute(
        "UPDATE category SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
        [victim],
    )
    .expect("archive category");

    seed_defaults(&db).expect("reseed");

    let still_archived: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM category WHERE id = ?1 AND archived_at IS NOT NULL",
            [victim],
            |r| r.get(0),
        )
        .expect("re-read category");
    assert_eq!(
        still_archived, 1,
        "reseeding must not un-archive a category"
    );
}

// ---------------------------------------------------------------------------
// System-category triggers
// ---------------------------------------------------------------------------

#[test]
fn a_system_category_cannot_be_deleted() {
    let db = seeded_db();
    let c = raw(&db);

    for key in SYSTEM_KEYS {
        let sql = "DELETE FROM category WHERE LOWER(system_key) = ?1";
        // Both halves matter: the extended code proves a trigger fired rather
        // than some other constraint, and the message proves it was the right
        // trigger rather than an unrelated one.
        assert_err_mentions(c.execute(sql, [key]), "cannot be deleted");
        assert_constraint(
            c.execute(sql, [key]),
            TRIGGER,
            "system-category delete guard",
        );

        assert_eq!(
            count(
                c,
                "SELECT COUNT(*) FROM category WHERE system_key IS NOT NULL"
            ),
            3,
            "all three system categories must survive the refused delete of {key}"
        );
    }
}

#[test]
fn a_system_category_cannot_be_archived() {
    // Archiving would hide Uncategorized from every picker while leaving the
    // code that assigns it working — the worst of both failure modes.
    let db = seeded_db();
    let c = raw(&db);

    for key in SYSTEM_KEYS {
        let sql = "UPDATE category SET archived_at = '2026-01-01T00:00:00Z'
                   WHERE LOWER(system_key) = ?1";
        assert_err_mentions(c.execute(sql, [key]), "cannot be archived");
        assert_constraint(
            c.execute(sql, [key]),
            TRIGGER,
            "system-category archive guard",
        );
    }

    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM category
             WHERE system_key IS NOT NULL AND archived_at IS NULL"
        ),
        3,
        "every system category must still be active"
    );
}

#[test]
fn clearing_system_key_cannot_be_used_to_get_around_those_guards() {
    // Without this the previous two triggers are one UPDATE away from useless:
    // set system_key = NULL, then delete freely.
    let db = seeded_db();
    let c = raw(&db);

    let sql = "UPDATE category SET system_key = NULL WHERE LOWER(system_key) = 'uncategorized'";
    assert_err_mentions(c.execute(sql, []), "immutable");
    assert_constraint(c.execute(sql, []), TRIGGER, "system_key immutability guard");

    assert_eq!(
        count(
            raw(&db),
            "SELECT COUNT(*) FROM category WHERE system_key IS NOT NULL"
        ),
        3,
    );
}

#[test]
fn an_ordinary_category_can_still_be_archived() {
    // Guards against a trigger written too broadly: if the archive block fires
    // for every row, the previous test passes and the app is unusable.
    let db = seeded_db();
    let c = raw(&db);

    let victim: i64 = c
        .query_row(
            "SELECT id FROM category WHERE system_key IS NULL ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("pick a non-system category");

    c.execute(
        "UPDATE category SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
        [victim],
    )
    .expect("archiving an ordinary category must be allowed");
}

#[test]
fn an_ordinary_unreferenced_category_can_still_be_deleted() {
    let db = seeded_db();
    let c = raw(&db);

    let victim: i64 = c
        .query_row(
            "SELECT id FROM category WHERE system_key IS NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("pick a non-system category");

    let removed = c
        .execute("DELETE FROM category WHERE id = ?1", [victim])
        .expect("deleting an unreferenced ordinary category must be allowed");
    assert_eq!(removed, 1);
}

// ---------------------------------------------------------------------------
// Rollover
// ---------------------------------------------------------------------------

#[test]
fn seed_defaults_creates_no_budget_lines() {
    // The implementation seeds the taxonomy only. Rollover therefore lives on
    // `category.default_rollover`, which seeds `budget_line.rollover` when the
    // user creates a line — see the next test. Pinned here because a future
    // seed that DOES create budget lines must come with a rollover assertion
    // over the real rows, not the vacuous one this would otherwise become.
    let db = seeded_db();
    assert_eq!(count(raw(&db), "SELECT COUNT(*) FROM budget_line"), 0);
}

#[test]
fn every_category_in_a_non_monthly_group_defaults_to_rollover() {
    // Rollover is what makes a sinking fund work: a $600 annual premium
    // budgeted at $50/month is only meaningful if the unspent $50s accumulate.
    // Without carry-over the envelope resets monthly and the whole
    // yearly-expense feature collapses.
    let db = seeded_db();
    let c = raw(&db);

    let sinking_groups = "('Non-Monthly', 'Savings')";

    let total = count(
        c,
        &format!(
            "SELECT COUNT(*) FROM category cat
             JOIN category_group g ON g.id = cat.group_id
             WHERE g.name IN {sinking_groups}"
        ),
    );
    assert!(
        total > 0,
        "expected the seed to create sinking-fund categories"
    );

    let offenders = count(
        c,
        &format!(
            "SELECT COUNT(*) FROM category cat
             JOIN category_group g ON g.id = cat.group_id
             WHERE g.name IN {sinking_groups} AND cat.default_rollover = 0"
        ),
    );
    assert_eq!(
        offenders, 0,
        "every sinking-fund category must seed default_rollover = 1"
    );
}

#[test]
fn ordinary_monthly_categories_do_not_default_to_rollover() {
    // The mirror image: if `default_rollover` were 1 everywhere, the previous
    // test would pass while every envelope silently carried its balance
    // forward — which is a different budgeting product.
    let db = seeded_db();
    let c = raw(&db);
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM category cat
             JOIN category_group g ON g.id = cat.group_id
             WHERE g.name = 'Frequent' AND cat.default_rollover = 1"
        ),
        0,
        "frequent-spending categories must not roll over by default"
    );
}

#[test]
fn system_categories_are_excluded_from_budgeting() {
    // Budgeting "Transfer" is meaningless and budgeting "Uncategorized"
    // rewards leaving things there.
    let db = seeded_db();
    assert_eq!(
        count(
            raw(&db),
            "SELECT COUNT(*) FROM category WHERE system_key IS NOT NULL AND is_budgeted = 1"
        ),
        0,
    );
}
