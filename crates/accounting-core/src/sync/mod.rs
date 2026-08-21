//! Syncing two devices without a server.
//!
//! # The shape
//!
//! Each device writes what it has changed into an encrypted bundle; the other
//! reads it. How the bundle travels is deliberately not this crate's business
//! — a synced folder, a USB stick, a phone cable. There is no server to run,
//! keep online, or trust with a household's finances, and no account to make.
//!
//! # Why the sync unit is the aggregate, not the row
//!
//! The schema's invariants are aggregate-level: `subtotal + tax = total` on a
//! transaction, splits summing to the total, line items summing to the split.
//! Merging *fields* from two devices — subtotal from one, tax from the other —
//! can produce a row that satisfies neither, a state that was never valid
//! anywhere and that the schema's own CHECK constraints would refuse. Taking
//! one side's whole aggregate cannot do that: whatever it is, it was valid on
//! the device that wrote it. It also matches the existing write path, which
//! already replaces a transaction and all its children in one SQLite
//! transaction.
//!
//! # Why deletes need nothing special
//!
//! They are already soft. "Nothing user-facing is ever hard-deleted" (001), so
//! `archived_at` is an ordinary column and a delete is an ordinary field
//! update. The usual hard part of offline sync — tombstones, and knowing when
//! it is safe to forget them — does not arise.
//!
//! # What does not sync
//!
//! `tax_area` and `tax_rate_release`. They are a published table, replaced
//! wholesale every quarter, identical on every device that has refreshed. They
//! would be by far the largest thing in every bundle and would fix nothing:
//! each device already downloads them, and a device that has not can be told
//! to rather than shipped 389 rows over a USB stick.

pub mod bundle;
pub mod device;
pub mod entity;
pub mod hlc;

pub use bundle::{Bundle, BundleError, Change};
pub use hlc::{DeviceId, Hlc};
