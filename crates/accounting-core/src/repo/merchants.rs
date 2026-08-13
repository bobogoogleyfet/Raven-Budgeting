//! Merchants — the store-memory object.
//!
//! Remembering category, location and detail level per store is what removes
//! three of the six decisions the naive entry flow demanded, and
//! six-decisions-before-a-dollar is the documented reason manual trackers get
//! abandoned in three or four weeks. Everything in this module exists to make
//! the *second* receipt from a store cheaper to enter than the first.

use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::db::{Db, new_uuid};
use crate::error::{CoreError, Result};
use crate::ids::{CategoryId, LocationId, MerchantId};
use crate::repo::not_found;
use crate::repo::transactions::DetailLevel;

/// One store, and what we remember about how the user shops there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merchant {
    pub id: MerchantId,
    pub uuid: String,
    pub name: String,
    pub default_category_id: Option<CategoryId>,
    pub default_location_id: Option<LocationId>,
    /// Per-store, not a global mode: you itemize the grocery run and not the
    /// gas station, and that preference is stable per store.
    pub default_detail_level: DetailLevel,
    /// ISO-8601 UTC, written by the app when a transaction is saved. Drives
    /// the recent-store chips; never read back as "today".
    pub last_used_at: Option<String>,
    pub notes: Option<String>,
    pub archived: bool,
}

const SELECT_MERCHANT: &str = "SELECT id, uuid, name, default_category_id, default_location_id,
                                      default_detail_level, last_used_at, notes,
                                      archived_at IS NOT NULL
                                 FROM merchant";

fn map_merchant(row: &Row<'_>) -> rusqlite::Result<Merchant> {
    Ok(Merchant {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        default_category_id: row.get(3)?,
        default_location_id: row.get(4)?,
        default_detail_level: row.get(5)?,
        last_used_at: row.get(6)?,
        notes: row.get(7)?,
        archived: row.get(8)?,
    })
}

/// Every live merchant, by name.
pub fn list(db: &Db) -> Result<Vec<Merchant>> {
    let sql = format!("{SELECT_MERCHANT} WHERE archived_at IS NULL ORDER BY name");
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt
        .query_map([], map_merchant)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The most recently used merchants, newest first.
///
/// Never-used merchants sort last rather than first: SQLite orders `NULL`
/// before every value in `DESC`, so without the explicit `last_used_at IS NULL`
/// key the recent-store chips would show the stores the user has never bought
/// anything from.
pub fn recent(db: &Db, limit: usize) -> Result<Vec<Merchant>> {
    let sql = format!(
        "{SELECT_MERCHANT}
          WHERE archived_at IS NULL
          ORDER BY last_used_at IS NULL, last_used_at DESC, name
          LIMIT ?1"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![i64::try_from(limit).unwrap_or(i64::MAX)],
            map_merchant,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One merchant by id, archived or not.
pub fn get(db: &Db, id: MerchantId) -> Result<Merchant> {
    let sql = format!("{SELECT_MERCHANT} WHERE id = ?1");
    db.conn()
        .query_row(&sql, params![id], map_merchant)
        .map_err(|e| not_found(e, &format!("merchant {id}")))
}

/// Exact, case-insensitive lookup among live merchants.
pub fn find_by_name(db: &Db, name: &str) -> Result<Option<Merchant>> {
    let needle = name.trim().to_lowercase();
    Ok(list(db)?
        .into_iter()
        .find(|m| m.name.trim().to_lowercase() == needle))
}

/// Find `name`, or create it.
///
/// Trims, and refuses a blank name: the schema's `CHECK (length(trim(name)) >
/// 0)` would too, but " " arriving from a UI field is a user mistake, not a
/// database error, and deserves to read like one.
pub fn get_or_create(db: &Db, name: &str) -> Result<Merchant> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Invalid("merchant name is empty".to_string()));
    }
    if let Some(found) = find_by_name(db, trimmed)? {
        return Ok(found);
    }

    let id = db.with_tx(|tx| {
        tx.execute(
            "INSERT INTO merchant (uuid, name) VALUES (?1, ?2)",
            params![new_uuid(), trimmed],
        )?;
        Ok(MerchantId::new(tx.last_insert_rowid()))
    })?;
    get(db, id)
}

/// Record what the user chose at this store, so the next receipt prefills.
///
/// # `None` means "leave it alone", not "clear it"
///
/// A receipt entered without a location must not erase the location the user
/// taught us last week — the memory would then be destroyed by exactly the
/// fast, sparse entries the memory exists to enable. `detail_level` is
/// different: it is never absent (it has a default), so it always overwrites.
pub fn remember(
    db: &Db,
    id: MerchantId,
    category_id: Option<CategoryId>,
    location_id: Option<LocationId>,
    detail_level: DetailLevel,
) -> Result<()> {
    let changed = db.conn().execute(
        "UPDATE merchant
            SET default_category_id  = COALESCE(?2, default_category_id),
                default_location_id  = COALESCE(?3, default_location_id),
                default_detail_level = ?4,
                updated_at           = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id, category_id, location_id, detail_level],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("merchant {id}")));
    }
    Ok(())
}

/// Stamp `last_used_at`, which orders [`recent`].
///
/// The timestamp is passed in rather than read here: core never reads the
/// clock, so that every test is deterministic without clock mocking and so
/// that chrono's local-timezone lookup — JNI on Android — stays out of the
/// core.
pub fn touch(db: &Db, id: MerchantId, when: &str) -> Result<()> {
    let changed = db.conn().execute(
        "UPDATE merchant
            SET last_used_at = ?2,
                updated_at   = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id, when],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("merchant {id}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;
    use crate::repo::{categories, locations};

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    #[test]
    fn get_or_create_is_idempotent_and_trims() {
        let db = seeded();
        let a = get_or_create(&db, "  H-E-B  ").unwrap();
        assert_eq!(a.name, "H-E-B");
        assert_eq!(a.default_detail_level, DetailLevel::Grouped);
        assert_eq!(a.last_used_at, None);

        let b = get_or_create(&db, "h-e-b").unwrap();
        assert_eq!(a.id, b.id);
        // Counted by name, not by table size: the seed installs suggested
        // stores, so "the table holds one row" stopped being the property.
        // What matters is that a second spelling did not add a second row.
        assert_eq!(
            list(&db)
                .unwrap()
                .iter()
                .filter(|m| m.name.eq_ignore_ascii_case("H-E-B"))
                .count(),
            1
        );

        assert!(matches!(
            get_or_create(&db, " "),
            Err(CoreError::Invalid(_))
        ));
    }

    #[test]
    fn remember_never_clears_what_it_was_not_told() {
        let db = seeded();
        let m = get_or_create(&db, "H-E-B").unwrap();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let home = locations::home(&db).unwrap().unwrap();

        remember(
            &db,
            m.id,
            Some(groceries.id),
            Some(home.id),
            DetailLevel::Itemized,
        )
        .unwrap();
        let stored = get(&db, m.id).unwrap();
        assert_eq!(stored.default_category_id, Some(groceries.id));
        assert_eq!(stored.default_location_id, Some(home.id));
        assert_eq!(stored.default_detail_level, DetailLevel::Itemized);

        // A quick entry with no location must not destroy the remembered one.
        remember(&db, m.id, None, None, DetailLevel::Grouped).unwrap();
        let stored = get(&db, m.id).unwrap();
        assert_eq!(stored.default_category_id, Some(groceries.id));
        assert_eq!(stored.default_location_id, Some(home.id));
        // …but the detail level always takes the new value.
        assert_eq!(stored.default_detail_level, DetailLevel::Grouped);
    }

    #[test]
    fn recent_puts_never_used_merchants_last() {
        let db = seeded();
        let old = get_or_create(&db, "Old Store").unwrap();
        let new = get_or_create(&db, "New Store").unwrap();
        let never = get_or_create(&db, "Never Store").unwrap();

        touch(&db, old.id, "2026-01-01T00:00:00.000Z").unwrap();
        touch(&db, new.id, "2026-08-01T00:00:00.000Z").unwrap();

        // Asserted as an ordering among these three rather than as the whole
        // list: the seed installs a couple of dozen never-used stores, and
        // pinning the exact vector would break every time that list changes
        // without saying anything about the ordering rule.
        let got: Vec<MerchantId> = recent(&db, 100)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        let at = |id: MerchantId| got.iter().position(|m| *m == id).expect("present");
        assert!(at(new.id) < at(old.id), "more recent first");
        assert!(at(old.id) < at(never.id), "used before never-used");

        assert_eq!(recent(&db, 1).unwrap().len(), 1);
        assert!(recent(&db, 0).unwrap().is_empty());
    }

    #[test]
    fn touching_and_remembering_a_missing_merchant_is_not_found() {
        let db = seeded();
        assert!(matches!(
            touch(&db, MerchantId::new(999), "2026-08-01T00:00:00.000Z"),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            remember(&db, MerchantId::new(999), None, None, DetailLevel::Grouped),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            get(&db, MerchantId::new(999)),
            Err(CoreError::NotFound(_))
        ));
    }
}
