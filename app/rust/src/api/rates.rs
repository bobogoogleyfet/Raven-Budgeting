//! Keeping the rate table current.
//!
//! Utah publishes a new workbook on the first day of each quarter. The core
//! decides everything worth deciding — which quarter, which URL, whether the
//! file is the one that was asked for, and what to write — and this module
//! does the one thing the core is not allowed to do: fetch bytes off the
//! network.

use accounting_core::repo::tax_areas;
use accounting_core::service::rates as rates_service;

use crate::api::db;
use crate::error::to_message;

/// What a refresh did.
pub struct RefreshDto {
    /// Whether a refresh was even due. False on the 361 days a year when the
    /// installed quarter is the current one, or when today's check already
    /// happened.
    pub attempted: bool,
    /// Whether a new table was installed.
    pub installed: bool,
    /// How many jurisdictions the new table holds.
    pub areas: u32,
    /// How many locations followed their published area to a new rate.
    pub locations_updated: u32,
    /// Why it did not happen, when it did not. Null on success and on "not
    /// due" alike.
    pub problem: Option<String>,
}

/// Refresh the rate table if the quarter has turned.
///
/// # Why a failure is not an error
///
/// Being offline is the normal state of a laptop, not a fault, and the app is
/// perfectly usable on last quarter's rates — they are estimates that a
/// receipt overrules anyway. What is *not* silent is the staleness itself:
/// `rate_release` names the quarter the rates came from and says when a newer
/// one exists, so a table that quietly stopped updating is visible where it
/// matters. This returns what happened rather than throwing, so the caller can
/// decide how loud to be.
///
/// # The lock is dropped before the network call
///
/// Holding it across a thirty-second captive-portal timeout would freeze every
/// other call the user made in the meantime.
pub fn refresh_rates() -> Result<RefreshDto, String> {
    let today = chrono::Local::now().date_naive();

    let due = {
        let db = db::locked()?;
        match tax_areas::release(&db) {
            Ok(release) => rates_service::refresh_due(release.as_ref(), today),
            Err(_) => false,
        }
    };
    if !due {
        return Ok(RefreshDto {
            attempted: false,
            installed: false,
            areas: 0,
            locations_updated: 0,
            problem: None,
        });
    }

    {
        let db = db::locked()?;
        // Recorded before the attempt, so a machine with no route out tries
        // once a day rather than on every launch.
        let _ = rates_service::record_check(&db);
    }

    let quarter = rates_service::wanted_quarter(today);
    let url = accounting_core::rates::workbook_url(quarter);
    let bytes = match download(&url) {
        Ok(bytes) => bytes,
        Err(problem) => {
            log::info!("rate refresh could not fetch {url}: {problem}");
            return Ok(RefreshDto {
                attempted: true,
                installed: false,
                areas: 0,
                locations_updated: 0,
                problem: Some(problem),
            });
        }
    };

    let db = db::locked()?;
    match rates_service::install_workbook(&db, &bytes, quarter) {
        Ok(installed) => {
            log::info!(
                "installed {quarter}: {} areas, {} locations updated",
                installed.areas,
                installed.locations_updated
            );
            Ok(RefreshDto {
                attempted: true,
                installed: true,
                areas: installed.areas as u32,
                locations_updated: installed.locations_updated as u32,
                problem: None,
            })
        }
        Err(error) => {
            // A workbook that is not the one asked for is refused by the core,
            // and keeping the rates already installed is the right answer.
            let problem = to_message(error);
            log::info!("rate refresh could not install {quarter}: {problem}");
            Ok(RefreshDto {
                attempted: true,
                installed: false,
                areas: 0,
                locations_updated: 0,
                problem: Some(problem),
            })
        }
    }
}

/// Fetch a URL into memory.
///
/// Deliberately duplicated from the CLI and the Tauri shell rather than
/// shared. The core does no networking — rule 1 — and an HTTP client is the
/// piece most likely to differ per platform, since an Android build may have
/// to go through the system stack for proxying and battery reasons. Every
/// decision worth getting right is shared instead: which quarter, which URL,
/// whether the file is the one that was asked for, and what to write all live
/// in `service::rates`.
fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url).call().map_err(to_message)?;
    // A quarter's workbook is a quarter of a megabyte. The cap is thirty times
    // that, so it only ever fires on something which is not the file.
    response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_vec()
        .map_err(to_message)
}
