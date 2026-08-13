//! The CSV export: the same fifteen columns, in a format anything can read.
//!
//! CSV is the fallback for every tool that is not Excel, so the bar is that a
//! naive reader gets the right answer. Three things decide that, and none of
//! them are visible by looking at the file in a text editor:
//!
//! * **The BOM.** Without `EF BB BF`, Excel on Windows opens a UTF-8 CSV in
//!   the system code page and a merchant called `Café` becomes `CafÃ©`. The
//!   BOM is three bytes that make the difference between a working export and
//!   a mojibake one, and only a byte-level assertion can see it.
//! * **Money as a bare decimal.** `$1,234.56` is three fields to a CSV reader
//!   that does not know better, and `1234.56` is one number to every reader
//!   there is.
//! * **Quoting.** A merchant with a comma in its name splits a row into
//!   sixteen columns if it is not quoted, silently shifting every column after
//!   it by one.
//!
//! The parser at the bottom of this file is deliberately hand-written: `csv`
//! is a dependency of the crate under test, not a dev-dependency, so a test
//! cannot name it — and re-parsing with the same library that wrote the file
//! would not prove much anyway.

mod common;

use accounting_core::export::{self, COLUMNS};
use accounting_core::repo::transactions::TxnFilter;
use accounting_core::{Db, Money};
use common::{
    amounts, assert_clean, cat, day, estimated_amounts, merchant, save, seeded_db, split, txn,
};

/// The bytes that mark a file as UTF-8 to Excel.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

// ---------------------------------------------------------------------------
// Reading a CSV back
// ---------------------------------------------------------------------------

/// A parsed export: the header record and the data records.
struct Csv {
    header: Vec<String>,
    records: Vec<Vec<String>>,
}

impl Csv {
    /// The value of one column of one data record, by column header.
    #[track_caller]
    fn get(&self, record: usize, header: &str) -> &str {
        let column = self
            .header
            .iter()
            .position(|h| h == header)
            .unwrap_or_else(|| panic!("no `{header}` column in {:?}", self.header));
        self.records
            .get(record)
            .unwrap_or_else(|| panic!("no record {record}; there are {}", self.records.len()))
            .get(column)
            .unwrap_or_else(|| panic!("record {record} has no column {column}"))
    }

    /// Every value in one column, over every record.
    fn column(&self, header: &str) -> Vec<&str> {
        (0..self.records.len())
            .map(|i| self.get(i, header))
            .collect()
    }
}

#[track_caller]
fn export_csv(db: &Db, filter: &TxnFilter) -> Vec<u8> {
    export::export_csv(db, filter).expect("export_csv")
}

/// Strip the BOM, decode, and parse — asserting the BOM on the way through,
/// because every test in this file would otherwise be testing a file Excel
/// mangles.
#[track_caller]
fn read(bytes: &[u8]) -> Csv {
    assert!(
        bytes.starts_with(&BOM),
        "the CSV must start with the UTF-8 BOM; got {:?}",
        &bytes[..bytes.len().min(8)]
    );
    let text = std::str::from_utf8(&bytes[BOM.len()..]).expect("the export must be valid UTF-8");

    let mut records = parse(text);
    assert!(!records.is_empty(), "even an empty export has a header row");
    let header = records.remove(0);
    Csv { header, records }
}

/// RFC 4180: fields separated by commas, records by newlines, either quotable
/// with `"`, and a literal `"` doubled inside a quoted field.
fn parse(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            other => field.push(other),
        }
    }
    // A final record with no trailing newline.
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

// ---------------------------------------------------------------------------
// The envelope
// ---------------------------------------------------------------------------

/// One $154.32 receipt, so the simple assertions have something to look at.
fn one_receipt() -> Db {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let costco = merchant(&db, "Costco");

    // $150.12 + $4.20 tax = $154.32.
    let mut new = txn(
        day("2026-03-14"),
        amounts(15_012, 420),
        vec![split(groceries, 15_012, 420)],
    );
    new.merchant_id = Some(costco);
    save(&db, &new);

    assert_clean(&db);
    db
}

#[test]
fn the_csv_begins_with_a_utf8_byte_order_mark() {
    // Excel on Windows reads a BOM-less UTF-8 CSV in the system code page, so
    // `Café` arrives as `CafÃ©` and there is nothing the user can do about it
    // from inside Excel. Three bytes fix it permanently.
    let db = one_receipt();
    let bytes = export_csv(&db, &TxnFilter::default());

    assert_eq!(
        &bytes[..3],
        &BOM[..],
        "the export must open with EF BB BF; got {:?}",
        &bytes[..bytes.len().min(8)]
    );
    assert_ne!(
        &bytes[3..6],
        &BOM[..],
        "one BOM, not two — a doubled BOM shows up as a stray character in the \
         first header cell"
    );
}

#[test]
fn the_header_line_names_every_column_in_the_declared_order() {
    let db = one_receipt();
    let csv = read(&export_csv(&db, &TxnFilter::default()));

    let contract: Vec<&str> = COLUMNS.iter().map(|c| c.header).collect();
    assert_eq!(
        csv.header, contract,
        "the CSV and the xlsx are the same fifteen columns in the same order — \
         a script written against one must work against the other"
    );
    assert_eq!(csv.header.len(), 15);
}

#[test]
fn an_empty_export_is_the_bom_and_the_header_alone() {
    // A month with no receipts is a legitimate question with a legitimate
    // answer, and the answer is a file with column names in it.
    let db = seeded_db();
    let bytes = export_csv(&db, &TxnFilter::default());

    let csv = read(&bytes);
    let contract: Vec<&str> = COLUMNS.iter().map(|c| c.header).collect();
    assert_eq!(csv.header, contract);
    assert!(
        csv.records.is_empty(),
        "there are no receipts, so there are no data rows; got {:?}",
        csv.records
    );
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[test]
fn money_is_a_plain_decimal_with_no_symbol_and_no_thousands_separator() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    // $150.12 + $4.20 = $154.32 — under a thousand, so nothing would group it.
    save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(15_012, 420),
            vec![split(groceries, 15_012, 420)],
        ),
    );
    // $1,234.56 + $0.00 — over a thousand, which is where a display formatter
    // would insert a comma and split the row into an extra column.
    save(
        &db,
        &txn(
            day("2026-03-15"),
            amounts(123_456, 0),
            vec![split(groceries, 123_456, 0)],
        ),
    );

    let csv = read(&export_csv(&db, &TxnFilter::default()));
    assert_eq!(csv.records.len(), 2);

    assert_eq!(csv.get(0, "Subtotal"), "150.12");
    assert_eq!(csv.get(0, "Tax"), "4.20", "two decimal places, always");
    assert_eq!(csv.get(0, "Total"), "154.32");

    assert_eq!(
        csv.get(1, "Total"),
        "1234.56",
        "an amount over a thousand must not carry a thousands separator: an \
         unquoted `1,234.56` splits the record into sixteen fields and shifts \
         every column after it"
    );

    for header in ["Subtotal", "Tax", "Total"] {
        for value in csv.column(header) {
            assert!(
                !value.contains('$') && !value.contains(','),
                "{header} holds {value:?}; money in a CSV is a bare decimal"
            );
            assert!(
                Money::parse(value).is_ok(),
                "{header} holds {value:?}, which does not parse back as money"
            );
        }
    }

    assert_clean(&db);
}

#[test]
fn dates_are_iso_and_the_derived_columns_come_through_as_written() {
    let db = one_receipt();
    let csv = read(&export_csv(&db, &TxnFilter::default()));

    assert_eq!(
        csv.get(0, "Date"),
        "2026-03-14",
        "ISO, so it sorts as text and every tool parses it the same way"
    );
    assert_eq!(csv.get(0, "Year"), "2026");
    assert_eq!(csv.get(0, "Quarter"), "1");
    assert_eq!(csv.get(0, "Month"), "3", "the month number, not `03`");
    assert_eq!(csv.get(0, "MonthName"), "Mar");
    assert_eq!(csv.get(0, "YearMonth"), "2026-03");
    assert_eq!(csv.get(0, "Category"), "Groceries");
    assert_eq!(csv.get(0, "Group"), "Frequent");
    assert_eq!(csv.get(0, "Merchant"), "Costco");
}

#[test]
fn the_tax_estimated_column_is_the_words_true_and_false() {
    // Not `1`/`0`, which a spreadsheet reads as a number and sums; not
    // `TRUE`/`FALSE`, which is a different string to every script that
    // compares it.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(5012, 420),
            vec![split(groceries, 5012, 420)],
        ),
    );
    save(
        &db,
        &txn(
            day("2026-03-15"),
            estimated_amounts(5000, 419),
            vec![split(groceries, 5000, 419)],
        ),
    );

    let csv = read(&export_csv(&db, &TxnFilter::default()));
    assert_eq!(
        csv.get(0, "TaxEstimated"),
        "false",
        "the tax was read off the receipt"
    );
    assert_eq!(
        csv.get(1, "TaxEstimated"),
        "true",
        "the tax was worked out from a rate"
    );

    assert_clean(&db);
}

#[test]
fn a_receipt_with_no_merchant_and_no_location_leaves_those_fields_empty() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(1000, 0),
            vec![split(groceries, 1000, 0)],
        ),
    );

    let csv = read(&export_csv(&db, &TxnFilter::default()));
    assert_eq!(csv.get(0, "Merchant"), "");
    assert_eq!(csv.get(0, "Location"), "");
    assert_eq!(csv.get(0, "Notes"), "");
    // The record still has all fifteen fields: an absent value is an empty
    // field, not a missing one, or every column after it shifts left.
    assert_eq!(
        csv.records[0].len(),
        COLUMNS.len(),
        "an empty value must still occupy its column"
    );

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Quoting and encoding
// ---------------------------------------------------------------------------

#[test]
fn a_merchant_name_containing_a_comma_or_a_quote_survives_the_round_trip() {
    // The failure this prevents is not an exception, it is a file that parses
    // cleanly into the wrong shape: an unquoted comma makes a sixteen-field
    // record and every column after Merchant reads one to the left, so the
    // Total column silently contains a location name.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let comma = merchant(&db, "Bait, Tackle & Beer");
    let quote = merchant(&db, "The \"Best\" Deli");

    let mut with_comma = txn(
        day("2026-03-14"),
        amounts(1000, 0),
        vec![split(groceries, 1000, 0)],
    );
    with_comma.merchant_id = Some(comma);
    save(&db, &with_comma);

    let mut with_quote = txn(
        day("2026-03-15"),
        amounts(2000, 0),
        vec![split(groceries, 2000, 0)],
    );
    with_quote.merchant_id = Some(quote);
    save(&db, &with_quote);

    let bytes = export_csv(&db, &TxnFilter::default());
    let csv = read(&bytes);

    assert_eq!(
        csv.get(0, "Merchant"),
        "Bait, Tackle & Beer",
        "a quoted field must re-parse to exactly the name that went in"
    );
    assert_eq!(
        csv.get(1, "Merchant"),
        "The \"Best\" Deli",
        "an embedded quote is doubled inside a quoted field, and comes back single"
    );

    for (i, record) in csv.records.iter().enumerate() {
        assert_eq!(
            record.len(),
            COLUMNS.len(),
            "record {i} parsed into {} fields, so the quoting is wrong and every \
             column after Merchant is displaced: {record:?}",
            record.len()
        );
    }
    // The amounts are the giveaway that nothing shifted.
    assert_eq!(csv.get(0, "Total"), "10.00");
    assert_eq!(csv.get(1, "Total"), "20.00");

    // And the encoding on the wire: the raw text must carry the doubled quote,
    // not a backslash escape, which no CSV reader understands.
    let text = String::from_utf8(bytes[BOM.len()..].to_vec()).expect("UTF-8");
    assert!(
        text.contains("\"The \"\"Best\"\" Deli\""),
        "quotes are escaped by doubling, per RFC 4180; got:\n{text}"
    );

    assert_clean(&db);
}

#[test]
fn a_non_ascii_merchant_name_survives_the_round_trip() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let cafe = merchant(&db, "Café Beñito — 東京");

    let mut new = txn(
        day("2026-03-14"),
        amounts(1000, 0),
        vec![split(groceries, 1000, 0)],
    );
    new.merchant_id = Some(cafe);
    save(&db, &new);

    let bytes = export_csv(&db, &TxnFilter::default());
    let csv = read(&bytes);
    assert_eq!(
        csv.get(0, "Merchant"),
        "Café Beñito — 東京",
        "the export is UTF-8 throughout; a name that survived SQLite must \
         survive the CSV writer too"
    );

    assert_clean(&db);
}

#[test]
fn a_note_containing_a_newline_does_not_break_the_record() {
    // The nastiest quoting case: an embedded newline inside a quoted field is
    // legal CSV, and a reader that splits on `\n` before parsing quotes sees
    // two broken records instead of one good one.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    let mut new = txn(
        day("2026-03-14"),
        amounts(1000, 0),
        vec![split(groceries, 1000, 0)],
    );
    new.notes = Some("first line\nsecond line".to_string());
    save(&db, &new);

    let csv = read(&export_csv(&db, &TxnFilter::default()));
    assert_eq!(
        csv.records.len(),
        1,
        "one receipt is one record however many newlines its note holds; got {:?}",
        csv.records
    );
    assert_eq!(csv.get(0, "Notes"), "first line\nsecond line");
    assert_eq!(csv.records[0].len(), COLUMNS.len());

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// The two entry points
// ---------------------------------------------------------------------------

#[test]
fn csv_bytes_produces_the_same_file_as_export_csv() {
    // `export_csv` is `rows` followed by `csv_bytes`; the second half is
    // public so a caller can export rows it assembled some other way, and the
    // two must not drift.
    let db = one_receipt();
    let rows = export::rows(&db, &TxnFilter::default()).expect("rows");

    assert_eq!(
        export::csv_bytes(&rows).expect("csv_bytes"),
        export_csv(&db, &TxnFilter::default()),
        "the two entry points must produce byte-identical files"
    );
}

/// A guard on this file's own parser: every assertion above is only as good as
/// the reader making it.
#[test]
fn the_test_parser_handles_the_cases_it_is_relied_on_for() {
    assert_eq!(parse("a,b\n1,2\n"), [["a", "b"], ["1", "2"]]);
    assert_eq!(parse("a,b\r\n1,2\r\n"), [["a", "b"], ["1", "2"]]);
    assert_eq!(parse("\"x,y\",z"), [["x,y", "z"]]);
    assert_eq!(parse("\"say \"\"hi\"\"\",z"), [["say \"hi\"", "z"]]);
    assert_eq!(parse("\"one\ntwo\",z"), [["one\ntwo", "z"]]);
    assert_eq!(parse("a,,c"), [["a", "", "c"]]);
    assert_eq!(parse("a,b"), [["a", "b"]], "no trailing newline");
}
