//! Explicit verification of the invariants the schema cannot express.
//!
//! # Why these are not constraints
//!
//! The two aggregate invariants —
//!
//! ```text
//! SUM(txn_split.amount_cents) = txn.total_cents
//! SUM(line_item.amount_cents) = txn_split.amount_cents
//! ```
//!
//! — cannot be CHECK constraints, because CHECK is evaluated per row and
//! cannot aggregate. The obvious workaround, an `AFTER INSERT ON txn_split`
//! trigger that re-sums and raises, is actively worse than nothing: it fires
//! after the *first* split of a two-split receipt and aborts a perfectly legal
//! edit. Any trigger correct enough to allow multi-row edits has to be
//! deferred, and SQLite has no deferred triggers.
//!
//! So the invariants are held by construction — one write path that replaces
//! an entire transaction aggregate inside one SQLite transaction — and audited
//! here. Running this after a corruption test is what makes an empty result
//! mean something: without a test that plants a violation and sees it
//! reported, "no violations" is indistinguishable from "no checks".
//!
//! [`check_all`] is cheap enough (a handful of grouped scans) to run at every
//! debug save, at integration-test teardown, and from `pacc doctor`.

use rusqlite::Row;

use crate::db::Db;
use crate::error::Result;
use crate::ids::{CategoryId, LineItemId, SplitId, TxnId};
use crate::money::Money;

/// How much a violation should alarm the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The data is wrong. A report built on it will not add up.
    Error,
    /// The data is coherent but suspicious, and probably wants a human.
    Warning,
}

/// One thing wrong with the database.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvariantViolation {
    /// A transaction's splits do not sum to its total. The single most
    /// important invariant in the schema: every category report is a `SUM`
    /// over splits, so a mismatch here means the budget and the bank
    /// statement disagree.
    SplitSumMismatch {
        txn_id: TxnId,
        total: Money,
        split_sum: Money,
    },
    /// A transaction's splits sum to the correct total but divide it
    /// differently between subtotal and tax.
    ///
    /// Not caught by [`InvariantViolation::SplitSumMismatch`]: the row-local
    /// CHECKs force `subtotal + tax = amount` on every split and on the
    /// transaction, so an error in one column can be exactly cancelled by the
    /// opposite error in the other while the totals still agree. The damage
    /// shows up in the export, where per-category tax is wrong.
    SplitTaxSumMismatch {
        txn_id: TxnId,
        subtotal: Money,
        subtotal_sum: Money,
        tax: Money,
        tax_sum: Money,
    },
    /// A split's line items do not sum to the split amount.
    LineItemSumMismatch {
        split_id: SplitId,
        amount: Money,
        item_sum: Money,
    },
    /// A transaction with no splits at all. Its money belongs to no category,
    /// so it silently vanishes from every category report while still
    /// appearing in the transaction list.
    TxnWithoutSplits { txn_id: TxnId, total: Money },
    /// A row pointing at a parent that does not exist — found by
    /// `PRAGMA foreign_key_check`, which catches rows written while
    /// `foreign_keys` was off (a restored backup, a manual `sqlite3` session).
    OrphanedRow {
        table: String,
        rowid: Option<i64>,
        parent_table: String,
    },
    /// A split still points at an archived category. Legal — archiving is how
    /// a category is retired without rewriting history — but it means the
    /// category will keep appearing in reports for this period, which is
    /// usually not what the user expected when they archived it.
    SplitOnArchivedCategory {
        split_id: SplitId,
        category_id: CategoryId,
        category_name: String,
    },
    /// A line item whose own category override is archived.
    LineItemOnArchivedCategory {
        line_item_id: LineItemId,
        category_id: CategoryId,
        category_name: String,
    },
    /// A transaction marked itemized whose splits carry no line items, or a
    /// grouped one that does. The two detail levels are one model, but a row
    /// that claims one and stores the other will render wrong.
    DetailLevelMismatch {
        txn_id: TxnId,
        detail_level: String,
        line_item_count: i64,
    },
}

impl InvariantViolation {
    /// Whether this breaks the arithmetic or merely looks odd.
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            InvariantViolation::SplitSumMismatch { .. }
            | InvariantViolation::SplitTaxSumMismatch { .. }
            | InvariantViolation::LineItemSumMismatch { .. }
            | InvariantViolation::TxnWithoutSplits { .. }
            | InvariantViolation::OrphanedRow { .. } => Severity::Error,
            InvariantViolation::SplitOnArchivedCategory { .. }
            | InvariantViolation::LineItemOnArchivedCategory { .. }
            | InvariantViolation::DetailLevelMismatch { .. } => Severity::Warning,
        }
    }
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvariantViolation::SplitSumMismatch {
                txn_id,
                total,
                split_sum,
            } => write!(
                f,
                "txn {txn_id}: splits sum to {} but the total is {}",
                split_sum.to_display_string(),
                total.to_display_string()
            ),
            InvariantViolation::SplitTaxSumMismatch {
                txn_id,
                subtotal,
                subtotal_sum,
                tax,
                tax_sum,
            } => write!(
                f,
                "txn {txn_id}: splits divide the total differently — \
                 subtotal {} vs {}, tax {} vs {}",
                subtotal_sum.to_display_string(),
                subtotal.to_display_string(),
                tax_sum.to_display_string(),
                tax.to_display_string()
            ),
            InvariantViolation::LineItemSumMismatch {
                split_id,
                amount,
                item_sum,
            } => write!(
                f,
                "split {split_id}: line items sum to {} but the split is {}",
                item_sum.to_display_string(),
                amount.to_display_string()
            ),
            InvariantViolation::TxnWithoutSplits { txn_id, total } => write!(
                f,
                "txn {txn_id}: {} with no splits, so it appears in no category",
                total.to_display_string()
            ),
            InvariantViolation::OrphanedRow {
                table,
                rowid,
                parent_table,
            } => match rowid {
                Some(id) => write!(f, "{table} row {id}: no matching {parent_table} row"),
                None => write!(f, "{table}: a row has no matching {parent_table} row"),
            },
            InvariantViolation::SplitOnArchivedCategory {
                split_id,
                category_id,
                category_name,
            } => write!(
                f,
                "split {split_id}: category {category_id} ({category_name}) is archived"
            ),
            InvariantViolation::LineItemOnArchivedCategory {
                line_item_id,
                category_id,
                category_name,
            } => write!(
                f,
                "line item {line_item_id}: category {category_id} ({category_name}) is archived"
            ),
            InvariantViolation::DetailLevelMismatch {
                txn_id,
                detail_level,
                line_item_count,
            } => write!(
                f,
                "txn {txn_id}: marked '{detail_level}' but has {line_item_count} line items"
            ),
        }
    }
}

/// Run every invariant check. An empty vector means the database is coherent.
///
/// Errors are returned before warnings, and within each group in the order the
/// checks are listed below, so output is stable and diffable.
pub fn check_all(db: &Db) -> Result<Vec<InvariantViolation>> {
    let mut out = Vec::new();
    check_split_sums(db, &mut out)?;
    check_split_tax_sums(db, &mut out)?;
    check_line_item_sums(db, &mut out)?;
    check_txns_without_splits(db, &mut out)?;
    check_orphans(db, &mut out)?;
    check_archived_categories(db, &mut out)?;
    check_detail_levels(db, &mut out)?;
    out.sort_by_key(InvariantViolation::severity);
    Ok(out)
}

/// Collect every row of `sql` through `map`.
fn collect<F>(db: &Db, sql: &str, mut map: F, out: &mut Vec<InvariantViolation>) -> Result<()>
where
    F: FnMut(&Row<'_>) -> rusqlite::Result<InvariantViolation>,
{
    let mut stmt = db.conn().prepare(sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(())
}

fn check_split_sums(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    // LEFT JOIN + COALESCE so a transaction with zero splits contributes 0
    // rather than dropping out of the result entirely.
    //
    // Archived transactions are skipped here and in the other txn-level
    // aggregate checks: an archived row is excluded from every report, so its
    // arithmetic no longer affects any number the user sees, and auditing it
    // would turn ordinary deletes into permanent `pacc doctor` noise.
    collect(
        db,
        "SELECT t.id, t.total_cents, COALESCE(SUM(s.amount_cents), 0) AS split_sum
           FROM txn t
           LEFT JOIN txn_split s ON s.txn_id = t.id
          WHERE t.archived_at IS NULL
          GROUP BY t.id
         HAVING split_sum <> t.total_cents
          ORDER BY t.id",
        |row| {
            Ok(InvariantViolation::SplitSumMismatch {
                txn_id: row.get(0)?,
                total: row.get(1)?,
                split_sum: row.get(2)?,
            })
        },
        out,
    )
}

fn check_split_tax_sums(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    // Inner JOIN: a transaction with no splits is check_txns_without_splits'
    // business, and reporting it twice would just add noise.
    //
    // This is not implied by check_split_sums. Because every row carries a
    // CHECK that subtotal + tax = amount, a split can overstate its subtotal
    // by exactly what it understates its tax, and the totals still reconcile.
    // Per-category tax in the export would be wrong with nothing flagged.
    collect(
        db,
        "SELECT t.id, t.subtotal_cents, t.tax_cents,
                COALESCE(SUM(s.subtotal_cents), 0) AS sub_sum,
                COALESCE(SUM(s.tax_cents), 0) AS tax_sum
           FROM txn t
           JOIN txn_split s ON s.txn_id = t.id
          WHERE t.archived_at IS NULL
          GROUP BY t.id
         HAVING sub_sum <> t.subtotal_cents OR tax_sum <> t.tax_cents
          ORDER BY t.id",
        |row| {
            Ok(InvariantViolation::SplitTaxSumMismatch {
                txn_id: row.get(0)?,
                subtotal: row.get(1)?,
                tax: row.get(2)?,
                subtotal_sum: row.get(3)?,
                tax_sum: row.get(4)?,
            })
        },
        out,
    )
}

fn check_line_item_sums(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    // Inner JOIN on purpose: a split with no line items is a *grouped* split,
    // which is the normal case, not a violation.
    collect(
        db,
        "SELECT s.id, s.amount_cents, SUM(li.amount_cents) AS item_sum
           FROM txn_split s
           JOIN line_item li ON li.split_id = s.id
          GROUP BY s.id
         HAVING item_sum <> s.amount_cents
          ORDER BY s.id",
        |row| {
            Ok(InvariantViolation::LineItemSumMismatch {
                split_id: row.get(0)?,
                amount: row.get(1)?,
                item_sum: row.get(2)?,
            })
        },
        out,
    )
}

fn check_txns_without_splits(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    // Not redundant with check_split_sums: a $0.00 transaction with no splits
    // satisfies `0 = 0` there and would otherwise pass unnoticed.
    //
    // Archived transactions are excluded. Soft-deleting a transaction cascades
    // nothing, so an archived row legitimately keeps or loses its splits;
    // reporting it here would make `pacc doctor` accumulate permanent noise
    // from ordinary deletes.
    collect(
        db,
        "SELECT t.id, t.total_cents
           FROM txn t
          WHERE t.archived_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM txn_split s WHERE s.txn_id = t.id)
          ORDER BY t.id",
        |row| {
            Ok(InvariantViolation::TxnWithoutSplits {
                txn_id: row.get(0)?,
                total: row.get(1)?,
            })
        },
        out,
    )
}

fn check_orphans(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    // One PRAGMA covers every FK in the schema, including ones added by future
    // migrations, which a hand-written list of LEFT JOINs would not.
    // Columns: (table, rowid, parent table, fkid). rowid is NULL for a
    // WITHOUT ROWID child table.
    collect(
        db,
        "PRAGMA foreign_key_check",
        |row| {
            Ok(InvariantViolation::OrphanedRow {
                table: row.get(0)?,
                rowid: row.get(1)?,
                parent_table: row.get(2)?,
            })
        },
        out,
    )
}

fn check_archived_categories(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    collect(
        db,
        "SELECT s.id, c.id, c.name
           FROM txn_split s
           JOIN category c ON c.id = s.category_id
          WHERE c.archived_at IS NOT NULL
          ORDER BY s.id",
        |row| {
            Ok(InvariantViolation::SplitOnArchivedCategory {
                split_id: row.get(0)?,
                category_id: row.get(1)?,
                category_name: row.get(2)?,
            })
        },
        out,
    )?;

    collect(
        db,
        "SELECT li.id, c.id, c.name
           FROM line_item li
           JOIN category c ON c.id = li.category_id
          WHERE c.archived_at IS NOT NULL
          ORDER BY li.id",
        |row| {
            Ok(InvariantViolation::LineItemOnArchivedCategory {
                line_item_id: row.get(0)?,
                category_id: row.get(1)?,
                category_name: row.get(2)?,
            })
        },
        out,
    )
}

fn check_detail_levels(db: &Db, out: &mut Vec<InvariantViolation>) -> Result<()> {
    collect(
        db,
        "SELECT t.id, t.detail_level, COUNT(li.id) AS n
           FROM txn t
           LEFT JOIN txn_split s ON s.txn_id = t.id
           LEFT JOIN line_item li ON li.split_id = s.id
          GROUP BY t.id
         HAVING (t.detail_level = 'itemized' AND n = 0)
             OR (t.detail_level = 'grouped'  AND n > 0)
          ORDER BY t.id",
        |row| {
            Ok(InvariantViolation::DetailLevelMismatch {
                txn_id: row.get(0)?,
                detail_level: row.get(1)?,
                line_item_count: row.get(2)?,
            })
        },
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed::seed_defaults;

    /// A seeded database with one $10.00 receipt, one split, no line items.
    fn db_with_one_receipt() -> Db {
        let db = Db::open_in_memory().unwrap();
        seed_defaults(&db).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO txn (id, uuid, txn_date, subtotal_cents, tax_cents, total_cents)
                 VALUES (1, 't-1', '2026-08-12', 900, 100, 1000);
                 INSERT INTO txn_split (id, uuid, txn_id, category_id,
                                        subtotal_cents, tax_cents, amount_cents)
                 VALUES (1, 's-1', 1,
                         (SELECT id FROM category WHERE system_key = 'uncategorized'),
                         900, 100, 1000);",
            )
            .unwrap();
        db
    }

    #[test]
    fn a_seeded_database_is_clean() {
        let db = Db::open_in_memory().unwrap();
        seed_defaults(&db).unwrap();
        assert_eq!(check_all(&db).unwrap(), vec![]);
    }

    #[test]
    fn a_well_formed_receipt_is_clean() {
        assert_eq!(check_all(&db_with_one_receipt()).unwrap(), vec![]);
    }

    #[test]
    fn splits_that_divide_the_total_differently_are_reported() {
        // The blind spot in check_split_sums. Every row-local CHECK still
        // holds and the amounts still sum to the total, but the split moved
        // a dollar from tax into subtotal. Per-category tax in the export
        // would be wrong with nothing else flagging it.
        let db = db_with_one_receipt();
        db.conn()
            .execute(
                "UPDATE txn_split SET subtotal_cents = 1000, tax_cents = 0 WHERE id = 1",
                [],
            )
            .unwrap();

        let found = check_all(&db).unwrap();
        assert!(
            found.iter().any(|v| matches!(
                v,
                InvariantViolation::SplitTaxSumMismatch { txn_id, .. } if txn_id.get() == 1
            )),
            "expected a tax-division mismatch, got {found:?}"
        );
        // The amounts still reconcile, which is the whole point.
        assert!(
            !found
                .iter()
                .any(|v| matches!(v, InvariantViolation::SplitSumMismatch { .. })),
            "the total still balances; only the division is wrong"
        );
    }

    #[test]
    fn an_archived_transaction_without_splits_is_not_reported() {
        // Soft-deleting a transaction is an ordinary user action. If it kept
        // showing up here, `pacc doctor` would accumulate permanent noise and
        // "0 invariant violations" would stop meaning anything.
        let db = db_with_one_receipt();
        db.conn()
            .execute_batch(
                "DELETE FROM txn_split WHERE txn_id = 1;
                 UPDATE txn SET archived_at = '2026-08-12T00:00:00.000Z' WHERE id = 1;",
            )
            .unwrap();

        assert_eq!(check_all(&db).unwrap(), vec![]);
    }

    #[test]
    fn a_live_transaction_without_splits_is_still_reported() {
        // The mirror of the test above, so excluding archived rows cannot
        // quietly disable the check altogether.
        let db = db_with_one_receipt();
        db.conn()
            .execute("DELETE FROM txn_split WHERE txn_id = 1", [])
            .unwrap();

        assert!(
            check_all(&db)
                .unwrap()
                .iter()
                .any(|v| matches!(v, InvariantViolation::TxnWithoutSplits { .. }))
        );
    }

    #[test]
    fn a_split_that_no_longer_sums_is_reported() {
        let db = db_with_one_receipt();
        // Corrupt the split, keeping its own row-local CHECK satisfied — this
        // is exactly the class of damage a CHECK constraint cannot catch.
        db.conn()
            .execute(
                "UPDATE txn_split SET subtotal_cents = 800, amount_cents = 900 WHERE id = 1",
                [],
            )
            .unwrap();

        // Losing a dollar off the split also shifts how the total divides, so
        // check_split_tax_sums fires as well. Assert on the finding that
        // matters rather than on the exact set, which would make this test
        // brittle against every new check.
        let v = check_all(&db).unwrap();
        let sum_mismatch = v
            .iter()
            .find(|f| matches!(f, InvariantViolation::SplitSumMismatch { .. }))
            .unwrap_or_else(|| panic!("expected a split-sum mismatch, got {v:?}"));
        assert_eq!(
            *sum_mismatch,
            InvariantViolation::SplitSumMismatch {
                txn_id: TxnId::new(1),
                total: Money::from_cents(1000),
                split_sum: Money::from_cents(900),
            }
        );
        assert_eq!(sum_mismatch.severity(), Severity::Error);
        assert!(sum_mismatch.to_string().contains("$9.00"));
    }

    #[test]
    fn line_items_that_do_not_sum_are_reported() {
        let db = db_with_one_receipt();
        db.conn()
            .execute_batch(
                "UPDATE txn SET detail_level = 'itemized' WHERE id = 1;
                 INSERT INTO line_item (id, uuid, split_id, description,
                                        subtotal_cents, tax_cents, amount_cents)
                 VALUES (1, 'li-1', 1, 'Apples', 400, 50, 450);",
            )
            .unwrap();

        let v = check_all(&db).unwrap();
        assert_eq!(
            v,
            vec![InvariantViolation::LineItemSumMismatch {
                split_id: SplitId::new(1),
                amount: Money::from_cents(1000),
                item_sum: Money::from_cents(450),
            }]
        );
    }

    #[test]
    fn a_zero_total_transaction_with_no_splits_is_still_reported() {
        // The case that slips past the SUM check: 0 = 0.
        let db = Db::open_in_memory().unwrap();
        seed_defaults(&db).unwrap();
        db.conn()
            .execute(
                "INSERT INTO txn (id, uuid, txn_date, subtotal_cents, tax_cents, total_cents)
                 VALUES (5, 't-5', '2026-08-12', 0, 0, 0)",
                [],
            )
            .unwrap();

        assert_eq!(
            check_all(&db).unwrap(),
            vec![InvariantViolation::TxnWithoutSplits {
                txn_id: TxnId::new(5),
                total: Money::from_cents(0),
            }]
        );
    }

    #[test]
    fn orphans_written_with_foreign_keys_off_are_found() {
        let db = db_with_one_receipt();
        db.conn()
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        db.conn()
            .execute("UPDATE txn_split SET txn_id = 999 WHERE id = 1", [])
            .unwrap();
        db.conn().pragma_update(None, "foreign_keys", "ON").unwrap();

        let v = check_all(&db).unwrap();
        assert!(
            v.iter().any(|x| matches!(
                x,
                InvariantViolation::OrphanedRow { table, parent_table, .. }
                    if table == "txn_split" && parent_table == "txn"
            )),
            "expected an orphaned txn_split, got {v:?}"
        );
    }

    #[test]
    fn a_split_on_an_archived_category_is_a_warning() {
        let db = db_with_one_receipt();
        // Move the split to a user category first: system categories cannot be
        // archived, by trigger.
        db.conn()
            .execute_batch(
                "UPDATE txn_split
                    SET category_id = (SELECT id FROM category WHERE name = 'Groceries');
                 UPDATE category SET archived_at = '2026-08-01T00:00:00Z'
                  WHERE name = 'Groceries';",
            )
            .unwrap();

        let v = check_all(&db).unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].severity(), Severity::Warning);
        assert!(matches!(
            v[0],
            InvariantViolation::SplitOnArchivedCategory { .. }
        ));
    }

    #[test]
    fn a_detail_level_that_contradicts_the_data_is_a_warning() {
        let db = db_with_one_receipt();
        db.conn()
            .execute("UPDATE txn SET detail_level = 'itemized' WHERE id = 1", [])
            .unwrap();

        let v = check_all(&db).unwrap();
        assert_eq!(
            v,
            vec![InvariantViolation::DetailLevelMismatch {
                txn_id: TxnId::new(1),
                detail_level: "itemized".to_string(),
                line_item_count: 0,
            }]
        );
    }

    #[test]
    fn errors_sort_before_warnings() {
        let db = db_with_one_receipt();
        db.conn()
            .execute_batch(
                "UPDATE txn SET detail_level = 'itemized' WHERE id = 1;
                 UPDATE txn_split SET subtotal_cents = 800, amount_cents = 900 WHERE id = 1;",
            )
            .unwrap();

        // Assert the ordering property rather than a count, so adding a check
        // doesn't break a test that is not about how many checks there are.
        let v = check_all(&db).unwrap();
        assert!(v.iter().any(|f| f.severity() == Severity::Error));
        assert!(v.iter().any(|f| f.severity() == Severity::Warning));
        assert!(
            v.windows(2).all(|w| w[0].severity() <= w[1].severity()),
            "errors must sort before warnings, got {v:?}"
        );
    }
}
