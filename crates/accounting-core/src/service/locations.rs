//! Location settings that need the published rate table as well as the row.
//!
//! [`crate::repo::locations`] stores what a location *is*. Deciding what its
//! rates *should be* means reading [`crate::repo::tax_areas`], and combining
//! two tables is this layer's job rather than either repository's.

use crate::db::Db;
use crate::error::Result;
use crate::repo::{locations, tax_areas};

/// Point every unlinked location at the published jurisdiction of the same
/// name, and take its rates. Returns how many were linked.
///
/// # Why an existing database needs this
///
/// Until the rate table existed, a location's rates were copied from a list of
/// forty-odd cities typed into the source by hand, and a location had no way to
/// say where its figures came from. Every install from that era therefore holds
/// rates that are frozen, unattributed, and — as it turns out — wrong: the
/// hand-written list had Highland at 7.25% and Salt Lake City at 7.75%, where
/// the state publishes 7.45% and 8.45%. Nobody mistyped them. They were right
/// when they were written down and the rates moved afterwards, which is the
/// entire argument for downloading them.
///
/// Linking by name is what converts those installs. Afterwards the location
/// follows its jurisdiction, so the next quarterly refresh reaches it.
///
/// # What it will not touch
///
/// A location already carrying an `area_code` is left alone; it is following
/// something already. A name that matches no published jurisdiction is left
/// alone, because there is nothing to link it to — and that is the right
/// outcome for the one case where overwriting would be wrong, since a rate
/// typed by hand belongs to a place the picker could not offer.
///
/// # It runs once, not every launch
///
/// This is a conversion, not a policy, and the difference matters. Somebody who
/// types `pacc location --name Provo --rate 7.6` has corrected a published
/// figure, and [`locations::set_rate`] records that by clearing the link. A
/// version of this that ran on every launch would find an unlinked Provo,
/// helpfully re-link it, and undo the correction — every launch, forever, with
/// nothing on screen to explain why the number keeps changing back.
///
/// So a marker in `app_setting` records that the conversion has happened. After
/// that, following a jurisdiction is something a user opts into by picking an
/// area, and never something the app decides on their behalf.
pub fn adopt_published_areas(db: &Db) -> Result<usize> {
    if link_generation(db)? >= LINK_GENERATION {
        return Ok(0);
    }
    let published = tax_areas::list(db)?;
    if published.is_empty() {
        // No table yet, so nothing to link against — and crucially the marker
        // is not set, so this runs properly once one is installed.
        return Ok(0);
    }

    let mut linked = 0;
    for location in locations::list(db)? {
        if location.area_code.is_some() {
            continue;
        }
        let needle = match_key(&location.name);
        let Some(area) = published.iter().find(|a| match_key(&a.name) == needle) else {
            continue;
        };
        locations::adopt_area(
            db,
            location.id,
            &area.code,
            area.combined_rate_ppm,
            area.food_rate_ppm,
        )?;
        linked += 1;
    }
    mark_linked(db)?;
    Ok(linked)
}

/// The `app_setting` key recording that the one-time linking has run.
const LINK_GENERATION_KEY: &str = "rates.location_link_generation";

/// Bump to make a later release re-run the conversion — if a future quarter
/// renames enough jurisdictions to be worth a second pass, for instance.
const LINK_GENERATION: i64 = 1;

fn link_generation(db: &Db) -> Result<i64> {
    let mut stmt = db
        .conn()
        .prepare("SELECT CAST(value AS INTEGER) FROM app_setting WHERE key = ?1")?;
    let mut rows = stmt.query_map([LINK_GENERATION_KEY], |row| row.get::<_, i64>(0))?;
    match rows.next() {
        Some(value) => Ok(value?),
        None => Ok(0),
    }
}

fn mark_linked(db: &Db) -> Result<()> {
    db.conn().execute(
        "INSERT INTO app_setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        rusqlite::params![LINK_GENERATION_KEY, LINK_GENERATION.to_string()],
    )?;
    Ok(())
}

/// The form of a name two spellings of one place agree on.
///
/// The state publishes bare town names — "Provo" — while locations created
/// before the table carry the state, because a picker offering "Provo" with no
/// context would have been ambiguous for the user. They are the same place, and
/// a match that missed on the suffix would leave every existing install
/// unlinked and quietly frozen at its old rate.
fn match_key(name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let base = lowered
        .strip_suffix(", ut")
        .or_else(|| lowered.strip_suffix(", utah"))
        .unwrap_or(&lowered);
    base.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_suffix_does_not_make_it_a_different_town() {
        assert_eq!(match_key("Provo, UT"), "provo");
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
}
