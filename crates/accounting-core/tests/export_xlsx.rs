//! What actually lands in the `.xlsx` bytes.
//!
//! # Why this file reads the XML rather than trusting the writer
//!
//! `rust_xlsxwriter` will happily produce a workbook that opens with "we found
//! a problem with some content", or one that opens cleanly and is silently
//! useless. Three failures in particular cannot be caught any other way:
//!
//! * **A date written as a string.** It looks right in the cell and sorts
//!   lexically, so every month filter quietly misses rows and no date formula
//!   over the column works. Nothing short of looking at the cell's type in the
//!   XML distinguishes it from a real date.
//! * **A table whose declared headers do not match the header row.** Excel
//!   treats this as corruption and offers to repair the file, which blanks the
//!   table.
//! * **An autofilter overlapping the table range.** Same repair prompt.
//!
//! # How the zip is opened
//!
//! By shelling out to `python3` and the standard library's `zipfile`. The
//! `zip` crate is in `Cargo.lock` — but only as a transitive dependency of
//! `rust_xlsxwriter`, not as a dev-dependency of this crate, so a test cannot
//! name it, and adding one is not this file's call to make.
//! `zipfile.testzip()` also CRC-checks every member, which is a stronger "this
//! is a valid zip" statement than merely finding the entries we were looking
//! for.
//!
//! The XML is then matched with string assertions, deliberately: a real parser
//! would be a third dependency, and every assertion here is about the
//! presence, absence or literal value of one attribute.

mod common;

use std::path::PathBuf;
use std::process::Command;

use accounting_core::Db;
use accounting_core::export::{self, COLUMNS};
use accounting_core::repo::transactions::TxnFilter;
use common::{amounts, assert_clean, cat, day, location, merchant, save, seeded_db, split, txn};

// ---------------------------------------------------------------------------
// Opening an xlsx
// ---------------------------------------------------------------------------

/// The python side of the zip reader: verify, list, extract.
const UNZIP_PY: &str = r#"
import sys, zipfile

src, dest = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(src) as z:
    broken = z.testzip()
    if broken is not None:
        raise SystemExit("corrupt zip member: " + broken)
    for name in z.namelist():
        print(name)
    z.extractall(dest)
"#;

/// An unpacked workbook.
struct Book {
    /// Kept alive: dropping it deletes everything `root` points at.
    _dir: tempfile::TempDir,
    root: PathBuf,
    /// Every entry in the zip, in the order the zip stores them.
    names: Vec<String>,
    /// `xl/sharedStrings.xml`, resolved. Cell text is stored there by index.
    shared: Vec<String>,
    styles: Styles,
}

#[track_caller]
fn open_xlsx(bytes: &[u8]) -> Book {
    assert!(
        bytes.starts_with(b"PK\x03\x04"),
        "an xlsx is a zip, and a zip starts with the local file header magic; \
         got {:?}",
        &bytes[..bytes.len().min(8)]
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("book.xlsx");
    let script = dir.path().join("unzip.py");
    let dest = dir.path().join("unpacked");
    std::fs::write(&src, bytes).expect("write the workbook");
    std::fs::write(&script, UNZIP_PY).expect("write the unzip script");

    let out = Command::new("python3")
        .arg(&script)
        .arg(&src)
        .arg(&dest)
        .output()
        .expect("python3 must be available to read the xlsx");
    assert!(
        out.status.success(),
        "the exported buffer must be a valid zip; python said:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();

    let mut book = Book {
        _dir: dir,
        root: dest,
        names,
        shared: Vec::new(),
        styles: Styles::default(),
    };
    book.shared = shared_strings(&book.part_or_empty("xl/sharedStrings.xml"));
    book.styles = Styles::parse(&book.part_or_empty("xl/styles.xml"));
    book
}

impl Book {
    #[track_caller]
    fn part(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join(name)).unwrap_or_else(|e| {
            panic!(
                "the workbook must contain `{name}`: {e}. It holds: {:?}",
                self.names
            )
        })
    }

    fn part_or_empty(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join(name)).unwrap_or_default()
    }

    fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    fn table_part_name(&self) -> Option<&String> {
        self.names.iter().find(|n| n.starts_with("xl/tables/"))
    }

    /// The single table part, whatever it is called.
    #[track_caller]
    fn table(&self) -> String {
        let name = self.table_part_name().cloned().unwrap_or_else(|| {
            panic!(
                "the Transactions sheet must be wrapped in a native table, which \
                 lives in its own part under xl/tables/. The workbook holds: {:?}",
                self.names
            )
        });
        self.part(&name)
    }

    /// A worksheet looked up by the name on its tab, via `workbook.xml` and
    /// its relationships. The `sheetN.xml` numbering is an implementation
    /// detail of the writer and says nothing about which tab is which.
    #[track_caller]
    fn sheet(&self, tab: &str) -> String {
        let workbook = self.part("xl/workbook.xml");
        let sheet_tag = open_tags(&workbook, "sheet")
            .into_iter()
            .find(|t| attr(t, "name").as_deref() == Some(tab))
            .unwrap_or_else(|| panic!("the workbook must have a `{tab}` sheet; got {workbook}"));
        let rid = attr(&sheet_tag, "r:id").expect("a sheet must reference its part");

        let rels = self.part("xl/_rels/workbook.xml.rels");
        let rel = open_tags(&rels, "Relationship")
            .into_iter()
            .find(|t| attr(t, "Id").as_deref() == Some(rid.as_str()))
            .unwrap_or_else(|| panic!("no relationship {rid} in {rels}"));
        let target = attr(&rel, "Target").expect("a relationship must have a target");
        self.part(&format!("xl/{}", target.trim_start_matches('/')))
    }

    /// The displayed text of a cell, shared strings resolved.
    fn text_of(&self, cell_xml: &str) -> String {
        match attr_of_open_tag(cell_xml, "t").as_deref() {
            Some("s") => {
                let index: usize = value_of(cell_xml)
                    .unwrap_or_default()
                    .parse()
                    .expect("a shared-string cell holds an index");
                self.shared
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| panic!("shared string {index} is out of range"))
            }
            Some("inlineStr") => inner_texts(cell_xml).concat(),
            _ => value_of(cell_xml).map(|v| unescape(&v)).unwrap_or_default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal XML picking
// ---------------------------------------------------------------------------

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Everything between each `open` and the next `close`, exclusive.
fn between_all(xml: &str, open: &str, close: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(open) {
        let body = &rest[at + open.len()..];
        let Some(end) = body.find(close) else { break };
        out.push(body[..end].to_string());
        rest = &body[end + close.len()..];
    }
    out
}

/// The text content of every `<t …>…</t>` in a fragment, unescaped.
fn inner_texts(xml: &str) -> Vec<String> {
    between_all(xml, "<t", "</t>")
        .iter()
        .map(|t| unescape(t.split_once('>').map_or("", |(_, rest)| rest)))
        .collect()
}

/// Every `<name …>` opening tag in `xml`, as raw text including the angle
/// brackets. Matches the element name exactly, so asking for `sheet` does not
/// also return `<sheetView>` or `<sheets>`.
fn open_tags(xml: &str, name: &str) -> Vec<String> {
    let needle = format!("<{name}");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + needle.len()..];
        let boundary_ok = after
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/');
        if boundary_ok {
            let end = after.find('>').map_or(after.len(), |e| e + 1);
            out.push(format!("{needle}{}", &after[..end]));
        }
        rest = after;
    }
    out
}

/// The value of `name="…"` in a raw tag, unescaped.
///
/// A match whose preceding character is part of a longer attribute name is
/// skipped, so asking for `name` on `<table displayName="x" name="y">` does not
/// answer `x`.
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let at = tag.find(&needle)?;
    let inside_a_longer_name = tag[..at]
        .chars()
        .last()
        .is_some_and(|c| c.is_alphanumeric() || c == ':' || c == '_');
    if inside_a_longer_name {
        return attr(&tag[at + needle.len()..], name);
    }
    let body = &tag[at + needle.len()..];
    body.find('"').map(|end| unescape(&body[..end]))
}

/// An attribute of the *opening* tag of an element given as raw text.
fn attr_of_open_tag(element: &str, name: &str) -> Option<String> {
    attr(
        element.split_once('>').map_or(element, |(head, _)| head),
        name,
    )
}

/// The content of the first `<v>…</v>` inside an element, or `Some("")` for a
/// self-closing `<v/>`.
fn value_of(element: &str) -> Option<String> {
    if let Some(at) = element.find("<v>") {
        let body = &element[at + 3..];
        return body.find("</v>").map(|e| body[..e].to_string());
    }
    element.find("<v/>").map(|_| String::new())
}

/// Whether the element carries a `<v>` element at all, empty or not.
fn has_value_element(element: &str) -> bool {
    element.contains("<v>") || element.contains("<v/>")
}

/// Every `<c …>…</c>` in a sheet, as raw text.
fn cells(sheet: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sheet;
    while let Some(at) = rest.find("<c ") {
        let body = &rest[at..];
        let Some(head_end) = body.find('>') else {
            break;
        };
        if body[..head_end].ends_with('/') {
            out.push(body[..=head_end].to_string());
            rest = &body[head_end + 1..];
        } else if let Some(end) = body.find("</c>") {
            out.push(body[..end + 4].to_string());
            rest = &body[end + 4..];
        } else {
            break;
        }
    }
    out
}

/// The whole `<c …>…</c>` (or `<c …/>`) for one cell reference.
///
/// The reference is matched with its closing quote included, so looking for
/// `A2` cannot land on `A20`.
fn cell(sheet: &str, reference: &str) -> Option<String> {
    let at = sheet.find(&format!("<c r=\"{reference}\""))?;
    let body = &sheet[at..];
    let head_end = body.find('>')?;
    if body[..head_end].ends_with('/') {
        return Some(body[..=head_end].to_string());
    }
    body.find("</c>").map(|end| body[..end + 4].to_string())
}

fn shared_strings(xml: &str) -> Vec<String> {
    between_all(xml, "<si>", "</si>")
        .iter()
        .map(|si| inner_texts(si).concat())
        .collect()
}

/// `0 -> "A"`, `12 -> "M"`, `14 -> "O"`. The export is fifteen columns wide,
/// so a single letter always suffices; anything wider is a contract change.
fn col_letter(index: usize) -> String {
    assert!(index < 26, "the export has {} columns", COLUMNS.len());
    String::from(char::from(b'A' + u8::try_from(index).expect("index < 26")))
}

/// `(column index, 0-based data row)` -> `"M2"`. Data row 0 is spreadsheet
/// row 2, because row 1 is the header.
fn data_ref(column_index: usize, row: usize) -> String {
    format!("{}{}", col_letter(column_index), row + 2)
}

/// The index of a column by its header, so no assertion hard-codes a letter.
#[track_caller]
fn column(header: &str) -> usize {
    COLUMNS
        .iter()
        .position(|c| c.header == header)
        .unwrap_or_else(|| panic!("the export must have a `{header}` column"))
}

// ---------------------------------------------------------------------------
// styles.xml
// ---------------------------------------------------------------------------

/// Enough of `styles.xml` to answer "does this cell have a date format".
#[derive(Default)]
struct Styles {
    /// `numFmtId` -> format code, for the custom formats.
    custom: Vec<(u32, String)>,
    /// `numFmtId` per `cellXfs` entry, in order — the `s="N"` on a cell is an
    /// index into this list.
    xfs: Vec<u32>,
}

impl Styles {
    fn parse(xml: &str) -> Styles {
        let custom = open_tags(xml, "numFmt")
            .iter()
            .filter_map(|t| {
                Some((
                    attr(t, "numFmtId")?.parse().ok()?,
                    attr(t, "formatCode").unwrap_or_default(),
                ))
            })
            .collect();

        // Only `cellXfs`: `cellStyleXfs` holds the same element name and is a
        // different table entirely, and a cell's `s` indexes this one.
        let cell_xfs = between_all(xml, "<cellXfs", "</cellXfs>")
            .first()
            .cloned()
            .unwrap_or_default();
        let xfs = open_tags(&cell_xfs, "xf")
            .iter()
            .map(|t| {
                attr(t, "numFmtId")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            })
            .collect();

        Styles { custom, xfs }
    }

    /// The number-format code a cell style resolves to. Built-in ids are
    /// spelled out only for the ones this export could plausibly use.
    fn format_code(&self, style_index: usize) -> String {
        let id = self.xfs.get(style_index).copied().unwrap_or(0);
        if let Some((_, code)) = self.custom.iter().find(|(i, _)| *i == id) {
            return code.clone();
        }
        match id {
            0 => "General".to_string(),
            14 => "mm-dd-yy".to_string(),
            15 => "d-mmm-yy".to_string(),
            16 => "d-mmm".to_string(),
            17 => "mmm-yy".to_string(),
            22 => "m/d/yy h:mm".to_string(),
            44 => "_(\"$\"* #,##0.00_)".to_string(),
            other => format!("builtin numFmtId {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Three receipts, oldest first as the export will order them:
///
/// | date       | merchant | category              | subtotal |  tax |  total |
/// |------------|----------|-----------------------|---------:|-----:|-------:|
/// | 2026-03-14 | Costco   | Groceries             |   150.12 | 4.20 | 154.32 |
/// | 2026-06-01 | —        | Gas/Fuel              |    20.00 | 0.00 |  20.00 |
/// | 2026-08-20 | Costco   | Restaurants & Takeout |    30.00 | 0.00 |  30.00 |
///
/// The `&` in the last category is not decoration: it must survive XML
/// escaping identically in the data cells and in the table part.
fn three_receipts() -> Db {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let fuel = cat(&db, "Gas/Fuel");
    let takeout = cat(&db, "Restaurants & Takeout");
    let costco = merchant(&db, "Costco");
    let sunnyvale = location(&db, "Costco Sunnyvale", 91_250);

    // $150.12 + $4.20 = $154.32.
    let mut first = txn(
        day("2026-03-14"),
        amounts(15_012, 420),
        vec![split(groceries, 15_012, 420)],
    );
    first.merchant_id = Some(costco);
    first.location_id = Some(sunnyvale);
    save(&db, &first);

    save(
        &db,
        &txn(
            day("2026-06-01"),
            amounts(2000, 0),
            vec![split(fuel, 2000, 0)],
        ),
    );

    let mut third = txn(
        day("2026-08-20"),
        amounts(3000, 0),
        vec![split(takeout, 3000, 0)],
    );
    third.merchant_id = Some(costco);
    save(&db, &third);

    assert_clean(&db);
    db
}

#[track_caller]
fn workbook(db: &Db) -> Book {
    let bytes = export::export_xlsx(db, &TxnFilter::default()).expect("export_xlsx");
    assert!(!bytes.is_empty(), "an xlsx is never zero bytes");
    open_xlsx(&bytes)
}

#[track_caller]
fn row_count(db: &Db) -> usize {
    export::rows(db, &TxnFilter::default())
        .expect("export::rows")
        .len()
}

/// The header row, read back out of the sheet.
fn header_texts(book: &Book, sheet: &str) -> Vec<String> {
    (0..COLUMNS.len())
        .map(|i| {
            cell(sheet, &format!("{}1", col_letter(i)))
                .map(|c| book.text_of(&c))
                .unwrap_or_default()
        })
        .collect()
}

/// The column names the table part declares, in order.
fn declared_headers(table: &str) -> Vec<String> {
    open_tags(table, "tableColumn")
        .iter()
        .filter_map(|t| attr(t, "name"))
        .collect()
}

/// Every formula on a sheet, unescaped.
fn formulas(sheet: &str) -> Vec<String> {
    between_all(sheet, "<f", "</f>")
        .iter()
        .map(|f| unescape(f.split_once('>').map_or("", |(_, rest)| rest)))
        .collect()
}

/// Every formula on a sheet paired with the cell it lives in, as
/// `("C2", "SUMIFS(…)")`. Which cell a formula sits in is the whole question
/// when the formula's own references are supposed to point back at it.
fn formula_cells(sheet: &str) -> Vec<(String, String)> {
    cells(sheet)
        .iter()
        .filter_map(|c| {
            let reference = attr_of_open_tag(c, "r")?;
            let formula = formulas(c).into_iter().next()?;
            Some((reference, formula))
        })
        .collect()
}

/// `"C12"` -> `("C", 12)`.
fn split_reference(reference: &str) -> (String, usize) {
    let letters: String = reference
        .chars()
        .take_while(char::is_ascii_uppercase)
        .collect();
    let digits: String = reference[letters.len()..].to_string();
    (
        letters,
        digits
            .parse()
            .unwrap_or_else(|_| panic!("bad ref {reference}")),
    )
}

/// The text of one cell of a sheet, empty when the cell was never written.
fn text_at(book: &Book, sheet: &str, reference: &str) -> String {
    cell(sheet, reference)
        .map(|c| book.text_of(&c))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The container
// ---------------------------------------------------------------------------

#[test]
fn the_exported_buffer_is_a_zip_holding_a_workbook_a_sheet_and_a_table() {
    let db = three_receipts();
    let book = workbook(&db);

    assert!(
        book.has("xl/workbook.xml"),
        "no workbook part; the zip holds {:?}",
        book.names
    );
    assert!(
        book.has("xl/worksheets/sheet1.xml"),
        "no first worksheet; the zip holds {:?}",
        book.names
    );
    assert!(
        book.table_part_name().is_some(),
        "the data must be a native Excel table, not a bare range — that is what \
         gives structured references and a filter row. The zip holds {:?}",
        book.names
    );
    assert!(
        book.has("[Content_Types].xml"),
        "without the content-type map Excel cannot open the file at all"
    );
}

// ---------------------------------------------------------------------------
// The date cell — the highest-value assertion in the milestone
// ---------------------------------------------------------------------------

#[test]
fn the_date_cell_is_a_number_with_a_date_format_and_not_a_string() {
    // A date written as text looks identical in the cell and is useless: it
    // sorts lexically, no date formula accepts it, and every month filter
    // silently misses rows. The only way to tell the two apart is the cell's
    // type in the XML — a numeric cell has no `t` attribute (or `t="n"`),
    // while text carries `t="s"` (shared), `t="str"` (formula result) or
    // `t="inlineStr"`.
    //
    // The serial: Excel's 1900 date system numbers 1900-01-01 as 1 and wrongly
    // believes 1900 was a leap year, so from 1900-03-01 onward the serial is
    // `(date - 1899-12-30).num_days()`. For 2026-03-14 that is 46095. The
    // assertion below is on `rust_xlsxwriter`'s actual output rather than on
    // that reasoning: if the writer were on the 1904 system the number found
    // would be 1462 lower, and the failure message says so.
    const EXPECTED_SERIAL: &str = "46095";

    let db = three_receipts();
    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    // Data row 0 is the 2026-03-14 receipt: the export is date-ascending and
    // that is the earliest of the three.
    let reference = data_ref(column("Date"), 0);
    let date_cell = cell(&sheet, &reference)
        .unwrap_or_else(|| panic!("no cell at {reference}; the sheet is:\n{sheet}"));

    let kind = attr_of_open_tag(&date_cell, "t");
    assert!(
        !matches!(kind.as_deref(), Some("s" | "str" | "inlineStr")),
        "the Date column must hold a real Excel date serial, not text. A text \
         date sorts lexically and breaks every month filter and date formula \
         built over the export, while looking perfectly correct on screen. \
         Cell {reference} is: {date_cell}"
    );

    let raw = value_of(&date_cell)
        .unwrap_or_else(|| panic!("cell {reference} has no value at all: {date_cell}"));
    assert!(
        raw.chars().all(|c| c.is_ascii_digit()),
        "a date serial is a whole number of days; {reference} holds {raw:?}"
    );

    let style_index: usize = attr_of_open_tag(&date_cell, "s")
        .unwrap_or_else(|| {
            panic!(
                "cell {reference} carries no style, so Excel would show the raw \
                 serial {raw} instead of a date: {date_cell}"
            )
        })
        .parse()
        .expect("a style index is a number");
    let code = book.styles.format_code(style_index);
    assert!(
        code.contains('y') || code.contains('d'),
        "the Date column needs a date number format, or the cell shows {raw} to \
         the user. Style {style_index} resolves to {code:?}"
    );

    assert_eq!(
        raw, EXPECTED_SERIAL,
        "2026-03-14 is day {EXPECTED_SERIAL} of Excel's 1900 date system. A \
         different number means the writer is on a different epoch — the 1904 \
         system is off by 1462 days — and every exported date is wrong by that \
         much."
    );
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

#[test]
fn the_tables_declared_headers_are_exactly_the_header_row() {
    // Excel stores the table's column names a second time, in the table part.
    // If the two copies disagree — by a stray space, by a different escaping
    // of `&`, by one being title-cased — Excel declares the workbook corrupt
    // and offers to repair it, and repairing blanks the table.
    let db = three_receipts();
    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    let contract: Vec<&str> = COLUMNS.iter().map(|c| c.header).collect();
    let in_sheet = header_texts(&book, &sheet);
    assert_eq!(
        in_sheet, contract,
        "the header row must be exactly the declared columns, in order"
    );
    assert_eq!(
        declared_headers(&book.table()),
        in_sheet,
        "the table's declared column names must match the header row byte for \
         byte, or Excel treats the workbook as corrupt and repairs it by \
         blanking the table"
    );
}

#[test]
fn the_table_range_covers_exactly_the_rows_that_are_there() {
    let db = three_receipts();
    let book = workbook(&db);

    let n = row_count(&db);
    // Header row plus one row per split. Three single-split receipts is three
    // data rows, so A1:O4.
    let expected = format!("A1:{}{}", col_letter(COLUMNS.len() - 1), n + 1);

    let range = open_tags(&book.table(), "table")
        .first()
        .and_then(|t| attr(t, "ref"))
        .expect("the table part must declare a ref range");
    assert_eq!(
        range, expected,
        "a table range wider than the data gives phantom blank rows in every \
         pivot built on it, and one narrower silently drops receipts"
    );
}

#[test]
fn the_transactions_sheet_carries_no_autofilter_of_its_own() {
    // A table already provides the filter dropdowns. Calling `autofilter()` as
    // well writes a second, overlapping filter range into the sheet, and Excel
    // refuses the file with a repair prompt. The assertion is on the *sheet*
    // part: the `<autoFilter>` a table declares inside its own part is
    // legitimate, and is a different element in a different file.
    let db = three_receipts();
    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    assert!(
        !sheet.contains("<autoFilter"),
        "the Transactions sheet must not declare an autofilter alongside the \
         table; the two ranges overlap and Excel offers to repair the file. \
         Found: {:?}",
        open_tags(&sheet, "autoFilter")
    );
}

#[test]
fn the_header_row_is_frozen() {
    // Scrolling a year of receipts with the headers off-screen is the
    // difference between a usable export and a wall of numbers.
    let db = three_receipts();
    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    let pane = open_tags(&sheet, "pane")
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no frozen pane on the Transactions sheet:\n{sheet}"));
    assert_eq!(
        attr(&pane, "state").as_deref(),
        Some("frozen"),
        "a split pane is not a frozen one; got {pane}"
    );
    assert_eq!(
        attr(&pane, "ySplit").as_deref(),
        Some("1"),
        "exactly the header row is frozen; got {pane}"
    );
}

// ---------------------------------------------------------------------------
// The Summary sheet
// ---------------------------------------------------------------------------

#[test]
fn the_summary_sheet_is_a_grid_of_live_sumifs_over_real_categories_and_months() {
    let db = three_receipts();
    let book = workbook(&db);
    let summary = book.sheet("Summary");

    let found = formulas(&summary);
    assert!(
        !found.is_empty(),
        "the Summary sheet must hold live formulas, not values baked in at \
         export time — the point of the sheet is that it keeps working when the \
         user edits or adds rows on the Transactions sheet"
    );
    assert!(
        found.iter().any(|f| f.contains("SUMIFS")),
        "the summary is categories by months, which is SUMIFS; got {found:?}"
    );
    // The formulas must reach into the exported data, and they do it through
    // the table's *defined name* rather than the sheet name — `TxnExport[Total]`,
    // not `Transactions!M:M`. So the name to look for is the one the table part
    // itself declares: hard-coding it here would let the two drift apart, and a
    // structured reference to a table that is not there is a `#NAME?` in every
    // cell.
    let table_name = open_tags(&book.table(), "table")
        .first()
        .and_then(|t| attr(t, "displayName"))
        .expect("the table part must declare a displayName to reference");
    assert!(
        found
            .iter()
            .any(|f| f.contains(&format!("{table_name}[Total]"))),
        "the SUMIFS must sum the table's Total column through a structured \
         reference to `{table_name}`; got {found:?}"
    );
    for axis in ["Category", "YearMonth"] {
        assert!(
            found
                .iter()
                .any(|f| f.contains(&format!("{table_name}[{axis}]"))),
            "the cross-tab's {axis} axis must be a criterion on the table's own \
             {axis} column; got {found:?}"
        );
    }

    // Real axis labels, so the grid is over this database rather than a
    // hard-coded skeleton.
    let texts: Vec<String> = cells(&summary).iter().map(|c| book.text_of(c)).collect();
    assert!(
        texts.iter().any(|t| t == "Groceries"),
        "the rows must be the real categories; the sheet's text is {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("2026-03")),
        "the columns must be the real months covered by the data; the sheet's \
         text is {texts:?}"
    );
}

#[test]
fn no_formula_cell_ships_a_cached_result() {
    // `rust_xlsxwriter` cannot evaluate formulas, so it writes a placeholder
    // cached result — `0` by default. Excel recalculates on open and never
    // shows it; LibreOffice trusts the cache and renders a summary of nothing
    // but zeros. `set_formula_result_default("")` is the fix, and this is the
    // assertion that it was called.
    let db = three_receipts();
    let book = workbook(&db);
    let summary = book.sheet("Summary");

    let formula_cells: Vec<String> = cells(&summary)
        .into_iter()
        .filter(|c| c.contains("<f"))
        .collect();
    // The negative control: an implementation that wrote no formulas at all
    // would otherwise pass this test by vacuum.
    assert!(
        !formula_cells.is_empty(),
        "the Summary sheet has no formulas at all"
    );

    let cached: Vec<&String> = formula_cells
        .iter()
        .filter(|c| value_of(c).is_some_and(|v| !v.is_empty()))
        .collect();
    assert!(
        cached.is_empty(),
        "every formula cell must ship an empty cached result, or LibreOffice \
         renders the whole Summary sheet as zeros without ever evaluating the \
         formulas. Offending cells: {cached:?}"
    );

    // The empty default can be encoded either as `<v></v>` or as no `<v>` at
    // all; both mean the same thing to Excel, and the assertion above accepts
    // either. This one records that no third encoding sneaked in.
    assert!(
        formula_cells
            .iter()
            .all(|c| !has_value_element(c) || value_of(c).as_deref() == Some("")),
        "a formula cell must carry either no <v> or an empty one; got \
         {formula_cells:?}"
    );
}

#[test]
fn every_summary_formula_points_at_the_labels_of_its_own_row_and_column() {
    // The failure this catches is the one that cannot be seen by reading the
    // numbers: a cross-tab whose criteria are off by a row sums the wrong
    // category into every cell, and still looks like a perfectly plausible
    // report. `contains("SUMIFS")` would pass on it happily.
    let db = three_receipts();
    let book = workbook(&db);
    let summary = book.sheet("Summary");

    let exported = export::rows(&db, &TxnFilter::default()).expect("rows");
    let mut expected_pairs: Vec<(String, String)> = exported
        .iter()
        .map(|r| (r.group.clone(), r.category.clone()))
        .collect();
    expected_pairs.sort();
    expected_pairs.dedup();
    let mut expected_months: Vec<String> = exported
        .iter()
        .map(|r| r.year_month.to_string())
        .collect::<Vec<_>>();
    expected_months.sort();
    expected_months.dedup();

    // The axes are the distinct values in the data, in sorted order.
    let in_grid: Vec<(String, String)> = (0..expected_pairs.len())
        .map(|i| {
            let row = i + 2;
            (
                text_at(&book, &summary, &format!("A{row}")),
                text_at(&book, &summary, &format!("B{row}")),
            )
        })
        .collect();
    assert_eq!(
        in_grid, expected_pairs,
        "the summary's rows must be the (group, category) pairs the data holds"
    );
    let months_in_grid: Vec<String> = (0..expected_months.len())
        .map(|i| text_at(&book, &summary, &format!("{}1", col_letter(2 + i))))
        .collect();
    assert_eq!(
        months_in_grid, expected_months,
        "the summary's columns must be the months the data holds"
    );

    // And every SUMIFS criterion must address the labels of the cell it sits
    // in — `$A5`/`$B5` for a formula on row 5, `D$1` for one in column D.
    let sumifs: Vec<(String, String)> = formula_cells(&summary)
        .into_iter()
        .filter(|(_, f)| f.contains("SUMIFS"))
        .collect();
    assert_eq!(
        sumifs.len(),
        expected_pairs.len() * expected_months.len(),
        "the grid must be complete: one SUMIFS per category per month"
    );
    for (reference, formula) in &sumifs {
        let (letter, row) = split_reference(reference);
        for anchor in [
            format!("$A{row}"),
            format!("$B{row}"),
            format!("{letter}$1"),
        ] {
            assert!(
                formula.contains(&anchor),
                "the formula in {reference} must be criteria'd on {anchor}, or it \
                 sums a different category or month than the one it is filed \
                 under; got {formula}"
            );
        }
    }

    // The row under the grid is the total, not a category.
    assert_eq!(
        text_at(&book, &summary, &format!("A{}", expected_pairs.len() + 2)),
        "Total"
    );
}

#[test]
fn two_categories_with_the_same_name_in_different_groups_stay_apart_in_the_summary() {
    // The schema's uniqueness index is `(group_id, name)`, so "Utilities"
    // under Bills and "Utilities" under Quality of Life are different
    // categories on purpose. A cross-tab keyed on the name alone silently adds
    // them together, and the user's Bills total quietly includes their hobby
    // spending.
    let db = seeded_db();
    let groups = accounting_core::repo::categories::list_groups(&db).expect("list_groups");
    let group = |name: &str| {
        groups
            .iter()
            .find(|g| g.name == name)
            .unwrap_or_else(|| panic!("the seed must have a `{name}` group"))
            .id
    };
    accounting_core::repo::categories::create(&db, group("Bills"), "Utilities")
        .expect("create Bills/Utilities");
    accounting_core::repo::categories::create(&db, group("Quality of Life"), "Utilities")
        .expect("create Quality of Life/Utilities");

    let bills = cat(&db, "Bills/Utilities");
    let hobby = cat(&db, "Quality of Life/Utilities");
    save(
        &db,
        &txn(
            day("2026-03-14"),
            amounts(1000, 0),
            vec![split(bills, 1000, 0)],
        ),
    );
    save(
        &db,
        &txn(
            day("2026-03-15"),
            amounts(2000, 0),
            vec![split(hobby, 2000, 0)],
        ),
    );

    let book = workbook(&db);
    let summary = book.sheet("Summary");

    let labels: Vec<(String, String)> = (2..=3)
        .map(|row| {
            (
                text_at(&book, &summary, &format!("A{row}")),
                text_at(&book, &summary, &format!("B{row}")),
            )
        })
        .collect();
    assert_eq!(
        labels,
        vec![
            ("Bills".to_string(), "Utilities".to_string()),
            ("Quality of Life".to_string(), "Utilities".to_string()),
        ],
        "two categories sharing a name must be two rows, told apart by group"
    );

    // And the criteria must actually use the group, or the two rows would show
    // the same number.
    let sumifs: Vec<String> = formulas(&summary)
        .into_iter()
        .filter(|f| f.contains("SUMIFS"))
        .collect();
    assert!(
        sumifs.iter().all(|f| f.contains("[Group]")),
        "without a Group criterion both rows sum both categories; got {sumifs:?}"
    );

    assert_clean(&db);
}

#[test]
fn a_flag_is_a_real_boolean_and_an_absent_value_is_a_blank_cell() {
    let db = three_receipts();
    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    // `TaxEstimated` as the text "false" would sort, filter and count as text;
    // as a boolean, Excel's own TRUE/FALSE machinery works on it.
    let flag = data_ref(column("TaxEstimated"), 0);
    let flag_cell = cell(&sheet, &flag).unwrap_or_else(|| panic!("no cell at {flag}"));
    assert_eq!(
        attr_of_open_tag(&flag_cell, "t").as_deref(),
        Some("b"),
        "TaxEstimated must be a native boolean, not text; {flag} is {flag_cell}"
    );
    assert_eq!(
        value_of(&flag_cell).as_deref(),
        Some("0"),
        "the first receipt's tax was read off the paper, so the flag is false"
    );

    // Data row 1 is the 2026-06-01 receipt, which has no merchant. A blank
    // cell is what `COUNTA` and every filter treat as missing; an empty string
    // is a value that happens to be invisible.
    let merchant_ref = data_ref(column("Merchant"), 1);
    assert!(
        cell(&sheet, &merchant_ref).is_none(),
        "an absent merchant must leave the cell unwritten, not hold \"\"; {merchant_ref} \
         is {:?}",
        cell(&sheet, &merchant_ref)
    );
    // The row is still there, though — the blank is one cell, not a short row.
    let total_ref = data_ref(column("Total"), 1);
    assert_eq!(
        value_of(&cell(&sheet, &total_ref).expect("the row exists")).as_deref(),
        Some("20"),
        "the second receipt is $20.00"
    );
}

// ---------------------------------------------------------------------------
// Boundaries
// ---------------------------------------------------------------------------

#[test]
fn an_empty_export_is_still_a_workbook_that_opens() {
    // The obvious failure is an error return, but the subtler one is a table
    // whose range is `A1:O1` — a header with no data rows. Either way the user
    // asked for an export of a month in which they bought nothing, and must
    // get a spreadsheet with headers rather than a stack trace.
    let db = seeded_db();
    assert_eq!(row_count(&db), 0, "the fixture must genuinely be empty");

    let bytes = export::export_xlsx(&db, &TxnFilter::default())
        .expect("an empty export is a valid request, not an error");
    let book = open_xlsx(&bytes);
    let sheet = book.sheet("Transactions");

    let contract: Vec<&str> = COLUMNS.iter().map(|c| c.header).collect();
    assert_eq!(
        header_texts(&book, &sheet),
        contract,
        "an empty export must still carry the full header row"
    );

    // Whether an empty export is worth wrapping in a table is a judgement
    // call. What is not a judgement call is that a table, if written, must
    // still agree with the header row — the same repair prompt applies.
    if book.table_part_name().is_some() {
        assert_eq!(declared_headers(&book.table()), contract);
    }
}

#[test]
fn a_single_row_export_is_still_a_workbook_that_opens() {
    // The `last_row <= first_row` boundary: with one data row the table range
    // is A1:O2, and an off-by-one that computed `first_row + count - 1` from a
    // count of one lands back on the header row.
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
    assert_eq!(row_count(&db), 1);

    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    let range = open_tags(&book.table(), "table")
        .first()
        .and_then(|t| attr(t, "ref"))
        .expect("the table part must declare a ref range");
    assert_eq!(range, format!("A1:{}2", col_letter(COLUMNS.len() - 1)));

    // And the one row is actually there, not merely declared.
    let reference = data_ref(column("Category"), 0);
    let cell_xml =
        cell(&sheet, &reference).unwrap_or_else(|| panic!("no cell at {reference}:\n{sheet}"));
    assert_eq!(book.text_of(&cell_xml), "Groceries");

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Money
// ---------------------------------------------------------------------------

#[test]
fn money_reaches_the_sheet_as_dollars_and_not_as_cents_or_a_float_artefact() {
    // Two failures with the same symptom of "the number looks wrong": writing
    // the raw cent count (15432), and round-tripping through a float badly
    // enough to store 154.31999999999999. The cell's stored text is the only
    // place either is visible — Excel's display rounds both away.
    let db = three_receipts();
    let book = workbook(&db);
    let sheet = book.sheet("Transactions");

    let reference = data_ref(column("Total"), 0);
    let total_cell =
        cell(&sheet, &reference).unwrap_or_else(|| panic!("no cell at {reference}:\n{sheet}"));

    assert!(
        attr_of_open_tag(&total_cell, "t").is_none(),
        "money must be a number, or no arithmetic works over the export; \
         {reference} is {total_cell}"
    );
    let raw = value_of(&total_cell).expect("a money cell has a value");
    assert_eq!(
        raw, "154.32",
        "the first receipt is $150.12 + $4.20 tax = $154.32. `15432` would mean \
         the raw cent count reached the sheet; `154.31999999999999` would mean \
         the conversion went through a float badly. {reference} holds {raw:?}"
    );

    // Subtotal and Tax on the same row, so the whole money family is covered
    // rather than one lucky column. xlsx stores the shortest text that
    // round-trips to the same double, so $4.20 is stored as `4.2`; the format
    // code is what puts the second decimal place back on screen.
    for (header, expected) in [("Subtotal", "150.12"), ("Tax", "4.2")] {
        let reference = data_ref(column(header), 0);
        let this = cell(&sheet, &reference).unwrap_or_else(|| panic!("no cell at {reference}"));
        let raw = value_of(&this).expect("a money cell has a value");
        assert_eq!(raw, expected, "{header} at {reference} holds {raw:?}");
    }

    let style_index: usize = attr_of_open_tag(&total_cell, "s")
        .expect("a money cell must be styled")
        .parse()
        .expect("a style index is a number");
    let code = book.styles.format_code(style_index);
    assert!(
        code.contains("0.00"),
        "money needs a two-decimal currency format, or the sheet shows `4.2` \
         where the receipt says $4.20; style {style_index} resolves to {code:?}"
    );
}

#[test]
fn workbook_bytes_renders_the_same_sheet_as_export_xlsx() {
    // `export_xlsx` is `rows` followed by `workbook_bytes`. Keeping the second
    // half callable on its own is what lets a caller export rows it assembled
    // some other way — and the two must not drift into producing different
    // files.
    let db = three_receipts();
    let rows = export::rows(&db, &TxnFilter::default()).expect("rows");

    let a = open_xlsx(&export::export_xlsx(&db, &TxnFilter::default()).expect("export_xlsx"));
    let b = open_xlsx(&export::workbook_bytes(&rows).expect("workbook_bytes"));

    let a_sheet = a.sheet("Transactions");
    let b_sheet = b.sheet("Transactions");
    let texts = |book: &Book, sheet: &str| -> Vec<String> {
        cells(sheet).iter().map(|c| book.text_of(c)).collect()
    };
    assert_eq!(
        texts(&a, &a_sheet),
        texts(&b, &b_sheet),
        "the two entry points must render the same rows"
    );
}

/// A guard on the helpers, not on the export: `col_letter` and `data_ref` are
/// what every assertion above addresses cells with, so an error in them would
/// quietly point the whole file at the wrong cells.
#[test]
fn the_cell_reference_helpers_agree_with_the_column_order() {
    assert_eq!(COLUMNS.len(), 15, "the export is fifteen columns wide");
    assert_eq!(col_letter(0), "A");
    assert_eq!(col_letter(COLUMNS.len() - 1), "O");
    assert_eq!(data_ref(0, 0), "A2", "data starts on row 2");
    assert_eq!(data_ref(12, 2), "M4");
    assert_eq!(column("Date"), 0);
    assert_eq!(column("Total"), 12);
    assert_eq!(column("Notes"), 14);
}
