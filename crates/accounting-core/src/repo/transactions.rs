//! The transaction aggregate: `txn` → `txn_split` → `line_item`.
//!
//! # One model at two detail levels, not two modes
//!
//! "Grouped" is a split with no line items; "itemized" is a split expanded
//! into them. Every report reads `txn_split` and is indifferent to which the
//! user chose, so adding itemization later never became a second code path.
//!
//! # This module owns every write to those three tables
//!
//! [`save`] is the only function in the workspace that inserts, updates or
//! deletes a `txn`, `txn_split` or `line_item` row. See the module docs of
//! [`crate::repo`] for why that is load-bearing rather than tidy.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use rusqlite::types::Value;
use rusqlite::{Row, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::db::{Db, new_uuid};
use crate::domain::reconcile::Amounts;
use crate::error::{CoreError, Result};
use crate::ids::{CategoryId, LineItemId, LocationId, MerchantId, SplitId, TxnId};
use crate::money::Money;
use crate::repo::{placeholders, sparse};
use crate::time::{MonthRange, YearMonth};

/// How much detail a transaction records.
///
/// Stored as the lowercase text the schema's `CHECK (… IN ('grouped',
/// 'itemized'))` accepts, so the database and the enum cannot drift apart.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailLevel {
    /// One row per category. The default, and what most receipts deserve.
    #[default]
    Grouped,
    /// Every line of the receipt, inside its split.
    Itemized,
}

impl DetailLevel {
    /// The exact text stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DetailLevel::Grouped => "grouped",
            DetailLevel::Itemized => "itemized",
        }
    }
}

impl fmt::Display for DetailLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DetailLevel {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("grouped") {
            Ok(DetailLevel::Grouped)
        } else if t.eq_ignore_ascii_case("itemized") {
            Ok(DetailLevel::Itemized)
        } else {
            Err(CoreError::Invalid(format!(
                "`{s}` is not a detail level (expected `grouped` or `itemized`)"
            )))
        }
    }
}

/// One line of a receipt, as handed to [`save`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItemRecord {
    /// Ignored on write — see [`SplitRecord::id`].
    pub id: Option<LineItemId>,
    /// Reused when `Some`, so a round-tripped item keeps its cross-device
    /// identity through an edit.
    pub uuid: Option<String>,
    pub description: String,
    /// Thousandths of a unit, so 1.5 lb of apples is `1_500`. Integer,
    /// because a `REAL` quantity times a price is how you get `$3.9299999`.
    pub quantity_milli: i64,
    pub unit_price: Option<Money>,
    pub subtotal: Money,
    pub tax: Money,
    /// `subtotal + tax`.
    pub amount: Money,
    /// Per-item category override; `None` means "same as the split".
    pub category_id: Option<CategoryId>,
    pub is_taxable: bool,
    /// `0` asks [`save`] for the sparse default (100, 200, 300 …).
    pub sort_order: i64,
    pub notes: Option<String>,
}

/// One category's share of a transaction, as handed to [`save`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitRecord {
    /// Accepted and **ignored**: [`save`] replaces the whole aggregate, so
    /// splits always get fresh rowids. The field exists so a loaded
    /// transaction can round-trip through these types without the caller
    /// having to strip it. Stable identity is the `uuid`; a rowid is local to
    /// one database file anyway.
    pub id: Option<SplitId>,
    /// Reused when `Some`, minted otherwise.
    pub uuid: Option<String>,
    pub category_id: CategoryId,
    pub subtotal: Money,
    pub tax: Money,
    /// `subtotal + tax`.
    pub amount: Money,
    /// `0` asks [`save`] for the sparse default (100, 200, 300 …).
    pub sort_order: i64,
    pub notes: Option<String>,
    pub line_items: Vec<LineItemRecord>,
}

/// A whole transaction aggregate, ready to be written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTransaction {
    /// `None` inserts. `Some(id)` **replaces** that transaction's aggregate in
    /// place, keeping its uuid and `created_at`.
    pub id: Option<TxnId>,
    pub txn_date: NaiveDate,
    pub merchant_id: Option<MerchantId>,
    pub location_id: Option<LocationId>,
    pub amounts: Amounts,
    /// The rate actually used at entry time, kept per row: the location's rate
    /// changes, and last year's receipt must not change with it.
    pub tax_rate_ppm: Option<u32>,
    pub detail_level: DetailLevel,
    pub needs_review: bool,
    pub notes: Option<String>,
    pub splits: Vec<SplitRecord>,
}

/// A line item as loaded from the database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedLineItem {
    pub id: LineItemId,
    pub uuid: String,
    pub description: String,
    pub quantity_milli: i64,
    pub unit_price: Option<Money>,
    pub subtotal: Money,
    pub tax: Money,
    pub amount: Money,
    pub category_id: Option<CategoryId>,
    /// `None` when the item has no category override of its own.
    pub category_name: Option<String>,
    pub is_taxable: bool,
    pub sort_order: i64,
    pub notes: Option<String>,
}

/// A split as loaded from the database, with its category names joined in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedSplit {
    pub id: SplitId,
    pub uuid: String,
    pub category_id: CategoryId,
    pub category_name: String,
    pub group_name: String,
    pub subtotal: Money,
    pub tax: Money,
    pub amount: Money,
    pub sort_order: i64,
    pub notes: Option<String>,
    pub line_items: Vec<LoadedLineItem>,
}

/// A whole transaction aggregate as loaded from the database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Txn {
    pub id: TxnId,
    pub uuid: String,
    pub txn_date: NaiveDate,
    pub merchant_id: Option<MerchantId>,
    pub merchant_name: Option<String>,
    pub location_id: Option<LocationId>,
    pub location_name: Option<String>,
    pub subtotal: Money,
    pub tax: Money,
    pub total: Money,
    /// True only when the tax was computed rather than read off the receipt.
    /// Surfaced in the UI and exported, because an estimate that silently
    /// looks like a fact is worse than no estimate.
    pub tax_is_estimated: bool,
    pub tax_rate_ppm: Option<u32>,
    pub detail_level: DetailLevel,
    pub needs_review: bool,
    pub notes: Option<String>,
    pub archived: bool,
    pub splits: Vec<LoadedSplit>,
}

impl Txn {
    /// The amounts of this transaction, in the shape the domain calculations
    /// take. Used to re-plan splits over an existing receipt without ever
    /// recomputing its tax.
    #[must_use]
    pub fn amounts(&self) -> Amounts {
        Amounts {
            subtotal: self.subtotal,
            tax: self.tax,
            total: self.total,
            tax_is_estimated: self.tax_is_estimated,
        }
    }
}

/// Which transactions [`list`] and [`count`] should return.
///
/// Every field is additive: an all-default filter matches every live
/// transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxnFilter {
    /// Inclusive lower bound on `txn_date`.
    pub from: Option<NaiveDate>,
    /// Inclusive upper bound on `txn_date`.
    pub to: Option<NaiveDate>,
    pub merchant_id: Option<MerchantId>,
    /// Matches a transaction if **any** of its splits is in this category.
    pub category_id: Option<CategoryId>,
    pub needs_review: Option<bool>,
    pub include_archived: bool,
    pub limit: Option<usize>,
}

impl TxnFilter {
    /// A filter for the needs-categorizing inbox.
    #[must_use]
    pub fn needs_review(limit: Option<usize>) -> Self {
        TxnFilter {
            needs_review: Some(true),
            limit,
            ..TxnFilter::default()
        }
    }

    /// The `WHERE` body and its bound values, numbered from `?1`.
    ///
    /// Values are always bound, never interpolated: a merchant name is user
    /// input and a date is user input, and the only difference between a
    /// filter and an injection is who wrote the string.
    fn where_clause(&self) -> (String, Vec<Value>) {
        let mut clauses: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        if !self.include_archived {
            clauses.push("t.archived_at IS NULL".to_string());
        }
        if let Some(from) = self.from {
            values.push(date_value(from));
            clauses.push(format!("t.txn_date >= ?{}", values.len()));
        }
        if let Some(to) = self.to {
            values.push(date_value(to));
            clauses.push(format!("t.txn_date <= ?{}", values.len()));
        }
        if let Some(merchant) = self.merchant_id {
            values.push(Value::Integer(merchant.get()));
            clauses.push(format!("t.merchant_id = ?{}", values.len()));
        }
        if let Some(category) = self.category_id {
            values.push(Value::Integer(category.get()));
            // EXISTS rather than a JOIN: a three-way split that touches the
            // category twice must still yield one transaction, not two.
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM txn_split s WHERE s.txn_id = t.id AND s.category_id = ?{})",
                values.len()
            ));
        }
        if let Some(needs) = self.needs_review {
            values.push(Value::Integer(i64::from(needs)));
            clauses.push(format!("t.needs_review = ?{}", values.len()));
        }

        if clauses.is_empty() {
            clauses.push("1 = 1".to_string());
        }
        (clauses.join(" AND "), values)
    }
}

/// `txn_date` is TEXT `'YYYY-MM-DD'` in the schema, which compares
/// chronologically as a string — that is the whole reason for the format.
fn date_value(d: NaiveDate) -> Value {
    Value::Text(d.format("%Y-%m-%d").to_string())
}

const SELECT_TXN: &str = "SELECT t.id, t.uuid, t.txn_date, t.merchant_id, m.name,
                                 t.location_id, l.name, t.subtotal_cents, t.tax_cents,
                                 t.total_cents, t.tax_is_estimated, t.tax_rate_ppm,
                                 t.detail_level, t.needs_review, t.notes,
                                 t.archived_at IS NOT NULL
                            FROM txn t
                            LEFT JOIN merchant m ON m.id = t.merchant_id
                            LEFT JOIN location l ON l.id = t.location_id";

const SELECT_SPLIT: &str = "SELECT s.id, s.uuid, s.txn_id, s.category_id, c.name, g.name,
                                   s.subtotal_cents, s.tax_cents, s.amount_cents,
                                   s.sort_order, s.notes
                              FROM txn_split s
                              JOIN category c ON c.id = s.category_id
                              JOIN category_group g ON g.id = c.group_id";

const SELECT_ITEM: &str = "SELECT li.id, li.uuid, li.split_id, li.description, li.quantity_milli,
                                  li.unit_price_cents, li.subtotal_cents, li.tax_cents,
                                  li.amount_cents, li.category_id, c.name, li.is_taxable,
                                  li.sort_order, li.notes
                             FROM line_item li
                             LEFT JOIN category c ON c.id = li.category_id";

fn map_txn(row: &Row<'_>) -> rusqlite::Result<Txn> {
    Ok(Txn {
        id: row.get(0)?,
        uuid: row.get(1)?,
        txn_date: row.get(2)?,
        merchant_id: row.get(3)?,
        merchant_name: row.get(4)?,
        location_id: row.get(5)?,
        location_name: row.get(6)?,
        subtotal: row.get(7)?,
        tax: row.get(8)?,
        total: row.get(9)?,
        tax_is_estimated: row.get(10)?,
        tax_rate_ppm: row.get(11)?,
        detail_level: row.get(12)?,
        needs_review: row.get(13)?,
        notes: row.get(14)?,
        archived: row.get(15)?,
        splits: Vec::new(),
    })
}

/// Returns the owning `txn_id` alongside the split, so the caller can group
/// without a second query.
fn map_split(row: &Row<'_>) -> rusqlite::Result<(TxnId, LoadedSplit)> {
    Ok((
        row.get(2)?,
        LoadedSplit {
            id: row.get(0)?,
            uuid: row.get(1)?,
            category_id: row.get(3)?,
            category_name: row.get(4)?,
            group_name: row.get(5)?,
            subtotal: row.get(6)?,
            tax: row.get(7)?,
            amount: row.get(8)?,
            sort_order: row.get(9)?,
            notes: row.get(10)?,
            line_items: Vec::new(),
        },
    ))
}

fn map_item(row: &Row<'_>) -> rusqlite::Result<(SplitId, LoadedLineItem)> {
    Ok((
        row.get(2)?,
        LoadedLineItem {
            id: row.get(0)?,
            uuid: row.get(1)?,
            description: row.get(3)?,
            quantity_milli: row.get(4)?,
            unit_price: row.get(5)?,
            subtotal: row.get(6)?,
            tax: row.get(7)?,
            amount: row.get(8)?,
            category_id: row.get(9)?,
            category_name: row.get(10)?,
            is_taxable: row.get(11)?,
            sort_order: row.get(12)?,
            notes: row.get(13)?,
        },
    ))
}

/// Write a whole transaction aggregate, inserting or replacing it.
///
/// # Why this deletes and re-inserts instead of diffing
///
/// A diffing write path updates the splits that changed, inserts the new ones
/// and deletes the removed ones — and between any two of those statements the
/// aggregate is *half replaced*: the splits no longer sum to the total. Inside
/// one SQLite transaction no other connection can observe that, but the
/// deferred consequence is worse: nothing in the database can prove the diff
/// was complete. The two aggregate invariants
/// (`SUM(txn_split.amount_cents) = txn.total_cents`,
/// `SUM(line_item.amount_cents) = txn_split.amount_cents`) cannot be CHECK
/// constraints, because CHECK is evaluated per row and cannot aggregate; and a
/// trigger that re-sums fires after the *first* split of a two-split receipt
/// and rejects a legal edit, since SQLite has no deferred triggers. So the
/// invariants are held by construction: this function computes the complete
/// new set of children and swaps it in atomically. A partial write is not
/// something we detect — it is something that cannot be expressed.
///
/// Everything that can be checked is checked **before** the transaction opens,
/// so a rejected receipt never costs a rollback and the error names the
/// arithmetic rather than a constraint name.
///
/// # Ordering
///
/// A split's `sort_order` is used when non-zero, and otherwise assigned
/// sparsely (100, 200, 300 …) from its position, matching the schema's
/// convention so a later drag-reorder rewrites one row. Line items work the
/// same way.
pub fn save(db: &Db, new: &NewTransaction) -> Result<TxnId> {
    validate(new)?;

    let id = db.with_tx(|tx| {
        let id = match new.id {
            Some(id) => {
                update_header(tx, id, new)?;
                // Line items cascade with their splits (ON DELETE CASCADE), so
                // this one statement clears the entire child set.
                tx.execute("DELETE FROM txn_split WHERE txn_id = ?1", params![id])?;
                id
            }
            None => insert_header(tx, new)?,
        };

        for (index, split) in new.splits.iter().enumerate() {
            insert_split(tx, id, index, split)?;
        }
        Ok(id)
    })?;

    #[cfg(debug_assertions)]
    {
        // The audit that turns "single write path" from a comment into a
        // checkable claim: if this function ever writes an aggregate that does
        // not add up, the very next debug run of any test that saves will say
        // so, naming the violation.
        //
        // Debug only, and errors only. It is O(table), so it must not run in a
        // release build; and the warning-severity findings (a split on a
        // category the user has since archived, an itemized transaction the
        // user has not itemized yet) are legitimate states that `save` is
        // required to allow, so asserting on them would turn documented
        // behaviour into a panic.
        let broken: Vec<String> = crate::db::check_all(db)?
            .into_iter()
            .filter(|v| v.severity() == crate::db::Severity::Error)
            .map(|v| v.to_string())
            .collect();
        debug_assert!(
            broken.is_empty(),
            "save left the database inconsistent: {}",
            broken.join("; ")
        );
    }

    Ok(id)
}

/// Everything checkable, checked before a single row is written.
///
/// The three sums are verified **separately**. Checking only the total would
/// admit a split that overstates its subtotal by exactly what it understates
/// its tax: every row-local CHECK still passes, the transaction still
/// balances, and per-category tax in the export is silently wrong. That is
/// precisely the damage [`crate::db::integrity::check_all`] reports as
/// `SplitTaxSumMismatch`, and `save` exists to make it unreachable.
fn validate(new: &NewTransaction) -> Result<()> {
    if new.splits.is_empty() {
        return Err(CoreError::Invalid(format!(
            "a transaction for {} has no splits; every amount must belong to a category",
            new.amounts.total.to_display_string()
        )));
    }
    if !new.amounts.is_consistent() {
        return Err(CoreError::Invalid(format!(
            "amounts do not balance: subtotal {} + tax {} is {}, but the total is {}",
            new.amounts.subtotal.to_display_string(),
            new.amounts.tax.to_display_string(),
            new.amounts
                .subtotal
                .checked_add(new.amounts.tax)
                .map_or_else(|| "an overflow".to_string(), Money::to_display_string),
            new.amounts.total.to_display_string()
        )));
    }

    let subtotal = sum(new.splits.iter().map(|s| s.subtotal), "split subtotals")?;
    let tax = sum(new.splits.iter().map(|s| s.tax), "split taxes")?;
    let amount = sum(new.splits.iter().map(|s| s.amount), "split amounts")?;
    check_equal("split subtotals", new.amounts.subtotal, subtotal)?;
    check_equal("split taxes", new.amounts.tax, tax)?;
    check_equal("split amounts", new.amounts.total, amount)?;

    for (index, split) in new.splits.iter().enumerate() {
        let n = index + 1;
        check_equal(
            &format!("split {n} (subtotal + tax)"),
            split.amount,
            split
                .subtotal
                .checked_add(split.tax)
                .ok_or_else(|| CoreError::Invalid(format!("split {n} overflowed")))?,
        )?;

        if split.line_items.is_empty() {
            continue;
        }
        let items = sum(
            split.line_items.iter().map(|li| li.amount),
            &format!("split {n} line items"),
        )?;
        check_equal(&format!("split {n} line items"), split.amount, items)?;

        for (item_index, item) in split.line_items.iter().enumerate() {
            let li = item_index + 1;
            check_equal(
                &format!("split {n} item {li} (subtotal + tax)"),
                item.amount,
                item.subtotal
                    .checked_add(item.tax)
                    .ok_or_else(|| CoreError::Invalid(format!("split {n} item {li} overflowed")))?,
            )?;
            if item.quantity_milli <= 0 {
                return Err(CoreError::Invalid(format!(
                    "split {n} item {li} has quantity {}; quantities are thousandths of a unit \
                     and must be positive",
                    item.quantity_milli
                )));
            }
            if item.description.trim().is_empty() {
                return Err(CoreError::Invalid(format!(
                    "split {n} item {li} has no description"
                )));
            }
        }
    }

    if let Some(ppm) = new.tax_rate_ppm
        && ppm > 1_000_000
    {
        return Err(CoreError::Invalid(format!(
            "tax rate {ppm} ppm is above 100% (1000000 ppm)"
        )));
    }

    Ok(())
}

fn sum(mut values: impl Iterator<Item = Money>, what: &str) -> Result<Money> {
    values.try_fold(Money::ZERO, |acc, m| {
        acc.checked_add(m)
            .ok_or_else(|| CoreError::Invalid(format!("{what} overflowed")))
    })
}

fn check_equal(what: &str, expected: Money, actual: Money) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    Err(CoreError::Invalid(format!(
        "{what} come to {}, but {} was expected (off by {})",
        actual.to_display_string(),
        expected.to_display_string(),
        actual.saturating_sub(expected).to_display_string()
    )))
}

fn insert_header(tx: &Transaction<'_>, new: &NewTransaction) -> Result<TxnId> {
    tx.execute(
        "INSERT INTO txn (uuid, txn_date, merchant_id, location_id,
                          subtotal_cents, tax_cents, total_cents,
                          tax_is_estimated, tax_rate_ppm, detail_level,
                          needs_review, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            new_uuid(),
            new.txn_date,
            new.merchant_id,
            new.location_id,
            new.amounts.subtotal,
            new.amounts.tax,
            new.amounts.total,
            new.amounts.tax_is_estimated,
            new.tax_rate_ppm,
            new.detail_level,
            new.needs_review,
            new.notes,
        ],
    )?;
    Ok(TxnId::new(tx.last_insert_rowid()))
}

/// `uuid` and `created_at` are deliberately absent from the SET list: the
/// aggregate is being replaced, but it is still the same receipt, and its
/// cross-device identity and creation time must survive an edit.
fn update_header(tx: &Transaction<'_>, id: TxnId, new: &NewTransaction) -> Result<()> {
    let changed = tx.execute(
        "UPDATE txn
            SET txn_date         = ?2,
                merchant_id      = ?3,
                location_id      = ?4,
                subtotal_cents   = ?5,
                tax_cents        = ?6,
                total_cents      = ?7,
                tax_is_estimated = ?8,
                tax_rate_ppm     = ?9,
                detail_level     = ?10,
                needs_review     = ?11,
                notes            = ?12,
                updated_at       = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![
            id,
            new.txn_date,
            new.merchant_id,
            new.location_id,
            new.amounts.subtotal,
            new.amounts.tax,
            new.amounts.total,
            new.amounts.tax_is_estimated,
            new.tax_rate_ppm,
            new.detail_level,
            new.needs_review,
            new.notes,
        ],
    )?;
    if changed == 0 {
        return Err(CoreError::NotFound(format!("transaction {id}")));
    }
    Ok(())
}

fn insert_split(
    tx: &Transaction<'_>,
    txn_id: TxnId,
    index: usize,
    split: &SplitRecord,
) -> Result<()> {
    let uuid = split.uuid.clone().unwrap_or_else(new_uuid);
    let sort_order = if split.sort_order == 0 {
        sparse(index)
    } else {
        split.sort_order
    };

    tx.execute(
        "INSERT INTO txn_split (uuid, txn_id, category_id, subtotal_cents,
                                tax_cents, amount_cents, sort_order, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            uuid,
            txn_id,
            split.category_id,
            split.subtotal,
            split.tax,
            split.amount,
            sort_order,
            split.notes,
        ],
    )?;
    let split_id = SplitId::new(tx.last_insert_rowid());

    for (item_index, item) in split.line_items.iter().enumerate() {
        let item_uuid = item.uuid.clone().unwrap_or_else(new_uuid);
        let item_sort = if item.sort_order == 0 {
            sparse(item_index)
        } else {
            item.sort_order
        };
        tx.execute(
            "INSERT INTO line_item (uuid, split_id, description, quantity_milli,
                                    unit_price_cents, subtotal_cents, tax_cents,
                                    amount_cents, category_id, is_taxable,
                                    sort_order, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                item_uuid,
                split_id,
                item.description.trim(),
                item.quantity_milli,
                item.unit_price,
                item.subtotal,
                item.tax,
                item.amount,
                item.category_id,
                item.is_taxable,
                item_sort,
                item.notes,
            ],
        )?;
    }
    Ok(())
}

/// One transaction, splits and line items included.
pub fn get(db: &Db, id: TxnId) -> Result<Txn> {
    let sql = format!("{SELECT_TXN} WHERE t.id = ?1");
    let mut stmt = db.conn().prepare(&sql)?;
    let mut txns = stmt
        .query_map(params![id], map_txn)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    if txns.is_empty() {
        return Err(CoreError::NotFound(format!("transaction {id}")));
    }
    hydrate(db, &mut txns)?;
    txns.pop()
        .ok_or_else(|| CoreError::NotFound(format!("transaction {id}")))
}

/// Matching transactions, newest first, splits and line items included.
///
/// Ties on `txn_date` break by id descending, so "the most recent" is
/// well-defined for the several receipts a day a real user enters.
pub fn list(db: &Db, filter: &TxnFilter) -> Result<Vec<Txn>> {
    let (where_sql, mut values) = filter.where_clause();
    let mut sql = format!("{SELECT_TXN} WHERE {where_sql} ORDER BY t.txn_date DESC, t.id DESC");
    if let Some(limit) = filter.limit {
        values.push(Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
        sql.push_str(&format!(" LIMIT ?{}", values.len()));
    }

    let mut stmt = db.conn().prepare(&sql)?;
    let mut txns = stmt
        .query_map(params_from_iter(values.iter()), map_txn)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    hydrate(db, &mut txns)?;
    Ok(txns)
}

/// How many transactions [`list`] would return — **including** the effect of
/// `limit`, so the two never disagree about the size of a page.
pub fn count(db: &Db, filter: &TxnFilter) -> Result<usize> {
    let (where_sql, mut values) = filter.where_clause();
    let mut inner = format!("SELECT t.id FROM txn t WHERE {where_sql}");
    if let Some(limit) = filter.limit {
        values.push(Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
        inner.push_str(&format!(" LIMIT ?{}", values.len()));
    }

    let n: i64 = db.conn().query_row(
        &format!("SELECT count(*) FROM ({inner})"),
        params_from_iter(values.iter()),
        |r| r.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Soft-delete a transaction. Idempotent, and the original archive time
/// survives a second call — "when was this deleted" must not drift every time
/// someone re-runs a script.
pub fn archive(db: &Db, id: TxnId) -> Result<()> {
    db.conn().execute(
        "UPDATE txn
            SET archived_at = COALESCE(archived_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at  = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// The merchant's most recent live transaction — the basis of "repeat last
/// receipt".
///
/// A query against `ix_txn_merchant_date`, not a `merchant.last_txn_id`
/// column: SQLite cannot `ALTER TABLE ADD CONSTRAINT`, so a merchant → txn →
/// merchant FK cycle would be a table-creation-order problem forever, and a
/// cached pointer can go stale where an index cannot.
pub fn last_for_merchant(db: &Db, merchant_id: MerchantId) -> Result<Option<Txn>> {
    let filter = TxnFilter {
        merchant_id: Some(merchant_id),
        limit: Some(1),
        ..TxnFilter::default()
    };
    Ok(list(db, &filter)?.into_iter().next())
}

/// Live spending per category per month, over an inclusive month range.
///
/// The actuals every budget and every report are built from. Three properties
/// are load-bearing:
///
/// * **Aggregated in SQL.** One `GROUP BY` instead of loading a year of
///   receipts and summing them in Rust. A year is thousands of splits and the
///   answer is a few dozen numbers.
/// * **Cash-flow dated, always.** A split is counted in the month its
///   *transaction* falls in, in every reporting basis. Amortization is a
///   presentation transform applied on top of this by
///   [`crate::service::reports`]; nothing rewrites when the money moved.
/// * **Archived transactions are excluded.** `txn_split` has no archived flag
///   of its own — a split has no life apart from its transaction — so the test
///   is on `txn.archived_at`.
///
/// `only` narrows to a single category; `None` returns every category that has
/// any spending in the range. The date bounds are bound as full `YYYY-MM-DD`
/// strings rather than compared with `substr`, so the query still uses
/// `ix_txn_date` instead of scanning the table.
pub fn spend_by_category_month(
    db: &Db,
    range: MonthRange,
    only: Option<CategoryId>,
) -> Result<BTreeMap<CategoryId, BTreeMap<YearMonth, Money>>> {
    if range.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut stmt = db.conn().prepare(
        "SELECT s.category_id, substr(t.txn_date, 1, 7), SUM(s.amount_cents)
           FROM txn_split s
           JOIN txn t ON t.id = s.txn_id
          WHERE t.archived_at IS NULL
            AND t.txn_date >= ?1 AND t.txn_date <= ?2
            AND (?3 IS NULL OR s.category_id = ?3)
          GROUP BY s.category_id, substr(t.txn_date, 1, 7)",
    )?;
    let rows = stmt
        .query_map(
            params![
                date_value(range.start.first_day()),
                date_value(range.end.last_day()),
                only
            ],
            |r| {
                Ok((
                    r.get::<_, CategoryId>(0)?,
                    r.get::<_, YearMonth>(1)?,
                    r.get::<_, Money>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out: BTreeMap<CategoryId, BTreeMap<YearMonth, Money>> = BTreeMap::new();
    for (category_id, ym, amount) in rows {
        out.entry(category_id).or_default().insert(ym, amount);
    }
    Ok(out)
}

/// Attach splits and line items to already-loaded transactions.
///
/// **Three queries total, whatever the row count**: one for the transactions,
/// one for all of their splits, one for all of those splits' line items,
/// joined in memory. The obvious alternative — a query per transaction — makes
/// `pacc list` over a year of receipts issue a thousand round trips and fall
/// off a cliff exactly when the data becomes worth looking at.
fn hydrate(db: &Db, txns: &mut [Txn]) -> Result<()> {
    if txns.is_empty() {
        return Ok(());
    }

    let txn_ids: Vec<Value> = txns.iter().map(|t| Value::Integer(t.id.get())).collect();
    let sql = format!(
        "{SELECT_SPLIT} WHERE s.txn_id IN ({}) ORDER BY s.txn_id, s.sort_order, s.id",
        placeholders(txn_ids.len())
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let split_rows = stmt
        .query_map(params_from_iter(txn_ids.iter()), map_split)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    // Insertion order per key is the ORDER BY order, so splits stay sorted.
    let mut by_txn: HashMap<i64, Vec<LoadedSplit>> = HashMap::new();
    let mut split_ids: Vec<Value> = Vec::with_capacity(split_rows.len());
    for (txn_id, split) in split_rows {
        split_ids.push(Value::Integer(split.id.get()));
        by_txn.entry(txn_id.get()).or_default().push(split);
    }

    if !split_ids.is_empty() {
        let sql = format!(
            "{SELECT_ITEM} WHERE li.split_id IN ({}) ORDER BY li.split_id, li.sort_order, li.id",
            placeholders(split_ids.len())
        );
        let mut stmt = db.conn().prepare(&sql)?;
        let item_rows = stmt
            .query_map(params_from_iter(split_ids.iter()), map_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut by_split: HashMap<i64, Vec<LoadedLineItem>> = HashMap::new();
        for (split_id, item) in item_rows {
            by_split.entry(split_id.get()).or_default().push(item);
        }
        for split in by_txn.values_mut().flatten() {
            if let Some(items) = by_split.remove(&split.id.get()) {
                split.line_items = items;
            }
        }
    }

    for txn in txns.iter_mut() {
        txn.splits = by_txn.remove(&txn.id.get()).unwrap_or_default();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;
    use crate::repo::{categories, locations, merchants};

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().expect("valid date")
    }

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    fn split(category: CategoryId, subtotal: i64, tax: i64) -> SplitRecord {
        SplitRecord {
            id: None,
            uuid: None,
            category_id: category,
            subtotal: m(subtotal),
            tax: m(tax),
            amount: m(subtotal + tax),
            sort_order: 0,
            notes: None,
            line_items: Vec::new(),
        }
    }

    fn item(description: &str, subtotal: i64, tax: i64) -> LineItemRecord {
        LineItemRecord {
            id: None,
            uuid: None,
            description: description.to_string(),
            quantity_milli: 1_000,
            unit_price: Some(m(subtotal)),
            subtotal: m(subtotal),
            tax: m(tax),
            amount: m(subtotal + tax),
            category_id: None,
            is_taxable: true,
            sort_order: 0,
            notes: None,
        }
    }

    fn draft(splits: Vec<SplitRecord>) -> NewTransaction {
        let subtotal: Money = splits.iter().map(|s| s.subtotal).sum();
        let tax: Money = splits.iter().map(|s| s.tax).sum();
        NewTransaction {
            id: None,
            txn_date: date("2026-08-12"),
            merchant_id: None,
            location_id: None,
            amounts: Amounts {
                subtotal,
                tax,
                total: subtotal + tax,
                tax_is_estimated: false,
            },
            tax_rate_ppm: None,
            detail_level: DetailLevel::Grouped,
            needs_review: false,
            notes: None,
            splits,
        }
    }

    #[test]
    fn detail_level_round_trips_through_text_and_sql() {
        assert_eq!(DetailLevel::default(), DetailLevel::Grouped);
        assert_eq!(DetailLevel::Grouped.to_string(), "grouped");
        assert_eq!(DetailLevel::Itemized.to_string(), "itemized");
        assert_eq!(
            "ITEMIZED".parse::<DetailLevel>().unwrap(),
            DetailLevel::Itemized
        );
        assert_eq!(
            " grouped ".parse::<DetailLevel>().unwrap(),
            DetailLevel::Grouped
        );
        assert!(matches!(
            "loose".parse::<DetailLevel>(),
            Err(CoreError::Invalid(_))
        ));
    }

    #[test]
    fn a_saved_transaction_loads_back_identically() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let household = categories::resolve(&db, "Household").unwrap();
        let store = merchants::get_or_create(&db, "H-E-B").unwrap();
        let home = locations::home(&db).unwrap().unwrap();

        let mut new = draft(vec![
            split(groceries.id, 3000, 0),
            split(household.id, 2000, 419),
        ]);
        new.merchant_id = Some(store.id);
        new.location_id = Some(home.id);
        new.tax_rate_ppm = Some(83_750);
        new.notes = Some("weekly shop".to_string());

        let id = save(&db, &new).unwrap();
        let got = get(&db, id).unwrap();

        assert_eq!(got.txn_date, date("2026-08-12"));
        assert_eq!(got.merchant_name.as_deref(), Some("H-E-B"));
        assert_eq!(got.location_name.as_deref(), Some("Home"));
        assert_eq!(got.total, m(5419));
        assert_eq!(got.tax, m(419));
        assert!(!got.tax_is_estimated);
        assert_eq!(got.tax_rate_ppm, Some(83_750));
        assert_eq!(got.notes.as_deref(), Some("weekly shop"));
        assert!(!got.archived);

        assert_eq!(got.splits.len(), 2);
        assert_eq!(got.splits[0].category_name, "Groceries");
        assert_eq!(got.splits[0].group_name, "Frequent");
        // Sparse ordering, assigned from position.
        assert_eq!(got.splits[0].sort_order, 100);
        assert_eq!(got.splits[1].sort_order, 200);
        assert_eq!(
            got.splits.iter().map(|s| s.amount).sum::<Money>(),
            got.total
        );
    }

    #[test]
    fn saving_over_a_transaction_replaces_its_children_wholesale() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let fuel = categories::resolve(&db, "Gas").unwrap();

        let id = save(&db, &draft(vec![split(groceries.id, 5000, 0)])).unwrap();
        let first = get(&db, id).unwrap();

        let mut edited = draft(vec![split(fuel.id, 2500, 0), split(groceries.id, 2500, 0)]);
        edited.id = Some(id);
        assert_eq!(save(&db, &edited).unwrap(), id);

        let after = get(&db, id).unwrap();
        assert_eq!(after.uuid, first.uuid, "identity survives an edit");
        assert_eq!(after.splits.len(), 2);
        assert_eq!(after.splits[0].category_name, "Gas");
        // The old split row is gone rather than orphaned.
        let rows: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM txn_split", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
        assert!(crate::db::check_all(&db).unwrap().is_empty());
    }

    #[test]
    fn line_items_are_stored_and_cascade_with_their_split() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let mut s = split(groceries.id, 1000, 0);
        s.line_items = vec![item("Milk", 349, 0), item("Bread", 651, 0)];
        let mut new = draft(vec![s]);
        new.detail_level = DetailLevel::Itemized;

        let id = save(&db, &new).unwrap();
        let got = get(&db, id).unwrap();
        assert_eq!(got.detail_level, DetailLevel::Itemized);
        assert_eq!(got.splits[0].line_items.len(), 2);
        assert_eq!(got.splits[0].line_items[0].description, "Milk");
        assert_eq!(got.splits[0].line_items[0].sort_order, 100);
        assert_eq!(got.splits[0].line_items[1].sort_order, 200);

        // Replacing the aggregate drops the items with their split.
        let mut edited = draft(vec![split(groceries.id, 1000, 0)]);
        edited.id = Some(id);
        save(&db, &edited).unwrap();
        let items: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM line_item", [], |r| r.get(0))
            .unwrap();
        assert_eq!(items, 0);
    }

    #[test]
    fn a_split_that_moves_money_between_subtotal_and_tax_is_refused() {
        // The blind spot a total-only check would have: the amounts still sum
        // to the total, but per-category tax would be wrong in the export.
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let mut new = draft(vec![split(groceries.id, 5000, 419)]);
        new.splits[0].subtotal = m(5419);
        new.splits[0].tax = Money::ZERO;

        let err = save(&db, &new).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("split subtotals"), "{msg}");
        assert!(msg.contains("$54.19") && msg.contains("$50.00"), "{msg}");
        assert!(matches!(err, CoreError::Invalid(_)));
        // Nothing was written.
        assert_eq!(count(&db, &TxnFilter::default()).unwrap(), 0);
    }

    #[test]
    fn the_other_refusals_all_name_their_numbers() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();

        // No splits at all.
        let mut empty = draft(vec![]);
        empty.amounts = Amounts {
            subtotal: m(100),
            tax: Money::ZERO,
            total: m(100),
            tax_is_estimated: false,
        };
        assert!(save(&db, &empty).unwrap_err().to_string().contains("$1.00"));

        // Splits that do not add up to the total.
        let mut short = draft(vec![split(groceries.id, 1000, 0)]);
        short.amounts.total = m(2000);
        short.amounts.subtotal = m(2000);
        let msg = save(&db, &short).unwrap_err().to_string();
        assert!(msg.contains("$10.00") && msg.contains("$20.00"), "{msg}");

        // Two splits whose errors cancel in every total but which do not
        // balance row by row. The schema CHECK would refuse this too; the
        // point of catching it here is that "split 1" beats "CHECK constraint
        // failed" by a mile.
        let mut bad = draft(vec![
            split(groceries.id, 600, 0),
            split(groceries.id, 400, 0),
        ]);
        bad.splits[0].amount = m(700);
        bad.splits[1].amount = m(300);
        let msg = save(&db, &bad).unwrap_err().to_string();
        assert!(msg.contains("split 1") && msg.contains("$7.00"), "{msg}");

        // Line items that do not add up to their split.
        let mut items = draft(vec![split(groceries.id, 1000, 0)]);
        items.splits[0].line_items = vec![item("Milk", 349, 0)];
        assert!(
            save(&db, &items)
                .unwrap_err()
                .to_string()
                .contains("line items")
        );

        // Nonsense line items.
        let mut zero_qty = draft(vec![split(groceries.id, 1000, 0)]);
        zero_qty.splits[0].line_items = vec![item("Milk", 1000, 0)];
        zero_qty.splits[0].line_items[0].quantity_milli = 0;
        assert!(
            save(&db, &zero_qty)
                .unwrap_err()
                .to_string()
                .contains("quantity")
        );

        let mut blank = draft(vec![split(groceries.id, 1000, 0)]);
        blank.splits[0].line_items = vec![item("   ", 1000, 0)];
        assert!(
            save(&db, &blank)
                .unwrap_err()
                .to_string()
                .contains("description")
        );

        // An impossible rate.
        let mut rate = draft(vec![split(groceries.id, 1000, 0)]);
        rate.tax_rate_ppm = Some(1_000_001);
        assert!(save(&db, &rate).unwrap_err().to_string().contains("100%"));

        assert_eq!(count(&db, &TxnFilter::default()).unwrap(), 0);
    }

    #[test]
    fn updating_a_transaction_that_does_not_exist_is_not_found() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let mut new = draft(vec![split(groceries.id, 1000, 0)]);
        new.id = Some(TxnId::new(999));
        assert!(matches!(save(&db, &new), Err(CoreError::NotFound(_))));
        assert!(matches!(
            get(&db, TxnId::new(999)),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn list_filters_and_orders_newest_first() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let fuel = categories::resolve(&db, "Gas").unwrap();
        let heb = merchants::get_or_create(&db, "H-E-B").unwrap();

        let mut a = draft(vec![split(groceries.id, 1000, 0)]);
        a.txn_date = date("2026-08-01");
        a.merchant_id = Some(heb.id);
        let a_id = save(&db, &a).unwrap();

        let mut b = draft(vec![split(fuel.id, 4000, 0)]);
        b.txn_date = date("2026-08-20");
        b.needs_review = true;
        let b_id = save(&db, &b).unwrap();

        let ids = |f: &TxnFilter| -> Vec<TxnId> {
            list(&db, f).unwrap().into_iter().map(|t| t.id).collect()
        };

        assert_eq!(ids(&TxnFilter::default()), vec![b_id, a_id]);
        assert_eq!(
            ids(&TxnFilter {
                from: Some(date("2026-08-10")),
                ..TxnFilter::default()
            }),
            vec![b_id]
        );
        assert_eq!(
            ids(&TxnFilter {
                to: Some(date("2026-08-01")),
                ..TxnFilter::default()
            }),
            vec![a_id],
            "`to` is inclusive"
        );
        assert_eq!(
            ids(&TxnFilter {
                merchant_id: Some(heb.id),
                ..TxnFilter::default()
            }),
            vec![a_id]
        );
        assert_eq!(
            ids(&TxnFilter {
                category_id: Some(fuel.id),
                ..TxnFilter::default()
            }),
            vec![b_id]
        );
        assert_eq!(ids(&TxnFilter::needs_review(None)), vec![b_id]);
        assert_eq!(
            ids(&TxnFilter {
                limit: Some(1),
                ..TxnFilter::default()
            }),
            vec![b_id]
        );
        assert_eq!(count(&db, &TxnFilter::default()).unwrap(), 2);
    }

    #[test]
    fn a_category_matched_by_two_splits_yields_one_transaction() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let id = save(
            &db,
            &draft(vec![
                split(groceries.id, 1000, 0),
                split(groceries.id, 2000, 0),
            ]),
        )
        .unwrap();

        let found = list(
            &db,
            &TxnFilter {
                category_id: Some(groceries.id),
                ..TxnFilter::default()
            },
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, id);
        assert_eq!(found[0].splits.len(), 2);
    }

    #[test]
    fn archiving_hides_a_transaction_without_deleting_it() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let id = save(&db, &draft(vec![split(groceries.id, 1000, 0)])).unwrap();

        archive(&db, id).unwrap();
        let archived_at: String = db
            .conn()
            .query_row(
                "SELECT archived_at FROM txn WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        // Idempotent, and the first archive time survives.
        archive(&db, id).unwrap();
        let again: String = db
            .conn()
            .query_row(
                "SELECT archived_at FROM txn WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(archived_at, again);

        assert!(list(&db, &TxnFilter::default()).unwrap().is_empty());
        assert_eq!(
            list(
                &db,
                &TxnFilter {
                    include_archived: true,
                    ..TxnFilter::default()
                }
            )
            .unwrap()
            .len(),
            1
        );
        assert!(get(&db, id).is_ok());
        // Archiving something that is not there is not an error worth raising.
        archive(&db, TxnId::new(999)).unwrap();
    }

    #[test]
    fn last_for_merchant_is_the_newest_live_one() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let heb = merchants::get_or_create(&db, "H-E-B").unwrap();
        assert_eq!(last_for_merchant(&db, heb.id).unwrap(), None);

        let mut older = draft(vec![split(groceries.id, 1000, 0)]);
        older.txn_date = date("2026-07-01");
        older.merchant_id = Some(heb.id);
        save(&db, &older).unwrap();

        let mut newer = draft(vec![split(groceries.id, 2000, 0)]);
        newer.txn_date = date("2026-08-01");
        newer.merchant_id = Some(heb.id);
        let newer_id = save(&db, &newer).unwrap();

        assert_eq!(
            last_for_merchant(&db, heb.id).unwrap().unwrap().id,
            newer_id
        );

        archive(&db, newer_id).unwrap();
        assert_eq!(
            last_for_merchant(&db, heb.id).unwrap().unwrap().total,
            m(1000),
            "an archived receipt is not the one to repeat"
        );
    }

    #[test]
    fn many_transactions_load_their_children_in_one_pass() {
        // `hydrate` fetches every split and every line item of the whole page
        // in one query each, then joins in memory — the N+1 alternative makes
        // `pacc list` over a year of receipts issue a thousand round trips.
        // What is asserted here is that the in-memory join lands each child on
        // the right parent, which is the part a bulk fetch can get wrong.
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let fuel = categories::resolve(&db, "Gas").unwrap();

        for day in 1..=10 {
            let mut s = split(groceries.id, 1000 * i64::from(day), 0);
            s.line_items = vec![item(&format!("day {day}"), 1000 * i64::from(day), 0)];
            let mut new = draft(vec![s, split(fuel.id, 500, 0)]);
            new.detail_level = DetailLevel::Itemized;
            new.txn_date = date(&format!("2026-08-{day:02}"));
            save(&db, &new).unwrap();
        }

        let all = list(&db, &TxnFilter::default()).unwrap();
        assert_eq!(all.len(), 10);
        for txn in &all {
            assert_eq!(txn.splits.len(), 2);
            let day = txn.txn_date.format("%-d").to_string();
            assert_eq!(txn.splits[0].line_items.len(), 1);
            assert_eq!(
                txn.splits[0].line_items[0].description,
                format!("day {day}")
            );
            assert_eq!(txn.splits[0].line_items[0].amount, txn.splits[0].amount);
            // The second split has no items, and must not inherit any.
            assert!(txn.splits[1].line_items.is_empty());
            assert_eq!(
                txn.splits.iter().map(|s| s.amount).sum::<Money>(),
                txn.total
            );
        }
    }

    #[test]
    fn spend_groups_by_category_and_month_and_ignores_archived() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let fuel = categories::resolve(&db, "Gas").unwrap();

        let save_on = |date_str: &str, splits: Vec<SplitRecord>| {
            let mut new = draft(splits);
            new.txn_date = date(date_str);
            save(&db, &new).unwrap()
        };
        save_on("2026-03-01", vec![split(groceries.id, 1000, 0)]);
        save_on("2026-03-28", vec![split(groceries.id, 2000, 0)]);
        save_on("2026-04-02", vec![split(fuel.id, 4000, 0)]);
        let archived = save_on("2026-03-15", vec![split(groceries.id, 9999, 0)]);
        archive(&db, archived).unwrap();

        let year = MonthRange::new(
            YearMonth::new(2026, 1).unwrap(),
            YearMonth::new(2026, 12).unwrap(),
        );
        let all = spend_by_category_month(&db, year, None).unwrap();
        assert_eq!(
            all[&groceries.id][&YearMonth::new(2026, 3).unwrap()],
            m(3000)
        );
        assert_eq!(all[&fuel.id][&YearMonth::new(2026, 4).unwrap()], m(4000));
        assert_eq!(all[&groceries.id].len(), 1, "one month, two receipts");

        // The month boundaries are inclusive of the whole first and last day.
        let march = MonthRange::single(YearMonth::new(2026, 3).unwrap());
        let only_march = spend_by_category_month(&db, march, None).unwrap();
        assert_eq!(only_march.len(), 1);
        assert_eq!(
            only_march[&groceries.id][&YearMonth::new(2026, 3).unwrap()],
            m(3000)
        );

        let one = spend_by_category_month(&db, year, Some(fuel.id)).unwrap();
        assert_eq!(one.len(), 1);
        assert!(one.contains_key(&fuel.id));

        let backwards = MonthRange::new(
            YearMonth::new(2026, 12).unwrap(),
            YearMonth::new(2026, 1).unwrap(),
        );
        assert!(
            spend_by_category_month(&db, backwards, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_reused_split_uuid_survives_the_replacement() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let id = save(&db, &draft(vec![split(groceries.id, 1000, 0)])).unwrap();
        let loaded = get(&db, id).unwrap();
        let uuid = loaded.splits[0].uuid.clone();

        let mut edited = draft(vec![split(groceries.id, 2000, 0)]);
        edited.id = Some(id);
        edited.splits[0].uuid = Some(uuid.clone());
        save(&db, &edited).unwrap();

        assert_eq!(get(&db, id).unwrap().splits[0].uuid, uuid);
    }

    #[test]
    fn explicit_sort_orders_are_honoured() {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").unwrap();
        let fuel = categories::resolve(&db, "Gas").unwrap();
        let mut new = draft(vec![split(groceries.id, 1000, 0), split(fuel.id, 1000, 0)]);
        new.splits[0].sort_order = 500;
        new.splits[1].sort_order = 250;

        let id = save(&db, &new).unwrap();
        let got = get(&db, id).unwrap();
        assert_eq!(got.splits[0].category_name, "Gas");
        assert_eq!(got.splits[0].sort_order, 250);
        assert_eq!(got.splits[1].sort_order, 500);
    }
}
