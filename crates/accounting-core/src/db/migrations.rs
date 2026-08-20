//! Schema migrations, versioned by `PRAGMA user_version`.
//!
//! Every migration file is listed **explicitly** below. The `from-directory`
//! feature of `rusqlite_migration` would do this by globbing, but it pulls in
//! `include_dir` and it hides the ordering in the filesystem, where a rename
//! silently reorders the schema. A literal list is three extra characters per
//! migration and it is reviewable in a diff.
//!
//! # Shipped migrations are immutable
//!
//! Once a migration has run against a real database, editing it makes fresh
//! installs diverge from existing ones with no error anywhere — `user_version`
//! still says 1, so nothing re-runs. Additions go in a new file.
//! [`MIGRATION_SOURCES`] is public so the test suite can hash each one and
//! fail loudly if that rule is broken.

use std::sync::LazyLock;

use rusqlite_migration::{M, Migrations};

/// Every shipped migration, as `(file name, SQL)`, in application order.
///
/// The file name is carried alongside the text so a hash-mismatch failure can
/// name the offending file instead of an index.
pub const MIGRATION_SOURCES: &[(&str, &str)] = &[
    ("001_init.sql", include_str!("schema/001_init.sql")),
    (
        "002_food_rate.sql",
        include_str!("schema/002_food_rate.sql"),
    ),
    (
        "003_utah_rates.sql",
        include_str!("schema/003_utah_rates.sql"),
    ),
    ("004_taxable.sql", include_str!("schema/004_taxable.sql")),
    (
        "005_restaurant_rate.sql",
        include_str!("schema/005_restaurant_rate.sql"),
    ),
];

/// The `user_version` a fully migrated database reports.
pub const SCHEMA_VERSION: usize = MIGRATION_SOURCES.len();

/// The migration set applied by [`crate::db::Db::open`].
pub static MIGRATIONS: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(
        MIGRATION_SOURCES
            .iter()
            .map(|(_, sql)| M::up(sql))
            .collect(),
    )
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migration_set_is_internally_valid() {
        // Catches malformed SQL and a down-migration that does not undo its
        // up-migration, without needing a database.
        MIGRATIONS.validate().expect("migrations are valid");
    }

    #[test]
    fn schema_version_tracks_the_file_list() {
        assert_eq!(SCHEMA_VERSION, 5);
        assert_eq!(MIGRATION_SOURCES[0].0, "001_init.sql");
        assert_eq!(MIGRATION_SOURCES[1].0, "002_food_rate.sql");
        assert_eq!(MIGRATION_SOURCES[2].0, "003_utah_rates.sql");
        assert_eq!(MIGRATION_SOURCES[3].0, "004_taxable.sql");
        assert_eq!(MIGRATION_SOURCES[4].0, "005_restaurant_rate.sql");
        assert!(MIGRATION_SOURCES.iter().all(|(_, sql)| !sql.is_empty()));
    }
}
