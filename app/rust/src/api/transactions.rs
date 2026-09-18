//! The expense list, its filter menus, and deleting a row.

use chrono::NaiveDate;

use accounting_core::repo::transactions::{self, TxnFilter, TxnSort};
use accounting_core::{CategoryId, MerchantId, TxnId};

use crate::api::db;
use crate::error::to_message;

/// One row of the expense list.
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

/// Everything a filter says, minus the page it asks for.
///
/// Built once and shared by [`list_transactions`] and [`transaction_totals`],
/// so the row of totals under the list can never describe a different set of
/// expenses from the list itself. Two copies of this would drift the first time
/// a filter was added to one of them.
fn build_filter(
    from: Option<String>,
    to: Option<String>,
    category_ids: Option<Vec<i64>>,
    merchant_ids: Option<Vec<i64>>,
    include_no_merchant: Option<bool>,
    sort: Option<String>,
) -> Result<TxnFilter, String> {
    Ok(TxnFilter {
        from: parse_bound(from)?,
        to: parse_bound(to)?,
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
    })
}

/// One page of the expense list.
///
/// `None` for either id list means "every one of them", which is not the same
/// as a list naming every one: an expense at a shop that did not exist when
/// the menu was built would fail the explicit list. Untouched has to mean
/// untouched.
///
/// The filtering, the ordering and the paging are all done in SQL. A limit
/// applied before a sort would order whichever fifty happened to come back,
/// which is a different page from the first fifty of the ordered set — and an
/// offset into that would be a page of nothing in particular.
pub async fn list_transactions(
    limit: u32,
    offset: Option<u32>,
    from: Option<String>,
    to: Option<String>,
    category_ids: Option<Vec<i64>>,
    merchant_ids: Option<Vec<i64>>,
    include_no_merchant: Option<bool>,
    sort: Option<String>,
) -> Result<Vec<TxnSummaryDto>, String> {
    let filter = TxnFilter {
        limit: Some(limit as usize),
        // Absent and zero are the same page, so the first page need not say so.
        offset: offset.map(|n| n as usize),
        ..build_filter(from, to, category_ids, merchant_ids, include_no_merchant, sort)?
    };

    let db = db::locked()?;
    let rows = transactions::list(&db, &filter).map_err(to_message)?;
    Ok(rows.iter().map(summarise).collect())
}

/// The expense list's bottom line: every money column, added up.
///
/// Deliberately takes no `limit` or `offset`. The number worth printing is
/// "everything matching this filter", and it must not change as the reader
/// pages — a per-page total would move under someone who only pressed Next,
/// which reads as an arithmetic bug rather than as a feature.
///
/// Summed by the core in SQL, from the same filter the list is built from, so
/// the total covers rows no page ever loaded. Never in Dart: money there is a
/// type that makes arithmetic a compile error, on purpose.
pub struct TxnTotalsDto {
    pub subtotal: i64,
    pub tax: i64,
    pub total: i64,
    /// Expenses in the sum — the size of the whole filtered set, which is also
    /// what the pager counts through.
    ///
    /// `u32` rather than `usize`: the bridge renders a `usize` as a Dart
    /// `BigInt`, which is a heavyweight number for a row count and a nuisance
    /// at every call site.
    pub expenses: u32,
}

/// Add up every expense the filter matches, however the list is paged.
pub async fn transaction_totals(
    from: Option<String>,
    to: Option<String>,
    category_ids: Option<Vec<i64>>,
    merchant_ids: Option<Vec<i64>>,
    include_no_merchant: Option<bool>,
) -> Result<TxnTotalsDto, String> {
    // Sort left at its default: the order of rows cannot change what they add
    // up to, and asking the screen for one it does not need would be a
    // parameter that could only ever be wrong.
    let filter = build_filter(from, to, category_ids, merchant_ids, include_no_merchant, None)?;

    let db = db::locked()?;
    let totals = transactions::totals(&db, &filter).map_err(to_message)?;
    Ok(TxnTotalsDto {
        subtotal: totals.subtotal.cents(),
        tax: totals.tax.cents(),
        total: totals.total.cents(),
        expenses: u32::try_from(totals.rows).unwrap_or(u32::MAX),
    })
}

/// The values the filter menus are built from.
pub async fn filter_values() -> Result<FilterValuesDto, String> {
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
pub async fn delete_transaction(id: i64) -> Result<(), String> {
    let db = db::locked()?;
    transactions::delete(&db, TxnId::new(id)).map_err(to_message)
}

pub(crate) fn summarise(txn: &transactions::Txn) -> TxnSummaryDto {
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

/// One inclusive date bound, or `None` for no bound at all.
///
/// Refused rather than dropped when it is not a date: a range whose lower
/// bound quietly vanished would answer a wider question than the one the
/// screen is asking, and a list of the right shape is the kind of wrong nobody
/// reports.
fn parse_bound(value: Option<String>) -> Result<Option<NaiveDate>, String> {
    value
        .map(|v| {
            v.parse::<NaiveDate>()
                .map_err(|e| format!("{v} is not a YYYY-MM-DD date: {e}"))
        })
        .transpose()
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
