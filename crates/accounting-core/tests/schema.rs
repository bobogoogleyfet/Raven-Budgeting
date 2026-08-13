//! Schema tests: the constraints that make a wrong row impossible to write.
//!
//! These deliberately go around any repository layer and issue raw SQL. The
//! point is that the *database* refuses bad data, so that a future write path —
//! or a human with `sqlite3` open — cannot corrupt the ledger.

mod common;

use common::{
    CHECK, DATATYPE, FOREIGN_KEY, RESTRICT, UNIQUE, assert_constraint, count,
    insert_balanced_aggregate, insert_category, insert_group, insert_group_with_category,
    insert_location, insert_txn, raw, seeded_db,
};

// ---------------------------------------------------------------------------
// STRICT
// ---------------------------------------------------------------------------

#[test]
fn every_table_is_declared_strict() {
    // Without STRICT, SQLite's type affinity happily stores the string "12.34"
    // in an INTEGER cents column. Every later SUM() then silently returns a
    // float, and money arithmetic is back to being approximate.
    let db = common::empty_db();
    let c = raw(&db);

    let mut stmt = c
        .prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .expect("prepare sqlite_master query");
    let tables: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query sqlite_master")
        .map(|r| r.expect("sqlite_master row"))
        .collect();

    assert!(
        !tables.is_empty(),
        "no user tables found — did the migration run?"
    );

    let lax: Vec<&str> = tables
        .iter()
        .filter(|(_, sql)| {
            // STRICT is a table option, so it can only appear after the final
            // close paren of the column list.
            let tail = sql.rsplit(')').next().unwrap_or("");
            !tail.to_ascii_uppercase().contains("STRICT")
        })
        .map(|(name, _)| name.as_str())
        .collect();

    assert!(
        lax.is_empty(),
        "these tables are not STRICT: {lax:?} — money columns must reject TEXT"
    );
}

#[test]
fn a_decimal_string_in_a_cents_column_is_rejected() {
    // The motivating case: a UI that forgets to convert dollars to cents sends
    // "12.34". Without STRICT this lands in the INTEGER column as TEXT and
    // every later SUM() over it is silently wrong.
    let db = common::empty_db();
    let c = raw(&db);
    let result = c.execute(
        "INSERT INTO txn (uuid, txn_date, subtotal_cents, tax_cents, total_cents)
         VALUES ('u-text-cents', '2026-03-14', '12.34', 0, 1234)",
        [],
    );
    assert_constraint(result, DATATYPE, "STRICT datatype");
}

#[test]
fn a_fractional_real_in_a_cents_column_is_rejected() {
    // The other half of the same mistake: a float that is not a whole number of
    // cents. STRICT refuses it rather than truncating.
    let db = common::empty_db();
    let c = raw(&db);
    let result = c.execute(
        "INSERT INTO txn (uuid, txn_date, subtotal_cents, tax_cents, total_cents)
         VALUES ('u-real-cents', '2026-03-14', 12.34, 0, 1234)",
        [],
    );
    assert_constraint(result, DATATYPE, "STRICT datatype");
}

#[test]
fn a_whole_number_string_in_a_cents_column_is_coerced_rather_than_rejected() {
    // Documenting the limit of STRICT, not endorsing it. STRICT rejects a value
    // only if the conversion would LOSE information, so the string '1000'
    // becomes the integer 1000 and no error is raised.
    //
    // This matters because it means STRICT is not a substitute for typing the
    // boundary: a caller that sends whole dollars as a string gets a silent
    // hundred-fold error that no constraint here can see. That is what
    // `Money` (i64 cents, no `From<f64>`) is for.
    let db = common::empty_db();
    let c = raw(&db);
    c.execute(
        "INSERT INTO txn (uuid, txn_date, subtotal_cents, tax_cents, total_cents)
         VALUES ('u-int-string', '2026-03-14', '1000', 0, 1000)",
        [],
    )
    .expect("a losslessly convertible string is accepted by STRICT");

    let (value, kind): (i64, String) = c
        .query_row(
            "SELECT subtotal_cents, typeof(subtotal_cents) FROM txn WHERE uuid = 'u-int-string'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read back");
    assert_eq!(value, 1000);
    assert_eq!(
        kind, "integer",
        "STRICT must at least guarantee the stored type, so SUM() stays exact"
    );
}

// ---------------------------------------------------------------------------
// Row-local CHECKs
// ---------------------------------------------------------------------------

#[test]
fn a_txn_whose_parts_do_not_sum_to_its_total_is_rejected() {
    let db = common::empty_db();
    let c = raw(&db);
    let result = c.execute(
        "INSERT INTO txn (uuid, txn_date, subtotal_cents, tax_cents, total_cents)
         VALUES ('u-bad-txn', '2026-03-14', 1000, 84, 1085)",
        [],
    );
    assert_constraint(result, CHECK, "CHECK constraint");
}

#[test]
fn a_txn_whose_parts_do_sum_to_its_total_is_accepted() {
    let db = common::empty_db();
    let c = raw(&db);
    let id = insert_txn(c, 1000, 84);
    assert_eq!(count(c, "SELECT COUNT(*) FROM txn"), 1);
    assert!(id > 0);
}

#[test]
fn a_split_whose_parts_do_not_sum_to_its_total_is_rejected() {
    let db = common::empty_db();
    let c = raw(&db);
    let (_, category_id) = insert_group_with_category(c, "Group", "Category");
    let txn_id = insert_txn(c, 1000, 84);
    let result = c.execute(
        "INSERT INTO txn_split (uuid, txn_id, category_id, subtotal_cents, tax_cents, amount_cents)
         VALUES ('u-bad-split', ?1, ?2, 1000, 84, 1085)",
        rusqlite::params![txn_id, category_id],
    );
    assert_constraint(result, CHECK, "CHECK constraint");
}

#[test]
fn a_negative_tax_rate_is_rejected() {
    let db = common::empty_db();
    let c = raw(&db);
    let result = c.execute(
        "INSERT INTO location (uuid, name, tax_rate_ppm) VALUES ('u-below', 'Below', -1)",
        [],
    );
    assert_constraint(result, CHECK, "CHECK constraint");
}

#[test]
fn a_tax_rate_above_one_hundred_percent_is_rejected() {
    let db = common::empty_db();
    let c = raw(&db);
    let result = c.execute(
        "INSERT INTO location (uuid, name, tax_rate_ppm) VALUES ('u-above', 'Above', 1000001)",
        [],
    );
    assert_constraint(result, CHECK, "CHECK constraint");
}

#[test]
fn the_tax_rate_bounds_themselves_are_accepted() {
    // 0 ppm (tax-free) and 1_000_000 ppm (100%) are both legal; the CHECK is
    // inclusive. 94_375 ppm is 9.4375% — the case that motivated ppm over a
    // 2-decimal percent, which would have rounded it away.
    let db = common::empty_db();
    let c = raw(&db);
    insert_location(c, "Tax free", 0, false);
    insert_location(c, "Hundred percent", 1_000_000, false);
    insert_location(c, "Nine point four three seven five", 94_375, false);
    assert_eq!(count(c, "SELECT COUNT(*) FROM location"), 3);
}

// ---------------------------------------------------------------------------
// Referential integrity
// ---------------------------------------------------------------------------

#[test]
fn a_split_referencing_a_missing_category_is_rejected() {
    let db = common::empty_db();
    let c = raw(&db);
    let txn_id = insert_txn(c, 1000, 0);
    let result = c.execute(
        "INSERT INTO txn_split (uuid, txn_id, category_id, subtotal_cents, tax_cents, amount_cents)
         VALUES ('u-orphan-split', ?1, 999999, 1000, 0, 1000)",
        rusqlite::params![txn_id],
    );
    assert_constraint(result, FOREIGN_KEY, "foreign key");
}

#[test]
fn deleting_a_txn_cascades_to_its_splits_and_their_line_items() {
    let db = common::empty_db();
    let c = raw(&db);
    let agg = insert_balanced_aggregate(c);

    assert_eq!(count(c, "SELECT COUNT(*) FROM txn_split"), 1);
    assert_eq!(count(c, "SELECT COUNT(*) FROM line_item"), 2);

    c.execute("DELETE FROM txn WHERE id = ?1", [agg.txn_id])
        .expect("delete txn");

    assert_eq!(
        count(c, "SELECT COUNT(*) FROM txn_split"),
        0,
        "splits must not outlive their transaction"
    );
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM line_item"),
        0,
        "the cascade must reach two levels down, not one"
    );
}

#[test]
fn deleting_a_category_still_referenced_by_a_split_is_blocked() {
    // RESTRICT, not CASCADE: deleting a category must never silently delete
    // spending history. The intended path is archiving.
    let db = common::empty_db();
    let c = raw(&db);
    let agg = insert_balanced_aggregate(c);

    let result = c.execute("DELETE FROM category WHERE id = ?1", [agg.category_id]);
    assert_constraint(result, RESTRICT, "ON DELETE RESTRICT");

    assert_eq!(
        count(c, "SELECT COUNT(*) FROM txn_split"),
        1,
        "the split must survive the refused delete"
    );
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM category"),
        1,
        "and so must the category"
    );
}

#[test]
fn deleting_a_category_group_that_still_holds_categories_is_blocked() {
    // Same reasoning one level up: a group delete must not take the user's
    // categories — and their history — with it.
    let db = common::empty_db();
    let c = raw(&db);
    let (group_id, _) = insert_group_with_category(c, "Group", "Category");

    let result = c.execute("DELETE FROM category_group WHERE id = ?1", [group_id]);
    assert_constraint(result, RESTRICT, "ON DELETE RESTRICT");
}

// ---------------------------------------------------------------------------
// Partial unique indexes
// ---------------------------------------------------------------------------

#[test]
fn two_active_categories_cannot_share_a_name_within_a_group() {
    let db = common::empty_db();
    let c = raw(&db);
    let group = insert_group(c, "Monthly", 100);
    insert_category(c, group, "Groceries", 100);

    let result = c.execute(
        "INSERT INTO category (uuid, group_id, name, sort_order)
         VALUES ('u-dupe', ?1, 'Groceries', 200)",
        [group],
    );
    assert_constraint(result, UNIQUE, "unique index");
}

#[test]
fn the_same_name_may_be_reused_in_a_different_group() {
    let db = common::empty_db();
    let c = raw(&db);
    let a = insert_group(c, "Monthly", 100);
    let b = insert_group(c, "Non-Monthly", 200);
    insert_category(c, a, "Insurance", 100);
    insert_category(c, b, "Insurance", 100);
    assert_eq!(count(c, "SELECT COUNT(*) FROM category"), 2);
}

#[test]
fn an_archived_category_may_share_a_name_with_an_active_one() {
    // The uniqueness index is partial (`WHERE archived_at IS NULL`) precisely
    // so that retiring "Groceries" and starting a fresh "Groceries" is legal.
    // A full unique index would make the archive-instead-of-delete rule
    // unusable after the first reuse.
    let db = common::empty_db();
    let c = raw(&db);
    let group = insert_group(c, "Monthly", 100);
    let old = insert_category(c, group, "Groceries", 100);

    c.execute(
        "UPDATE category SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
        [old],
    )
    .expect("archive category");

    insert_category(c, group, "Groceries", 200);

    assert_eq!(count(c, "SELECT COUNT(*) FROM category"), 2);
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM category WHERE archived_at IS NULL"),
        1
    );
}

#[test]
fn two_archived_categories_may_share_a_name() {
    let db = common::empty_db();
    let c = raw(&db);
    let group = insert_group(c, "Monthly", 100);
    let first = insert_category(c, group, "Gifts", 100);
    c.execute(
        "UPDATE category SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
        [first],
    )
    .expect("archive first");
    let second = insert_category(c, group, "Gifts", 200);
    c.execute(
        "UPDATE category SET archived_at = '2026-02-01T00:00:00Z' WHERE id = ?1",
        [second],
    )
    .expect("archive second");
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM category WHERE archived_at IS NOT NULL"
        ),
        2
    );
}

#[test]
fn only_one_location_may_be_marked_home() {
    let db = common::empty_db();
    let c = raw(&db);
    insert_location(c, "Home town", 94_375, true);

    let result = c.execute(
        "INSERT INTO location (uuid, name, tax_rate_ppm, is_home)
         VALUES ('u-second-home', 'Second home', 0, 1)",
        [],
    );
    assert_constraint(result, UNIQUE, "unique index");

    // Any number of non-home locations is fine — the index is partial.
    insert_location(c, "Vacation", 60_000, false);
    insert_location(c, "Airport", 85_000, false);
    assert_eq!(
        count(c, "SELECT COUNT(*) FROM location WHERE is_home = 1"),
        1
    );
}

// ---------------------------------------------------------------------------
// sort_order
// ---------------------------------------------------------------------------

#[test]
fn seeded_sort_orders_leave_room_to_reorder_without_rewriting_every_row() {
    // Dense 1,2,3… means a single drag renumbers every sibling. Gaps of >= 10
    // let a move become one UPDATE of one row.
    let db = seeded_db();
    let c = raw(&db);

    assert_gaps_of_at_least_ten(
        c,
        "SELECT sort_order FROM category_group ORDER BY sort_order",
    );

    let mut stmt = c
        .prepare("SELECT id FROM category_group ORDER BY sort_order")
        .expect("prepare groups");
    let groups: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .expect("query groups")
        .map(|r| r.expect("group row"))
        .collect();

    for g in groups {
        let sql = format!(
            "SELECT sort_order FROM category WHERE group_id = {g} \
             AND archived_at IS NULL ORDER BY sort_order"
        );
        assert_gaps_of_at_least_ten(c, &sql);
    }
}

#[track_caller]
fn assert_gaps_of_at_least_ten(c: &rusqlite::Connection, sql: &str) {
    let mut stmt = c
        .prepare(sql)
        .unwrap_or_else(|e| panic!("prepare `{sql}`: {e}"));
    let values: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .unwrap_or_else(|e| panic!("query `{sql}`: {e}"))
        .map(|r| r.expect("sort_order row"))
        .collect();

    assert!(!values.is_empty(), "expected at least one row from `{sql}`");
    for pair in values.windows(2) {
        assert!(
            pair[1] - pair[0] >= 10,
            "sort_order values {} and {} are only {} apart (`{sql}`) — \
             seed sparsely (100, 200, 300) so a reorder rewrites one row",
            pair[0],
            pair[1],
            pair[1] - pair[0],
        );
    }
}
