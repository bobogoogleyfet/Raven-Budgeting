//! Repositories: the only code in the crate that writes rows.
//!
//! # One write path per aggregate
//!
//! [`transactions::save`] is the *only* function that writes a `txn`,
//! `txn_split` or `line_item` row. That is not a style preference: the two
//! aggregate invariants (splits sum to the total, line items sum to the split)
//! cannot be expressed as SQLite constraints — CHECK cannot aggregate, and a
//! trigger fires between the first split and the second — so they are held by
//! construction instead. "By construction" means exactly one function may
//! construct them, and it replaces a whole aggregate inside one
//! [`Db::with_tx`](crate::db::Db::with_tx). A second write path is a second
//! chance to leave a receipt half-written, with nothing to catch it.
//!
//! [`budgets::upsert`] is the second place this matters, for a different
//! reason: two live budget lines covering the same month double every figure
//! derived from them, and no SQLite constraint can express general interval
//! non-overlap. So the check and the write share one transaction — a check
//! outside it races itself.
//!
//! # These functions take `&Db`, not `&Transaction`
//!
//! Each is atomic on its own. Composition of several writes into one unit is
//! the *service* layer's business, and the two places that need it
//! ([`locations::set_home`], [`transactions::save`]) open their own
//! transaction internally.

pub mod budgets;
pub mod categories;
pub mod locations;
pub mod merchants;
pub mod tax_areas;
pub mod transactions;

// Re-exported here as well as from the crate root: `DetailLevel` is declared in
// `transactions` but describes a merchant's remembered preference too, so
// `repo::DetailLevel` is the name callers reach for first.
pub use transactions::DetailLevel;

use crate::error::CoreError;

/// Turn "no rows" into [`CoreError::NotFound`] and leave every other SQLite
/// failure alone.
///
/// `query_row` reports a missing row as an ordinary `Err`, so without this
/// every lookup would report "the database refused a statement" for the
/// entirely routine case of an id that does not exist, and callers could not
/// tell the two apart without matching on a rusqlite variant.
pub(crate) fn not_found(err: rusqlite::Error, what: &str) -> CoreError {
    match err {
        rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound(what.to_string()),
        other => CoreError::Sqlite(other),
    }
}

/// `n` comma-separated `?` placeholders, for an `IN (…)` clause.
///
/// The ids themselves are always bound as parameters
/// ([`rusqlite::params_from_iter`]); only the *number* of placeholders is
/// interpolated. Interpolating the values instead would be an injection hole
/// and would defeat the statement cache, which keys on SQL text.
pub(crate) fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n.saturating_mul(2));
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

/// Sparse ordering: 100, 200, 300 … for a 0-based index.
///
/// Matches the schema's convention, so that dragging one row to a new position
/// rewrites one row instead of renumbering every sibling.
pub(crate) fn sparse(index: usize) -> i64 {
    (i64::try_from(index).unwrap_or(i64::MAX / 100 - 1) + 1) * 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_match_the_count() {
        assert_eq!(placeholders(0), "");
        assert_eq!(placeholders(1), "?");
        assert_eq!(placeholders(3), "?,?,?");
    }

    #[test]
    fn sparse_leaves_room_between_rows() {
        assert_eq!(sparse(0), 100);
        assert_eq!(sparse(2), 300);
    }

    #[test]
    fn missing_rows_become_not_found_and_nothing_else_does() {
        let e = not_found(rusqlite::Error::QueryReturnedNoRows, "category 7");
        assert!(matches!(e, CoreError::NotFound(m) if m == "category 7"));

        let e = not_found(rusqlite::Error::MultipleStatement, "category 7");
        assert!(matches!(e, CoreError::Sqlite(_)));
    }
}
