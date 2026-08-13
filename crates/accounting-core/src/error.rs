//! The crate-wide error type.
//!
//! Domain modules define their own precise error enums ([`ReconcileError`],
//! [`SplitError`], …) so that callers who care can match exhaustively;
//! `CoreError` is the funnel those flow into once they leave a domain
//! boundary. It is `#[non_exhaustive]` because M2 adds storage variants
//! (`Sqlite`, `Migration`, `NotFound`) and adding them must not be a breaking
//! change.

use crate::domain::reconcile::ReconcileError;
use crate::domain::splits::SplitError;
use crate::money::MoneyParseError;
use crate::time::ParseYearMonthError;

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Anything this crate can fail with.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A monetary amount could not be parsed.
    #[error(transparent)]
    MoneyParse(#[from] MoneyParseError),

    /// A `YYYY-MM` month could not be parsed.
    #[error(transparent)]
    ParseYearMonth(#[from] ParseYearMonthError),

    /// Subtotal / tax / total could not be reconciled.
    #[error(transparent)]
    Reconcile(#[from] ReconcileError),

    /// A split draft could not be resolved.
    #[error(transparent)]
    Split(#[from] SplitError),

    /// A value was outside its permitted range or otherwise unusable.
    #[error("invalid input: {0}")]
    Invalid(String),

    /// A referenced entity does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// SQLite refused a statement, or the file could not be opened.
    ///
    /// Constraint violations arrive here too — a CHECK on
    /// `subtotal + tax = total`, or one of the triggers protecting a system
    /// category — so this variant is not only "the disk broke".
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// The schema could not be brought to the latest version.
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Money;

    #[test]
    fn domain_errors_convert_into_core_error() {
        let e: CoreError = Money::parse("nope").unwrap_err().into();
        assert!(matches!(e, CoreError::MoneyParse(_)));
        assert!(!e.to_string().is_empty());
    }
}
