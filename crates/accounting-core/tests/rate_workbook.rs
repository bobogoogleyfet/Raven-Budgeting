//! Reading the Tax Commission's quarterly workbook.
//!
//! The real file is a 260KB download that changes every quarter, so it is not
//! checked in and these tests do not reach the network. They build workbooks in
//! the shape the publisher uses — a printed document with a title block,
//! instructions and footnotes sharing the grid with the data — and assert the
//! parser picks the jurisdictions out of it.
//!
//! What that leaves uncovered is whether the *real* file still has this shape,
//! which no offline test can answer. `cargo run -p accounting-core --example
//! snapshot` is how that gets checked, and the bundled snapshot is its output.

use accounting_core::rates::{GROCERY_RATE_PPM, parse_workbook};
use accounting_core::time::Quarter;
use rust_xlsxwriter::{Workbook, Worksheet};

fn q3() -> Quarter {
    Quarter::new(2026, 3).expect("valid quarter")
}

/// The publisher's preamble: a title block, then a header split over rows.
fn write_preamble(sheet: &mut Worksheet, effective: &str) {
    sheet
        .write(2, 4, "RATES APPLIED TO CERTAIN TRANSACTIONS")
        .unwrap();
    sheet.write(5, 4, effective).unwrap();
    // The chart stacks each heading over three rows, and the parser reads the
    // bottom one. Reproduced faithfully because the stack is what makes "Food"
    // ambiguous across the sheet: it is the second line of "Grocery Food" and
    // the second line of "Prepared Food" both. Only the bottom row
    // disambiguates, and only because the parser never looks anywhere else.
    sheet.write(8, 2, "Cnty/").unwrap();
    sheet.write(8, 6, "Prepared").unwrap();
    sheet.write(9, 2, "City").unwrap();
    sheet.write(9, 3, "Combined").unwrap();
    sheet.write(9, 4, "Grocery").unwrap();
    sheet.write(9, 5, "Transient").unwrap();
    sheet.write(9, 6, "Food").unwrap();
    // The heading row the parser looks for.
    sheet.write(10, 0, "Location").unwrap();
    sheet.write(10, 2, "Code").unwrap();
    sheet.write(10, 3, "Sales and Use").unwrap();
    sheet.write(10, 4, "Food").unwrap();
    sheet.write(10, 5, "Room").unwrap();
    sheet.write(10, 6, "(Restaurant)").unwrap();
    // Instructions, in the same columns as the data below them.
    sheet
        .write(
            11,
            0,
            "INSTRUCTIONS: the combined sales rate applies to all",
        )
        .unwrap();
    sheet
        .write(12, 0, "These rate charts should not be used for sourcing")
        .unwrap();
}

/// One jurisdiction row, laid out where the real file puts them.
/// `prepared` is passed rather than derived as `fraction + 0.01`: the crate
/// lints float arithmetic, and a fixture that computes its own expected value
/// can only ever agree with itself.
fn write_area(
    sheet: &mut Worksheet,
    row: u32,
    code: &str,
    name: &str,
    fraction: f64,
    prepared: f64,
) {
    sheet.write(row, 0, name).unwrap();
    sheet.write(row, 2, code).unwrap();
    sheet.write(row, 3, fraction).unwrap();
    sheet.write(row, 4, 0.03).unwrap();
    sheet.write(row, 5, 0.120_5).unwrap();
    sheet.write(row, 6, prepared).unwrap();
}

/// A workbook with `count` synthetic jurisdictions plus two real ones.
fn workbook_with(count: u32, sheet_name: &str, effective: &str) -> Vec<u8> {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name(sheet_name).unwrap();
    write_preamble(sheet, effective);

    write_area(sheet, 17, "25-099", "Highland", 0.074_5, 0.084_5);
    write_area(sheet, 18, "18-122", "Salt Lake City", 0.084_5, 0.094_5);
    for i in 0..count {
        let row = 19 + i;
        write_area(
            sheet,
            row,
            &format!("30-{i:03}"),
            &format!("Township {i}"),
            0.07,
            0.08,
        );
    }
    // A footnote below the data, which must not be read as a jurisdiction.
    sheet
        .write(19 + count + 1, 0, "* ST rate includes the state portion")
        .unwrap();
    book.save_to_buffer().expect("build workbook")
}

#[test]
fn jurisdictions_are_read_and_prose_is_not() {
    let bytes = workbook_with(250, "Simplified", "Rates In effect as of July 1, 2026");
    let table = parse_workbook(&bytes, q3()).expect("parse");

    assert_eq!(table.areas.len(), 252, "250 synthetic plus the two named");
    let highland = &table.areas[0];
    assert_eq!(highland.name, "Highland");
    assert_eq!(highland.code, "25-099");
    assert_eq!(highland.combined_rate_ppm, 74_500);
    // All three rates come out of the chart now. The grocery rate matches the
    // constant that used to be hardcoded, which is the point: it is read
    // rather than assumed, so a state that varies it needs no code change.
    assert_eq!(highland.food_rate_ppm, Some(GROCERY_RATE_PPM));
    // The bug this column was added for: prepared food is taxed *above* the
    // general rate, so a $13.59 subtotal owes $1.15 and not the $1.01 that
    // 7.45% predicts.
    assert_eq!(highland.restaurant_rate_ppm, Some(84_500));

    assert!(
        !table
            .areas
            .iter()
            .any(|a| a.name.starts_with("INSTRUCTIONS")
                || a.name.starts_with('*')
                || a.name == "Location"),
        "a prose row was read as a jurisdiction"
    );
}

#[test]
fn the_effective_date_comes_out_of_the_files_own_header() {
    // Answers "is this the quarter I asked for?" — the publisher puts next
    // quarter's workbook up months early, so a URL alone does not settle it.
    let bytes = workbook_with(210, "Simplified", "Rates In effect as of July 1, 2026");
    let table = parse_workbook(&bytes, q3()).expect("parse");
    assert_eq!(
        table.effective_on,
        Some(chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
    );
}

#[test]
fn a_missing_effective_line_is_not_an_error() {
    // It is a printed nicety, not the data. Losing it must not cost a refresh.
    let bytes = workbook_with(210, "Simplified", "Rates for the third quarter");
    let table = parse_workbook(&bytes, q3()).expect("parse");
    assert_eq!(table.effective_on, None);
    assert!(table.areas.len() > 200, "the rates still parsed");
}

#[test]
fn columns_are_found_by_heading_not_by_letter() {
    // The rate lives in column Z today. A single column inserted upstream
    // would move it, and reading the neighbouring column would still *parse* —
    // a wrong rate installed silently is the failure this guards.
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("Simplified").unwrap();
    // A decoy "Food" one row above, exactly where the real stacked heading
    // puts one: a parser that matched headings across rows rather than within
    // a single row would pair this column with the grocery rate.
    sheet.write(9, 7, "Food").unwrap();
    sheet.write(10, 1, "Location").unwrap();
    sheet.write(10, 5, "Code").unwrap();
    sheet.write(10, 9, "Sales and Use").unwrap();
    sheet.write(10, 10, "Food").unwrap();
    sheet.write(10, 11, "(Restaurant)").unwrap();
    for i in 0..205 {
        let row = 11 + i;
        sheet.write(row, 1, format!("Town {i}")).unwrap();
        sheet.write(row, 5, format!("31-{i:03}")).unwrap();
        sheet.write(row, 7, 0.99).unwrap(); // a decoy under the stacked heading
        sheet.write(row, 8, 0.99).unwrap(); // a decoy in the old position
        sheet.write(row, 9, 0.081_25).unwrap();
        sheet.write(row, 10, 0.03).unwrap();
        sheet.write(row, 11, 0.091_25).unwrap();
    }
    let bytes = book.save_to_buffer().expect("build workbook");

    let table = parse_workbook(&bytes, q3()).expect("parse");
    assert_eq!(table.areas.len(), 205);
    assert!(
        table.areas.iter().all(|a| a.combined_rate_ppm == 81_250),
        "the decoy column was read"
    );
    assert!(
        table
            .areas
            .iter()
            .all(|a| a.restaurant_rate_ppm == Some(91_250)),
        "the prepared-food column was misread"
    );
}

#[test]
fn a_workbook_that_lost_the_sheet_says_what_it_had_instead() {
    let bytes = workbook_with(210, "Rates 2026", "Rates In effect as of July 1, 2026");
    let error = parse_workbook(&bytes, q3()).expect_err("should not parse");
    let message = error.to_string();
    assert!(
        message.contains("Simplified") && message.contains("Rates 2026"),
        "a shape change should name both what was wanted and what was there: {message}"
    );
}

#[test]
fn a_near_empty_result_is_refused_rather_than_installed() {
    // The dangerous failure is not a crash, it is a "successful" refresh that
    // empties the picker because the layout moved and the filter matched five
    // rows.
    let bytes = workbook_with(3, "Simplified", "Rates In effect as of July 1, 2026");
    let error = parse_workbook(&bytes, q3()).expect_err("should not parse");
    assert!(
        error.to_string().contains("expected hundreds"),
        "unexpected message: {error}"
    );
}

#[test]
fn something_that_is_not_a_spreadsheet_is_refused() {
    // The Tax Commission serves an HTML error page with a 200 for a file that
    // does not exist yet, so this is the ordinary path for asking too early.
    let error = parse_workbook(b"<html>Not Found</html>", q3()).expect_err("should not parse");
    assert!(
        error.to_string().contains("not a readable xlsx"),
        "unexpected message: {error}"
    );
}
