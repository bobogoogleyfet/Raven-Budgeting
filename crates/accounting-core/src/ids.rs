//! Typed row identifiers.
//!
//! Every id is a distinct newtype over `i64` (SQLite's native rowid type), so
//! passing a `CategoryId` where a `MerchantId` is expected is a compile error
//! rather than a silent lookup against the wrong table. They are
//! `#[serde(transparent)]`, so they cross the UI boundary as plain integers.

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            /// Wrap a raw rowid.
            #[must_use]
            pub const fn new(id: i64) -> Self {
                Self(id)
            }

            /// The raw rowid.
            #[must_use]
            pub const fn get(self) -> i64 {
                self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> i64 {
                id.0
            }
        }
    };
}

define_id!(
    /// A transaction.
    TxnId
);
define_id!(
    /// One split of a transaction.
    SplitId
);
define_id!(
    /// One line item within a split.
    LineItemId
);
define_id!(
    /// A spending category.
    CategoryId
);
define_id!(
    /// A group of categories.
    CategoryGroupId
);
define_id!(
    /// A merchant.
    MerchantId
);
define_id!(
    /// A physical location (which determines the sales-tax rate).
    LocationId
);
define_id!(
    /// One line of a budget.
    BudgetLineId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_formats() {
        let id = CategoryId::new(42);
        assert_eq!(id.get(), 42);
        assert_eq!(i64::from(id), 42);
        assert_eq!(format!("{id:?}"), "CategoryId(42)");
        assert_eq!(id.to_string(), "42");
    }

    #[test]
    fn ids_are_ordered_and_hashable() {
        let mut v = vec![TxnId::new(3), TxnId::new(1), TxnId::new(2)];
        v.sort();
        assert_eq!(v, vec![TxnId::new(1), TxnId::new(2), TxnId::new(3)]);

        let mut set = std::collections::HashSet::new();
        assert!(set.insert(SplitId::new(1)));
        assert!(!set.insert(SplitId::new(1)));
    }
}
