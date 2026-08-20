//! Utah's published sales-tax rates, and the quarterly file they come from.
//!
//! # Why the rates are downloaded rather than typed
//!
//! The Utah State Tax Commission publishes every taxing jurisdiction in the
//! state — city, county, and the special districts stacked on top — and revises
//! the list on the first day of each quarter. Rates move: a transit district
//! passes, a city adds a local option, and the combined figure for one town
//! changes by a quarter of a percent.
//!
//! A list compiled by hand is therefore not merely incomplete, it is wrong on a
//! schedule, and only a new release of this app could fix it. Reading the
//! state's own file makes a rate change a download instead of a release, which
//! is the only way an app can still be right in April.
//!
//! Source: <https://tax.utah.gov/business/sales-tax/sales/rates/>
//!
//! # Utah only, on purpose
//!
//! Every rate here comes out of one state's publication, in one state's file
//! format. Nothing about that generalises: another state means another
//! publisher, another schedule and another layout, so pretending otherwise
//! would mean a plausible-looking abstraction over a single case. The table
//! [`crate::repo::tax_areas`] stores is state-agnostic; this module is not, and
//! it says so in its name.
//!
//! # These are still estimates
//!
//! The rule the rest of the app runs on does not change: the receipt always
//! wins. A published rate prefills a location, and every transaction records
//! the rate actually used at entry time, so a refresh never rewrites history —
//! a receipt entered in March keeps March's rate however many quarters land on
//! top of it.

mod workbook;

use chrono::NaiveDate;

use crate::error::{CoreError, Result};
use crate::time::Quarter;

/// Utah's combined rate on unprepared food: 1.75% state plus 1.25% local.
///
/// Flat statewide today, and now read from the publication rather than assumed
/// — the Simplified chart carries a Grocery Food column per jurisdiction, so a
/// state that ever varies it by locality needs no code change. This remains as
/// the fallback for a row whose grocery cell is missing or unreadable, which is
/// the one case where a constant beats dropping the jurisdiction entirely.
pub const GROCERY_RATE_PPM: u32 = 30_000;

/// The tab holding one row per jurisdiction with every rate that applies to it.
///
/// The `Simplified` chart rather than `Combined Rate`, which was read until
/// M7. Both publish the same 389 jurisdictions and the same combined figure,
/// but the combined sheet carries *only* that figure: the grocery rate had to
/// be a hardcoded constant, and the prepared-food rate was simply absent, so
/// every restaurant receipt was taxed 1% light. This sheet has all three in
/// adjacent columns.
const SIMPLIFIED_SHEET: &str = "Simplified";

/// One taxing jurisdiction and what it charges.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TaxArea {
    /// Utah's "Cnty/City Code", e.g. `25-029`. The stable identity: names get
    /// re-spelled between quarters, codes do not.
    pub code: String,
    pub name: String,
    /// Parts per million: 7.25% is `72_500`.
    pub combined_rate_ppm: u32,
    /// The reduced rate on unprepared food, where the state has one.
    pub food_rate_ppm: Option<u32>,
    /// The rate on prepared food, where the county levies a restaurant tax.
    ///
    /// The *whole* rate as published, not the increment over
    /// [`Self::combined_rate_ppm`] — that is the figure the chart prints, and
    /// deriving the increment here would disagree with the source the quarter
    /// a county moved both.
    pub restaurant_rate_ppm: Option<u32>,
}

/// One quarter's publication, parsed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RateTable {
    pub quarter: Quarter,
    /// The date printed in the file's own header, when it carries one.
    ///
    /// Checked against the quarter that was asked for. The Tax Commission
    /// serves a friendly HTML error page with a 200 for a file that does not
    /// exist yet, and publishes next quarter's workbook months early, so
    /// "did I get the file I asked for" is a real question with a real wrong
    /// answer.
    pub effective_on: Option<NaiveDate>,
    pub areas: Vec<TaxArea>,
}

/// The quarter whose rates apply on a given day.
#[must_use]
pub fn quarter_for(today: NaiveDate) -> Quarter {
    Quarter::from_date(today)
}

/// Where a quarter's rate workbook lives.
///
/// `2026Q3` → `.../26q3simple.xlsx`. The two-digit year and the `simple`
/// suffix are both the publisher's convention, not choices made here.
#[must_use]
pub fn workbook_url(quarter: Quarter) -> String {
    format!(
        "https://files.tax.utah.gov/tax/salestax/rate/{:02}q{}simple.xlsx",
        quarter.year.rem_euclid(100),
        quarter.quarter
    )
}

/// The table shipped with the app, for a machine that has never been online.
///
/// A new install must be able to tax a receipt before it has downloaded
/// anything: the first run may well be on a phone in a checkout queue. This is
/// the same data in the same shape, just fixed at the quarter it was built.
///
/// Regenerate with `cargo run -p accounting-core --example snapshot` and bump
/// [`BUNDLED_QUARTER`] to match.
///
/// # Panics
/// Never in a shipped build: the file is compiled in and a malformed one fails
/// [`bundled_is_parseable`](self) in the test suite.
#[must_use]
pub fn bundled() -> RateTable {
    parse_snapshot(BUNDLED_CSV, BUNDLED_QUARTER).expect("the bundled snapshot is valid")
}

/// The quarter [`bundled`] was built from.
pub const BUNDLED_QUARTER: Quarter = Quarter {
    year: 2026,
    quarter: 3,
};

const BUNDLED_CSV: &str = include_str!("utah_2026q3.csv");

/// Read a quarter's workbook, as downloaded.
///
/// `quarter` is what the caller asked the publisher for; it is not read from
/// the file, because a file that turned out to be the wrong quarter would then
/// happily relabel itself.
pub fn parse_workbook(bytes: &[u8], quarter: Quarter) -> Result<RateTable> {
    let sheet = workbook::read_sheet(bytes, SIMPLIFIED_SHEET)?;
    let header = find_header(&sheet)?;
    let mut areas = Vec::new();
    let mut grocery_seen = 0usize;
    let mut prepared_seen = 0usize;

    for row in &sheet {
        let (Some(name), Some(code), Some(rate)) = (
            row.get(&header.name).map(|s| s.trim()),
            row.get(&header.code).map(|s| s.trim()),
            row.get(&header.combined),
        ) else {
            continue;
        };
        // The sheet is a printed document: a title block, instructions, and
        // footnotes share the grid with the data. A row is a jurisdiction only
        // if it carries a code shaped like one, which no prose row does.
        if !is_area_code(code) || name.is_empty() {
            continue;
        }
        // One torn cell is not worth dropping a jurisdiction over, so both of
        // these are lenient per row — but leniency per row is exactly how a
        // *column* that moved gets absorbed silently, so the counts are
        // checked in aggregate below.
        let grocery = row.get(&header.grocery).and_then(|v| rate_to_ppm(v).ok());
        let prepared = row.get(&header.prepared).and_then(|v| rate_to_ppm(v).ok());
        grocery_seen += usize::from(grocery.is_some());
        prepared_seen += usize::from(prepared.is_some());

        areas.push(TaxArea {
            code: code.to_string(),
            name: name.to_string(),
            combined_rate_ppm: rate_to_ppm(rate)?,
            // Falling back to the constant rather than to None on an isolated
            // torn row: None would quietly re-tax food at the general rate,
            // and the aggregate guard below already catches the case where the
            // whole column is gone.
            food_rate_ppm: Some(grocery.unwrap_or(GROCERY_RATE_PPM)),
            restaurant_rate_ppm: prepared,
        });
    }

    if areas.len() < MINIMUM_PLAUSIBLE_AREAS {
        // Utah has 29 counties and some 250 municipalities. A handful of rows
        // means the layout moved and the filter above quietly matched almost
        // nothing — which would otherwise install as a "successful" refresh
        // that empties the picker.
        return Err(CoreError::RateData(format!(
            "only {} jurisdictions in the {quarter} workbook; expected hundreds",
            areas.len()
        )));
    }
    // The same guard as the row count, applied to the two columns added after
    // the combined rate. Per-row leniency is what makes them absorb a moved
    // column silently: every cell fails, every row falls back, the parse
    // "succeeds", and the app goes back to taxing groceries at a hardcoded
    // constant and prepared food at the general rate — with nothing to show
    // for it. A column that parsed for not one jurisdiction out of hundreds is
    // a layout change, not a state that stopped publishing.
    for (column, seen) in [
        ("Grocery Food", grocery_seen),
        ("Prepared Food (Restaurant)", prepared_seen),
    ] {
        if seen == 0 {
            return Err(CoreError::RateData(format!(
                "the {column} column of the {quarter} workbook parsed no rates \
                 in {} jurisdictions; the layout has moved",
                areas.len()
            )));
        }
    }

    Ok(RateTable {
        quarter,
        effective_on: find_effective_date(&sheet),
        areas,
    })
}

/// Below this, a parse has found a layout it does not understand.
const MINIMUM_PLAUSIBLE_AREAS: usize = 200;

/// Which columns hold the fields, found by their headings.
struct Header {
    name: String,
    code: String,
    combined: String,
    grocery: String,
    prepared: String,
}

/// Locate the header row and the five columns worth reading.
///
/// By heading text rather than by letter. Column D is the combined rate today,
/// and a single inserted column in a hand-maintained workbook would silently
/// turn it into the grocery rate — a filter that reads the wrong column and
/// still parses is the failure worth engineering against, and here the wrong
/// column is *also* a plausible-looking tax rate.
///
/// The chart stacks its headings over three rows, so each column is named by
/// its own last line: `Combined / Sales and Use`, `Grocery / Food`, `Prepared
/// Food / (Restaurant)`. Matching on those last lines is what makes one row
/// carry all five. `Food` is ambiguous across the whole sheet — it appears in
/// both the grocery and prepared-food stacks — but not *within* the bottom
/// row, which is the only row this looks at.
fn find_header(sheet: &workbook::Sheet) -> Result<Header> {
    for row in sheet {
        // Exactly one match, or none. `Food` is a bare one-word heading in a
        // hand-maintained printed chart, and a second cell reading exactly
        // `Food` — a legend, a footnote column — would otherwise bind
        // whichever letter sorts first in the map. That order is
        // lexicographic on the column *letter*, so past column Z it is not
        // even left-to-right ("AA" < "B"): the parser would pick an arbitrary
        // column, still parse, and install a wrong rate silently. Refusing an
        // ambiguous heading is the same trade as `MINIMUM_PLAUSIBLE_AREAS`.
        let find = |wanted: &str| {
            let mut hits = row
                .iter()
                .filter(|(_, v)| v.trim().eq_ignore_ascii_case(wanted))
                .map(|(col, _)| col.clone());
            match (hits.next(), hits.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        };
        if let (Some(name), Some(code)) = (find("Location"), find("Code"))
            && let (Some(combined), Some(grocery), Some(prepared)) =
                (find("Sales and Use"), find("Food"), find("(Restaurant)"))
        {
            return Ok(Header {
                name,
                code,
                combined,
                grocery,
                prepared,
            });
        }
    }
    Err(CoreError::RateData(
        "no header row with exactly one each of Location, Code, Sales and Use, Food \
         and (Restaurant)"
            .to_string(),
    ))
}

/// The "Rates In effect as of July 1, 2026" line, if the file still carries it.
fn find_effective_date(sheet: &workbook::Sheet) -> Option<NaiveDate> {
    const PREFIX: &str = "rates in effect as of";
    for row in sheet {
        for value in row.values() {
            let text = value.trim();
            if text.len() > PREFIX.len() && text[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
                let date = text[PREFIX.len()..].trim().trim_end_matches('.');
                // "%B %e" takes both "July 1" and "July  1"; the publisher has
                // used both.
                if let Ok(parsed) = NaiveDate::parse_from_str(date, "%B %e, %Y") {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

/// `01-000`, `25-029`: two digits, a dash, three digits.
fn is_area_code(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 6
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[2] == b'-'
        && bytes[3..].iter().all(u8::is_ascii_digit)
}

/// `"7.2500000000000009E-2"` → `72_500` ppm.
///
/// The one float in the crate, and it is here because the publisher stores a
/// fraction rather than a percentage — the value arrives as a double and there
/// is no exact decimal to recover. It is converted once, at the boundary, and
/// everything downstream is the integer. Money never passes through here.
#[allow(
    clippy::float_arithmetic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn rate_to_ppm(text: &str) -> Result<u32> {
    let fraction: f64 = text
        .trim()
        .parse()
        .map_err(|_| CoreError::RateData(format!("{text:?} is not a rate")))?;
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        return Err(CoreError::RateData(format!(
            "{fraction} is not a sales-tax rate between 0% and 100%"
        )));
    }
    // Rounded, not truncated: 0.0725 is stored as 7.2500000000000009E-2 and
    // truncating gives 72_499 — a rate one part per million light, on every
    // row, forever.
    Ok((fraction * 1_000_000.0).round() as u32)
}

/// Read the bundled CSV:
/// `code,name,combined_rate_ppm,food_rate_ppm,restaurant_rate_ppm`.
fn parse_snapshot(csv_text: &str, quarter: Quarter) -> Result<RateTable> {
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .from_reader(csv_text.as_bytes());
    let mut areas = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| CoreError::RateData(format!("snapshot: {e}")))?;
        let field = |i: usize| -> Result<&str> {
            record
                .get(i)
                .ok_or_else(|| CoreError::RateData(format!("snapshot row {i} is short")))
        };
        let number = |raw: &str| -> Result<u32> {
            raw.trim()
                .parse()
                .map_err(|_| CoreError::RateData(format!("snapshot: {raw:?} is not a rate")))
        };
        // An optional trailing column: a snapshot taken before M7 has four
        // fields, and reading it as "no prepared-food rate published" is both
        // true of that file and the safe reading of it.
        let optional = |i: usize| -> Result<Option<u32>> {
            match record.get(i).map(str::trim) {
                None | Some("") => Ok(None),
                Some(raw) => Ok(Some(number(raw)?)),
            }
        };
        areas.push(TaxArea {
            code: field(0)?.trim().to_string(),
            name: field(1)?.trim().to_string(),
            combined_rate_ppm: number(field(2)?)?,
            food_rate_ppm: optional(3)?,
            restaurant_rate_ppm: optional(4)?,
        });
    }
    Ok(RateTable {
        quarter,
        effective_on: Some(quarter.effective_on()),
        areas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_maps_to_the_publishers_file_name() {
        let q = Quarter::new(2026, 3).expect("valid");
        assert_eq!(
            workbook_url(q),
            "https://files.tax.utah.gov/tax/salestax/rate/26q3simple.xlsx"
        );
        let q1 = Quarter::new(2027, 1).expect("valid");
        assert!(workbook_url(q1).ends_with("27q1simple.xlsx"));
    }

    #[test]
    fn a_fraction_becomes_parts_per_million() {
        // The literals the publisher actually stores, doubles and all.
        assert_eq!(rate_to_ppm("7.2500000000000009E-2").expect("parse"), 72_500);
        assert_eq!(rate_to_ppm("7.0000000000000007E-2").expect("parse"), 70_000);
        assert_eq!(rate_to_ppm("0.0885").expect("parse"), 88_500);
        assert_eq!(rate_to_ppm("0").expect("parse"), 0);
    }

    #[test]
    fn a_percentage_mistaken_for_a_fraction_is_refused() {
        // 7.25 as a *fraction* is 725% — the sign of a column read as a rate
        // that is not one, and worth failing on rather than storing.
        assert!(rate_to_ppm("7.25").is_err());
        assert!(rate_to_ppm("").is_err());
        assert!(rate_to_ppm("n/a").is_err());
    }

    #[test]
    fn only_a_real_jurisdiction_code_counts_as_data() {
        assert!(is_area_code("01-000"));
        assert!(is_area_code("25-029"));
        assert!(!is_area_code("Location"));
        assert!(!is_area_code("1-000"));
        assert!(!is_area_code("01-0000"));
        assert!(!is_area_code(""));
    }

    #[test]
    fn bundled_is_parseable() {
        let table = bundled();
        assert_eq!(table.quarter, BUNDLED_QUARTER);
        assert!(
            table.areas.len() >= MINIMUM_PLAUSIBLE_AREAS,
            "only {} areas bundled",
            table.areas.len()
        );
    }

    #[test]
    fn every_bundled_rate_is_a_plausible_sales_tax() {
        for area in bundled().areas {
            // Not a check of the figure, which only the publisher can settle —
            // a guard against a decimal point that moved in transcription.
            assert!(
                (10_000..=120_000).contains(&area.combined_rate_ppm),
                "{} has an implausible rate: {} ppm",
                area.name,
                area.combined_rate_ppm
            );
            assert_eq!(
                area.food_rate_ppm,
                Some(GROCERY_RATE_PPM),
                "{} lost Utah's flat grocery rate",
                area.name
            );
        }
    }

    #[test]
    fn bundled_codes_are_unique() {
        // The code is the identity a refresh matches on, so a duplicate would
        // make one jurisdiction overwrite another every quarter.
        let mut seen = std::collections::HashSet::new();
        for area in bundled().areas {
            assert!(
                seen.insert(area.code.clone()),
                "duplicate code {}",
                area.code
            );
        }
    }

    #[test]
    fn the_places_this_install_shops_are_present() {
        let table = bundled();
        let by_name = |wanted: &str| table.areas.iter().find(|a| a.name == wanted).cloned();
        // Spot-checks against the published Q3 2026 figures. If a refresh or a
        // regenerated snapshot moves these, that is a real change and this test
        // is where it gets noticed.
        //
        // Both were also wrong in the hand-written table these replaced —
        // Highland was 7.25% there and Salt Lake City 7.75%. Nobody had
        // mistyped them; the rates simply moved after they were written down,
        // which is the argument for downloading them in one line.
        let highland = by_name("Highland").expect("Highland is in the table");
        assert_eq!(highland.combined_rate_ppm, 74_500);
        let slc = by_name("Salt Lake City").expect("Salt Lake City is in the table");
        assert_eq!(slc.combined_rate_ppm, 84_500);
    }
}
