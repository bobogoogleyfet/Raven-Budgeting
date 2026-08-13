//! CSV export: the same 15 columns, in a form that parses back.
//!
//! # Why this is not "the xlsx export without formatting"
//!
//! Every value here is written so that reading the file back produces the same
//! data. Amounts are [`Money::to_major_string`] — `"54.19"`, no `$`, no
//! thousands separator — because `"$1,234.56"` needs a currency-aware parser
//! and `1234.56` as a float has already lost the argument. Dates are ISO
//! `YYYY-MM-DD`, which sorts chronologically as text and is unambiguous in
//! every locale, unlike `08/09/2026`.

use ::csv::Writer;

use crate::db::Db;
use crate::error::Result;
use crate::export::{COLUMNS, CellValue, ExportRow, rows};
use crate::money::Money;
use crate::repo::transactions::TxnFilter;

/// A UTF-8 byte-order mark.
///
/// The `csv` crate does not write one, and Excel on Windows opens a BOM-less
/// `.csv` in the system ANSI code page: a merchant called `Café` comes back as
/// `CafÃ©`, and the user's only clue is that their own data looks corrupted.
/// Three bytes buy a correct decode in Excel, LibreOffice and Sheets alike;
/// every CSV parser worth using either skips a BOM or leaves it on the first
/// header cell, which is why the BOM goes here and not in front of every line.
const UTF8_BOM: &[u8] = "\u{FEFF}".as_bytes();

/// Every matching split as CSV bytes.
///
/// Returns bytes rather than writing a file — see the [module docs of
/// `export`](crate::export) for why the core never touches a path.
pub fn export_csv(db: &Db, filter: &TxnFilter) -> Result<Vec<u8>> {
    csv_bytes(&rows(db, filter)?)
}

/// The CSV body for rows already in hand.
///
/// Split out from [`export_csv`] so the formatting can be exercised without a
/// database, which is also how the CLI could stream a filtered set it already
/// loaded.
pub fn csv_bytes(rows: &[ExportRow]) -> Result<Vec<u8>> {
    let mut writer = Writer::from_writer(Vec::from(UTF8_BOM));

    writer.write_record(COLUMNS.iter().map(|column| column.header))?;
    for row in rows {
        writer.write_record(row.cells().iter().map(field))?;
    }

    // `into_inner` flushes first, and can only fail if that flush fails —
    // impossible for a `Vec` sink. The error is still propagated rather than
    // unwrapped: a panic inside a mobile export is a crash report, not a
    // message the user can act on.
    Ok(writer
        .into_inner()
        .map_err(|e| ::csv::Error::from(e.into_error()))?)
}

/// One cell as the text that goes in the file.
///
/// `None` becomes an empty field, not the literal `"None"` — an absent
/// merchant is a blank cell, and any re-import reads it back as absent.
fn field(cell: &CellValue<'_>) -> String {
    match cell {
        CellValue::Date(d) => d.format("%Y-%m-%d").to_string(),
        CellValue::Int(n) => n.to_string(),
        CellValue::Text(t) => t.as_deref().unwrap_or_default().to_string(),
        CellValue::Money(m) => Money::to_major_string(*m),
        CellValue::Bool(b) => b.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::sample_rows;

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("utf-8")
    }

    #[test]
    fn the_file_starts_with_a_bom_then_the_header_row() {
        let bytes = csv_bytes(&sample_rows()).expect("csv");
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "UTF-8 BOM");

        let body = text(&bytes[3..]);
        let header = body.lines().next().expect("a header line");
        assert_eq!(
            header,
            "Date,Year,Quarter,Month,MonthName,YearMonth,Category,Group,Merchant,Location,\
             Subtotal,Tax,Total,TaxEstimated,Notes"
        );
    }

    #[test]
    fn values_are_written_in_re_importable_forms() {
        let bytes = csv_bytes(&sample_rows()).expect("csv");
        let body = text(&bytes[3..]);
        let first = body.lines().nth(1).expect("a data line");

        assert!(
            first.starts_with("2026-07-31,2026,3,7,Jul,2026-07,"),
            "{first}"
        );
        // No currency symbol and no thousands separator, so `Money::parse`
        // reads its own output back.
        assert!(first.contains(",10.00,0.00,10.00,false,"), "{first}");
        assert!(!first.contains('$'), "{first}");
        // Non-ASCII survives, and the comma inside the note is quoted rather
        // than splitting the row.
        assert!(first.ends_with("\"café, naïve\""), "{first}");
    }

    #[test]
    fn every_row_has_exactly_fifteen_fields() {
        let bytes = csv_bytes(&sample_rows()).expect("csv");
        let mut reader = ::csv::Reader::from_reader(&bytes[3..]);
        let headers = reader.headers().expect("headers").clone();
        assert_eq!(headers.len(), COLUMNS.len());

        let mut count = 0;
        for record in reader.records() {
            assert_eq!(record.expect("record").len(), COLUMNS.len());
            count += 1;
        }
        assert_eq!(count, 4);
    }

    #[test]
    fn an_empty_export_is_a_bom_and_a_header_row() {
        let bytes = csv_bytes(&[]).expect("csv");
        let body = text(&bytes[3..]);
        assert_eq!(body.lines().count(), 1);
    }
}
