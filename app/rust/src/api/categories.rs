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

/// One of the groups a category can belong to.
pub struct CategoryGroupDto {
    pub id: i64,
    pub name: String,
    /// The System group holds the three categories the app itself depends on,
    /// and is no answer to "where does this belong".
    pub is_hidden: bool,
}

/// The groups, in the order the seed put them in.
pub fn list_category_groups() -> Result<Vec<CategoryGroupDto>, String> {
    let db = db::locked()?;
    Ok(categories::list_groups(&db)
        .map_err(to_message)?
        .into_iter()
        .map(|g| CategoryGroupDto {
            id: g.id.get(),
            name: g.name,
            is_hidden: g.is_hidden,
        })
        .collect())
}

/// Add a category.
///
/// `taxable` says whether a till adds sales tax to it. Asked rather than
/// assumed: the column defaults to taxable, and a car payment taxed at 7.45%
/// is $537.25 where the statement says $500.00.
pub fn create_category(
    group_id: i64,
    name: String,
    taxable: bool,
) -> Result<CategoryDto, String> {
    let db = db::locked()?;
    let created = categories::create_with_tax(
        &db,
        accounting_core::CategoryGroupId::new(group_id),
        &name,
        taxable,
    )
    .map_err(to_message)?;
    Ok(CategoryDto {
        id: created.id.get(),
        name: created.name,
        group_name: created.group_name,
        is_budgeted: created.is_budgeted,
    })
}

/// Add a group for categories to go in.
///
/// Wanted because the seeded six are one person's idea of how spending divides
/// up. Education ships as an empty group for the same reason: the shape is
/// useful, the contents are personal.
pub fn create_category_group(name: String) -> Result<CategoryGroupDto, String> {
    let db = db::locked()?;
    let created = categories::create_group(&db, &name).map_err(to_message)?;
    Ok(CategoryGroupDto {
        id: created.id.get(),
        name: created.name,
        is_hidden: created.is_hidden,
    })
}
