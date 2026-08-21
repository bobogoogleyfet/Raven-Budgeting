//! What one device hands another.
//!
//! # A bundle is a set of current values, not a history
//!
//! It carries what each changed thing *is* now, not the edits that got it
//! there. A receipt corrected five times travels once; a device offline for a
//! month sends one copy of each thing rather than a month of replay. The cost
//! is that intermediate states are not recoverable from a bundle — which is
//! not a loss, because they were never recoverable from the database either.
//!
//! # Why the payload is opaque here
//!
//! Each entity's value is carried as a JSON document this module does not
//! interpret. That is deliberate. A bundle written by an older release will be
//! read by a newer one — that is the normal case for two devices updated on
//! different days — and a format that pinned every field of every table would
//! turn any schema addition into a sync outage. The merge layer knows the
//! shapes; this layer knows only that something has a kind, an identity, a
//! timestamp and a body.
//!
//! # What identifies a thing
//!
//! `uuid`, never a rowid. The schema has said so since 001: "the uuid is the
//! stable cross-device identity; the INTEGER PRIMARY KEY is the rowid and is
//! local to one file." Two devices independently assign rowid 7 to different
//! receipts on their first day.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::hlc::{DeviceId, Hlc};

/// The format this bundle was written in.
///
/// Checked before anything is applied. A device that cannot read a bundle must
/// say so plainly rather than apply the half it understands — a partial merge
/// is indistinguishable from a complete one afterwards, and leaves two
/// databases that disagree with no record of why.
pub const FORMAT: u32 = 1;

/// One thing that changed, as it now stands.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Change {
    /// `"txn"`, `"category"`, `"location"`, `"merchant"`, `"budget_line"`, …
    pub entity: String,
    /// The row's stable cross-device identity.
    pub uuid: String,
    /// When the sending device last wrote it.
    pub hlc: Hlc,
    /// The whole aggregate, as JSON. Opaque to this layer on purpose.
    pub body: serde_json::Value,
}

/// Everything one device has to tell another.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Bundle {
    pub format: u32,
    /// Who wrote it.
    pub from: DeviceId,
    /// The sender's clock when it was written. The receiver folds this in, so
    /// its own later edits are guaranteed to sort after everything here.
    pub at: Hlc,
    /// The high-water mark this bundle covers, per device the sender knows
    /// about — including devices it has only ever heard about second-hand.
    ///
    /// This is what makes a three-device household work without every pair
    /// syncing directly: a laptop that has seen the phone's edits can pass
    /// them on to a tablet, and say truthfully how far along it is.
    pub covers: BTreeMap<DeviceId, Hlc>,
    /// Ordered oldest first, so applying them in order is applying them in the
    /// order they happened.
    pub changes: Vec<Change>,
}

impl Bundle {
    /// Whether this bundle carries nothing worth applying.
    ///
    /// Written and exchanged anyway when empty: "I have nothing new" is an
    /// answer, and it still advances the clocks and the watermarks, which is
    /// what stops the next bundle re-sending things the peer already has.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The highest timestamp this bundle carries from a given device.
    #[must_use]
    pub fn high_water(&self, device: &str) -> Option<&Hlc> {
        self.changes
            .iter()
            .filter(|c| c.hlc.device == device)
            .map(|c| &c.hlc)
            .max()
    }
}

/// A bundle that could not be read.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// Written by a release this one does not understand.
    #[error(
        "this sync file is format {found}, and this version of the app reads format {ours}; \
         update the app on this device"
    )]
    Format { found: u32, ours: u32 },

    /// Not a bundle at all, or damaged in transit.
    #[error("the sync file could not be read: {0}")]
    Unreadable(String),
}

/// Parse a bundle, refusing one written in a format this release cannot read.
///
/// The version check comes first and is fatal. Applying the understood half of
/// a newer bundle would leave two databases that disagree, with the record of
/// why sitting in a file nobody will look at again.
pub fn parse(bytes: &[u8]) -> Result<Bundle, BundleError> {
    let bundle: Bundle =
        serde_json::from_slice(bytes).map_err(|e| BundleError::Unreadable(e.to_string()))?;
    if bundle.format != FORMAT {
        return Err(BundleError::Format {
            found: bundle.format,
            ours: FORMAT,
        });
    }
    Ok(bundle)
}

/// Render a bundle for sending.
///
/// # Errors
/// Only if an entity body cannot be serialised, which would be a bug in the
/// layer that built it rather than anything a user did.
pub fn render(bundle: &Bundle) -> Result<Vec<u8>, BundleError> {
    serde_json::to_vec(bundle).map_err(|e| BundleError::Unreadable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc(millis: u64, device: &str) -> Hlc {
        Hlc {
            millis,
            counter: 0,
            device: device.to_string(),
        }
    }

    fn bundle(changes: Vec<Change>) -> Bundle {
        Bundle {
            format: FORMAT,
            from: "laptop".to_string(),
            at: hlc(9_000, "laptop"),
            covers: BTreeMap::new(),
            changes,
        }
    }

    fn change(entity: &str, uuid: &str, millis: u64, device: &str) -> Change {
        Change {
            entity: entity.to_string(),
            uuid: uuid.to_string(),
            hlc: hlc(millis, device),
            body: serde_json::json!({ "name": "Groceries" }),
        }
    }

    #[test]
    fn a_bundle_round_trips() {
        let original = bundle(vec![change("category", "u-1", 1_000, "laptop")]);
        let bytes = render(&original).expect("render");
        assert_eq!(parse(&bytes).expect("parse"), original);
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_half_applied() {
        // Applying the understood half of a newer bundle leaves two databases
        // that disagree, and the record of why in a file nobody reopens.
        let mut newer = bundle(vec![]);
        newer.format = FORMAT + 1;
        let bytes = serde_json::to_vec(&newer).expect("render");

        let error = parse(&bytes).expect_err("should refuse");
        assert!(matches!(error, BundleError::Format { .. }));
        // The message has to tell the user what to do about it.
        assert!(error.to_string().contains("update the app"));
    }

    #[test]
    fn damaged_bytes_are_refused_with_something_readable() {
        let error = parse(b"not a bundle").expect_err("should refuse");
        assert!(matches!(error, BundleError::Unreadable(_)));
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn an_unknown_entity_still_parses() {
        // A bundle from a newer release of the *same* format may carry a kind
        // this one has never heard of. The format is the compatibility
        // boundary; the entity list is not, or every new table would be a sync
        // outage for whoever updated second.
        let original = bundle(vec![change("something_new", "u-9", 1_000, "phone")]);
        let bytes = render(&original).expect("render");
        assert_eq!(parse(&bytes).expect("parse").changes.len(), 1);
    }

    #[test]
    fn high_water_is_per_device() {
        // Three devices: the mark for one must not be advanced by another's
        // edits, or a bundle would claim to have passed on work it never saw.
        let b = bundle(vec![
            change("txn", "a", 1_000, "phone"),
            change("txn", "b", 5_000, "phone"),
            change("txn", "c", 3_000, "tablet"),
        ]);

        assert_eq!(b.high_water("phone").expect("phone").millis, 5_000);
        assert_eq!(b.high_water("tablet").expect("tablet").millis, 3_000);
        assert!(b.high_water("laptop").is_none());
    }

    #[test]
    fn an_empty_bundle_is_still_a_bundle() {
        // "I have nothing new" is an answer, and exchanging it still advances
        // the watermarks that stop the next one re-sending everything.
        let b = bundle(vec![]);
        assert!(b.is_empty());
        let bytes = render(&b).expect("render");
        assert!(parse(&bytes).expect("parse").is_empty());
    }
}
