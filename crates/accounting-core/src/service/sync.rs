//! Syncing, as a frontend asks for it.
//!
//! # Where the file is, is not this crate's business
//!
//! [`export_file`] hands back bytes and [`import_file`] takes them, the same
//! shape as [`crate::export_xlsx`] and for the same reason (rule 1): on
//! Android and iOS the user picks a destination through a system document
//! picker and the app is handed a stream, so file handling has to be the
//! frontend's job or it is nobody's. It also means the transport is genuinely
//! free — a synced folder, a stick, a cable — because nothing here knows which
//! one was used.

use crate::db::Db;
use crate::error::{CoreError, Result};
use crate::sync::{bundle, crypto, device, merge};

pub use crate::sync::merge::Applied;

/// Write everything `peer` has not been sent, sealed with `phrase`.
///
/// The watermark is *not* advanced here. A file that was written but never
/// reached the other device must not count as delivered, or everything in it
/// would never be sent again — so [`mark_delivered`] is a separate step the
/// caller takes once the bytes are somewhere the peer can actually read.
pub fn export_file(db: &Db, peer: &str, phrase: &str, now_millis: u64) -> Result<Vec<u8>> {
    let bundle = merge::export(db, peer, now_millis)?;
    let plain = bundle::render(&bundle).map_err(|e| CoreError::Invalid(e.to_string()))?;
    crypto::seal(&plain, phrase)
}

/// Write everything this device holds, sealed with `phrase`, for a device that
/// will join from it.
///
/// Not an exchange, so there is nothing to mark afterwards: the file is
/// addressed to nobody, because the device that reads it has not been met
/// yet. It introduces itself on its first file back, and that file
/// acknowledges what this one carried.
pub fn export_for_new_device(db: &Db, phrase: &str, now_millis: u64) -> Result<Vec<u8>> {
    let bundle = merge::export_everything(db, now_millis)?;
    let plain = bundle::render(&bundle).map_err(|e| CoreError::Invalid(e.to_string()))?;
    crypto::seal(&plain, phrase)
}

/// Record that a bundle reached its peer.
pub fn mark_delivered(db: &Db, peer: &str, phrase: &str, bytes: &[u8]) -> Result<()> {
    let plain = crypto::open(bytes, phrase)?;
    let bundle = bundle::parse(&plain).map_err(|e| CoreError::Invalid(e.to_string()))?;
    merge::mark_sent(db, peer, &bundle)
}

/// Apply a sealed bundle.
pub fn import_file(db: &Db, bytes: &[u8], phrase: &str, now_millis: u64) -> Result<Applied> {
    let plain = crypto::open(bytes, phrase)?;
    let bundle = bundle::parse(&plain).map_err(|e| CoreError::Invalid(e.to_string()))?;
    merge::import(db, &bundle, now_millis)
}

/// Adopt another device's data wholesale, from a sealed bundle.
///
/// The first handshake, and the only one that is destructive. Two devices that
/// have each been opened have each seeded the same chart of accounts under
/// different uuids; merging them yields two of everything. See
/// [`crate::sync::merge::join`].
pub fn join_file(db: &Db, bytes: &[u8], phrase: &str, now_millis: u64) -> Result<Applied> {
    let plain = crypto::open(bytes, phrase)?;
    let bundle = bundle::parse(&plain).map_err(|e| CoreError::Invalid(e.to_string()))?;
    merge::join(db, &bundle, now_millis)
}

/// Whether this device could join somebody else's sync group, and why not.
///
/// Asked before offering the button, so a device holding real work is told
/// what it would lose rather than discovering it afterwards.
pub fn can_join(db: &Db) -> Result<std::result::Result<(), merge::JoinRefused>> {
    merge::can_join(db)
}

/// This device's name, minting one if it has never had one.
pub fn this_device(db: &Db) -> Result<String> {
    device::id(db)
}

/// The devices this one has exchanged anything with.
pub fn known_peers(db: &Db) -> Result<Vec<String>> {
    merge::peers(db)
}

/// The peer to use when the caller did not name one.
///
/// With exactly one known peer there is no ambiguity and asking would be
/// ceremony — the overwhelmingly common case is two devices. With none, the
/// caller is exporting the first bundle of a pairing and has nobody to address
/// it to yet, which is fine: the watermark starts empty either way. With
/// several, guessing would silently send the wrong device's history, so it
/// refuses and says which names it knows.
pub fn default_peer(db: &Db) -> Result<Option<String>> {
    let mut peers = known_peers(db)?;
    match peers.len() {
        0 => Ok(None),
        1 => Ok(Some(peers.remove(0))),
        _ => Err(CoreError::Invalid(format!(
            "this database syncs with several devices ({}); name the one you mean",
            peers.join(", ")
        ))),
    }
}

/// How many disagreements are waiting for somebody to look.
pub fn open_conflicts(db: &Db) -> Result<usize> {
    let n: i64 = db.conn().query_row(
        "SELECT count(*) FROM sync_conflict WHERE resolved_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
}
