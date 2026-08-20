//! The published rate table: every taxing jurisdiction and what it charges.
//!
//! Reference data, not the user's data. Nothing here is edited by hand and
//! nothing points at it — a receipt copies the rate it used rather than
//! referring to a row that could later change underneath it. That is what makes
//! wholesale replacement every quarter a safe operation instead of a rewrite of
//! history.
//!
//! The parsing lives in [`crate::rates`], which knows one state's file. This
//! module knows a table of codes and rates and would store another state's just
//! as happily.

use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Result;

/// One jurisdiction as stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxArea {
    /// The publisher's own code, e.g. `25-029`. Unique, and the identity a
    /// refresh matches on.
    pub code: String,
    pub name: String,
    pub combined_rate_ppm: u32,
    pub food_rate_ppm: Option<u32>,
    /// The published rate on prepared food, where the county levies a
    /// restaurant tax on top of the combined rate.
    pub restaurant_rate_ppm: Option<u32>,
}

/// Which publication is installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// `2026Q3`.
    pub quarter: String,
    /// ISO date the rates took effect.
    pub effective_on: String,
    pub source_url: String,
    pub installed_at: String,
    /// Last refresh *attempt*, successful or not.
    pub checked_at: Option<String>,
}

const SELECT_AREA: &str =
    "SELECT code, name, combined_rate_ppm, food_rate_ppm, restaurant_rate_ppm FROM tax_area";

fn map_area(row: &Row<'_>) -> rusqlite::Result<TaxArea> {
    Ok(TaxArea {
        code: row.get(0)?,
        name: row.get(1)?,
        combined_rate_ppm: row.get(2)?,
        food_rate_ppm: row.get(3)?,
        restaurant_rate_ppm: row.get(4)?,
    })
}

/// Every jurisdiction, by name.
pub fn list(db: &Db) -> Result<Vec<TaxArea>> {
    let sql = format!("{SELECT_AREA} ORDER BY name");
    let mut stmt = db.conn().prepare(&sql)?;
    Ok(stmt
        .query_map([], map_area)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Jurisdictions a person could say they live in.
///
/// The publication also carries special taxing districts — inland-port and
/// military-development authorities — which are real jurisdictions with real
/// rates and are not answers to "where do you shop". They stay in the table,
/// because dropping rows at import would make the stored table disagree with
/// the published one; they are filtered here, where the question is what to
/// put in a picker.
pub fn list_places(db: &Db) -> Result<Vec<TaxArea>> {
    Ok(list(db)?
        .into_iter()
        .filter(|a| !is_special_district(&a.name))
        .collect())
}

/// `UIPA Provo`, `MIDA Hideout` — an authority's district, not a town.
fn is_special_district(name: &str) -> bool {
    ["UIPA ", "MIDA "].iter().any(|p| name.starts_with(p))
}

/// Look up a jurisdiction by the name a person would type for it.
pub fn find_by_name(db: &Db, name: &str) -> Result<Option<TaxArea>> {
    let needle = match_key(name);
    Ok(list(db)?.into_iter().find(|a| match_key(&a.name) == needle))
}

/// The form of a name two spellings of one place agree on.
///
/// The state publishes bare names — "Highland" — while a person naming the
/// place they live writes "Highland, UT", and every location this app created
/// before the rate table existed carries that suffix. They are the same place.
///
/// Matching exactly instead is not a smaller feature, it is a wrong answer that
/// looks like a working one: `pacc location --name "Highland, UT"` found no
/// jurisdiction, so it created the location at 0% and every receipt filed
/// against it recorded no tax at all.
#[must_use]
pub fn match_key(name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let base = lowered
        .strip_suffix(", ut")
        .or_else(|| lowered.strip_suffix(", utah"))
        .unwrap_or(&lowered);
    base.trim().to_string()
}

/// How many jurisdictions are stored.
pub fn count(db: &Db) -> Result<usize> {
    let n: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM tax_area", [], |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Replace the whole table with a new publication, in one transaction.
///
/// Whether the stored table carries any prepared-food rate at all.
///
/// The upgrade tell. A table installed before the prepared-food column existed
/// has a value in every other column and `NULL` in this one for all 389 rows,
/// which is indistinguishable per-row from "this county levies no restaurant
/// tax" — but not in aggregate, because no state publishes a chart where the
/// column is empty everywhere.
pub fn has_restaurant_rates(db: &Db) -> Result<bool> {
    let n: i64 = db.conn().query_row(
        "SELECT count(*) FROM tax_area WHERE restaurant_rate_ppm IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Wholesale, not incremental. A quarter can dissolve a district or annex a
/// town, and a merge that only ever adds and updates would leave the dissolved
/// one behind forever, quietly offering a rate nobody charges any more.
///
/// Returns the number of jurisdictions now stored.
pub fn replace_all(db: &Db, areas: &[TaxArea]) -> Result<usize> {
    db.with_tx(|tx| {
        tx.execute("DELETE FROM tax_area", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tax_area (code, name, combined_rate_ppm, food_rate_ppm,
                                       restaurant_rate_ppm)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for area in areas {
                stmt.execute(params![
                    area.code,
                    area.name,
                    area.combined_rate_ppm,
                    area.food_rate_ppm,
                    area.restaurant_rate_ppm
                ])?;
            }
        }
        Ok(areas.len())
    })
}

/// Which publication is installed, if any.
pub fn release(db: &Db) -> Result<Option<Release>> {
    let mut stmt = db.conn().prepare(
        "SELECT quarter, effective_on, source_url, installed_at, checked_at
           FROM tax_rate_release WHERE id = 1",
    )?;
    let mut rows = stmt.query_map([], |row| {
        Ok(Release {
            quarter: row.get(0)?,
            effective_on: row.get(1)?,
            source_url: row.get(2)?,
            installed_at: row.get(3)?,
            checked_at: row.get(4)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Record which publication is now installed.
///
/// `checked_at` is set at the same time: installing a publication is the most
/// conclusive possible check.
pub fn record_release(db: &Db, quarter: &str, effective_on: &str, source_url: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO tax_rate_release (id, quarter, effective_on, source_url, installed_at, checked_at)
         VALUES (1, ?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT (id) DO UPDATE SET
             quarter      = excluded.quarter,
             effective_on = excluded.effective_on,
             source_url   = excluded.source_url,
             installed_at = excluded.installed_at,
             checked_at   = excluded.checked_at",
        params![quarter, effective_on, source_url],
    )?;
    Ok(())
}

/// Note that a refresh was attempted, whatever came of it.
///
/// Separate from [`record_release`] because the common case is a check that
/// installs nothing — the quarter has not turned, or the machine is offline.
/// Without this the app would retry a dead network every time it launched.
pub fn record_check(db: &Db) -> Result<()> {
    db.conn().execute(
        "UPDATE tax_rate_release
            SET checked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = 1",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_suffix_does_not_make_it_a_different_town() {
        assert_eq!(match_key("Highland, UT"), "highland");
        assert_eq!(match_key("  provo, Utah "), "provo");
        assert_eq!(match_key("Provo"), "provo");
        assert_eq!(match_key("Salt Lake City, UT"), "salt lake city");
    }

    #[test]
    fn a_name_that_merely_ends_in_those_letters_is_left_whole() {
        // No comma, so no suffix — and there is no town called "Provo" hiding
        // inside one called "Provo Ut".
        assert_eq!(match_key("Provo Ut"), "provo ut");
    }

    #[test]
    fn an_authoritys_district_is_not_a_town() {
        assert!(is_special_district("UIPA Beaver County"));
        assert!(is_special_district("MIDA Brigham City"));
        assert!(!is_special_district("Provo"));
        // A town whose name merely starts with those letters is not one.
        assert!(!is_special_district("Midway"));
        assert!(!is_special_district("Midvale"));
    }
}
