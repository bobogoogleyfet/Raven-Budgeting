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
//! "Gas" to "Petrol" does not end up with both.

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
///
/// 4: `uses_restaurant_rate`. Like `uses_food_rate` before it, this is a fact
/// about what the category *is*, so `upsert_category` re-applies it to
/// categories that already exist rather than only to freshly seeded ones —
/// which is also why the bump is belt-and-braces rather than the mechanism.
///
/// The flag alone does not fix an upgraded install: it says which rate to
/// reach for, and the *location* has to have one. Filling that in is
/// `service::rates::ensure_installed`'s backfill.
const SEED_VERSION: i64 = 5;

/// A seed category. `rollover` seeds [`budget_line.rollover`] for lines later
/// created against it.
struct SeedCategory {
    name: &'static str,
    rollover: bool,
    system_key: Option<&'static str>,
    is_budgeted: bool,
    /// Whether a location's reduced food rate applies. Unprepared food only:
    /// Restaurants are prepared food and taxed *above* the general rate, which
    /// is [`SeedCategory::restaurant`].
    food: bool,
    /// Whether the county's restaurant tax on prepared food applies.
    /// Mutually exclusive with [`Self::food`].
    restaurant: bool,
    /// Whether the amount entered here is a pre-tax subtotal that the app
    /// should add sales tax to. See [`SeedCategory::billed`].
    taxable: bool,
}

impl SeedCategory {
    const fn user(name: &'static str) -> Self {
        SeedCategory {
            name,
            rollover: false,
            system_key: None,
            is_budgeted: true,
            food: false,
            restaurant: false,
            taxable: true,
        }
    }

    /// A category you are *billed* for rather than shop for.
    ///
    /// The amount entered is what was paid — a rent cheque, a phone bill, a
    /// tank of fuel, a hotel folio — with whatever taxes and fees already
    /// inside it. Adding a sales rate on top invents money nobody was charged:
    /// $1,500 of rent became $1,611.75, matching no statement anywhere.
    ///
    /// Several of these are not sales-taxed at all in Utah. Residential rent
    /// is outside the tax base; broadband is barred from state taxation by the
    /// federal Internet Tax Freedom Act; insurance carries a premium tax paid
    /// by the insurer, never billed to the customer; motor fuel pays a
    /// per-gallon excise instead, already in the pump price.
    ///
    /// The rest are taxed, but not at the general rate and not in a way this
    /// app could reconstruct: electricity carries a reduced state rate plus a
    /// municipal energy tax, a phone bill carries 911 fees and a telecom
    /// licence tax, and a hotel stay stacks transient room taxes that can add
    /// half again as much as sales tax. In every one of those cases the figure
    /// on the bill is already the whole story.
    const fn billed(name: &'static str) -> Self {
        SeedCategory::user(name).untaxed()
    }

    /// No sales tax is added to what is entered here.
    const fn untaxed(self) -> Self {
        SeedCategory {
            taxable: false,
            ..self
        }
    }

    /// A category taxed at the county's prepared-food rate where one exists.
    ///
    /// Utah counties may levy a Restaurant Tax of up to 1% on prepared food
    /// (Utah Code 59-12-603) on top of the combined rate, so a meal is taxed
    /// *higher* than a shirt, not lower. Before this existed the app charged
    /// the general rate here and every restaurant receipt came out 1% light —
    /// $1.01 on a $13.59 subtotal where the receipt said $1.15.
    const fn restaurant(name: &'static str) -> Self {
        SeedCategory {
            restaurant: true,
            ..SeedCategory::user(name)
        }
    }

    /// A category taxed at the reduced grocery rate where one exists.
    const fn groceries(name: &'static str) -> Self {
        SeedCategory {
            food: true,
            ..SeedCategory::user(name)
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

    /// The same category, but accumulating month to month. See
    /// [`SeedCategory::sinking`].
    const fn sinking_fund(self) -> Self {
        SeedCategory {
            rollover: true,
            ..self
        }
    }

    const fn system(name: &'static str, key: &'static str) -> Self {
        SeedCategory {
            name,
            rollover: false,
            food: false,
            // Taxable, because the two that see real money stand in for an
            // ordinary purchase: Uncategorized is whatever the receipt turned
            // out to be, and a reimbursable lunch was bought at a till like
            // any other. Only Transfer is exempted, below, and that is because
            // moving money between your own accounts is not buying anything.
            //
            // It makes no difference to `quick_add`, which resolves no
            // location and so has no rate to apply either way; it matters when
            // a receipt is filed against one of these at a place that does.
            taxable: true,
            restaurant: false,
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
    /// Whether money budgeted here is *owed* rather than merely available.
    ///
    /// Decides one word on the budget screen and it is the difference between
    /// sense and nonsense: rent budgeted and unpaid is $900 behind, groceries
    /// budgeted and unspent is $250 left.
    obligation: bool,
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
        // A bill not yet paid is a debt that has not gone away.
        obligation: true,
        categories: &[
            // Every one of these is a bill, not a shop. See `billed`.
            SeedCategory::billed("Rent"),
            SeedCategory::billed("Electric"),
            SeedCategory::billed("Internet"),
            SeedCategory::billed("Phone"),
            SeedCategory::billed("Car Insurance"),
            SeedCategory::billed("Health Insurance"),
            SeedCategory::billed("Subscriptions"),
        ],
    },
    SeedGroup {
        // Where the receipts actually come from. These are the categories the
        // entry screen should surface first.
        name: "Frequent",
        hidden: false,
        obligation: false,
        categories: &[
            SeedCategory::groceries("Groceries"),
            SeedCategory::restaurant("Restaurants"),
            // Motor fuel pays a per-gallon excise, not sales tax, and it is
            // already in the price on the sign.
            SeedCategory::billed("Gas"),
            SeedCategory::user("Household"),
            SeedCategory::user("Toiletries"),
        ],
    },
    SeedGroup {
        // The group that makes the whole yearly-expense feature work. All
        // rollover: a $900 car repair in one month is not a $900 overspend, it
        // is nine months of $100 finally being spent.
        name: "Occasional",
        hidden: false,
        obligation: false,
        categories: &[
            // Utah taxes repair labour as well as parts, unlike most states.
            SeedCategory::sinking("Car Repair"),
            // Doctor visits are a service and untaxed; prescriptions are
            // exempt. Over-the-counter medicine is taxed, and is the exception
            // — correct it on the receipt when it comes up.
            SeedCategory::billed("Medical").sinking_fund(),
            SeedCategory::sinking("Gifts"),
            SeedCategory::sinking("Clothing"),
            // A hotel folio stacks transient room taxes on top of sales tax,
            // adding half again as much; the figure on the bill is the whole
            // story. Airfare carries federal excise and no Utah sales tax.
            SeedCategory::billed("Travel").sinking_fund(),
        ],
    },
    SeedGroup {
        name: "Quality of Life",
        hidden: false,
        obligation: false,
        categories: &[
            SeedCategory::user("Entertainment"),
            SeedCategory::user("Hobbies"),
            SeedCategory::user("Fitness"),
        ],
    },
    SeedGroup {
        // Seeded empty, on purpose. Education costs are somebody's tuition,
        // somebody else's music lessons and somebody else's textbooks, and
        // guessing wrong here fills a list with categories nobody will file
        // against. The group is what makes them easy to add.
        name: "Education",
        hidden: false,
        obligation: false,
        categories: &[],
    },
    SeedGroup {
        name: "Savings",
        hidden: false,
        // Money you owe your future self. Not putting it aside is falling
        // behind in exactly the sense the word means.
        obligation: true,
        // Rollover for the same reason as Non-Monthly, more obviously: an
        // emergency fund that resets each month is a chequing account.
        // Money moved into savings is not a purchase either.
        categories: &[SeedCategory::sinking("Emergency Fund").untaxed()],
    },
    SeedGroup {
        // Hidden, so the three load-bearing categories do not clutter the
        // picker or the budget screen. They are still ordinary rows the user
        // may rename; the triggers only stop deletion and archiving.
        name: "System",
        hidden: true,
        obligation: false,
        categories: &[
            SeedCategory::system("Uncategorized", SYSTEM_UNCATEGORIZED),
            // Money moved between the user's own accounts is not a purchase.
            SeedCategory::system("Transfer", SYSTEM_TRANSFER).untaxed(),
            SeedCategory::system("Reimbursable", SYSTEM_REIMBURSABLE),
        ],
    },
];

/// Seed names that changed after release, old to new.
///
/// The seed matches categories by name, so renaming one in `SEED_GROUPS`
/// without this would leave the old row in place and add a second one beside
/// it — the user would end up with both "Personal Care" and "Toiletries", and
/// their history split across the two. Applied only when the old name is still
/// present and the new one is not, so a user who already renamed the category
/// themselves is left alone.
/// Group renames, for the same reason as [`SEED_RENAMES`] and with the same
/// caution.
///
/// Groups are matched by name too, so renaming one here without this would
/// leave the old group in place with every category still inside it, and add
/// an empty one beside it.
///
/// "Non-Monthly" said when the spending happens; what someone is choosing
/// between is what the group is *for*. It is money set aside for things that
/// come round now and then, and "Occasional" says that without a negation.
const SEED_GROUP_RENAMES: &[(&str, &str)] = &[("Non-Monthly", "Occasional")];

const SEED_RENAMES: &[(&str, &str)] = &[
    ("Rent/Mortgage", "Rent"),
    ("Insurance (Auto)", "Car Insurance"),
    ("Insurance (Health)", "Health Insurance"),
    ("Restaurants & Takeout", "Restaurants"),
    ("Gas/Fuel", "Gas"),
    ("Household Supplies", "Household"),
    ("Personal Care", "Toiletries"),
    ("Auto Maintenance", "Car Repair"),
    ("Medical & Dental", "Medical"),
];

/// The one location every install starts with.
///
/// Rate 0 on purpose. A wrong prefilled rate is worse than no prefill: it
/// silently produces plausible tax figures that disagree with the receipt.
/// The user sets their real rate once, in settings.
const HOME_LOCATION: &str = "Home";

/// A store the app suggests before the user has been anywhere.
///
/// `category` names a seeded category to prefill. That is what makes the
/// store-memory feature useful on day one: picking Maverik selects Gas/Fuel
/// without the user teaching it first. Stores that genuinely span categories
/// (Amazon, Target) carry `None` rather than a guess, because a wrong prefill
/// that looks confident is worse than an empty one.
struct SeedMerchant {
    name: &'static str,
    category: Option<&'static str>,
}

impl SeedMerchant {
    const fn new(name: &'static str, category: &'static str) -> Self {
        SeedMerchant {
            name,
            category: Some(category),
        }
    }

    const fn mixed(name: &'static str) -> Self {
        SeedMerchant {
            name,
            category: None,
        }
    }
}

/// Stores within reach of Highland, Utah, plus the online ones everyone uses.
///
/// Regional on purpose: Macey's, Harmons, Maverik and Swig are Utah chains and
/// would be noise anywhere else, while a generic national list would miss the
/// places this user actually shops. A store that is wrong for them costs one
/// archive; a missing one costs typing it every visit.
const SEED_MERCHANTS: &[SeedMerchant] = &[
    // Groceries and warehouse
    SeedMerchant::new("Costco", "Groceries"),
    SeedMerchant::new("Sam's Club", "Groceries"),
    SeedMerchant::new("Macey's", "Groceries"),
    SeedMerchant::new("Smith's", "Groceries"),
    SeedMerchant::new("Harmons", "Groceries"),
    SeedMerchant::new("WinCo Foods", "Groceries"),
    SeedMerchant::new("Walmart", "Groceries"),
    // Mixed-basket stores get no default: guessing Groceries for a Target run
    // that was mostly household goods files the money in the wrong envelope.
    SeedMerchant::mixed("Target"),
    // Fuel
    SeedMerchant::new("Maverik", "Gas"),
    SeedMerchant::new("Chevron", "Gas"),
    SeedMerchant::new("Sinclair", "Gas"),
    SeedMerchant::new("Holiday Oil", "Gas"),
    // Home
    SeedMerchant::new("Home Depot", "Household"),
    SeedMerchant::new("Lowe's", "Household"),
    // Pharmacy
    SeedMerchant::new("Walgreens", "Toiletries"),
    SeedMerchant::new("CVS", "Toiletries"),
    // Eating out
    SeedMerchant::new("Cafe Rio", "Restaurants"),
    SeedMerchant::new("Costa Vida", "Restaurants"),
    SeedMerchant::new("Chick-fil-A", "Restaurants"),
    SeedMerchant::new("In-N-Out Burger", "Restaurants"),
    SeedMerchant::new("Swig", "Restaurants"),
    // Online
    SeedMerchant::mixed("Amazon"),
    SeedMerchant::mixed("eBay"),
];

/// Install the default categories, home location and store suggestions.
/// Idempotent.
pub fn seed_defaults(db: &Db) -> Result<()> {
    db.with_tx(|tx| {
        // Before the upserts, or they would insert the new names alongside the
        // old ones instead of finding them.
        for (old, new) in SEED_GROUP_RENAMES {
            rename_group(tx, old, new)?;
        }
        for (old, new) in SEED_RENAMES {
            rename_category(tx, old, new)?;
        }
        for (gi, group) in SEED_GROUPS.iter().enumerate() {
            let group_id = upsert_group(tx, group, sparse(gi))?;
            for (ci, cat) in group.categories.iter().enumerate() {
                upsert_category(tx, group_id, cat, sparse(ci))?;
            }
        }
        upsert_home_location(tx)?;
        for merchant in SEED_MERCHANTS {
            upsert_merchant(tx, merchant)?;
        }
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
        // Re-applied to a group that already exists, like the category flags
        // below: whether a group holds obligations is a fact about what it is,
        // not a preference the user set, and an upgraded database would
        // otherwise keep calling an unspent grocery envelope "behind" forever.
        tx.execute(
            "UPDATE category_group SET is_obligation = ?2 WHERE id = ?1",
            params![id, i64::from(group.obligation)],
        )?;
        return Ok(id);
    }

    // A group added to the seed later lands in the middle of the list, and on
    // a database that already exists its slot belongs to somebody else —
    // Education's 500 was Savings'. Two groups sharing a sort order put their
    // order in the hands of the tiebreak, so a new group goes after the
    // existing ones instead. A fresh database still gets 100, 200, 300…
    let taken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM category_group WHERE sort_order = ?1)",
        params![sort_order],
        |r| r.get(0),
    )?;
    let sort_order = if taken {
        tx.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 100 FROM category_group",
            [],
            |r| r.get(0),
        )?
    } else {
        sort_order
    };

    tx.execute(
        "INSERT INTO category_group (uuid, name, is_hidden, is_obligation, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            new_uuid(),
            group.name,
            i64::from(group.hidden),
            i64::from(group.obligation),
            sort_order
        ],
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
        // Both flags are set even on a category that already exists: they
        // arrived in later releases, and a database seeded before them would
        // otherwise keep taxing groceries at the full rate and adding sales
        // tax to rent forever. They are facts about what the category *is*,
        // not preferences the user set.
        tx.execute(
            "UPDATE category
                SET uses_food_rate = ?2, uses_restaurant_rate = ?3, taxable = ?4
              WHERE id = ?1",
            params![
                id,
                i64::from(cat.food),
                i64::from(cat.restaurant),
                i64::from(cat.taxable)
            ],
        )?;
        return Ok(id);
    }

    tx.execute(
        "INSERT INTO category
             (uuid, group_id, name, system_key, is_budgeted, uses_food_rate,
              uses_restaurant_rate, taxable, default_rollover, sort_order)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            new_uuid(),
            group_id,
            cat.name,
            cat.system_key,
            i64::from(cat.is_budgeted),
            i64::from(cat.food),
            i64::from(cat.restaurant),
            i64::from(cat.taxable),
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

/// Rename a seeded category that was given a shorter name after release.
///
/// Renames the row rather than adding one, so every transaction, budget line
/// and adjustment already pointing at it follows automatically — they hold its
/// id, not its name. A no-op unless the old name is present and the new one is
/// free.
/// Rename a group, unless the new name is already in use.
///
/// Same rule as [`rename_category`]: a user who has already made a group of
/// that name is left alone rather than having two merged behind their back.
fn rename_group(tx: &Transaction<'_>, old: &str, new: &str) -> Result<()> {
    let taken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM category_group WHERE name = ?1)",
        params![new],
        |r| r.get(0),
    )?;
    if taken {
        return Ok(());
    }
    tx.execute(
        "UPDATE category_group SET name = ?2 WHERE name = ?1",
        params![old, new],
    )?;
    Ok(())
}

fn rename_category(tx: &Transaction<'_>, old: &str, new: &str) -> Result<()> {
    let taken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM category WHERE name = ?1)",
        params![new],
        |r| r.get(0),
    )?;
    if taken {
        return Ok(());
    }
    tx.execute(
        "UPDATE category SET name = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE name = ?1",
        params![old, new],
    )?;
    Ok(())
}

/// Insert a suggested store, unless one by that name is already there.
///
/// Matched on name including archived rows, for the same reason groups are:
/// resurrecting a store the user archived would be as rude as duplicating it.
/// A store they hard-deleted does come back, which is the same bargain the
/// default categories make.
fn upsert_merchant(tx: &Transaction<'_>, merchant: &SeedMerchant) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM merchant WHERE name = ?1)",
        params![merchant.name],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(());
    }

    // Resolved by name rather than hard-coded id: the categories were inserted
    // moments ago in this same transaction and their ids are whatever the
    // database assigned. A missing name yields NULL instead of failing, so
    // renaming a seeded category cannot break seeding.
    let category_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM category WHERE name = ?1 AND archived_at IS NULL",
            params![merchant.category],
            |r| r.get(0),
        )
        .ok();

    tx.execute(
        "INSERT INTO merchant (uuid, name, default_category_id) VALUES (?1, ?2, ?3)",
        params![new_uuid(), merchant.name, category_id],
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
        // Seven: the six the app started with, plus Education, which seeds as
        // a group with nothing in it.
        assert_eq!(count(&db, "SELECT count(*) FROM category_group"), 7);
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
                "Car Repair",
                "Clothing",
                "Emergency Fund",
                "Gifts",
                "Medical",
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
        assert_eq!(orders, [100, 200, 300, 400, 500, 600, 700]);
        assert!(orders.windows(2).all(|w| w[1] - w[0] >= 100));
    }

    #[test]
    fn seeding_twice_changes_nothing() {
        let db = seeded();
        let before = count(&db, "SELECT count(*) FROM category");
        seed_defaults(&db).unwrap();
        seed_defaults(&db).unwrap();
        assert_eq!(count(&db, "SELECT count(*) FROM category"), before);
        assert_eq!(count(&db, "SELECT count(*) FROM category_group"), 7);
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
