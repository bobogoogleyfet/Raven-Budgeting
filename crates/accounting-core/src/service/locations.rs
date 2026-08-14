//! Location settings that need the area table as well as the location row.
//!
//! [`crate::repo::locations`] stores what a location *is*. Deciding what its
//! rates *should be* means reading [`crate::domain::areas`], and combining a
//! domain table with the database is this layer's job, not the repository's.

use crate::db::Db;
use crate::domain::areas;
use crate::error::Result;
use crate::repo::locations;

/// Give every location named after a known area the reduced food rate that
/// area has, and report how many changed.
///
/// # Why this exists
///
/// `location.food_rate_ppm` arrived in migration 002, which leaves it NULL on
/// every row that already existed. NULL means "food is taxed like everything
/// else" — the right reading for most of the US, and the wrong one for a
/// database whose home is "Provo, UT", where groceries are taxed at 3% rather
/// than 7.25%. Those rows were not making that claim; they predate anything
/// being able to make it.
///
/// Nothing else closes that gap. The first-run prompt sets the food rate now,
/// but it is asked once and an existing install has already answered it, so
/// without this every user who set the app up before 002 keeps overestimating
/// grocery tax with no way to notice beyond the receipt in their hand.
///
/// # Why it is safe to run repeatedly
///
/// Only a NULL is filled in, so a location that has a food rate keeps it,
/// including one the area table would now disagree with. An area the table
/// does not know is left alone rather than guessed at, and an area with no
/// reduced rate leaves NULL as NULL, because "this state does not tax food
/// differently" is exactly what NULL already says.
pub fn adopt_area_food_rates(db: &Db) -> Result<usize> {
    let mut changed = 0;
    for location in locations::list(db)? {
        if location.food_rate_ppm.is_some() {
            continue;
        }
        let Some(area) = areas::find(&location.name) else {
            continue;
        };
        let Some(food_rate_ppm) = area.food_rate_ppm else {
            continue;
        };
        locations::set_food_rate(db, location.id, Some(food_rate_ppm))?;
        changed += 1;
    }
    Ok(changed)
}
