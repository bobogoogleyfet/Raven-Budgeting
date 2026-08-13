//! Flattening the transaction aggregate into rows a spreadsheet can hold.
//!
//! # Core returns bytes; the frontend owns the file
//!
//! [`xlsx::export_xlsx`] and [`csv::export_csv`] hand back a `Vec<u8>`. They
//! never open a path. On Android and iOS the application cannot choose where a
//! file lands — the user picks a destination through a system document picker
//! and the platform hands the app a stream — so a core that wrote files would
//! have to be rewritten for every platform after the first. The CLI is the one
//! layer that gets to call `std::fs::write`.
//!
//! # One row per split, not per transaction
//!
//! A receipt split across Groceries and Household is two rows. That is
//! the entire reason splits exist: a per-transaction export would force the
//! reader to pick one category for a $54 shop that was really $30 of food and
//! $24 of soap, and every category total downstream would be wrong.
//!
//! # The columns are defined once
//!
//! [`COLUMNS`] is the single definition, and both the header row and the cell
//! writes are driven from it. This is not tidiness. Excel silently *repairs*
//! (i.e. blanks) a workbook whose table column definitions do not byte-match
//! the header row cells, and a second, hand-maintained list of the same 15
//! names drifts from the first the day someone adds a sixteenth.

pub mod csv;
pub mod xlsx;

// Re-exported so a caller writes `export::export_xlsx(..)` without having to
// know which submodule it lives in. `self::` is load-bearing: a bare `csv`
// in a `use` path resolves to the *crate* of that name, not this module.
pub use self::csv::{csv_bytes, export_csv};
pub use self::xlsx::{export_xlsx, workbook_bytes};

use std::borrow::Cow;

use chrono::{Datelike, NaiveDate};
use serde::Serialize;

use crate::db::Db;
use crate::error::Result;
use crate::ids::TxnId;
use crate::money::Money;
use crate::repo::transactions::{self, LoadedSplit, Txn, TxnFilter};
use crate::time::{YearMonth, month_short_name, quarter_of};

/// One exported row: one transaction split, flattened.
///
/// The calendar fields are carried as **values**, not as formulas over `date`.
/// A `=YEAR([@Date])` in a table column is re-evaluated by whatever opens the
/// file, cannot be filtered on until it has been, and comes back as `0` in a
/// reader that trusts a cached result — see [`xlsx`] for the same trap on the
/// summary sheet.
///
/// `Deserialize` is deliberately not derived: `month_name` is a `&'static str`
/// borrowed from [`month_short_name`], and this type is an output shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExportRow {
    pub txn_id: TxnId,
    pub date: NaiveDate,
    pub year: i32,
    pub quarter: u32,
    pub month: u32,
    pub month_name: &'static str,
    pub year_month: YearMonth,
    pub category: String,
    pub group: String,
    pub merchant: Option<String>,
    pub location: Option<String>,
    /// The **split's** subtotal, not the transaction's.
    pub subtotal: Money,
    /// The split's own share of the tax.
    pub tax: Money,
    /// The split's amount, i.e. `subtotal + tax`.
    pub total: Money,
    /// True when the tax was computed from a rate rather than read off the
    /// receipt. Exported because an estimate that looks like a fact is worse
    /// than no estimate at all.
    pub tax_estimated: bool,
    pub notes: Option<String>,
}

/// What kind of value a column holds, which is what decides the spreadsheet
/// number format and how the CSV renders it.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize)]
pub enum ColumnKind {
    /// A real date. In xlsx this is a numeric serial with a date format, never
    /// a string — a string date sorts lexically and silently breaks every
    /// month filter the export exists to support.
    Date,
    /// A whole number: year, quarter, month.
    Int,
    /// Free text.
    Text,
    /// An amount. `i64` cents everywhere in this crate; a double only at the
    /// moment it enters a cell.
    Money,
    /// A flag, rendered `true`/`false`.
    Bool,
}

/// A column of the export, defined once.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Column {
    pub header: &'static str,
    pub kind: ColumnKind,
}

/// Every column of the export, in order.
///
/// Adding one here is a compile error until [`ExportRow::cells`] grows a
/// matching entry, because that function returns `[CellValue; COLUMNS.len()]`.
/// That is the mechanism that keeps the header row and the data in step.
pub const COLUMNS: [Column; 15] = [
    Column {
        header: "Date",
        kind: ColumnKind::Date,
    },
    Column {
        header: "Year",
        kind: ColumnKind::Int,
    },
    Column {
        header: "Quarter",
        kind: ColumnKind::Int,
    },
    Column {
        header: "Month",
        kind: ColumnKind::Int,
    },
    Column {
        header: "MonthName",
        kind: ColumnKind::Text,
    },
    Column {
        header: "YearMonth",
        kind: ColumnKind::Text,
    },
    Column {
        header: "Category",
        kind: ColumnKind::Text,
    },
    Column {
        header: "Group",
        kind: ColumnKind::Text,
    },
    Column {
        header: "Merchant",
        kind: ColumnKind::Text,
    },
    Column {
        header: "Location",
        kind: ColumnKind::Text,
    },
    Column {
        header: "Subtotal",
        kind: ColumnKind::Money,
    },
    Column {
        header: "Tax",
        kind: ColumnKind::Money,
    },
    Column {
        header: "Total",
        kind: ColumnKind::Money,
    },
    Column {
        header: "TaxEstimated",
        kind: ColumnKind::Bool,
    },
    Column {
        header: "Notes",
        kind: ColumnKind::Text,
    },
];

/// One cell's value, in the shape its [`Column`] declares.
///
/// This is what lets a single `for` loop over [`COLUMNS`] write both formats:
/// the xlsx writer matches on it to pick `write_date_with_format` versus
/// `write_number_with_format`, and the CSV writer matches on it to pick
/// `to_major_string` over a currency-formatted double.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellValue<'a> {
    Date(NaiveDate),
    Int(i64),
    /// Borrowed where the row already owns the string, owned for the derived
    /// `YearMonth` label. `None` is an empty cell — distinct from `Some("")`
    /// only in intent, but the xlsx writer leaves the cell truly blank.
    Text(Option<Cow<'a, str>>),
    Money(Money),
    Bool(bool),
}

impl<'a> CellValue<'a> {
    fn text(s: &'a str) -> Self {
        CellValue::Text(Some(Cow::Borrowed(s)))
    }

    fn opt_text(s: Option<&'a str>) -> Self {
        CellValue::Text(s.map(Cow::Borrowed))
    }

    /// The column kind this value belongs in. Used to assert, in tests, that
    /// [`ExportRow::cells`] and [`COLUMNS`] have not drifted apart.
    #[must_use]
    pub fn kind(&self) -> ColumnKind {
        match self {
            CellValue::Date(_) => ColumnKind::Date,
            CellValue::Int(_) => ColumnKind::Int,
            CellValue::Text(_) => ColumnKind::Text,
            CellValue::Money(_) => ColumnKind::Money,
            CellValue::Bool(_) => ColumnKind::Bool,
        }
    }
}

impl ExportRow {
    /// This row's cells, in [`COLUMNS`] order.
    ///
    /// The return type is `[CellValue; COLUMNS.len()]` on purpose: adding a
    /// column to `COLUMNS` without adding its value here does not compile.
    #[must_use]
    pub fn cells(&self) -> [CellValue<'_>; COLUMNS.len()] {
        [
            CellValue::Date(self.date),
            CellValue::Int(i64::from(self.year)),
            CellValue::Int(i64::from(self.quarter)),
            CellValue::Int(i64::from(self.month)),
            CellValue::text(self.month_name),
            CellValue::Text(Some(Cow::Owned(self.year_month.to_string()))),
            CellValue::text(&self.category),
            CellValue::text(&self.group),
            CellValue::opt_text(self.merchant.as_deref()),
            CellValue::opt_text(self.location.as_deref()),
            CellValue::Money(self.subtotal),
            CellValue::Money(self.tax),
            CellValue::Money(self.total),
            CellValue::Bool(self.tax_estimated),
            CellValue::opt_text(self.notes.as_deref()),
        ]
    }

    /// Build the row for one split of one transaction.
    ///
    /// Everything calendar-shaped is derived from `txn.txn_date`, which is
    /// stored data — no clock is read, here or anywhere below this line.
    fn from_split(txn: &Txn, split: &LoadedSplit) -> Self {
        let date = txn.txn_date;
        let month = date.month();
        ExportRow {
            txn_id: txn.id,
            date,
            year: date.year(),
            quarter: quarter_of(month),
            month,
            month_name: month_short_name(month),
            year_month: YearMonth::from_date(date),
            category: split.category_name.clone(),
            group: split.group_name.clone(),
            merchant: txn.merchant_name.clone(),
            location: txn.location_name.clone(),
            subtotal: split.subtotal,
            tax: split.tax,
            total: split.amount,
            tax_estimated: txn.tax_is_estimated,
            notes: txn.notes.clone(),
        }
    }
}

/// Every matching split, flattened and sorted **chronologically**.
///
/// [`transactions::list`] returns newest-first because that is what a CLI
/// listing and an inbox want. A spreadsheet is read top to bottom in time
/// order, and a reverse-chronological export makes every running total and
/// every chart come out backwards, so the order is reversed exactly once,
/// here, rather than left to the reader.
///
/// The key is date, then transaction id, then the split's `sort_order`, so two
/// receipts on the same day and two splits of the same receipt both land in a
/// defined order — an export that reshuffles itself between runs makes `diff`
/// useless for checking what changed.
pub fn rows(db: &Db, filter: &TxnFilter) -> Result<Vec<ExportRow>> {
    let txns = transactions::list(db, filter)?;

    let mut keyed: Vec<(NaiveDate, i64, i64, ExportRow)> = Vec::new();
    for txn in &txns {
        for split in &txn.splits {
            keyed.push((
                txn.txn_date,
                txn.id.get(),
                split.sort_order,
                ExportRow::from_split(txn, split),
            ));
        }
    }
    keyed.sort_by_key(|(date, txn_id, sort_order, _)| (*date, *txn_id, *sort_order));

    Ok(keyed.into_iter().map(|(_, _, _, row)| row).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;
    use crate::domain::reconcile::Amounts;
    use crate::ids::CategoryId;
    use crate::repo::transactions::{DetailLevel, NewTransaction, SplitRecord, save};
    use crate::repo::{categories, merchants};

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().expect("valid date")
    }

    pub(super) fn seeded() -> Db {
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

    fn draft(when: &str, splits: Vec<SplitRecord>) -> NewTransaction {
        let subtotal: Money = splits.iter().map(|s| s.subtotal).sum();
        let tax: Money = splits.iter().map(|s| s.tax).sum();
        NewTransaction {
            id: None,
            txn_date: date(when),
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

    /// A database with three receipts across two months, one of them split two
    /// ways. Shared with the `xlsx` and `csv` unit tests.
    pub(super) fn sample_rows() -> Vec<ExportRow> {
        let db = seeded();
        let groceries = categories::resolve(&db, "Groceries").expect("category");
        let household = categories::resolve(&db, "Household").expect("category");
        let heb = merchants::get_or_create(&db, "H-E-B").expect("merchant");

        let mut july = draft("2026-07-31", vec![split(groceries.id, 1000, 0)]);
        july.merchant_id = Some(heb.id);
        july.notes = Some("café, naïve".to_string());
        save(&db, &july).expect("save");

        let mut two_way = draft(
            "2026-08-02",
            vec![split(groceries.id, 3000, 0), split(household.id, 2000, 419)],
        );
        two_way.merchant_id = Some(heb.id);
        two_way.amounts.tax_is_estimated = true;
        save(&db, &two_way).expect("save");

        save(&db, &draft("2026-08-01", vec![split(groceries.id, 500, 0)])).expect("save");

        rows(&db, &TxnFilter::default()).expect("rows")
    }

    #[test]
    fn every_cell_lands_in_a_column_of_its_own_kind() {
        // The drift check: `cells()` returns an array the size of `COLUMNS`,
        // so a missing entry does not compile, and this asserts the entries
        // are also in the right *order*.
        for row in sample_rows() {
            for (column, cell) in COLUMNS.iter().zip(row.cells()) {
                assert_eq!(cell.kind(), column.kind, "column `{}`", column.header);
            }
        }
    }

    #[test]
    fn column_headers_are_unique() {
        // Excel refuses a table with duplicate column captions, and the
        // failure mode is a repair prompt rather than an error from us.
        let mut seen: Vec<&str> = COLUMNS.iter().map(|c| c.header).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count);
    }

    #[test]
    fn rows_are_one_per_split_and_chronological() {
        let rows = sample_rows();
        // 1 + 2 + 1 splits across three receipts.
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter().map(|r| r.date).collect::<Vec<_>>(),
            vec![
                date("2026-07-31"),
                date("2026-08-01"),
                date("2026-08-02"),
                date("2026-08-02"),
            ],
            "ascending, though `list` returns newest first"
        );
        // The two-way split keeps its own amounts, not the transaction's.
        let two_way: Vec<&ExportRow> = rows.iter().filter(|r| r.date.day() == 2).collect();
        assert_eq!(two_way[0].category, "Groceries");
        assert_eq!(two_way[0].total, m(3000));
        assert_eq!(two_way[1].category, "Household");
        assert_eq!(two_way[1].subtotal, m(2000));
        assert_eq!(two_way[1].tax, m(419));
        assert_eq!(two_way[1].total, m(2419));
        assert!(two_way[1].tax_estimated);
        assert_eq!(
            two_way.iter().map(|r| r.total).sum::<Money>(),
            m(5419),
            "the splits still add up to the receipt"
        );
    }

    #[test]
    fn calendar_fields_are_values_derived_from_the_stored_date() {
        let row = &sample_rows()[0];
        assert_eq!(row.year, 2026);
        assert_eq!(row.quarter, 3);
        assert_eq!(row.month, 7);
        assert_eq!(row.month_name, "Jul");
        assert_eq!(row.year_month.to_string(), "2026-07");
        assert_eq!(row.group, "Frequent");
        assert_eq!(row.merchant.as_deref(), Some("H-E-B"));
    }

    #[test]
    fn an_empty_database_yields_no_rows_rather_than_an_error() {
        let db = seeded();
        assert!(rows(&db, &TxnFilter::default()).expect("rows").is_empty());
    }
}
