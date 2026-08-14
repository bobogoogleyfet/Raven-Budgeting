//! Calendar-month arithmetic.
//!
//! Budgets in this app are month-granular: a month is an indivisible bucket,
//! never pro-rated by days. February is worth exactly as much as March and a
//! leap year changes nothing. Everything here is pure calendar math over
//! values — nothing reads the clock.

use core::fmt;
use core::str::FromStr;

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

/// A calendar month, e.g. `2026-08`.
///
/// Ordering is chronological (`year` then `month`), which is what the derived
/// `Ord` gives us because the fields are declared in that order.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

/// Widen a 1-based month number for signed arithmetic. Months are always
/// small; a nonsensical value from a hand-built struct clamps to 1 rather
/// than wrapping.
fn month_as_i32(month: u32) -> i32 {
    i32::try_from(month).unwrap_or(1)
}

/// Failure parsing a `"YYYY-MM"` string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseYearMonthError {
    /// Not of the shape `YYYY-MM`.
    #[error("`{0}` is not a YYYY-MM month")]
    Malformed(String),
    /// Month outside `1..=12`.
    #[error("`{0}` is not a valid month number (expected 1..=12)")]
    MonthOutOfRange(u32),
}

impl YearMonth {
    /// Construct, validating `month` is in `1..=12`.
    #[must_use]
    pub const fn new(year: i32, month: u32) -> Option<Self> {
        if month >= 1 && month <= 12 {
            Some(YearMonth { year, month })
        } else {
            None
        }
    }

    /// The month a date falls in.
    #[must_use]
    pub fn from_date(date: NaiveDate) -> Self {
        YearMonth {
            year: date.year(),
            month: date.month(),
        }
    }

    /// First calendar day of the month.
    ///
    /// # Panics
    /// If `month` is not in `1..=12` (only reachable by building the struct
    /// through its public fields rather than [`YearMonth::new`]) or if the
    /// year is outside chrono's supported range.
    #[must_use]
    pub fn first_day(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 1).expect("valid year-month")
    }

    /// Last calendar day of the month (28/29/30/31 as appropriate).
    #[must_use]
    pub fn last_day(self) -> NaiveDate {
        self.next()
            .first_day()
            .pred_opt()
            .expect("month has a predecessor day")
    }

    /// The following month.
    #[must_use]
    pub fn next(self) -> Self {
        self.add_months(1)
    }

    /// The preceding month.
    #[must_use]
    pub fn prev(self) -> Self {
        self.add_months(-1)
    }

    /// Shift by a signed number of months.
    #[must_use]
    pub fn add_months(self, delta: i32) -> Self {
        let zero_based = self.year * 12 + (month_as_i32(self.month) - 1) + delta;
        YearMonth {
            year: zero_based.div_euclid(12),
            month: u32::try_from(zero_based.rem_euclid(12) + 1).unwrap_or(1),
        }
    }

    /// Signed month distance: `self - other`, in months.
    ///
    /// `2026-08.months_since(2026-08) == 0`, `2027-01.months_since(2026-11) == 2`.
    #[must_use]
    pub fn months_since(self, other: Self) -> i32 {
        (self.year - other.year) * 12 + (month_as_i32(self.month) - month_as_i32(other.month))
    }

    /// The quarter (1..=4) this month falls in.
    #[must_use]
    pub const fn quarter(self) -> u32 {
        quarter_of(self.month)
    }
}

impl fmt::Display for YearMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl FromStr for YearMonth {
    type Err = ParseYearMonthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let (y, m) = t
            .split_once('-')
            .ok_or_else(|| ParseYearMonthError::Malformed(s.to_string()))?;
        let year: i32 = y
            .parse()
            .map_err(|_| ParseYearMonthError::Malformed(s.to_string()))?;
        let month: u32 = m
            .parse()
            .map_err(|_| ParseYearMonthError::Malformed(s.to_string()))?;
        YearMonth::new(year, month).ok_or(ParseYearMonthError::MonthOutOfRange(month))
    }
}

/// An inclusive range of months, `start..=end`.
///
/// A range whose `end` precedes its `start` is empty rather than an error.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MonthRange {
    pub start: YearMonth,
    pub end: YearMonth,
}

impl MonthRange {
    /// Build an inclusive range.
    #[must_use]
    pub const fn new(start: YearMonth, end: YearMonth) -> Self {
        MonthRange { start, end }
    }

    /// A range covering exactly one month.
    #[must_use]
    pub const fn single(ym: YearMonth) -> Self {
        MonthRange { start: ym, end: ym }
    }

    /// Number of months covered; `0` if `end < start`.
    #[must_use]
    pub fn len(&self) -> usize {
        let span = self.end.months_since(self.start) + 1;
        usize::try_from(span).unwrap_or(0)
    }

    /// `true` when the range covers no months.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` when `ym` is within the (inclusive) range.
    #[must_use]
    pub fn contains(&self, ym: YearMonth) -> bool {
        ym >= self.start && ym <= self.end
    }

    /// Every month in the range, ascending.
    pub fn iter(&self) -> impl Iterator<Item = YearMonth> + use<> {
        let start = self.start;
        let n = self.len();
        (0..n).map(move |i| start.add_months(i32::try_from(i).unwrap_or(i32::MAX)))
    }
}

impl IntoIterator for MonthRange {
    type Item = YearMonth;
    type IntoIter = std::vec::IntoIter<YearMonth>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter().collect::<Vec<_>>().into_iter()
    }
}

/// The quarter (1..=4) a 1-based month number falls in.
///
/// Out-of-range input is clamped so this can never panic in a formatter.
#[must_use]
pub const fn quarter_of(month: u32) -> u32 {
    match month {
        1..=3 => 1,
        4..=6 => 2,
        7..=9 => 3,
        10..=12 => 4,
        _ => 4,
    }
}

/// A calendar quarter, e.g. `2026Q3`.
///
/// Sales-tax rates are published and revised on this cycle, not monthly, so
/// "which rates are these?" is a question with a quarter for an answer. Like
/// everything else here it is pure calendar math — deciding which quarter is
/// *current* needs today's date, which the caller supplies.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Quarter {
    pub year: i32,
    /// 1..=4.
    pub quarter: u32,
}

impl Quarter {
    /// Construct, validating `quarter` is in `1..=4`.
    #[must_use]
    pub const fn new(year: i32, quarter: u32) -> Option<Self> {
        if quarter >= 1 && quarter <= 4 {
            Some(Quarter { year, quarter })
        } else {
            None
        }
    }

    /// The quarter a date falls in.
    #[must_use]
    pub fn from_date(date: NaiveDate) -> Self {
        Quarter {
            year: date.year(),
            quarter: quarter_of(date.month()),
        }
    }

    /// The first month of the quarter: January, April, July or October.
    #[must_use]
    pub fn first_month(self) -> YearMonth {
        let month = (self.quarter.clamp(1, 4) - 1) * 3 + 1;
        YearMonth {
            year: self.year,
            month,
        }
    }

    /// The day the quarter's rates take effect.
    #[must_use]
    pub fn effective_on(self) -> NaiveDate {
        self.first_month().first_day()
    }

    /// The following quarter.
    #[must_use]
    pub fn next(self) -> Self {
        Self::from_month(self.first_month().add_months(3))
    }

    /// The preceding quarter.
    #[must_use]
    pub fn prev(self) -> Self {
        Self::from_month(self.first_month().add_months(-3))
    }

    /// The quarter a month falls in.
    #[must_use]
    pub fn from_month(ym: YearMonth) -> Self {
        Quarter {
            year: ym.year,
            quarter: ym.quarter(),
        }
    }
}

impl fmt::Display for Quarter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}Q{}", self.year, self.quarter)
    }
}

/// Three-letter English month abbreviation; `"???"` for out-of-range input.
#[must_use]
pub const fn month_short_name(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ym(y: i32, m: u32) -> YearMonth {
        YearMonth::new(y, m).expect("valid month")
    }

    #[test]
    fn new_validates_the_month() {
        assert!(YearMonth::new(2026, 0).is_none());
        assert!(YearMonth::new(2026, 13).is_none());
        assert_eq!(ym(2026, 8).to_string(), "2026-08");
    }

    #[test]
    fn day_boundaries() {
        assert_eq!(
            ym(2026, 2).last_day(),
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(
            ym(2024, 2).last_day(),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );
        assert_eq!(
            ym(2026, 12).last_day(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
        );
        assert_eq!(
            ym(2026, 8).first_day(),
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
        );
    }

    #[test]
    fn month_arithmetic_wraps_years() {
        assert_eq!(ym(2026, 12).next(), ym(2027, 1));
        assert_eq!(ym(2026, 1).prev(), ym(2025, 12));
        assert_eq!(ym(2026, 8).add_months(-20), ym(2024, 12));
        assert_eq!(ym(2026, 8).add_months(0), ym(2026, 8));
        assert_eq!(ym(2027, 1).months_since(ym(2026, 11)), 2);
        assert_eq!(ym(2026, 11).months_since(ym(2027, 1)), -2);
    }

    #[test]
    fn round_trip_add_months_and_months_since() {
        let base = ym(2026, 8);
        for d in -40..40 {
            assert_eq!(base.add_months(d).months_since(base), d);
        }
    }

    #[test]
    fn parses_and_orders() {
        assert_eq!("2026-08".parse::<YearMonth>().unwrap(), ym(2026, 8));
        assert!("2026-13".parse::<YearMonth>().is_err());
        assert!("nope".parse::<YearMonth>().is_err());
        assert!(ym(2026, 1) < ym(2026, 2));
        assert!(ym(2025, 12) < ym(2026, 1));
    }

    #[test]
    fn ranges_iterate_inclusively() {
        let r = MonthRange::new(ym(2026, 11), ym(2027, 2));
        assert_eq!(r.len(), 4);
        assert_eq!(
            r.iter().collect::<Vec<_>>(),
            vec![ym(2026, 11), ym(2026, 12), ym(2027, 1), ym(2027, 2)]
        );
        assert!(r.contains(ym(2027, 1)));
        assert!(!r.contains(ym(2027, 3)));

        let empty = MonthRange::new(ym(2027, 2), ym(2026, 11));
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
        assert_eq!(empty.iter().count(), 0);
    }

    #[test]
    fn helpers() {
        assert_eq!(quarter_of(1), 1);
        assert_eq!(quarter_of(7), 3);
        assert_eq!(quarter_of(12), 4);
        assert_eq!(month_short_name(1), "Jan");
        assert_eq!(month_short_name(12), "Dec");
    }
}
