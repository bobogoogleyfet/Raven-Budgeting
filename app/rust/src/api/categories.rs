//! Listing categories.

use accounting_core::repo::categories;

use crate::api::db;
use crate::error::to_message;

/// One row of the category list.
///
/// Mirrors the Tauri shell's `CategoryDto` field for field, so the two
/// frontends render the same thing. The core's `Category` is not exposed
/// directly: its ids are `#[serde(transparent)]` newtypes, and serde is not in
/// play at an FFI boundary, so each would surface in Dart as a wrapper class.
///
/// `id` is an `i64` **in a struct field**, which the bridge maps to
/// `PlatformInt64` (`typedef = int` on native). A bare `Vec<i64>` would arrive
/// as `BigInt` instead — exact, but needing an unwrap at every use.
pub struct CategoryDto {
    pub id: i64,
    pub name: String,
    pub group_name: String,
    pub is_budgeted: bool,
}

/// Every category that is not archived, ordered by group, then by the group's
/// own sort order, then by name.
pub fn list_categories() -> Result<Vec<CategoryDto>, String> {
    let db = db::locked()?;
    let rows = categories::list(&db).map_err(to_message)?;
    Ok(rows
        .into_iter()
        .map(|c| CategoryDto {
            id: c.id.get(),
            name: c.name,
            group_name: c.group_name,
            is_budgeted: c.is_budgeted,
        })
        .collect())
}
