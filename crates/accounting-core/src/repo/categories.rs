//! The chart of accounts: category groups, categories, and name resolution.
//!
//! # Why resolution lives here
//!
//! Every frontend needs "the user typed `groceries`, which category is that?".
//! Putting that in the CLI would mean writing it again for the GUI, and the
//! two would disagree about ambiguity — which is the interesting half. So the
//! whole ladder (`Group/Name` → exact name → unique prefix) is
//! [`resolve`], and a UI is a picker over [`list`] plus a call to it.

use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::db::seed::SYSTEM_UNCATEGORIZED;
use crate::db::{Db, new_uuid};
use crate::error::{CoreError, Result};
use crate::ids::{CategoryGroupId, CategoryId};
use crate::repo::not_found;

/// One spending category, with its group's name already joined in.
///
/// `group_name` is carried rather than looked up on demand because every
/// display of a category is `Group/Name` — a category name alone is ambiguous
/// by design ("Insurance" under Bills and under Non-Monthly are different
/// envelopes), so a struct that omits the group cannot be rendered correctly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub uuid: String,
    pub group_id: CategoryGroupId,
    pub group_name: String,
    pub name: String,
    /// `Some` for the three categories the application itself depends on.
    /// Such a category cannot be deleted or archived (schema triggers).
    pub system_key: Option<String>,
    pub is_budgeted: bool,
    pub default_rollover: bool,
    pub sort_order: i64,
    pub archived: bool,
}

/// A group of categories — one level of nesting, never two.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryGroup {
    pub id: CategoryGroupId,
    pub uuid: String,
    pub name: String,
    /// Hidden groups exist but are not offered in the picker; the System group
    /// is hidden so Uncategorized/Transfer/Reimbursable do not clutter the
    /// user's own structure.
    pub is_hidden: bool,
    pub sort_order: i64,
    pub archived: bool,
}

impl Category {
    /// `"Group/Name"` — the unambiguous form, and the one [`resolve`] accepts
    /// back.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.group_name, self.name)
    }
}

const SELECT_CATEGORY: &str = "SELECT c.id, c.uuid, c.group_id, g.name, c.name, c.system_key,
                                      c.is_budgeted, c.default_rollover, c.sort_order,
                                      c.archived_at IS NOT NULL
                                 FROM category c
                                 JOIN category_group g ON g.id = c.group_id";

const SELECT_GROUP: &str = "SELECT id, uuid, name, is_hidden, sort_order, archived_at IS NOT NULL
                              FROM category_group";

fn map_category(row: &Row<'_>) -> rusqlite::Result<Category> {
    Ok(Category {
        id: row.get(0)?,
        uuid: row.get(1)?,
        group_id: row.get(2)?,
        group_name: row.get(3)?,
        name: row.get(4)?,
        system_key: row.get(5)?,
        is_budgeted: row.get(6)?,
        default_rollover: row.get(7)?,
        sort_order: row.get(8)?,
        archived: row.get(9)?,
    })
}

fn map_group(row: &Row<'_>) -> rusqlite::Result<CategoryGroup> {
    Ok(CategoryGroup {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        is_hidden: row.get(3)?,
        sort_order: row.get(4)?,
        archived: row.get(5)?,
    })
}

/// Every live category, in picker order: group order, then category order,
/// then name.
///
/// Archived categories are excluded. They still resolve through [`get`], which
/// is what keeps a historical transaction's category name renderable after the
/// user retires it.
pub fn list(db: &Db) -> Result<Vec<Category>> {
    let sql = format!(
        "{SELECT_CATEGORY} WHERE c.archived_at IS NULL
          ORDER BY g.sort_order, c.sort_order, c.name"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt
        .query_map([], map_category)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every category, **archived included**, in the same order as [`list`].
///
/// A report is over history, and history points at categories the user has
/// since retired: a year of Hobbies spending does not stop existing because the
/// category was archived in November. Rendering those rows as "category 17"
/// (which is what falling back to an id does) makes the report unreadable
/// exactly where it is most surprising, and looking each one up individually is
/// the N+1 this exists to avoid.
pub fn list_all(db: &Db) -> Result<Vec<Category>> {
    let sql = format!("{SELECT_CATEGORY} ORDER BY g.sort_order, c.sort_order, c.name");
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt
        .query_map([], map_category)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every live group, in display order.
pub fn list_groups(db: &Db) -> Result<Vec<CategoryGroup>> {
    let sql = format!("{SELECT_GROUP} WHERE archived_at IS NULL ORDER BY sort_order, name");
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = stmt
        .query_map([], map_group)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One category by id, archived or not.
pub fn get(db: &Db, id: CategoryId) -> Result<Category> {
    let sql = format!("{SELECT_CATEGORY} WHERE c.id = ?1");
    db.conn()
        .query_row(&sql, params![id], map_category)
        .map_err(|e| not_found(e, &format!("category {id}")))
}

/// Exact, case-insensitive lookup by bare category name.
///
/// Returns `Ok(None)` both when nothing matches **and** when the name is
/// ambiguous across groups. That is deliberate: names are unique per group,
/// not globally, so "Insurance" may legitimately name two envelopes and
/// picking one at random would silently file the money in the wrong place.
/// Callers that want to tell "missing" from "ambiguous" apart, and get the
/// candidate list, use [`resolve`].
pub fn find_by_name(db: &Db, name: &str) -> Result<Option<Category>> {
    let needle = fold(name);
    let all = list(db)?;
    let mut hits = all.into_iter().filter(|c| fold(&c.name) == needle);
    let first = hits.next();
    if hits.next().is_some() {
        return Ok(None);
    }
    Ok(first)
}

/// Turn whatever the user typed into exactly one category.
///
/// Tried in order, first strategy that yields a single hit wins:
///
/// 1. `"Group/Name"`, both parts matched case-insensitively.
/// 2. The whole string as an exact category name.
/// 3. The whole string as a prefix of a category name.
///
/// # The slash is tried first but is not a commitment
///
/// The default taxonomy ships `Rent/Mortgage` and `Gas/Fuel`, whose *names*
/// contain a slash. So a failed `Group/Name` interpretation falls through to
/// the whole-string strategies rather than reporting "not found" — otherwise
/// `--category Rent/Mortgage` would be unusable out of the box.
///
/// # Ambiguity is an error, never a guess
///
/// [`CoreError::Invalid`] lists the candidates as sorted `Group/Name` strings,
/// because the only useful thing a user can do with "ambiguous" is retype an
/// unambiguous one, and they cannot do that without seeing the options.
pub fn resolve(db: &Db, needle: &str) -> Result<Category> {
    let q = needle.trim();
    if q.is_empty() {
        return Err(CoreError::Invalid(
            "category name is empty; pass a name like `Groceries` or `Frequent/Groceries`"
                .to_string(),
        ));
    }
    let all = list(db)?;

    // 1. Group/Name. split_once (not rsplit) so "Bills/Rent/Mortgage" reads as
    //    group "Bills" and the category name that itself contains a slash.
    if let Some((group, name)) = q.split_once('/') {
        let (group, name) = (fold(group), fold(name));
        let hits: Vec<&Category> = all
            .iter()
            .filter(|c| fold(&c.group_name) == group && fold(&c.name) == name)
            .collect();
        match hits.as_slice() {
            [one] => return Ok((*one).clone()),
            [] => {} // fall through: the name may itself contain the slash
            many => return Err(ambiguous(q, many)),
        }
    }

    // 2. Exact name.
    let folded = fold(q);
    let exact: Vec<&Category> = all.iter().filter(|c| fold(&c.name) == folded).collect();
    match exact.as_slice() {
        [one] => return Ok((*one).clone()),
        [] => {}
        many => return Err(ambiguous(q, many)),
    }

    // 3. Unique prefix.
    let prefix: Vec<&Category> = all
        .iter()
        .filter(|c| fold(&c.name).starts_with(&folded))
        .collect();
    match prefix.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(CoreError::NotFound(format!(
            "no category matching `{q}` (try `pacc categories` for the list)"
        ))),
        many => Err(ambiguous(q, many)),
    }
}

/// The fallback category every uncategorised amount lands in.
///
/// Looked up by `system_key`, never by name: the user is allowed to rename
/// "Uncategorized" to "Inbox", and quick-add must keep working when they do.
pub fn uncategorized(db: &Db) -> Result<Category> {
    let sql = format!("{SELECT_CATEGORY} WHERE c.system_key = ?1");
    db.conn()
        .query_row(&sql, params![SYSTEM_UNCATEGORIZED], map_category)
        .map_err(|e| {
            not_found(
                e,
                "the Uncategorized system category (run `pacc init` to reinstall the defaults)",
            )
        })
}

/// Add a category to a group, sorted after everything already in it.
pub fn create(db: &Db, group_id: CategoryGroupId, name: &str) -> Result<Category> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("category name is empty".to_string()));
    }

    let id = db.with_tx(|tx| {
        // Checked explicitly so a bad group id reads as "not found" rather
        // than as a raw FOREIGN KEY failure, which tells the user nothing.
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM category_group WHERE id = ?1)",
            params![group_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(CoreError::NotFound(format!("category group {group_id}")));
        }

        // Sparse, like the seed: the next slot is 100 past the last one, so a
        // later drag-reorder rewrites one row instead of the whole group.
        let sort_order: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 100 FROM category WHERE group_id = ?1",
            params![group_id],
            |r| r.get(0),
        )?;

        tx.execute(
            "INSERT INTO category (uuid, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
            params![new_uuid(), group_id, name, sort_order],
        )?;
        Ok(CategoryId::new(tx.last_insert_rowid()))
    })?;

    get(db, id)
}

/// Retire a category without touching the history that points at it.
///
/// Nothing user-facing is ever hard-deleted: a historical transaction must
/// keep resolving its category name, so archiving is the only retirement.
///
/// A system category is refused **by the schema trigger**, surfacing as
/// [`CoreError::Sqlite`]. That is not pre-empted here on purpose — a Rust-side
/// guard would be a second, drifting copy of a rule the database already
/// enforces for every writer, including `sqlite3` on the command line.
pub fn archive(db: &Db, id: CategoryId) -> Result<()> {
    // Existence is checked first so a typo'd id is "not found" rather than a
    // silent no-op that looks like success.
    let _ = get(db, id)?;
    db.conn().execute(
        "UPDATE category
            SET archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                updated_at  = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Case-insensitive comparison key.
///
/// `to_lowercase` rather than `eq_ignore_ascii_case`, so a user with an
/// accented or non-Latin category name gets the same case-insensitivity
/// everyone else does.
fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

fn ambiguous(needle: &str, candidates: &[&Category]) -> CoreError {
    let mut names: Vec<String> = candidates.iter().map(|c| c.qualified_name()).collect();
    names.sort();
    CoreError::Invalid(format!(
        "`{needle}` matches {} categories: {}; use one of those exactly",
        names.len(),
        names.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    #[test]
    fn list_is_in_picker_order_and_hides_archived() {
        let db = seeded();
        let all = list(&db).unwrap();
        assert!(all.iter().all(|c| !c.archived));

        // Bills (group 100) before Frequent (group 200), and within Bills the
        // seed order is preserved rather than alphabetised.
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        let rent = names.iter().position(|n| *n == "Rent/Mortgage").unwrap();
        let groceries = names.iter().position(|n| *n == "Groceries").unwrap();
        let electric = names.iter().position(|n| *n == "Electric").unwrap();
        assert!(rent < electric && electric < groceries);

        let hobbies = find_by_name(&db, "Hobbies").unwrap().unwrap();
        archive(&db, hobbies.id).unwrap();
        assert!(list(&db).unwrap().iter().all(|c| c.name != "Hobbies"));
        // …but it still loads by id, so old transactions keep their name.
        assert!(get(&db, hobbies.id).unwrap().archived);
        // …and `list_all` still carries it, so a report over last year's
        // spending can still name the category it was filed under.
        let all_including_archived = list_all(&db).unwrap();
        assert_eq!(all_including_archived.len(), all.len());
        assert!(
            all_including_archived
                .iter()
                .any(|c| c.name == "Hobbies" && c.archived)
        );
    }

    #[test]
    fn resolve_accepts_the_three_forms() {
        let db = seeded();
        assert_eq!(
            resolve(&db, "Frequent/Groceries").unwrap().name,
            "Groceries"
        );
        assert_eq!(
            resolve(&db, "frequent/groceries").unwrap().name,
            "Groceries"
        );
        assert_eq!(resolve(&db, "groceries").unwrap().name, "Groceries");
        assert_eq!(resolve(&db, "groc").unwrap().name, "Groceries");
        assert_eq!(resolve(&db, "  Groceries  ").unwrap().name, "Groceries");
    }

    #[test]
    fn a_category_name_containing_a_slash_still_resolves() {
        // The seed ships Rent/Mortgage and Gas/Fuel. Reading the slash as a
        // group separator and stopping there would make both unreachable.
        let db = seeded();
        assert_eq!(resolve(&db, "Rent/Mortgage").unwrap().name, "Rent/Mortgage");
        assert_eq!(resolve(&db, "Gas/Fuel").unwrap().name, "Gas/Fuel");
        assert_eq!(
            resolve(&db, "Bills/Rent/Mortgage").unwrap().name,
            "Rent/Mortgage"
        );
    }

    #[test]
    fn an_ambiguous_needle_lists_the_candidates() {
        let db = seeded();
        // "Insurance (Auto)" and "Insurance (Health)" both start with "ins".
        let err = resolve(&db, "ins").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("`ins`"), "{msg}");
        assert!(msg.contains("Bills/Insurance (Auto)"), "{msg}");
        assert!(msg.contains("Bills/Insurance (Health)"), "{msg}");
        assert!(matches!(err, CoreError::Invalid(_)));
    }

    #[test]
    fn an_unknown_needle_names_itself() {
        let db = seeded();
        let err = resolve(&db, "zzz").unwrap_err();
        assert!(err.to_string().contains("zzz"), "{err}");
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[test]
    fn a_duplicated_bare_name_is_ambiguous_not_a_coin_flip() {
        let db = seeded();
        let groups = list_groups(&db).unwrap();
        let non_monthly = groups.iter().find(|g| g.name == "Non-Monthly").unwrap();
        create(&db, non_monthly.id, "Groceries").unwrap();

        assert_eq!(find_by_name(&db, "Groceries").unwrap(), None);
        let err = resolve(&db, "Groceries").unwrap_err();
        assert!(err.to_string().contains("Frequent/Groceries"), "{err}");
        // …and qualifying it disambiguates.
        assert_eq!(
            resolve(&db, "Non-Monthly/Groceries").unwrap().group_name,
            "Non-Monthly"
        );
    }

    #[test]
    fn created_categories_are_sparse_and_land_in_their_group() {
        let db = seeded();
        let groups = list_groups(&db).unwrap();
        let savings = groups.iter().find(|g| g.name == "Savings").unwrap();
        let before: i64 = db
            .conn()
            .query_row(
                "SELECT MAX(sort_order) FROM category WHERE group_id = ?1",
                params![savings.id],
                |r| r.get(0),
            )
            .unwrap();

        let made = create(&db, savings.id, "  Vacation Fund  ").unwrap();
        assert_eq!(made.name, "Vacation Fund");
        assert_eq!(made.group_id, savings.id);
        assert_eq!(made.group_name, "Savings");
        assert_eq!(made.sort_order, before + 100);
        assert!(!made.archived);

        assert!(create(&db, savings.id, "   ").is_err());
        assert!(create(&db, CategoryGroupId::new(9999), "Nope").is_err());
    }

    #[test]
    fn uncategorized_survives_a_rename() {
        let db = seeded();
        db.conn()
            .execute(
                "UPDATE category SET name = 'Inbox' WHERE system_key = 'uncategorized'",
                [],
            )
            .unwrap();
        assert_eq!(uncategorized(&db).unwrap().name, "Inbox");
    }

    #[test]
    fn archiving_a_system_category_is_refused_by_the_database() {
        let db = seeded();
        let unc = uncategorized(&db).unwrap();
        let err = archive(&db, unc.id).unwrap_err();
        assert!(matches!(err, CoreError::Sqlite(_)), "{err}");
        assert!(err.to_string().contains("cannot be archived"), "{err}");
    }

    #[test]
    fn a_missing_id_is_not_found() {
        let db = seeded();
        assert!(matches!(
            get(&db, CategoryId::new(9999)),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            archive(&db, CategoryId::new(9999)),
            Err(CoreError::NotFound(_))
        ));
    }
}
