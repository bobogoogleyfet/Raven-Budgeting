//! Reading and writing a syncable thing by its cross-device identity.
//!
//! # The problem this solves
//!
//! Every foreign key in the schema is a rowid, and a rowid means nothing on
//! another device. Two databases independently hand out id 7 on their first
//! day — to a grocery category on one and a rent category on the other — so a
//! bundle that carried `group_id: 7` would file the receipt under whatever
//! happened to be seventh over there. The schema said as much in 001: the uuid
//! is the stable cross-device identity, and the INTEGER PRIMARY KEY is local
//! to one file.
//!
//! So every reference is translated on the way out (`group_id` becomes
//! `group_uuid`) and back on the way in. A row is otherwise carried
//! column-for-column, read generically from `PRAGMA table_info`, which means a
//! column added by a later migration travels without this module being taught
//! about it.
//!
//! # Order of application
//!
//! Bundles are ordered by time, but time is the wrong order to *apply* in: a
//! receipt written at noon may refer to a category created at one o'clock on
//! the other device, and inserting the receipt first would fail its foreign
//! key. [`ENTITIES`] is therefore in dependency order, and a merge walks it
//! kind by kind rather than change by change.
//!
//! A reference that is still missing after all that means the peer sent a
//! receipt without the category it points at — a bundle built from a partial
//! range, or a bug. The change is skipped and reported rather than forced:
//! there is no honest value to invent for it.

use rusqlite::types::{Value, ValueRef};

use crate::db::Db;
use crate::error::{CoreError, Result};

/// One kind of thing that syncs.
pub struct EntitySpec {
    /// The name used in a bundle. Same as the table today; kept separate
    /// because a table rename must not change the wire format.
    pub name: &'static str,
    pub table: &'static str,
    /// `(column, entity it points at)`. Carried as `{column-minus-_id}_uuid`.
    pub refs: &'static [(&'static str, &'static str)],
}

/// Every syncable kind, in dependency order.
///
/// Groups before categories, categories before anything that files against
/// one, and the receipt last because it points at the most. Applying in this
/// order means a foreign key is satisfiable by the time it is needed.
///
/// `tax_area` and `tax_rate_release` are deliberately absent: a published
/// table, identical on every device that has refreshed, and the largest thing
/// that could possibly be in a bundle.
pub const ENTITIES: &[EntitySpec] = &[
    EntitySpec {
        name: "category_group",
        table: "category_group",
        refs: &[],
    },
    EntitySpec {
        name: "category",
        table: "category",
        refs: &[("group_id", "category_group")],
    },
    EntitySpec {
        name: "location",
        table: "location",
        refs: &[],
    },
    EntitySpec {
        name: "merchant",
        table: "merchant",
        refs: &[
            ("default_category_id", "category"),
            ("default_location_id", "location"),
        ],
    },
    EntitySpec {
        name: "budget_line",
        table: "budget_line",
        refs: &[("category_id", "category")],
    },
    EntitySpec {
        name: "budget_adjustment",
        table: "budget_adjustment",
        refs: &[("category_id", "category")],
    },
    EntitySpec {
        name: "budget_contribution",
        table: "budget_contribution",
        refs: &[("category_id", "category")],
    },
    EntitySpec {
        name: "txn",
        table: "txn",
        refs: &[("merchant_id", "merchant"), ("location_id", "location")],
    },
];

/// The spec for a kind, if this release knows it.
///
/// `None` for a kind from a newer release, which is an ordinary outcome rather
/// than an error: the bundle *format* is the compatibility boundary, not the
/// entity list, or every new table would break sync for whoever updated second.
#[must_use]
pub fn spec(name: &str) -> Option<&'static EntitySpec> {
    ENTITIES.iter().find(|e| e.name == name)
}

/// A reference that could not be resolved on this device.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MissingRef {
    pub entity: String,
    pub uuid: String,
    /// The kind and uuid that could not be found.
    pub wanted: (String, String),
}

/// `group_id` → `group_uuid`.
fn ref_key(column: &str) -> String {
    format!("{}_uuid", column.strip_suffix("_id").unwrap_or(column))
}

/// Read one row as a bundle body, with references as uuids.
///
/// `None` when the row is gone. That is not the same as a delete — deletes are
/// soft here, so a missing row means it never existed on this device.
pub fn read(db: &Db, spec: &EntitySpec, uuid: &str) -> Result<Option<serde_json::Value>> {
    let sql = format!("SELECT * FROM {} WHERE uuid = ?1", spec.table);
    let mut stmt = db.conn().prepare(&sql)?;
    let names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|c| (*c).to_string())
        .collect();

    let mut rows = stmt.query([uuid])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };

    let mut body = serde_json::Map::new();
    for (i, name) in names.iter().enumerate() {
        // The rowid is this file's business and nobody else's.
        if name == "id" {
            continue;
        }
        if let Some((_, points_at)) = spec.refs.iter().find(|(c, _)| c == name) {
            let target: Option<i64> = row.get(i)?;
            let as_uuid = match target {
                Some(id) => uuid_of(db, points_at, id)?,
                None => None,
            };
            body.insert(ref_key(name), json_of_opt_string(as_uuid));
            continue;
        }
        body.insert(name.clone(), json_of(row.get_ref(i)?));
    }
    Ok(Some(serde_json::Value::Object(body)))
}

/// Write a bundle body, translating references back to local rowids.
///
/// Replaces the row wholesale when it already exists. The aggregate is the
/// unit, and a field-by-field merge is what produces a receipt whose splits do
/// not sum to its total.
pub fn write(
    db: &Db,
    spec: &EntitySpec,
    uuid: &str,
    body: &serde_json::Value,
) -> Result<std::result::Result<(), MissingRef>> {
    let Some(map) = body.as_object() else {
        return Err(CoreError::Invalid(format!(
            "{} {uuid}: body is not an object",
            spec.name
        )));
    };

    let known = columns(db, spec.table)?;
    let mut columns_out: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    for (key, value) in map {
        if key == "id" {
            continue;
        }
        // A column this release does not have: a bundle from a newer app.
        // Dropping it is the only honest option and is better than refusing
        // the whole row, which would mean one device's upgrade stopping the
        // other from syncing at all.
        if let Some(column) = spec
            .refs
            .iter()
            .find(|(c, _)| &ref_key(c) == key)
            .map(|(c, _)| *c)
        {
            let points_at = spec
                .refs
                .iter()
                .find(|(c, _)| *c == column)
                .map(|(_, t)| *t)
                .unwrap_or_default();
            let target = value.as_str();
            let local = match target {
                None => None,
                Some(target_uuid) => match rowid_of(db, points_at, target_uuid)? {
                    Some(id) => Some(id),
                    None => {
                        return Ok(Err(MissingRef {
                            entity: spec.name.to_string(),
                            uuid: uuid.to_string(),
                            wanted: (points_at.to_string(), target_uuid.to_string()),
                        }));
                    }
                },
            };
            columns_out.push(column.to_string());
            values.push(local.map_or(Value::Null, Value::Integer));
            continue;
        }
        if !known.contains(key) {
            continue;
        }
        columns_out.push(key.clone());
        values.push(sql_of(value));
    }

    if columns_out.is_empty() {
        return Err(CoreError::Invalid(format!(
            "{} {uuid}: nothing to write",
            spec.name
        )));
    }

    let placeholders: Vec<String> = (1..=columns_out.len()).map(|i| format!("?{i}")).collect();
    let assignments: Vec<String> = columns_out
        .iter()
        .filter(|c| *c != "uuid")
        .map(|c| format!("{c} = excluded.{c}"))
        .collect();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})
         ON CONFLICT (uuid) DO UPDATE SET {}",
        spec.table,
        columns_out.join(", "),
        placeholders.join(", "),
        assignments.join(", ")
    );
    db.conn()
        .execute(&sql, rusqlite::params_from_iter(values.iter()))?;
    Ok(Ok(()))
}

fn columns(db: &Db, table: &str) -> Result<std::collections::BTreeSet<String>> {
    let mut stmt = db
        .conn()
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

fn uuid_of(db: &Db, table: &str, id: i64) -> Result<Option<String>> {
    let mut stmt = db
        .conn()
        .prepare(&format!("SELECT uuid FROM {table} WHERE id = ?1"))?;
    let mut rows = stmt.query([id])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

fn rowid_of(db: &Db, table: &str, uuid: &str) -> Result<Option<i64>> {
    let mut stmt = db
        .conn()
        .prepare(&format!("SELECT id FROM {table} WHERE uuid = ?1"))?;
    let mut rows = stmt.query([uuid])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

fn json_of(value: ValueRef<'_>) -> serde_json::Value {
    match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::from(i),
        ValueRef::Real(f) => serde_json::Value::from(f),
        ValueRef::Text(t) => serde_json::Value::from(String::from_utf8_lossy(t).into_owned()),
        // No BLOB column exists in this schema; if one ever does, it needs a
        // deliberate encoding rather than a lossy guess.
        ValueRef::Blob(_) => serde_json::Value::Null,
    }
}

fn json_of_opt_string(value: Option<String>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, serde_json::Value::from)
}

fn sql_of(value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Integer(i64::from(*b)),
        serde_json::Value::Number(n) => n.as_i64().map_or_else(
            || n.as_f64().map_or(Value::Null, Value::Real),
            Value::Integer,
        ),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        // Arrays and objects have no column to land in. The aggregate bodies
        // that need nesting carry it under their own keys and are handled by
        // the merge layer, not here.
        other => Value::Text(other.to_string()),
    }
}

/// Every entity of one kind that this device has a change logged for, oldest
/// first, restricted to what is newer than `since`.
pub fn dirty_since(db: &Db, entity: &str, since: Option<&str>) -> Result<Vec<(String, String)>> {
    let mut stmt = db.conn().prepare(
        "SELECT entity_uuid, hlc FROM sync_change
          WHERE entity = ?1 AND (?2 IS NULL OR hlc > ?2)
          ORDER BY hlc",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![entity, since], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The timestamp this device last wrote for one entity, if it has.
pub fn local_hlc(db: &Db, entity: &str, uuid: &str) -> Result<Option<String>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT hlc FROM sync_change WHERE entity = ?1 AND entity_uuid = ?2")?;
    let mut rows = stmt.query(rusqlite::params![entity, uuid])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}
