//! Scalars this device remembers between runs.
//!
//! `app_setting` has been in the schema since 001, but every user of it so far
//! has hand-rolled the same upsert privately — because every key so far has
//! been a one-time migration marker that exactly one function read. A setting
//! a *person* changes is a different shape: it is read and written by name
//! from two layers away, and a fourth private copy of the same SQL in the one
//! crate that is allowed to contain SQL is not the way to serve that.
//!
//! # Nothing here syncs
//!
//! `app_setting` is deliberately absent from [`crate::sync::entity::ENTITIES`],
//! so a value written here stays on the device that wrote it. That is what
//! makes this the right home for the pairing phrase: a phrase carried inside
//! the bundle it seals would be protecting nothing, and a phrase copied onto
//! the other device would silently un-pair both the moment either changed it.

use crate::db::Db;
use crate::error::Result;

/// The value stored under `key`, or `None` if nothing ever wrote one.
///
/// A missing key is not an error. Every setting has to have an answer for the
/// run before anybody set it, and that answer belongs to the caller who knows
/// what the default means — not to a lookup that would have to invent one.
pub fn get(db: &Db, key: &str) -> Result<Option<String>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT value FROM app_setting WHERE key = ?1")?;
    let mut rows = stmt.query_map([key], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(value) => Ok(Some(value?)),
        None => Ok(None),
    }
}

/// Write `value` under `key`, replacing whatever was there.
///
/// `updated_at` is re-stamped rather than left at whatever the first insert
/// defaulted to: an upsert that only touches `value` leaves the column
/// claiming a setting changed years before it did, which is worse than having
/// no column at all. Reading SQLite's clock here is the schema's own
/// convention (`001_init.sql` defaults this column the same way) and is not
/// the thing rule 3 forbids — no date *logic* is derived from it, and nothing
/// in the crate reads the column back.
pub fn set(db: &Db, key: &str, value: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO app_setting (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET
             value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().expect("in-memory database")
    }

    #[test]
    fn a_key_nobody_wrote_reads_as_nothing() {
        let db = db();
        assert_eq!(get(&db, "sync.lan.phrase").expect("read"), None);
    }

    #[test]
    fn the_last_write_is_what_reads_back() {
        let db = db();
        set(&db, "sync.lan.enabled", "1").expect("write");
        set(&db, "sync.lan.enabled", "0").expect("overwrite");
        assert_eq!(
            get(&db, "sync.lan.enabled").expect("read"),
            Some("0".into())
        );
    }

    #[test]
    fn keys_do_not_read_each_other() {
        let db = db();
        set(&db, "a", "1").expect("write");
        set(&db, "b", "2").expect("write");
        assert_eq!(get(&db, "a").expect("read"), Some("1".into()));
        assert_eq!(get(&db, "b").expect("read"), Some("2".into()));
    }
}
