//! Installing a published rate table, and deciding when to go looking for one.
//!
//! # Where the network is
//!
//! Not here. This layer takes bytes that somebody else downloaded and it hands
//! back a decision about whether downloading was worth doing. The core has no
//! async runtime and no HTTP client on purpose — those are the parts that
//! differ most between a desktop binary, an Android service and whatever a
//! future iOS build turns out to need, and a core that owned them would have to
//! be ported rather than compiled.
//!
//! So a frontend's whole job is: ask [`refresh_due`], fetch [`wanted_url`] if it
//! says yes, and pass the bytes to [`install_workbook`]. Everything else —
//! which quarter, which URL, whether the file is the one that was asked for,
//! what to write and what to leave alone — is decided here, once, for every
//! frontend.

use chrono::NaiveDate;

use crate::db::Db;
use crate::error::{CoreError, Result};
use crate::rates::{self, RateTable};
use crate::repo::{locations, tax_areas};
use crate::time::Quarter;

/// What happened when a publication was installed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Installed {
    /// Jurisdictions now in the table.
    pub areas: usize,
    /// Locations whose rates moved as a result.
    pub locations_updated: usize,
}

/// The quarter whose rates apply today.
#[must_use]
pub fn wanted_quarter(today: NaiveDate) -> Quarter {
    Quarter::from_date(today)
}

/// Where today's publication lives.
#[must_use]
pub fn wanted_url(today: NaiveDate) -> String {
    rates::workbook_url(wanted_quarter(today))
}

/// Whether it is worth going to the network right now.
///
/// True when nothing is installed, or when the installed publication is not
/// this quarter's. False for the rest of the quarter — the file does not change
/// between revisions, so checking daily would be asking a government file
/// server the same question ninety times for one answer.
///
/// The once-a-day floor is what stops a failed check becoming a loop. A machine
/// that is offline on the first of July would otherwise retry on every launch,
/// which on a phone is every time the app is opened.
#[must_use]
pub fn refresh_due(release: Option<&tax_areas::Release>, today: NaiveDate) -> bool {
    let Some(release) = release else {
        return true;
    };
    if release.quarter == wanted_quarter(today).to_string() {
        return false;
    }
    match &release.checked_at {
        // Timestamps are stored as `YYYY-MM-DDT…`, so the date is the prefix.
        Some(checked) => !checked.starts_with(&today.to_string()),
        None => true,
    }
}

/// Install a publication: the table, the release note, and the locations that
/// follow it.
pub fn install(db: &Db, table: &RateTable) -> Result<Installed> {
    verify(table)?;
    let areas: Vec<tax_areas::TaxArea> = table
        .areas
        .iter()
        .map(|a| tax_areas::TaxArea {
            code: a.code.clone(),
            name: a.name.clone(),
            combined_rate_ppm: a.combined_rate_ppm,
            food_rate_ppm: a.food_rate_ppm,
        })
        .collect();

    let stored = tax_areas::replace_all(db, &areas)?;
    tax_areas::record_release(
        db,
        &table.quarter.to_string(),
        &table.quarter.effective_on().to_string(),
        &rates::workbook_url(table.quarter),
    )?;
    // The step that makes the download matter. Without it the table would be
    // current and every receipt would still be taxed at the figure copied into
    // `location` the day it was created.
    let locations_updated = locations::refresh_followed_rates(db)?;
    Ok(Installed {
        areas: stored,
        locations_updated,
    })
}

/// Parse a downloaded workbook and install it.
///
/// `quarter` is what was asked for, not what the file claims, so that a file
/// which turns out to be a different quarter is caught rather than relabelled.
pub fn install_workbook(db: &Db, bytes: &[u8], quarter: Quarter) -> Result<Installed> {
    let table = rates::parse_workbook(bytes, quarter)?;
    install(db, &table)
}

/// Refuse a publication that is not what it was supposed to be.
fn verify(table: &RateTable) -> Result<()> {
    if let Some(effective) = table.effective_on
        && effective != table.quarter.effective_on()
    {
        // The publisher puts the next quarter's workbook up months early and
        // serves a 200 for files that do not exist yet, so "the URL said Q3"
        // is not evidence that the contents are Q3. Installing April's rates
        // in July would be wrong in a way nothing downstream could detect.
        return Err(CoreError::RateData(format!(
            "the {} workbook says it takes effect on {effective}, not {}",
            table.quarter,
            table.quarter.effective_on()
        )));
    }
    Ok(())
}

/// Put the bundled table in place if the database has none.
///
/// A brand-new install must be able to tax a receipt before it has ever been
/// online — the first run is as likely to be in a checkout queue as at a desk.
/// Returns `true` if it installed something.
///
/// Never overwrites: a database that has downloaded a newer publication must
/// not be dragged back to the one compiled into the binary.
pub fn ensure_installed(db: &Db) -> Result<bool> {
    if tax_areas::release(db)?.is_some() && tax_areas::count(db)? > 0 {
        return Ok(false);
    }
    install(db, &rates::bundled())?;
    Ok(true)
}

/// Note that a refresh was attempted, whatever came of it.
pub fn record_check(db: &Db) -> Result<()> {
    tax_areas::record_check(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(quarter: &str, checked: Option<&str>) -> tax_areas::Release {
        tax_areas::Release {
            quarter: quarter.to_string(),
            effective_on: "2026-07-01".to_string(),
            source_url: String::new(),
            installed_at: "2026-07-01T00:00:00.000Z".to_string(),
            checked_at: checked.map(str::to_string),
        }
    }

    fn day(iso: &str) -> NaiveDate {
        iso.parse().expect("a date")
    }

    #[test]
    fn nothing_installed_means_go_and_look() {
        assert!(refresh_due(None, day("2026-08-13")));
    }

    #[test]
    fn this_quarters_table_is_left_alone() {
        let current = release("2026Q3", Some("2026-07-02T09:00:00.000Z"));
        assert!(!refresh_due(Some(&current), day("2026-08-13")));
        // Including on the last day of the quarter.
        assert!(!refresh_due(Some(&current), day("2026-09-30")));
    }

    #[test]
    fn the_turn_of_the_quarter_is_what_triggers_a_check() {
        let stale = release("2026Q3", Some("2026-09-30T09:00:00.000Z"));
        assert!(refresh_due(Some(&stale), day("2026-10-01")));
    }

    #[test]
    fn a_failed_check_is_not_retried_until_tomorrow() {
        // Offline on the first of the quarter: one attempt, then wait. On a
        // phone every launch is a launch, and a per-launch retry against a
        // government file server is a loop nobody asked for.
        let tried_today = release("2026Q3", Some("2026-10-01T08:00:00.000Z"));
        assert!(!refresh_due(Some(&tried_today), day("2026-10-01")));
        assert!(refresh_due(Some(&tried_today), day("2026-10-02")));
    }

    #[test]
    fn a_release_never_checked_is_checked() {
        let never = release("2026Q2", None);
        assert!(refresh_due(Some(&never), day("2026-08-13")));
    }

    #[test]
    fn a_workbook_from_the_wrong_quarter_is_refused() {
        let table = RateTable {
            quarter: Quarter::new(2026, 3).expect("valid"),
            effective_on: Some(day("2026-04-01")),
            areas: Vec::new(),
        };
        let error = verify(&table).expect_err("should be refused");
        assert!(
            error.to_string().contains("2026-04-01"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn a_workbook_with_no_stated_date_is_taken_at_its_url() {
        // The header line is a printed nicety. Losing it must not cost a
        // refresh, because the alternative is rates that never update at all.
        let table = RateTable {
            quarter: Quarter::new(2026, 3).expect("valid"),
            effective_on: None,
            areas: Vec::new(),
        };
        assert!(verify(&table).is_ok());
    }
}
