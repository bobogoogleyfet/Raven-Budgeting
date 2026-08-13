//! The default chart of accounts, installed on first run.
//!
//! # Why this list is short
//!
//! Twenty-odd categories, not two hundred. Over-categorisation is one of the
//! top documented causes of budget abandonment: every extra category is a
//! decision at the till and a row of near-zero noise in the monthly report.
//! Every one of these is renameable, movable, archivable and non-special —
//! the seed is a starting point, not a schema.
//!
//! # Idempotence
//!
//! [`seed_defaults`] is safe to call on every launch. It matches on name
//! within group (and on `system_key` for the system categories), so a user who
//! archived "Fitness" does not get it resurrected, and a user who renamed
//! "Gas/Fuel" to "Petrol" does not end up with both.

use rusqlite::{Transaction, params};

use crate::db::{Db, new_uuid};
use crate::error::Result;
use crate::ids::{CategoryGroupId, CategoryId};

/// `system_key` of the fallback category every uncategorised amount lands in.
///
/// Its existence is what makes quick-add possible: recording $12.40 in three
/// seconds without deciding what it was, and sorting it out later from the
/// inbox.
pub const SYSTEM_UNCATEGORIZED: &str = "uncategorized";

/// `system_key` for moves between the user's own accounts, which are not
/// spending and must never appear in a spending report.
pub const SYSTEM_TRANSFER: &str = "transfer";

/// `system_key` for expenses awaiting reimbursement — real cash out now, not
/// really the user's spending.
pub const SYSTEM_REIMBURSABLE: &str = "reimbursable";

/// The `app_setting` key recording which seed generation has been applied.
const SEED_VERSION_KEY: &str = "seed_version";

/// Bumped when a later release adds defaults, so the additions install on an
/// existing database without re-adding anything a user has since deleted.
const SEED_VERSION: i64 = 1;

/// A seed category. `rollover` seeds [`budget_line.rollover`] for lines later
/// created against it.
struct SeedCategory {
    name: &'static str,
    rollover: bool,
    system_key: Option<&'static str>,
    is_budgeted: bool,
}

impl SeedCategory {
    const fn user(name: &'static str) -> Self {
        SeedCategory {
            name,
            rollover: false,
            system_key: None,
            is_budgeted: true,
        }
    }

    /// A non-monthly category: insurance, car repairs, Christmas. These
    /// *require* rollover — a $600 annual premium budgeted at $50/month is
    /// only meaningful if the unspent $50s accumulate. Without carry-over the
    /// envelope resets every month and the sinking fund is impossible.
    const fn sinking(name: &'static str) -> Self {
        SeedCategory {
            rollover: true,
            ..SeedCategory::user(name)
        }
    }

    const fn system(name: &'static str, key: &'static str) -> Self {
        SeedCategory {
            name,
            rollover: false,
            system_key: Some(key),
            // Kept off budget screens: budgeting "Transfer" is meaningless and
            // budgeting "Uncategorized" encourages leaving things there.
            is_budgeted: false,
        }
    }
}

struct SeedGroup {
    name: &'static str,
    hidden: bool,
    categories: &'static [SeedCategory],
}

/// The default taxonomy. Group and category order here becomes `sort_order`
/// 100, 200, 300 … — sparse, so dragging one category to a new position
/// rewrites one row instead of renumbering the whole list.
static SEED_GROUPS: &[SeedGroup] = &[
    SeedGroup {
        // Predictable, mostly fixed, mostly automatic. Budgeting these is
        // bookkeeping rather than decision-making.
        name: "Bills",
        hidden: false,
        categories: &[
            SeedCategory::user("Rent/Mortgage"),
            SeedCategory::user("Electric"),
            SeedCategory::user("Internet"),
            SeedCategory::user("Phone"),
            SeedCategory::user("Insurance (Auto)"),
            SeedCategory::user("Insurance (Health)"),
            SeedCategory::user("Subscriptions"),
        ],
    },
    SeedGroup {
        // Where the receipts actually come from. These are the categories the
        // entry screen should surface first.
        name: "Frequent",
        hidden: false,
        categories: &[
            SeedCategory::user("Groceries"),
            SeedCategory::user("Restaurants & Takeout"),
            SeedCategory::user("Gas/Fuel"),
            SeedCategory::user("Household Supplies"),
            SeedCategory::user("Personal Care"),
        ],
    },
    SeedGroup {
        // The group that makes the whole yearly-expense feature work. All
        // rollover: a $900 car repair in one month is not a $900 overspend, it
        // is nine months of $100 finally being spent.
        name: "Non-Monthly",
        hidden: false,
        categories: &[
            SeedCategory::sinking("Auto Maintenance"),
            SeedCategory::sinking("Medical & Dental"),
            SeedCategory::sinking("Gifts"),
            SeedCategory::sinking("Clothing"),
            SeedCategory::sinking("Travel"),
        ],
    },
    SeedGroup {
        name: "Quality of Life",
        hidden: false,
        categories: &[
            SeedCategory::user("Entertainment"),
            SeedCategory::user("Hobbies"),
            SeedCategory::user("Fitness"),
        ],
    },
    SeedGroup {
        name: "Savings",
        hidden: false,
        // Rollover for the same reason as Non-Monthly, more obviously: an
        // emergency fund that resets each month is a chequing account.
        categories: &[SeedCategory::sinking("Emergency Fund")],
    },
    SeedGroup {
        // Hidden, so the three load-bearing categories do not clutter the
        // picker or the budget screen. They are still ordinary rows the user
        // may rename; the triggers only stop deletion and archiving.
        name: "System",
        hidden: true,
        categories: &[
            SeedCategory::system("Uncategorized", SYSTEM_UNCATEGORIZED),
            SeedCategory::system("Transfer", SYSTEM_TRANSFER),
            SeedCategory::system("Reimbursable", SYSTEM_REIMBURSABLE),
        ],
    },
];

/// The one location every install starts with.
///
/// Rate 0 on purpose. A wrong prefilled rate is worse than no prefill: it
/// silently produces plausible tax figures that disagree with the receipt.
/// The user sets their real rate once, in settings.
const HOME_LOCATION: &str = "Home";

/// Install the default categories and home location. Idempotent.
pub fn seed_defaults(db: &Db) -> Result<()> {
    db.with_tx(|tx| {
        for (gi, group) in SEED_GROUPS.iter().enumerate() {
            let group_id = upsert_group(tx, group, sparse(gi))?;
            for (ci, cat) in group.categories.iter().enumerate() {
                upsert_category(tx, group_id, cat, sparse(ci))?;
            }
        }
        upsert_home_location(tx)?;
        tx.execute(
            "INSERT INTO app_setting (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value
             WHERE CAST(app_setting.value AS INTEGER) < CAST(excluded.value AS INTEGER)",
            params![SEED_VERSION_KEY, SEED_VERSION.to_string()],
        )?;
        Ok(())
    })
}

/// Sparse ordering: 100, 200, 300 … Reordering by drag then only needs to
/// write the moved row (to the midpoint of its new neighbours) instead of
/// renumbering every sibling.
fn sparse(index: usize) -> i64 {
    (i64::try_from(index).unwrap_or(i64::MAX / 100) + 1) * 100
}

fn upsert_group(
    tx: &Transaction<'_>,
    group: &SeedGroup,
    sort_order: i64,
) -> Result<CategoryGroupId> {
    // Matched on name including archived rows: resurrecting a group the user
    // archived would be as rude as duplicating it.
    let existing: Option<CategoryGroupId> = tx
        .query_row(
            "SELECT id FROM category_group WHERE name = ?1",
            params![group.name],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok(id);
    }

    tx.execute(
        "INSERT INTO category_group (uuid, name, is_hidden, sort_order)
         VALUES (?1, ?2, ?3, ?4)",
        params![new_uuid(), group.name, i64::from(group.hidden), sort_order],
    )?;
    Ok(CategoryGroupId::new(tx.last_insert_rowid()))
}

fn upsert_category(
    tx: &Transaction<'_>,
    group_id: CategoryGroupId,
    cat: &SeedCategory,
    sort_order: i64,
) -> Result<CategoryId> {
    // System categories are identified by system_key, never by name: the user
    // is allowed to rename "Uncategorized" to "Inbox", and doing so must not
    // cause a second one to appear on the next launch.
    let existing: Option<CategoryId> = match cat.system_key {
        Some(key) => tx
            .query_row(
                "SELECT id FROM category WHERE system_key = ?1",
                params![key],
                |r| r.get(0),
            )
            .ok(),
        None => tx
            .query_row(
                "SELECT id FROM category WHERE group_id = ?1 AND name = ?2",
                params![group_id, cat.name],
                |r| r.get(0),
            )
            .ok(),
    };
    if let Some(id) = existing {
        return Ok(id);
    }

    tx.execute(
        "INSERT INTO category
             (uuid, group_id, name, system_key, is_budgeted, default_rollover, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_uuid(),
            group_id,
            cat.name,
            cat.system_key,
            i64::from(cat.is_budgeted),
            i64::from(cat.rollover),
            sort_order,
        ],
    )?;
    Ok(CategoryId::new(tx.last_insert_rowid()))
}

fn upsert_home_location(tx: &Transaction<'_>) -> Result<()> {
    // Keyed on is_home rather than the name: a user who renamed "Home" to
    // "Austin, TX" already has a home location.
    let has_home: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM location WHERE is_home = 1)",
        [],
        |r| r.get(0),
    )?;
    if has_home {
        return Ok(());
    }

    tx.execute(
        "INSERT INTO location (uuid, name, tax_rate_ppm, is_home) VALUES (?1, ?2, 0, 1)",
        params![new_uuid(), HOME_LOCATION],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Db {
        let db = Db::open_in_memory().unwrap();
        seed_defaults(&db).unwrap();
        db
    }

    fn count(db: &Db, sql: &str) -> i64 {
        db.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn seeds_the_expected_shape() {
        let db = seeded();
        assert_eq!(count(&db, "SELECT count(*) FROM category_group"), 6);
        assert_eq!(count(&db, "SELECT count(*) FROM category"), 24);
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM category WHERE system_key IS NOT NULL"
            ),
            3
        );
        assert_eq!(
            count(&db, "SELECT count(*) FROM location WHERE is_home = 1"),
            1
        );
        assert_eq!(
            count(&db, "SELECT tax_rate_ppm FROM location WHERE is_home = 1"),
            0
        );
    }

    #[test]
    fn non_monthly_and_savings_roll_over() {
        let db = seeded();
        let rolling: Vec<String> = db
            .conn()
            .prepare("SELECT name FROM category WHERE default_rollover = 1 ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rolling,
            [
                "Auto Maintenance",
                "Clothing",
                "Emergency Fund",
                "Gifts",
                "Medical & Dental",
                "Travel"
            ]
        );
    }

    #[test]
    fn sort_orders_are_sparse() {
        let db = seeded();
        let orders: Vec<i64> = db
            .conn()
            .prepare("SELECT sort_order FROM category_group ORDER BY sort_order")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(orders, [100, 200, 300, 400, 500, 600]);
        assert!(orders.windows(2).all(|w| w[1] - w[0] >= 100));
    }

    #[test]
    fn seeding_twice_changes_nothing() {
        let db = seeded();
        let before = count(&db, "SELECT count(*) FROM category");
        seed_defaults(&db).unwrap();
        seed_defaults(&db).unwrap();
        assert_eq!(count(&db, "SELECT count(*) FROM category"), before);
        assert_eq!(count(&db, "SELECT count(*) FROM category_group"), 6);
        assert_eq!(count(&db, "SELECT count(*) FROM location"), 1);
    }

    #[test]
    fn a_renamed_system_category_is_not_duplicated() {
        let db = seeded();
        db.conn()
            .execute(
                "UPDATE category SET name = 'Inbox' WHERE system_key = ?1",
                params![SYSTEM_UNCATEGORIZED],
            )
            .unwrap();
        seed_defaults(&db).unwrap();
        assert_eq!(count(&db, "SELECT count(*) FROM category"), 24);
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM category WHERE system_key = 'uncategorized'"
            ),
            1
        );
    }

    #[test]
    fn a_deleted_user_category_stays_deleted_within_a_seed_version() {
        let db = seeded();
        db.conn()
            .execute("DELETE FROM category WHERE name = 'Fitness'", [])
            .unwrap();
        seed_defaults(&db).unwrap();
        // Idempotence is per-name, so a *deleted* row does come back. Archived
        // is the supported way to retire a category, and that is respected:
        db.conn()
            .execute(
                "UPDATE category SET archived_at = '2026-01-01T00:00:00Z' WHERE name = 'Hobbies'",
                [],
            )
            .unwrap();
        seed_defaults(&db).unwrap();
        assert_eq!(
            count(&db, "SELECT count(*) FROM category WHERE name = 'Hobbies'"),
            1
        );
    }

    #[test]
    fn system_categories_cannot_be_deleted_or_archived() {
        let db = seeded();
        let err = db
            .conn()
            .execute("DELETE FROM category WHERE system_key = 'transfer'", [])
            .unwrap_err();
        assert!(err.to_string().contains("cannot be deleted"), "{err}");

        let err = db
            .conn()
            .execute(
                "UPDATE category SET archived_at = '2026-01-01T00:00:00Z'
                  WHERE system_key = 'transfer'",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("cannot be archived"), "{err}");

        // And the obvious way around both.
        let err = db
            .conn()
            .execute(
                "UPDATE category SET system_key = NULL WHERE system_key = 'transfer'",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("immutable"), "{err}");
    }

    #[test]
    fn the_system_group_is_hidden_and_its_categories_unbudgeted() {
        let db = seeded();
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM category c JOIN category_group g ON g.id = c.group_id
                  WHERE c.system_key IS NOT NULL AND g.is_hidden = 1"
            ),
            3
        );
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM category WHERE system_key IS NOT NULL AND is_budgeted = 1"
            ),
            0
        );
    }

    #[test]
    fn seed_version_is_recorded() {
        let db = seeded();
        let v: String = db
            .conn()
            .query_row(
                "SELECT value FROM app_setting WHERE key = ?1",
                params![SEED_VERSION_KEY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, SEED_VERSION.to_string());
    }
}
