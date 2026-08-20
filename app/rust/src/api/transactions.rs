//! The expense list, its filter menus, and deleting a row.

use accounting_core::repo::transactions::{self, TxnFilter, TxnSort};
use accounting_core::{CategoryId, MerchantId, TxnId};

use crate::api::db;
use crate::error::to_message;

/// One row of the expense list. Mirrors the Tauri shell's `TxnSummaryDto`.
pub struct TxnSummaryDto {
    pub id: i64,
    /// `YYYY-MM-DD`, as the core formats it. Not reparsed on the way through:
    /// turning it into a `DateTime` and back is how a UTC shift gets
    /// reintroduced on a screen that never needed a timezone.
    pub txn_date: String,
    /// `None` for a quick-added expense, which has no store.
    pub merchant_name: Option<String>,
    pub total: i64,
    /// True only when the tax was computed rather than read off the receipt.
    pub tax_is_estimated: bool,
    pub needs_review: bool,
    /// One entry per split, in split order. Duplicates are not removed.
    pub categories: Vec<String>,
}

/// One value a filterable column actually holds.
pub struct FilterValueDto {
    pub id: i64,
    pub name: String,
}

/// The stores and categories that actually appear on an expense.
///
/// The filter menus are built from this rather than from the full chart of
/// accounts: the seed installs two dozen suggested shops, and a menu mostly
/// made of rows that match nothing is one where ticking a box does nothing
/// visible. Deliberately not narrowed by the list's own filters — hiding
/// Costco must not remove Costco from the menu that is hiding it.
pub struct FilterValuesDto {
    pub merchants: Vec<FilterValueDto>,
    pub categories: Vec<FilterValueDto>,
    /// Whether any expense has no store — what quick-add leaves behind.
    pub any_without_merchant: bool,
}

/// The expense list.
///
/// `None` for either id list means "every one of them", which is not the same
/// as a list naming every one: an expense at a shop that did not exist when
/// the menu was built would fail the explicit list. Untouched has to mean
/// untouched.
///
/// The filtering, the ordering and the limit are all done in SQL. A limit
/// applied before a sort would order whichever two hundred happened to come
/// back, which is a different list from the first two hundred of the ordered
/// set.
pub fn list_transactions(
    limit: u32,
    category_ids: Option<Vec<i64>>,
    merchant_ids: Option<Vec<i64>>,
    include_no_merchant: Option<bool>,
    sort: Option<String>,
) -> Result<Vec<TxnSummaryDto>, String> {
    let filter = TxnFilter {
        limit: Some(limit as usize),
        category_ids: category_ids
            .unwrap_or_default()
            .into_iter()
            .map(CategoryId::new)
            .collect(),
        merchant_ids: merchant_ids
            .unwrap_or_default()
            .into_iter()
            .map(MerchantId::new)
            .collect(),
        include_no_merchant: include_no_merchant.unwrap_or(false),
        sort: parse_sort(sort.as_deref())?,
        ..TxnFilter::default()
    };

    let db = db::locked()?;
    let rows = transactions::list(&db, &filter).map_err(to_message)?;
    Ok(rows.iter().map(summarise).collect())
}

/// The values the filter menus are built from.
pub fn filter_values() -> Result<FilterValuesDto, String> {
    let db = db::locked()?;
    let values = transactions::filter_values(&db).map_err(to_message)?;
    let named = |v: transactions::FilterValue| FilterValueDto {
        id: v.id,
        name: v.name,
    };
    Ok(FilterValuesDto {
        merchants: values.merchants.into_iter().map(&named).collect(),
        categories: values.categories.into_iter().map(&named).collect(),
        any_without_merchant: values.any_without_merchant,
    })
}

/// Delete a receipt, with its splits and line items.
///
/// Irreversible, and deliberately not softened here: the confirmation belongs
/// on screen, next to the row the user is pointing at. Deleting something
/// already gone is an error rather than a silent success — two windows open on
/// one database is a normal thing, and "that receipt is no longer there" is
/// worth saying.
pub fn delete_transaction(id: i64) -> Result<(), String> {
    let db = db::locked()?;
    transactions::delete(&db, TxnId::new(id)).map_err(to_message)
}

fn summarise(txn: &transactions::Txn) -> TxnSummaryDto {
    TxnSummaryDto {
        id: txn.id.get(),
        txn_date: txn.txn_date.to_string(),
        merchant_name: txn.merchant_name.clone(),
        total: txn.total.cents(),
        tax_is_estimated: txn.tax_is_estimated,
        needs_review: txn.needs_review,
        categories: txn.splits.iter().map(|s| s.category_name.clone()).collect(),
    }
}

/// The sort names the UI sends, which are the enum's own variants in the
/// spelling JSON uses.
///
/// An unknown name is refused rather than quietly treated as the default: a
/// typo that silently reorders a list is a bug nobody reports, because the
/// list still looks plausible.
fn parse_sort(name: Option<&str>) -> Result<TxnSort, String> {
    match name {
        None | Some("dateDesc") => Ok(TxnSort::DateDesc),
        Some("dateAsc") => Ok(TxnSort::DateAsc),
        Some("totalDesc") => Ok(TxnSort::TotalDesc),
        Some("totalAsc") => Ok(TxnSort::TotalAsc),
        Some(other) => Err(format!("{other} is not a sort order")),
    }
}
