//! Building a bundle, and applying one.
//!
//! # Last writer wins, but the loser is kept
//!
//! When both devices have changed the same thing, one value has to be chosen,
//! and the later timestamp is as good a rule as any — better than most, since
//! [`Hlc`] makes "later" mean something both sides agree on. What it is not is
//! a judgement that the other value was wrong. Two people entering receipts
//! will occasionally edit the same one, and a ledger that silently drops one
//! person's correction, showing no sign it ever existed, is a ledger neither
//! person trusts. So the losing version is written to `sync_conflict` and the
//! thing it belongs to is flagged for review.
//!
//! # An applied change is not a local edit
//!
//! Writing a row fires the change-log triggers, which stamp it with *this*
//! device's clock — so a freshly applied remote change would look like
//! something this device had just done. It would be sent straight back, and
//! worse, it would outrank the original it came from. Every apply therefore
//! rewrites the log entry with the timestamp the change arrived carrying. The
//! entity's recorded version stays the version the peer wrote.

use crate::db::Db;
use crate::error::Result;

use super::bundle::{Bundle, Change};
use super::entity::{self, MissingRef};
use super::hlc::Hlc;
use super::{device, hlc::DeviceId};

/// What applying a bundle did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Applied {
    /// Changes taken, because they were newer than anything here.
    pub applied: usize,
    /// Changes ignored, because this device already had something newer.
    pub ignored: usize,
    /// Changes where both devices had edited the same thing. Counted
    /// separately because it is the number worth telling a person about.
    pub conflicts: usize,
    /// Changes that could not be applied because something they point at has
    /// not arrived. Reported rather than forced.
    pub deferred: Vec<MissingRef>,
    /// Kinds this release does not know, from a newer peer.
    pub unknown_kinds: Vec<String>,
}

/// Everything this device has that `peer` has not been sent.
pub fn export(db: &Db, peer: &str, now_millis: u64) -> Result<Bundle> {
    let from = device::id(db)?;
    let at = device::observe(db, &Hlc::zero(from.clone()), now_millis)?;
    let since = watermark(db, peer, "sent_through")?;

    let mut changes = Vec::new();
    for spec in entity::ENTITIES {
        for (uuid, hlc_text) in entity::dirty_since(db, spec.name, since.as_deref())? {
            // Absent means the row was hard-deleted, which nothing
            // user-facing does. Skipping is right either way: there is no body
            // to send.
            let Some(body) = entity::read(db, spec, &uuid)? else {
                continue;
            };
            let Ok(hlc) = hlc_text.parse::<Hlc>() else {
                continue;
            };
            // Never hand a device its own edit back. It would be ignored on
            // arrival — their timestamp is not older than itself — but it
            // would make every bundle carry the whole shared history forever,
            // growing with each exchange rather than with each change.
            if hlc.device == peer {
                continue;
            }
            changes.push(Change {
                entity: spec.name.to_string(),
                uuid,
                hlc,
                body,
            });
        }
    }
    // Oldest first: applying them in order is applying them in the order they
    // happened, which matters for anything the receiver logs or reports.
    changes.sort_by(|a, b| a.hlc.cmp(&b.hlc));

    let mut covers = std::collections::BTreeMap::new();
    if let Some(high) = changes.iter().map(|c| &c.hlc).max() {
        covers.insert(from.clone(), high.clone());
    }

    Ok(Bundle {
        format: super::bundle::FORMAT,
        from,
        at,
        covers,
        changes,
    })
}

/// Record that a bundle was handed over, so the next one carries only what has
/// changed since.
///
/// Separate from [`export`] on purpose: a bundle that was written but never
/// reached the other device must not advance the mark, or the things in it
/// would never be sent again. The caller marks it once the file is somewhere
/// the peer can actually read.
pub fn mark_sent(db: &Db, peer: &str, bundle: &Bundle) -> Result<()> {
    let Some(high) = bundle.changes.iter().map(|c| c.hlc.to_string()).max() else {
        return Ok(());
    };
    db.conn().execute(
        "INSERT INTO sync_peer (device, sent_through, synced_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT (device) DO UPDATE
             SET sent_through = MAX(COALESCE(sent_through, ''), excluded.sent_through),
                 synced_at    = excluded.synced_at",
        rusqlite::params![peer, high],
    )?;
    Ok(())
}

/// Apply a bundle.
pub fn import(db: &Db, bundle: &Bundle, now_millis: u64) -> Result<Applied> {
    // Before anything is applied, so every edit this device makes afterwards
    // sorts after everything in the bundle.
    device::observe(db, &bundle.at, now_millis)?;
    let ours = device::id(db)?;
    // What the peer had already seen from us. Anything we changed after this
    // they had not seen when they wrote theirs, which is what makes two edits
    // concurrent rather than sequential.
    let they_had_seen = watermark(db, &bundle.from, "sent_through")?;

    let mut out = Applied::default();

    // By kind, in dependency order — not in the order the bundle lists them.
    for spec in entity::ENTITIES {
        let mut mine: Vec<&Change> = bundle
            .changes
            .iter()
            .filter(|c| c.entity == spec.name)
            .collect();
        mine.sort_by(|a, b| a.hlc.cmp(&b.hlc));

        for change in mine {
            let incoming = change.hlc.to_string();
            let local = entity::local_hlc(db, spec.name, &change.uuid)?;

            let we_changed_it_since = local
                .as_deref()
                .is_some_and(|l| l.ends_with(&ours) && Some(l) > they_had_seen.as_deref());

            match local.as_deref() {
                // Newer here already. Their version loses — and is kept, if
                // this was a genuine disagreement rather than them simply
                // being behind.
                Some(existing) if existing >= incoming.as_str() => {
                    out.ignored += 1;
                    if we_changed_it_since {
                        record_conflict(db, spec.name, &change.uuid, &change.hlc, &change.body)?;
                        out.conflicts += 1;
                    }
                }
                _ => {
                    // Theirs wins. Keep ours first if we would otherwise lose
                    // an edit they had never seen.
                    if we_changed_it_since
                        && let Some(existing) = entity::read(db, spec, &change.uuid)?
                        && let Some(hlc) = local.as_deref().and_then(|l| l.parse::<Hlc>().ok())
                    {
                        record_conflict(db, spec.name, &change.uuid, &hlc, &existing)?;
                        out.conflicts += 1;
                    }
                    match entity::write(db, spec, &change.uuid, &change.body)? {
                        Ok(()) => {
                            stamp(db, spec.name, &change.uuid, &incoming)?;
                            out.applied += 1;
                        }
                        Err(missing) => out.deferred.push(missing),
                    }
                }
            }
        }
    }

    for change in &bundle.changes {
        if entity::spec(&change.entity).is_none() && !out.unknown_kinds.contains(&change.entity) {
            out.unknown_kinds.push(change.entity.clone());
        }
    }

    // Never record ourselves as a peer. Re-importing the file you just
    // exported is an ordinary thing to do — checking it opened, recovering
    // after a failed copy — and it must not leave this device in its own
    // peer list. The damage is silent and total: `default_peer` would start
    // returning our own name, and export skips changes authored by the peer it
    // is addressing, so every later bundle would be empty. Sync would appear
    // to work and carry nothing.
    if bundle.from == ours {
        return Ok(out);
    }

    let high = bundle.changes.iter().map(|c| c.hlc.to_string()).max();
    db.conn().execute(
        "INSERT INTO sync_peer (device, seen_through, synced_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT (device) DO UPDATE
             SET seen_through = MAX(COALESCE(seen_through, ''), COALESCE(excluded.seen_through, '')),
                 synced_at    = excluded.synced_at",
        rusqlite::params![bundle.from, high],
    )?;

    Ok(out)
}

/// Overwrite the log entry the triggers just wrote with the timestamp the
/// change arrived carrying.
///
/// Without this an applied change looks like a local edit: it would be sent
/// straight back, and it would outrank the original it came from.
fn stamp(db: &Db, entity: &str, uuid: &str, hlc: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO sync_change (entity, entity_uuid, hlc) VALUES (?1, ?2, ?3)
         ON CONFLICT (entity, entity_uuid) DO UPDATE SET hlc = excluded.hlc",
        rusqlite::params![entity, uuid, hlc],
    )?;
    Ok(())
}

fn record_conflict(
    db: &Db,
    entity: &str,
    uuid: &str,
    hlc: &Hlc,
    body: &serde_json::Value,
) -> Result<()> {
    db.conn().execute(
        "INSERT INTO sync_conflict (entity, entity_uuid, payload, hlc)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![entity, uuid, body.to_string(), hlc.to_string()],
    )?;
    // A receipt that two people disagreed about goes back through the inbox
    // that already exists for "this needs a human". Reusing it rather than
    // inventing a second queue means there is one place to look.
    if entity == "txn" {
        db.conn()
            .execute("UPDATE txn SET needs_review = 1 WHERE uuid = ?1", [uuid])?;
    }
    Ok(())
}

fn watermark(db: &Db, peer: &str, column: &str) -> Result<Option<String>> {
    let sql = format!("SELECT {column} FROM sync_peer WHERE device = ?1");
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query([peer])?;
    Ok(match rows.next()? {
        Some(row) => row.get(0)?,
        None => None,
    })
}

/// Every device this database has exchanged anything with.
pub fn peers(db: &Db) -> Result<Vec<DeviceId>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT device FROM sync_peer ORDER BY device")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
