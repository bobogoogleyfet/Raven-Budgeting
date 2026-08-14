//! Locations — where the *store* is, which is what determines sales tax.
//!
//! The rate stored here only prefills an editable estimate. The receipt prints
//! the real tax and the real tax always wins; the per-transaction
//! `tax_rate_ppm` records what was actually used at entry time, so changing a
//! location's rate never rewrites last year's numbers.

use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::db::{Db, new_uuid};
use crate::error::{CoreError, Result};
use crate::ids::LocationId;
use crate::repo::not_found;

/// One place the user shops, and the tax rate in force there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub id: LocationId,
    pub uuid: String,
    pub name: String,
    /// Parts per million: 8.375% is exactly `83_750`. Percent-with-2-decimals
    /// would round that away and a float would be differently wrong.
    pub tax_rate_ppm: u32,
    /// The reduced rate for food categories here, if this place has one.
    /// `None` means food is taxed like everything else.
    pub food_rate_ppm: Option<u32>,
    /// The published jurisdiction these rates came from, e.g. `25-090`.
    ///
    /// `None` means somebody typed the rate, and a quarterly refresh leaves it
    /// alone: a figure read off a receipt outranks a published estimate.
    pub area_code: Option<String>,
    /// Shelf prices already include tax (rare in the US, normal under VAT).
    pub prices_include_tax: bool,
    pub is_home: bool,
    pub notes: Option<String>,
    pub archived: bool,
}

/// The largest legal rate, matching the schema's `CHECK … BETWEEN 0 AND
/// 1000000` — 100%.
const MAX_RATE_PPM: u32 = 1_000_000;

const SELECT_LOCATION: &str =
    "SELECT id, uuid, name, tax_rate_ppm, food_rate_ppm, area_code,
                                      prices_include_tax,
                                      is_home, notes, archived_at IS NOT NULL
                                 FROM location";

fn map_location(row: &Row<'_>) -> rusqlite::Result<Location> {
    Ok(Location {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        tax_rate_ppm: row.get(3)?,
        food_rate_ppm: row.get(4)?,
        area_code: row.get(5)?,
        prices_include_tax: row.get(6)?,
        is_home: row.get(7)?,
        notes: row.get(8)?,
        archived: row.get(9)?,
    })
}

/// Every live location, by name.
pub fn list(db: &Db) -> Result<Vec<Location>> {
    let sql = format!("{SELECT_LOCATION} WHERE archived_at IS NULL ORDER BY name");
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt
        .query_map([], map_location)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One location by id, archived or not — historical transactions must keep
/// resolving their location name.
pub fn get(db: &Db, id: LocationId) -> Result<Location> {
    let sql = format!("{SELECT_LOCATION} WHERE id = ?1");
    db.conn()
        .query_row(&sql, params![id], map_location)
        .map_err(|e| not_found(e, &format!("location {id}")))
}

/// Exact, case-insensitive lookup among live locations.
pub fn find_by_name(db: &Db, name: &str) -> Result<Option<Location>> {
    let needle = name.trim().to_lowercase();
    Ok(list(db)?
        .into_iter()
        .find(|l| l.name.trim().to_lowercase() == needle))
}

/// The user's home location, if one is set.
///
/// At most one exists — a partial unique index enforces it — because a second
/// home is an entry bug rather than a preference.
pub fn home(db: &Db) -> Result<Option<Location>> {
    let sql = format!("{SELECT_LOCATION} WHERE is_home = 1 AND archived_at IS NULL");
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query_map([], map_location)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Find `name`, or create it with `tax_rate_ppm`.
///
/// **An existing location wins and its rate is left alone.** Overwriting it
/// here would let a single mistyped `--rate` on one receipt silently change
/// the prefill for every future entry at that store; changing a rate is
/// [`set_rate`], which is a deliberate act.
pub fn get_or_create(db: &Db, name: &str, tax_rate_ppm: u32) -> Result<Location> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Invalid("location name is empty".to_string()));
    }
    check_rate(tax_rate_ppm)?;

    if let Some(found) = find_by_name(db, trimmed)? {
        return Ok(found);
    }

    let id = db.with_tx(|tx| {
        tx.execute(
            "INSERT INTO location (uuid, name, tax_rate_ppm) VALUES (?1, ?2, ?3)",
            params![new_uuid(), trimmed, tax_rate_ppm],
        )?;
        Ok(LocationId::new(tx.last_insert_rowid()))
    })?;
    get(db, id)
}

/// Set the tax rate that prefills future entries at this location.
/// Rename a location.
///
/// Used when the user names where they live on first run: the seed installs a
/// placeholder "Home", and this replaces it rather than adding a second row,
/// so every receipt already pointing at it follows automatically — they hold
/// its id, not its name.
pub fn rename(db: &Db, id: LocationId, name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Invalid("a location needs a name".to_string()));
    }
    let changed = db.conn().execute(
        "UPDATE location
            SET name = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id, trimmed],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("location {id}")));
    }
    Ok(())
}

/// Set the reduced rate for food categories at this location.
///
/// `None` removes it, meaning food is taxed like everything else — which is
/// correct for most states and is what a location starts with.
pub fn set_food_rate(db: &Db, id: LocationId, food_rate_ppm: Option<u32>) -> Result<()> {
    if let Some(ppm) = food_rate_ppm
        && ppm > 1_000_000
    {
        return Err(CoreError::Invalid(format!(
            "food rate {ppm} ppm is above 100%"
        )));
    }
    let changed = db.conn().execute(
        "UPDATE location
            SET food_rate_ppm = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id, food_rate_ppm],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("location {id}")));
    }
    Ok(())
}

/// Set the rate by hand, and stop following any published jurisdiction.
///
/// Clearing `area_code` is the point as much as the rate is. A hand-typed
/// figure is a correction — somebody had the receipt in front of them — and the
/// next quarterly refresh must not quietly reinstate the number they replaced.
pub fn set_rate(db: &Db, id: LocationId, tax_rate_ppm: u32) -> Result<()> {
    check_rate(tax_rate_ppm)?;
    let changed = db.conn().execute(
        "UPDATE location
            SET tax_rate_ppm = ?2,
                area_code    = NULL,
                updated_at   = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id, tax_rate_ppm],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("location {id}")));
    }
    Ok(())
}

/// Take both rates from a published jurisdiction, and keep following it.
///
/// The recorded `area_code` is what a quarterly refresh looks for: this
/// location's rates are the state's figures, so when the state revises them
/// this location moves too. That is the whole mechanism by which a download
/// reaches a receipt.
pub fn adopt_area(
    db: &Db,
    id: LocationId,
    area_code: &str,
    tax_rate_ppm: u32,
    food_rate_ppm: Option<u32>,
) -> Result<()> {
    check_rate(tax_rate_ppm)?;
    if let Some(ppm) = food_rate_ppm
        && ppm > MAX_RATE_PPM
    {
        return Err(CoreError::Invalid(format!(
            "food rate {ppm} ppm is above 100%"
        )));
    }
    let changed = db.conn().execute(
        "UPDATE location
            SET tax_rate_ppm  = ?2,
                food_rate_ppm = ?3,
                area_code     = ?4,
                updated_at    = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id, tax_rate_ppm, food_rate_ppm, area_code],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("location {id}")));
    }
    Ok(())
}

/// Push a revised publication into every location that follows one.
///
/// Matched on `area_code`, so a location whose rate was typed by hand is
/// untouched however many quarters go by. Returns how many moved.
pub fn refresh_followed_rates(db: &Db) -> Result<usize> {
    let changed = db.conn().execute(
        "UPDATE location
            SET tax_rate_ppm  = (SELECT combined_rate_ppm FROM tax_area
                                  WHERE tax_area.code = location.area_code),
                food_rate_ppm = (SELECT food_rate_ppm FROM tax_area
                                  WHERE tax_area.code = location.area_code),
                updated_at    = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE area_code IS NOT NULL
            AND EXISTS (SELECT 1 FROM tax_area WHERE tax_area.code = location.area_code)
            -- Only where something actually differs, so updated_at still marks
            -- a real change rather than the last time the app was launched.
            AND (tax_rate_ppm,  COALESCE(food_rate_ppm, -1))
             <> (SELECT combined_rate_ppm, COALESCE(food_rate_ppm, -1) FROM tax_area
                  WHERE tax_area.code = location.area_code)",
        [],
    )?;
    Ok(changed)
}

/// Make this the home location, demoting whichever one held the flag.
///
/// Both statements run in one transaction because `ux_location_home` permits
/// exactly one live home row: setting the new flag before clearing the old one
/// is a constraint violation, and clearing the old one in a *separate*
/// transaction leaves the user with no home at all if the second write fails.
pub fn set_home(db: &Db, id: LocationId) -> Result<()> {
    db.with_tx(|tx| {
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM location WHERE id = ?1)",
            params![id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(CoreError::NotFound(format!("location {id}")));
        }

        tx.execute(
            "UPDATE location
                SET is_home = 0,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE is_home = 1 AND id <> ?1",
            params![id],
        )?;
        tx.execute(
            "UPDATE location
                SET is_home = 1,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    })
}

/// A clear domain error beats the raw `CHECK` failure the schema would
/// otherwise produce, which names a constraint rather than the mistake.
fn check_rate(ppm: u32) -> Result<()> {
    if ppm > MAX_RATE_PPM {
        return Err(CoreError::Invalid(format!(
            "tax rate {ppm} ppm is above 100% ({MAX_RATE_PPM} ppm)"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    #[test]
    fn the_seed_has_exactly_one_home_at_zero_percent() {
        let db = seeded();
        let home = home(&db).unwrap().expect("seeded home");
        assert_eq!(home.name, "Home");
        // Zero on purpose: a wrong prefilled rate silently produces plausible
        // tax figures that disagree with the receipt.
        assert_eq!(home.tax_rate_ppm, 0);
        assert!(home.is_home);
        assert_eq!(list(&db).unwrap().len(), 1);
    }

    #[test]
    fn get_or_create_never_overwrites_an_existing_rate() {
        let db = seeded();
        let made = get_or_create(&db, "Costco Austin", 82_500).unwrap();
        assert_eq!(made.tax_rate_ppm, 82_500);

        let again = get_or_create(&db, "  costco austin ", 0).unwrap();
        assert_eq!(again.id, made.id);
        assert_eq!(again.tax_rate_ppm, 82_500, "the stored rate must survive");
        assert_eq!(list(&db).unwrap().len(), 2);
    }

    #[test]
    fn set_rate_changes_it_deliberately() {
        let db = seeded();
        let l = get_or_create(&db, "HEB", 0).unwrap();
        set_rate(&db, l.id, 82_500).unwrap();
        assert_eq!(get(&db, l.id).unwrap().tax_rate_ppm, 82_500);

        assert!(matches!(
            set_rate(&db, l.id, 1_000_001),
            Err(CoreError::Invalid(_))
        ));
        assert!(matches!(
            set_rate(&db, LocationId::new(999), 0),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn set_home_moves_the_flag_rather_than_duplicating_it() {
        let db = seeded();
        let old = home(&db).unwrap().unwrap();
        let new = get_or_create(&db, "Austin, TX", 82_500).unwrap();

        set_home(&db, new.id).unwrap();
        assert_eq!(home(&db).unwrap().unwrap().id, new.id);
        assert!(!get(&db, old.id).unwrap().is_home);

        // Idempotent: setting the current home again must not trip the unique
        // index by clearing and re-setting the same row.
        set_home(&db, new.id).unwrap();
        assert_eq!(home(&db).unwrap().unwrap().id, new.id);

        assert!(matches!(
            set_home(&db, LocationId::new(999)),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn empty_and_out_of_range_input_is_refused() {
        let db = seeded();
        assert!(matches!(
            get_or_create(&db, "   ", 0),
            Err(CoreError::Invalid(_))
        ));
        assert!(matches!(
            get_or_create(&db, "Somewhere", 1_000_001),
            Err(CoreError::Invalid(_))
        ));
    }

    #[test]
    fn find_by_name_is_case_insensitive_and_misses_cleanly() {
        let db = seeded();
        assert!(find_by_name(&db, "home").unwrap().is_some());
        assert!(find_by_name(&db, "nowhere").unwrap().is_none());
        assert!(matches!(
            get(&db, LocationId::new(999)),
            Err(CoreError::NotFound(_))
        ));
    }
}
