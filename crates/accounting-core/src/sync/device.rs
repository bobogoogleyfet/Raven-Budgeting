//! This database's identity within a sync group, and its clock.
//!
//! # Why a database has a name at all
//!
//! It is the last tiebreak in [`Hlc`], and the thing peers track watermarks
//! against. It names the *replica*, not the user and not the hardware: two
//! devices are two databases, and restoring a backup onto a new laptop
//! continues the same replica rather than starting a new one — which is right,
//! because that database has already told its peers how far along it is.
//!
//! Minted on demand rather than at migration time. A migration runs inside a
//! transaction and has no good source of randomness; a uuid does, and until a
//! device has synced once its name has never mattered to anybody.

use crate::db::{Db, new_uuid};
use crate::error::Result;

use super::hlc::{DeviceId, Hlc};

/// This database's device id, minting one the first time it is asked for.
pub fn id(db: &Db) -> Result<DeviceId> {
    let existing: String =
        db.conn()
            .query_row("SELECT device FROM sync_clock WHERE id = 1", [], |r| {
                r.get(0)
            })?;
    if !existing.is_empty() {
        return Ok(existing);
    }

    // Short, but a whole uuid's worth of distinctness: the id is repeated in
    // every timestamp this device ever writes, and a change log is mostly
    // timestamps.
    let minted = new_uuid();
    db.conn().execute(
        "UPDATE sync_clock SET device = ?1 WHERE id = 1 AND device = ''",
        [&minted],
    )?;
    // Re-read rather than assume: two processes opening the same file at once
    // is ordinary — the CLI while the app is running — and exactly one of them
    // wins. The loser must adopt the winner's name, not keep its own.
    Ok(db
        .conn()
        .query_row("SELECT device FROM sync_clock WHERE id = 1", [], |r| {
            r.get(0)
        })?)
}

/// The clock as it stands, without advancing it.
pub fn clock(db: &Db) -> Result<Hlc> {
    let device = id(db)?;
    let (millis, counter): (i64, i64) = db.conn().query_row(
        "SELECT millis, counter FROM sync_clock WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Hlc {
        millis: u64::try_from(millis).unwrap_or(0),
        counter: u32::try_from(counter).unwrap_or(0),
        device,
    })
}

/// Fold a peer's timestamp into this device's clock.
///
/// Called once per bundle received, before anything is applied. It is what
/// makes clock skew stop mattering: after this, every edit this device makes
/// sorts after everything in the bundle, so a peer three minutes fast cannot
/// keep out-ranking a correction made here afterwards.
pub fn observe(db: &Db, remote: &Hlc, now_millis: u64) -> Result<Hlc> {
    let merged = clock(db)?.receive(remote, now_millis);
    db.conn().execute(
        "UPDATE sync_clock SET millis = ?1, counter = ?2 WHERE id = 1",
        rusqlite::params![
            i64::try_from(merged.millis).unwrap_or(i64::MAX),
            i64::from(merged.counter)
        ],
    )?;
    Ok(merged)
}
