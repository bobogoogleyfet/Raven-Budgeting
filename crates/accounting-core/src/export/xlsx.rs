//! xlsx export: a `Transactions` table and a live `Summary` cross-tab.
//!
//! # What makes an export like this actually useful
//!
//! Three things, each of which is quietly ruinous to get wrong:
//!
//! 1. **Dates are date serials, not strings.** A text `"2026-08-02"` sorts
//!    lexically, cannot be grouped by month, and turns every date filter into
//!    a text filter. [`write_date_with_format`](rust_xlsxwriter::Worksheet::write_date_with_format)
//!    writes the number Excel means by a date.
//! 2. **The header row and the table definition are the same strings.**
//!    [`Worksheet::add_table`](rust_xlsxwriter::Worksheet::add_table) writes
//!    the header cells from the [`TableColumn`] captions, so they cannot
//!    disagree. When they do disagree, Excel does not report an error — it
//!    offers to "repair" the file and then blanks the sheet.
//! 3. **No `autofilter` over a table range.** A table already carries filter
//!    dropdowns; an overlapping `autoFilter` element is one of the reliable
//!    ways to trigger that same repair prompt.
//!
//! # Why a formula cross-tab instead of a pivot table
//!
//! `rust_xlsxwriter` cannot write a pivot table (a pivot cache is a separate
//! part of the package with its own schema). A `SUMIFS` grid over structured
//! references is a deliberate substitute: it recalculates when the underlying
//! table changes, survives round-tripping through LibreOffice and Google
//! Sheets, which mangle pivot caches, and is legible as a formula the user can
//! read and edit.

use rust_xlsxwriter::{
    ColNum, Format, RowNum, Table, TableColumn, Workbook, Worksheet, column_number_to_name,
};

use crate::db::Db;
use crate::error::{CoreError, Result};
use crate::export::{COLUMNS, CellValue, Column, ColumnKind, ExportRow, rows};
use crate::money::Money;
use crate::repo::transactions::TxnFilter;
use crate::time::YearMonth;

/// The sheet holding one row per split.
const DATA_SHEET: &str = "Transactions";
/// The sheet holding the category × month cross-tab.
const SUMMARY_SHEET: &str = "Summary";

/// The table's *defined name*, which is what structured references such as
/// `TxnExport[Total]` resolve against.
///
/// Deliberately not `Transactions`: a table name and a worksheet name both
/// live in the workbook's name space, and a collision between them is the kind
/// of thing that opens fine in one reader and prompts for repair in another.
/// Costing nothing to avoid, it is avoided.
const TABLE_NAME: &str = "TxnExport";

/// Excel's last row index, zero-based (`1_048_576` rows).
const MAX_ROW: usize = 1_048_575;

/// `0.00`-style currency. `$#,##0.00` shows the symbol and groups thousands
/// while the underlying cell stays a plain number, so `SUM` over the column
/// still works — the format is presentation, never data.
const MONEY_FORMAT: &str = "$#,##0.00";
/// ISO, for the same reason the CSV uses it: unambiguous in every locale.
const DATE_FORMAT: &str = "yyyy-mm-dd";

/// Every matching split as the bytes of an `.xlsx` file.
///
/// Returns bytes rather than writing a file — see the [module docs of
/// `export`](crate::export) for why the core never touches a path.
pub fn export_xlsx(db: &Db, filter: &TxnFilter) -> Result<Vec<u8>> {
    workbook_bytes(&rows(db, filter)?)
}

/// The workbook for rows already in hand.
///
/// Split out from [`export_xlsx`] so the whole spreadsheet — table geometry,
/// number formats, the empty-dataset path — can be tested without a database.
pub fn workbook_bytes(rows: &[ExportRow]) -> Result<Vec<u8>> {
    if rows.len() > MAX_ROW {
        // One row is spent on the header. Saying so here beats
        // `RowColumnLimitError` from three layers down.
        return Err(CoreError::Invalid(format!(
            "{} rows will not fit in a worksheet (the limit is {MAX_ROW})",
            rows.len()
        )));
    }

    let mut workbook = Workbook::new();
    write_transactions(workbook.add_worksheet().set_name(DATA_SHEET)?, rows)?;
    write_summary(workbook.add_worksheet().set_name(SUMMARY_SHEET)?, rows)?;
    Ok(workbook.save_to_buffer()?)
}

/// Header row 0, one data row per split from row 1, wrapped in a table.
fn write_transactions(sheet: &mut Worksheet, rows: &[ExportRow]) -> Result<()> {
    let money = Format::new().set_num_format(MONEY_FORMAT);
    let date = Format::new().set_num_format(DATE_FORMAT);

    for (index, column) in COLUMNS.iter().enumerate() {
        sheet.set_column_width(col_num(index)?, column_width(column))?;
    }

    for (offset, row) in rows.iter().enumerate() {
        // +1 for the header row; `rows.len()` is bounded above, so this cannot
        // overflow the conversion.
        let r = row_num(offset + 1)?;
        for (index, cell) in row.cells().iter().enumerate() {
            write_cell(sheet, r, col_num(index)?, cell, &money, &date)?;
        }
    }

    // The header stays visible while scrolling a year of receipts. Deliberately
    // `(1, 0)`: freezing a column too would hide the date on a narrow window.
    sheet.set_freeze_panes(1, 0)?;

    // NOTE: do **not** call `sheet.autofilter(...)` here. The table below
    // already provides the filter dropdowns, and an autofilter range that
    // overlaps a table range makes Excel declare the file corrupt and offer to
    // repair it — which silently discards the table.

    if rows.is_empty() {
        // `add_table` refuses a range whose only row is the header, and it is
        // right to: a table needs at least one data row. The alternative —
        // a one-row table over a blank data row — would put a phantom record
        // into every filter, every `COUNTA`, and any re-import, which is a
        // worse lie than "no table". So the header row is written by hand and
        // the table is skipped. Exporting before entering anything is the very
        // first thing a new user does; it must produce a file that opens.
        for (index, column) in COLUMNS.iter().enumerate() {
            sheet.write_string(0, col_num(index)?, column.header)?;
        }
        return Ok(());
    }

    // The captions come from `COLUMNS`, and `add_table` writes the header row
    // cells from these same captions. That is what makes the two byte-equal
    // rather than merely intended to be.
    let columns: Vec<TableColumn> = COLUMNS
        .iter()
        .map(|column| TableColumn::new().set_header(column.header))
        .collect();
    let table = Table::new().set_name(TABLE_NAME).set_columns(&columns);
    let last_row = row_num(rows.len())?;
    let last_col = col_num(COLUMNS.len() - 1)?;
    sheet.add_table(0, 0, last_row, last_col, &table)?;

    Ok(())
}

/// One cell, written in the type its column declared.
fn write_cell(
    sheet: &mut Worksheet,
    row: RowNum,
    col: ColNum,
    cell: &CellValue<'_>,
    money: &Format,
    date: &Format,
) -> Result<()> {
    match cell {
        // A real Excel date serial. The single most common way an export like
        // this becomes quietly useless is writing this as a string.
        CellValue::Date(d) => {
            sheet.write_date_with_format(row, col, *d, date)?;
        }
        CellValue::Int(n) => {
            sheet.write_number(row, col, i32::try_from(*n).unwrap_or(i32::MAX))?;
        }
        CellValue::Text(t) => {
            // An absent merchant is a blank cell, not the string "". A blank
            // is what `COUNTA` and every filter treat as missing.
            if let Some(text) = t.as_deref().filter(|s| !s.is_empty()) {
                sheet.write_string(row, col, text)?;
            }
        }
        // The only float in the crate, at the only place one belongs: xlsx
        // stores numbers as IEEE doubles, and a cell must hold 54.19 rather
        // than 5419 for `SUM` over the column to mean anything.
        CellValue::Money(m) => {
            sheet.write_number_with_format(row, col, Money::to_excel_f64(*m), money)?;
        }
        CellValue::Bool(b) => {
            sheet.write_boolean(row, col, *b)?;
        }
    }
    Ok(())
}

/// Categories down the rows, months across the columns, every cell a live
/// `SUMIFS` against the `Transactions` table.
///
/// # Why two label columns
///
/// The schema's uniqueness index on category names is `(group_id, name)`, not
/// `name` — "Insurance" under Bills and "Insurance" under Auto are different
/// categories on purpose. A cross-tab keyed on the category name alone would
/// silently add them together, so both the label and the `SUMIFS` criteria
/// carry the group.
fn write_summary(sheet: &mut Worksheet, rows: &[ExportRow]) -> Result<()> {
    // `rust_xlsxwriter` caches a result of `0` for every formula it writes,
    // because it cannot evaluate them. Excel recalculates on open and never
    // sees the cache; LibreOffice trusts it and shows a grid of zeros until
    // the user finds Ctrl-Shift-F9. An empty cached result forces the
    // recalculation instead, which turns a one-line omission that makes the
    // whole export look broken on the user's own machine into a non-issue.
    sheet.set_formula_result_default("");

    let money = Format::new().set_num_format(MONEY_FORMAT);
    let bold = Format::new().set_bold();

    sheet.write_string_with_format(0, 0, "Group", &bold)?;
    sheet.write_string_with_format(0, 1, "Category", &bold)?;
    sheet.set_column_width(0, 18_u32)?;
    sheet.set_column_width(1, 24_u32)?;

    let months = distinct_months(rows);
    let categories = distinct_categories(rows);
    if months.is_empty() || categories.is_empty() {
        // Nothing to cross-tabulate, and — more to the point — with no data
        // there is no `TxnExport` table for a structured reference to resolve
        // against, so every formula would be a `#NAME?`.
        return Ok(());
    }

    // Columns: 0 Group, 1 Category, then one per month, then the total.
    const FIRST_MONTH_COL: usize = 2;
    let total_col = FIRST_MONTH_COL + months.len();
    for (index, month) in months.iter().enumerate() {
        let col = col_num(FIRST_MONTH_COL + index)?;
        sheet.write_string_with_format(0, col, month.to_string(), &bold)?;
        sheet.set_column_width(col, 12_u32)?;
    }
    sheet.write_string_with_format(0, col_num(total_col)?, "Total", &bold)?;
    sheet.set_column_width(col_num(total_col)?, 14_u32)?;

    let first_month_name = column_number_to_name(col_num(FIRST_MONTH_COL)?);
    let last_month_name = column_number_to_name(col_num(total_col - 1)?);

    for (index, (group, category)) in categories.iter().enumerate() {
        let row = row_num(index + 1)?;
        // Spreadsheet rows are 1-based in formulas and 0-based in this API.
        let a1_row = index + 2;
        sheet.write_string(row, 0, group)?;
        sheet.write_string(row, 1, category)?;

        for month_index in 0..months.len() {
            let col = col_num(FIRST_MONTH_COL + month_index)?;
            let month_name = column_number_to_name(col);
            // `$A`/`$B` and `$1` so the block can be dragged in either
            // direction without rewriting the references, which is the first
            // thing a user does to a grid like this.
            let formula = format!(
                "=SUMIFS({TABLE_NAME}[Total],{TABLE_NAME}[Group],$A{a1_row},\
                 {TABLE_NAME}[Category],$B{a1_row},{TABLE_NAME}[YearMonth],{month_name}$1)"
            );
            sheet.write_formula_with_format(row, col, formula.as_str(), &money)?;
        }

        let across = format!("=SUM({first_month_name}{a1_row}:{last_month_name}{a1_row})");
        sheet.write_formula_with_format(row, col_num(total_col)?, across.as_str(), &money)?;
    }

    // The total row sums the month columns rather than re-running SUMIFS, so
    // it agrees with what is displayed above it by construction: if a category
    // is ever missing from the grid, the total says so instead of hiding it.
    let total_row = row_num(categories.len() + 1)?;
    let first_data_a1 = 2;
    let last_data_a1 = categories.len() + 1;
    let total_a1 = categories.len() + 2;
    sheet.write_string_with_format(total_row, 0, "Total", &bold)?;

    for month_index in 0..months.len() {
        let col = col_num(FIRST_MONTH_COL + month_index)?;
        let name = column_number_to_name(col);
        let down = format!("=SUM({name}{first_data_a1}:{name}{last_data_a1})");
        sheet.write_formula_with_format(total_row, col, down.as_str(), &money)?;
    }
    let grand = format!("=SUM({first_month_name}{total_a1}:{last_month_name}{total_a1})");
    sheet.write_formula_with_format(total_row, col_num(total_col)?, grand.as_str(), &money)?;

    sheet.set_freeze_panes(1, 2)?;
    Ok(())
}

/// The distinct months present in the data, ascending.
///
/// Months present, not a dense range between the extremes: a gap in the data
/// is information, and a column of zeros for a month the user did not spend in
/// only widens the sheet.
fn distinct_months(rows: &[ExportRow]) -> Vec<YearMonth> {
    let mut months: Vec<YearMonth> = rows.iter().map(|r| r.year_month).collect();
    months.sort_unstable();
    months.dedup();
    months
}

/// The distinct `(group, category)` pairs present, in group then category
/// order.
fn distinct_categories(rows: &[ExportRow]) -> Vec<(String, String)> {
    let mut categories: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.group.clone(), r.category.clone()))
        .collect();
    categories.sort_unstable();
    categories.dedup();
    categories
}

/// A sensible width: wide enough for the header caption, and at least what the
/// kind of value typically needs.
fn column_width(column: &Column) -> u32 {
    let by_kind = match column.kind {
        ColumnKind::Date => 12,
        ColumnKind::Int => 9,
        ColumnKind::Text => 20,
        ColumnKind::Money => 12,
        ColumnKind::Bool => 14,
    };
    let by_header = u32::try_from(column.header.len())
        .unwrap_or(by_kind)
        .saturating_add(2);
    by_kind.max(by_header)
}

/// Widen a column index for the writer, naming the problem if it will not fit.
///
/// `try_from` rather than `as`: a silent wrap here writes cells into the wrong
/// column, which is the class of bug this crate's lints exist to make loud.
fn col_num(index: usize) -> Result<ColNum> {
    ColNum::try_from(index).map_err(|_| {
        CoreError::Invalid(format!(
            "column {index} is beyond the last column of a sheet"
        ))
    })
}

/// The same for a row index.
fn row_num(index: usize) -> Result<RowNum> {
    RowNum::try_from(index)
        .map_err(|_| CoreError::Invalid(format!("row {index} is beyond the last row of a sheet")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::sample_rows;

    /// The bytes of an `.xlsx` are a zip archive; every reader checks for the
    /// local file header magic first, so this is the cheapest real "did we
    /// produce a workbook" assertion available without a zip crate.
    fn is_zip(bytes: &[u8]) -> bool {
        bytes.starts_with(b"PK\x03\x04")
    }

    #[test]
    fn a_populated_export_is_a_workbook() {
        let bytes = workbook_bytes(&sample_rows()).expect("workbook");
        assert!(is_zip(&bytes));
        assert!(bytes.len() > 1_000, "a real workbook, not a stub");
    }

    #[test]
    fn an_empty_export_still_produces_an_openable_workbook() {
        // Exporting before entering any data is the first thing anyone tries.
        // `add_table` refuses a header-only range, so this path must not.
        let bytes = workbook_bytes(&[]).expect("workbook");
        assert!(is_zip(&bytes));
    }

    #[test]
    fn months_and_categories_are_distinct_and_ordered() {
        let rows = sample_rows();
        assert_eq!(
            distinct_months(&rows)
                .iter()
                .map(YearMonth::to_string)
                .collect::<Vec<_>>(),
            vec!["2026-07", "2026-08"]
        );
        assert_eq!(
            distinct_categories(&rows),
            vec![
                ("Frequent".to_string(), "Groceries".to_string()),
                ("Frequent".to_string(), "Household".to_string()),
            ]
        );
    }

    #[test]
    fn column_widths_cover_their_headers() {
        for column in &COLUMNS {
            let width = column_width(column);
            assert!(
                usize::try_from(width).unwrap() >= column.header.len(),
                "`{}` is wider than its column",
                column.header
            );
        }
    }

    #[test]
    fn out_of_range_indices_are_named_rather_than_wrapped() {
        assert!(matches!(col_num(70_000), Err(CoreError::Invalid(_))));
        assert!(row_num(1_000_000).is_ok());
    }
}
