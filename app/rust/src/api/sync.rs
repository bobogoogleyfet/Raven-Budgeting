//! Exchanging changes with another device, through a file.
//!
//! There is no server: each device writes what the other has not seen into one
//! sealed file, and how that file travels — a synced folder, a stick, a cable
//! — is deliberately nobody's business here. The core hands back bytes and
//! takes bytes (`service::sync`), because on Android and iOS the destination
//! comes from a system document picker and the app is handed a stream, so
//! file handling has to be the frontend's job or it is nobody's. This module
//! is that frontend's half of the boundary: it moves bytes across the bridge
//! and does no merging, no crypto and no policy of its own.

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use std::time::{SystemTime, UNIX_EPOCH};

use accounting_core::repo::settings;
use accounting_core::service::sync as sync_service;

use crate::api::db;
use crate::error::to_message;

/// Who a first bundle is addressed to, when there is nobody to address it to.
///
/// Exporting before either device has heard of the other is the ordinary way a
/// pairing starts, and a bundle still has to name the peer it was built for —
/// that name is what a watermark is kept against. Parenthesised so it cannot
/// collide with a real device name, which is always a uuid. The same constant,
/// for the same reason, as the one in `accounting-cli`; nothing is ever
/// *recorded* under it, see [`mark_delivered`].
const UNPAIRED: &str = "(unpaired)";

/// Where this database has got to with the devices it syncs with.
pub struct SyncStatusDto {
    /// This database's own name within the sync group.
    ///
    /// It names the *replica*, not the hardware and not the person: peers
    /// track how far along it is by this name, so a database restored onto a
    /// new laptop carries on as the same device rather than becoming a third
    /// one.
    pub this_device: String,
    /// Every device this one has exchanged anything with. Empty until the
    /// first bundle has been imported from somewhere.
    pub peers: Vec<String>,
    /// Disagreements nobody has looked at yet.
    ///
    /// `u32` rather than `usize`: the bridge renders a `usize` as a Dart
    /// `BigInt`, which is a heavyweight number for a count and a nuisance at
    /// every call site.
    pub open_conflicts: u32,
}

/// What importing a bundle did.
///
/// The three counts are the headline; the two lists are the part that must not
/// be reduced to a count. `deferred` and `unknown_kinds` both mean *data did
/// not arrive*, and a bare number is indistinguishable from a clean import a
/// minute later — so they cross the bridge as sentences a person can act on
/// rather than as ids a screen would have to learn to phrase.
pub struct AppliedDto {
    /// Changes taken, because they were newer than anything here.
    pub applied: u32,
    /// Changes ignored, because this device already had the same or newer.
    pub ignored: u32,
    /// Changes where both devices had edited the same thing. The later edit
    /// won and the earlier one was kept rather than dropped — losing a
    /// conflict is not a judgement that the value was wrong.
    pub conflicts: u32,
    /// Changes that could not be applied because something they point at has
    /// not reached this device. One line each, naming what is missing.
    pub deferred: Vec<String>,
    /// Kinds written by a newer release than this one, named as they arrived.
    pub unknown_kinds: Vec<String>,
}

/// This device's name within the sync group, minting one if it has never had
/// one.
pub async fn this_device() -> Result<String, String> {
    let db = db::locked()?;
    sync_service::this_device(&db).map_err(to_message)
}

/// This device's name, its peers, and anything waiting for a human.
pub async fn sync_status() -> Result<SyncStatusDto, String> {
    let db = db::locked()?;
    let this_device = sync_service::this_device(&db).map_err(to_message)?;
    let peers = sync_service::known_peers(&db).map_err(to_message)?;
    let open_conflicts = sync_service::open_conflicts(&db).map_err(to_message)?;
    Ok(SyncStatusDto {
        this_device,
        peers,
        open_conflicts: u32::try_from(open_conflicts).unwrap_or(u32::MAX),
    })
}

/// Build the sealed file holding everything `peer` has not been sent.
///
/// The watermark is deliberately *not* advanced here: a file that was written
/// but never reached the other device must not count as delivered, or
/// everything in it would never be sent again. [`mark_delivered`] is the
/// separate step the caller takes once the bytes are somewhere the peer can
/// read them — which, from this side, is once the file is on disk.
///
/// A `None` peer means "the only one you sync with". With several the core
/// refuses and names them, because guessing would send one device's history to
/// another. With none this is the first bundle of a pairing, addressed to
/// [`UNPAIRED`] since a bundle has to name somebody.
pub async fn export_bundle(peer: Option<String>, phrase: String) -> Result<Vec<u8>, String> {
    let phrase = checked_phrase(phrase)?;
    let db = db::locked()?;
    let addressed_to = match resolve_peer(&db, peer)? {
        Some(peer) => peer,
        None => UNPAIRED.to_string(),
    };
    sync_service::export_file(&db, &addressed_to, &phrase, now_millis()).map_err(to_message)
}

/// Build the sealed file a device with nothing of its own joins from: every
/// record here, addressed to nobody.
///
/// Not the same file as [`export_bundle`]'s once this device is paired. That
/// one carries only what its peer has not been sent, and a join keeps nothing
/// but what the file holds, so joining from it would leave the new device
/// with the last few changes and none of the ledger — the core refuses such a
/// file, and this is the one to make instead.
///
/// Never marked as delivered, and there is no one to mark it against: the
/// device that reads it has not been met. It introduces itself on its first
/// file back, which also acknowledges what this one carried.
pub async fn export_for_new_device(phrase: String) -> Result<Vec<u8>, String> {
    let phrase = checked_phrase(phrase)?;
    let db = db::locked()?;
    sync_service::export_for_new_device(&db, &phrase, now_millis()).map_err(to_message)
}

/// Record that a bundle reached its peer, so the next one carries only what
/// has changed since.
///
/// Called only once the bytes are safely written: writing the file *is* the
/// delivery from this side, because the transport is a folder or a stick whose
/// far end this app cannot see.
///
/// Does nothing when there is no peer to record it against. A watermark names
/// a device, and inventing a row for `(unpaired)` would make it the "only
/// peer" that every later export and status screen talked about, while the
/// real device's watermark stayed empty. The first bundle of a pairing is
/// simply sent again after the reply comes back, which costs one re-send and
/// no correctness.
pub async fn mark_delivered(peer: Option<String>, phrase: String, bytes: Vec<u8>) -> Result<(), String> {
    let phrase = checked_phrase(phrase)?;
    let db = db::locked()?;
    let Some(peer) = resolve_peer(&db, peer)? else {
        return Ok(());
    };
    sync_service::mark_delivered(&db, &peer, &phrase, &bytes).map_err(to_message)
}

/// Apply a sealed file written by another device.
pub async fn import_bundle(bytes: Vec<u8>, phrase: String) -> Result<AppliedDto, String> {
    let phrase = checked_phrase(phrase)?;
    let db = db::locked()?;
    let applied =
        sync_service::import_file(&db, &bytes, &phrase, now_millis()).map_err(to_message)?;
    Ok(rendered(applied))
}

/// Adopt another device's data wholesale, from a sealed file it wrote.
///
/// The first handshake of a pairing, and the only destructive one. Two devices
/// that have each been opened have each seeded the same chart of accounts
/// under uuids of their own; sync matches on uuid, so merging them yields two
/// of everything. Joining throws this device's copy away and takes the other's
/// instead, which is why [`can_join`] is asked first and why nothing in the
/// app reaches this without somebody pressing a button that says so.
pub async fn join_bundle(bytes: Vec<u8>, phrase: String) -> Result<AppliedDto, String> {
    let phrase = checked_phrase(phrase)?;
    let db = db::locked()?;
    let applied =
        sync_service::join_file(&db, &bytes, &phrase, now_millis()).map_err(to_message)?;
    Ok(rendered(applied))
}

/// Why this device cannot join somebody else's sync group — `None` when it can.
///
/// A refusal rather than a boolean: a device holding work of its own has to be
/// told what joining would cost it, and "false" is not something a screen can
/// phrase for a user. `None` reads as "nothing stands in the way", which is
/// the only case with nothing to say.
pub async fn can_join() -> Result<Option<String>, String> {
    let db = db::locked()?;
    Ok(sync_service::can_join(&db)
        .map_err(to_message)?
        .err()
        .map(|refused| refused.to_string()))
}

/// Whether this device syncs over the local network, and the phrase it uses.
pub struct LanSyncDto {
    /// Off until somebody turns it on. Discovery broadcasts this device's
    /// existence to every machine on the Wi-Fi, which is not something to
    /// start on a user's behalf.
    pub enabled: bool,
    /// Empty until a pairing phrase has been set. Kept so the exchange can
    /// happen without a human present — see [`set_lan_sync`].
    pub phrase: String,
}

/// What this device has been told about syncing over the local network.
pub async fn lan_sync() -> Result<LanSyncDto, String> {
    let db = db::locked()?;
    let enabled = settings::get(&db, LAN_ENABLED_KEY).map_err(to_message)?;
    let phrase = settings::get(&db, LAN_PHRASE_KEY).map_err(to_message)?;
    Ok(LanSyncDto {
        // Anything that is not exactly the stored "1" is off, including a row
        // some future version wrote in another shape. Off is the safe reading
        // of a value this release does not understand: it broadcasts nothing.
        enabled: enabled.as_deref() == Some("1"),
        phrase: phrase.unwrap_or_default(),
    })
}

/// Remember whether to sync over the local network, and with what phrase.
///
/// # Why the phrase is stored at all
///
/// Everywhere else in this app the phrase is typed and forgotten, because
/// everywhere else a human is present at the moment a file is written. An
/// automatic exchange has nobody to ask: a receipt entered on the phone has to
/// reach the laptop while the laptop's owner is asleep, and a phrase held only
/// in memory would mean re-typing it on both devices after every restart —
/// which is the same as the feature not existing.
///
/// It goes in `app_setting`, in the same database file as the ledger it
/// protects. That sounds worse than it is: the phrase seals bundles *in
/// transit*, and anybody who can read this table can already read every
/// receipt in the file next to it, so storing it here gives an attacker
/// nothing they did not already have. What it must never do is travel —
/// `app_setting` is not a synced entity, so it cannot.
///
/// Enabling with an empty phrase is refused rather than saved: the switch
/// would read as on while nothing could ever be sealed, and the failure would
/// only show up as silence.
pub async fn set_lan_sync(enabled: bool, phrase: String) -> Result<(), String> {
    if enabled {
        checked_phrase(phrase.clone())?;
    }
    let db = db::locked()?;
    settings::set(&db, LAN_ENABLED_KEY, if enabled { "1" } else { "0" }).map_err(to_message)?;
    settings::set(&db, LAN_PHRASE_KEY, &phrase).map_err(to_message)
}

/// The `app_setting` key holding whether local-network sync is on.
const LAN_ENABLED_KEY: &str = "sync.lan.enabled";

/// The `app_setting` key holding the pairing phrase for automatic exchanges.
const LAN_PHRASE_KEY: &str = "sync.lan.phrase";

/// One merge outcome, in the shape a screen can read.
///
/// Shared by importing and joining because they differ in what they do to the
/// database, not in what there is to say about it afterwards.
fn rendered(applied: sync_service::Applied) -> AppliedDto {
    AppliedDto {
        applied: u32::try_from(applied.applied).unwrap_or(u32::MAX),
        ignored: u32::try_from(applied.ignored).unwrap_or(u32::MAX),
        conflicts: u32::try_from(applied.conflicts).unwrap_or(u32::MAX),
        deferred: applied
            .deferred
            .iter()
            .map(|missing| {
                format!(
                    "{} {} needs {} {}, which is not on this device",
                    missing.entity, missing.uuid, missing.wanted.0, missing.wanted.1
                )
            })
            .collect(),
        unknown_kinds: applied.unknown_kinds,
    }
}

/// The peer to address a bundle to: the one named, else the only one known.
///
/// `None` out means this database has no peers yet, which is not an error —
/// it is what the first export of a pairing looks like.
fn resolve_peer(
    db: &accounting_core::db::Db,
    peer: Option<String>,
) -> Result<Option<String>, String> {
    match peer.filter(|p| !p.trim().is_empty()) {
        Some(peer) => Ok(Some(peer)),
        None => sync_service::default_peer(db).map_err(to_message),
    }
}

/// Refuse an empty phrase rather than sealing a file with one.
///
/// The phrase is the *only* thing protecting a bundle, which is a household's
/// whole ledger left in a folder something else can read. An empty one seals
/// the file with a key anybody can derive, and it would fail silently: the
/// bytes would look encrypted and would open for everyone.
///
/// The phrase itself is never in the message. An error is the last place a
/// secret should turn up, because errors get pasted into bug reports.
fn checked_phrase(phrase: String) -> Result<String, String> {
    if phrase.is_empty() {
        return Err(
            "a sync file needs a pairing phrase — it is the same phrase on both \
                    devices, and it is the only thing protecting the file on the way over"
                .to_string(),
        );
    }
    Ok(phrase)
}

/// Milliseconds since the epoch, for the sync clock.
///
/// The core takes the clock as a parameter on purpose: it is not allowed to
/// ask what time it is, and a frontend is. Reading `SystemTime` is the same
/// allowance `db::new_uuid` already relies on — what rule 3 forbids is *date
/// logic* derived from the clock (`Local::now().date()`), which is untestable
/// and pulls chrono's timezone lookup, i.e. JNI on Android. `SystemTime` has
/// neither problem.
///
/// A clock before the epoch reads as 0 rather than failing. The HLC only ever
/// moves forward — it takes the greater of its own reading and the wall clock
/// — so a nonsense wall clock costs nothing but the physical component
/// standing still, which is precisely what the logical counter is for.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// The same, in a browser, where `SystemTime` has no clock behind it and
/// panics — so the web build asks JavaScript's.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
fn now_millis() -> u64 {
    crate::browser::now_millis()
}
