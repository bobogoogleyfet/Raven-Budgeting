//! [`Money`] — the only representation of a monetary amount in this workspace.
//!
//! # Invariants
//!
//! * Amounts are **`i64` minor units** (USD cents). There is no `f64`
//!   anywhere in this module except [`Money::to_excel_f64`], which exists
//!   solely for the `rust_xlsxwriter` boundary in a later milestone.
//! * There is deliberately no `Mul<f64>`, no `From<f64>` and no `Div`.
//!   Scaling happens through [`Money::mul_ppm`] / [`Money::extract_ppm`]
//!   (parts-per-million rates, `i128` intermediates, half-away-from-zero) and
//!   splitting happens through [`Money::allocate`] / [`Money::split_evenly`],
//!   which are sum-preserving by construction.
//!
//! The single most important property in the crate:
//!
//! ```text
//! m.allocate(&w).iter().copied().sum::<Money>() == m      // for all m, all w
//! ```

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};
use core::str::FromStr;

use serde::{Deserialize, Serialize};

/// A USD amount stored as a whole number of cents.
///
/// `Money` is `#[serde(transparent)]`: it round-trips as a bare JSON integer
/// (`1234` == `$12.34`), never as a float and never as an object.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(i64);

/// Everything that can go wrong turning a human-typed string into [`Money`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MoneyParseError {
    /// The input was empty or contained only whitespace.
    #[error("amount is empty")]
    Empty,
    /// More than one `.` was present (e.g. `"12.3.4"`).
    #[error("amount `{0}` contains more than one decimal point")]
    MultipleDecimalPoints(String),
    /// More than two digits after the decimal point — cents are the smallest
    /// unit we represent, so this is data loss and is refused rather than
    /// silently rounded.
    #[error("amount `{input}` has {found} decimal places; at most 2 are supported")]
    TooManyDecimalPlaces { input: String, found: usize },
    /// A character that cannot appear in an amount.
    #[error("amount `{input}` contains unexpected character `{c}`")]
    InvalidCharacter { input: String, c: char },
    /// Structurally sign/symbol-only, or otherwise digit-free.
    #[error("amount `{0}` is not a number")]
    NotANumber(String),
    /// A `(` without a matching `)` (or vice versa).
    #[error("amount `{0}` has unbalanced parentheses")]
    UnbalancedParens(String),
    /// A thousands separator in an impossible position (`",5"`, `"1,,2"`, `"1,"`).
    #[error("amount `{0}` has a misplaced thousands separator")]
    MisplacedComma(String),
    /// The value does not fit in `i64` cents (roughly ±92 quadrillion dollars).
    #[error("amount `{0}` is out of range")]
    Overflow(String),
}

/// Integer division rounding half **away from zero**.
///
/// `den` must be strictly positive. Half-away-from-zero (rather than
/// banker's rounding) is what every US sales-tax authority and every
/// spreadsheet a user will check our numbers against does.
const fn div_round_half_away(num: i128, den: i128) -> i128 {
    debug_assert!(den > 0);
    let q = num / den;
    let r = num % den;
    // `r` carries the sign of `num`; `r.abs()` cannot overflow because
    // |r| < den <= i128::MAX.
    if r.abs() * 2 >= den {
        if num < 0 { q - 1 } else { q + 1 }
    } else {
        q
    }
}

/// Narrow an `i128` cent count back to `Money`, saturating at the `i64` bounds.
///
/// Every internal caller produces a value provably within `i64` range; the
/// saturation is a defensive backstop for absurd `ppm` inputs, not a normal
/// code path.
fn from_i128_saturating(v: i128) -> Money {
    match i64::try_from(v) {
        Ok(c) => Money(c),
        Err(_) if v < 0 => Money(i64::MIN),
        Err(_) => Money(i64::MAX),
    }
}

impl Money {
    /// Zero dollars.
    pub const ZERO: Money = Money(0);
    /// The largest representable amount.
    pub const MAX: Money = Money(i64::MAX);
    /// The smallest (most negative) representable amount.
    pub const MIN: Money = Money(i64::MIN);

    /// Build from a raw cent count.
    #[must_use]
    pub const fn from_cents(cents: i64) -> Self {
        Money(cents)
    }

    /// The raw cent count.
    #[must_use]
    pub const fn cents(self) -> i64 {
        self.0
    }

    /// Build from whole dollars, saturating rather than wrapping.
    #[must_use]
    pub const fn from_dollars(dollars: i64) -> Self {
        Money(dollars.saturating_mul(100))
    }

    /// `true` if the amount is exactly zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// `true` if the amount is strictly less than zero.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// `true` if the amount is strictly greater than zero.
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Absolute value. Saturates at [`Money::MAX`] for [`Money::MIN`] rather
    /// than panicking.
    #[must_use]
    pub const fn abs(self) -> Self {
        Money(self.0.saturating_abs())
    }

    /// Addition, `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, rhs: Money) -> Option<Money> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Some(Money(v)),
            None => None,
        }
    }

    /// Subtraction, `None` on overflow.
    #[must_use]
    pub const fn checked_sub(self, rhs: Money) -> Option<Money> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(Money(v)),
            None => None,
        }
    }

    /// Saturating addition.
    #[must_use]
    pub const fn saturating_add(self, rhs: Money) -> Money {
        Money(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction.
    #[must_use]
    pub const fn saturating_sub(self, rhs: Money) -> Money {
        Money(self.0.saturating_sub(rhs.0))
    }

    /// Multiply by a parts-per-million rate: `self * ppm / 1_000_000`.
    ///
    /// Used to compute tax *added on top of* a tax-exclusive price. The
    /// product is formed in `i128`, so no intermediate can overflow for any
    /// realistic amount, and the division rounds half away from zero.
    ///
    /// ```
    /// # use accounting_core::Money;
    /// // 8.375% of $54.32 -> $4.55 (4.5493 rounds up)
    /// assert_eq!(Money::from_cents(5432).mul_ppm(83_750), Money::from_cents(455));
    /// ```
    #[must_use]
    pub fn mul_ppm(self, ppm: u32) -> Money {
        let num = i128::from(self.0) * i128::from(ppm);
        from_i128_saturating(div_round_half_away(num, 1_000_000))
    }

    /// Back a tax amount *out of* a tax-inclusive total:
    /// `self * ppm / (1_000_000 + ppm)`.
    ///
    /// This is the inverse of [`Money::mul_ppm`] applied to the subtotal: if
    /// `X = S + S*rate`, then `X.extract_ppm(rate) ≈ X - S`. `i128`
    /// intermediates, half-away-from-zero rounding.
    #[must_use]
    pub fn extract_ppm(self, ppm: u32) -> Money {
        let num = i128::from(self.0) * i128::from(ppm);
        let den = 1_000_000_i128 + i128::from(ppm);
        from_i128_saturating(div_round_half_away(num, den))
    }

    /// Split this amount across `weights` using **largest-remainder
    /// (Hamilton) apportionment**.
    ///
    /// The returned vector always has `weights.len()` elements and always
    /// sums to exactly `self` — for positive amounts, negative amounts, zero
    /// weights, and an all-zero weight vector alike. This is the guarantee the
    /// rest of the codebase is built on: no penny is ever created or
    /// destroyed by a split.
    ///
    /// Algorithm: give each bucket the truncated-toward-zero exact share,
    /// then hand the `|leftover|` remaining units (each carrying the sign of
    /// the amount) to the buckets with the largest fractional remainders,
    /// **ties broken by lowest index** so the result is deterministic.
    ///
    /// # Edge cases
    ///
    /// * Empty `weights` → empty result.
    /// * `sum(weights) == 0` → falls back to [`Money::split_evenly`].
    /// * **Negative weights are clamped to zero** (documented, never
    ///   panics). A negative weight has no meaningful proportional
    ///   interpretation, and a panic here would take down the caller on
    ///   legitimate data such as a refund row; callers that need magnitude
    ///   weighting (e.g. [`crate::domain::tax::distribute_tax`]) pass
    ///   `.abs()` explicitly.
    /// * A bucket whose weight is zero always receives exactly `ZERO`; the
    ///   leftover units can only land on buckets with a non-zero remainder.
    #[must_use]
    pub fn allocate(self, weights: &[i64]) -> Vec<Money> {
        let n = weights.len();
        if n == 0 {
            return Vec::new();
        }

        let w: Vec<i128> = weights
            .iter()
            .map(|&x| if x < 0 { 0 } else { i128::from(x) })
            .collect();
        let total_w: i128 = w.iter().fold(0_i128, |acc, x| acc.saturating_add(*x));
        if total_w == 0 {
            return self.split_evenly(n);
        }

        let amount = i128::from(self.0);
        let mut shares: Vec<i128> = Vec::with_capacity(n);
        let mut remainders: Vec<i128> = Vec::with_capacity(n);
        let mut assigned: i128 = 0;

        for wi in &w {
            let num = amount * wi;
            let q = num / total_w; // truncated toward zero
            let r = (num - q * total_w).abs(); // fractional part, magnitude
            assigned += q;
            shares.push(q);
            remainders.push(r);
        }

        // `leftover` carries the sign of `amount` and |leftover| < n, because
        // every truncation moved the share toward zero by strictly less than
        // one unit.
        let leftover = amount - assigned;
        let step: i128 = if leftover < 0 { -1 } else { 1 };
        let units = usize::try_from(leftover.abs()).unwrap_or(n).min(n);

        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| remainders[b].cmp(&remainders[a]).then(a.cmp(&b)));
        for &i in order.iter().take(units) {
            shares[i] += step;
        }

        shares.into_iter().map(from_i128_saturating).collect()
    }

    /// Split into `n` as-equal-as-possible parts that sum to exactly `self`.
    ///
    /// The first `|self| % n` parts absorb the extra unit (in the sign of the
    /// amount), matching the lowest-index tie-break used by
    /// [`Money::allocate`]. `n == 0` yields an empty vector.
    #[must_use]
    pub fn split_evenly(self, n: usize) -> Vec<Money> {
        if n == 0 {
            return Vec::new();
        }
        let denom = i128::try_from(n).unwrap_or(i128::MAX);
        let amount = i128::from(self.0);
        let base = amount / denom;
        let rem = amount % denom; // sign of `amount`
        let step: i128 = if amount < 0 { -1 } else { 1 };
        let extra = usize::try_from(rem.abs()).unwrap_or(0);

        (0..n)
            .map(|i| {
                let v = if i < extra { base + step } else { base };
                from_i128_saturating(v)
            })
            .collect()
    }

    /// Machine-readable major-unit form: `"1234.56"`, `"-1234.56"`, `"0.00"`.
    ///
    /// No currency symbol and no thousands separators — this is the form used
    /// for CSV export and anywhere a value must round-trip through
    /// [`Money::parse`].
    #[must_use]
    pub fn to_major_string(self) -> String {
        let magnitude = self.0.unsigned_abs();
        let sign = if self.0 < 0 { "-" } else { "" };
        format!("{sign}{}.{:02}", magnitude / 100, magnitude % 100)
    }

    /// Human-readable form: `"$1,234.56"`, negatives as `"-$1,234.56"`.
    #[must_use]
    pub fn to_display_string(self) -> String {
        let magnitude = self.0.unsigned_abs();
        let sign = if self.0 < 0 { "-" } else { "" };
        let whole = group_thousands(magnitude / 100);
        format!("{sign}${whole}.{:02}", magnitude % 100)
    }

    /// The **only** `f64` in this crate: the xlsx writer takes doubles, and a
    /// cell must hold `1234.56` rather than `123456` so that spreadsheet
    /// formulas over the export behave. Never use this for arithmetic.
    #[must_use]
    #[expect(
        clippy::float_arithmetic,
        clippy::cast_precision_loss,
        reason = "documented xlsx boundary; the only f64 in the crate"
    )]
    pub fn to_excel_f64(self) -> f64 {
        self.0 as f64 / 100.0
    }

    /// Parse a human-typed amount.
    ///
    /// Accepted: `"12.34"`, `"$1,234.5"`, `".50"`, `"1234"`, `"-3.00"`,
    /// `"  12.34  "`, `"(4.00)"` (accounting negative), `"$-3"`, `"-$3"`.
    ///
    /// Rejected: empty/whitespace-only, more than two decimal places,
    /// more than one decimal point, non-numeric junk, misplaced thousands
    /// separators, unbalanced parentheses, and anything that overflows `i64`
    /// cents.
    pub fn parse(s: &str) -> Result<Money, MoneyParseError> {
        let original = s;
        let mut body = s.trim();
        if body.is_empty() {
            return Err(MoneyParseError::Empty);
        }

        // Accounting negatives: (4.00) == -4.00
        let mut negative = false;
        if let Some(inner) = body.strip_prefix('(') {
            let Some(inner) = inner.strip_suffix(')') else {
                return Err(MoneyParseError::UnbalancedParens(original.to_string()));
            };
            negative = true;
            body = inner.trim();
        } else if body.ends_with(')') {
            return Err(MoneyParseError::UnbalancedParens(original.to_string()));
        }

        // Sign and currency symbol, in either order, at most one of each.
        let mut seen_sign = false;
        let mut seen_symbol = false;
        loop {
            if let Some(rest) = body.strip_prefix('-') {
                if seen_sign {
                    return Err(MoneyParseError::NotANumber(original.to_string()));
                }
                seen_sign = true;
                negative = !negative;
                body = rest.trim_start();
            } else if let Some(rest) = body.strip_prefix('+') {
                if seen_sign {
                    return Err(MoneyParseError::NotANumber(original.to_string()));
                }
                seen_sign = true;
                body = rest.trim_start();
            } else if let Some(rest) = body.strip_prefix('$') {
                if seen_symbol {
                    return Err(MoneyParseError::NotANumber(original.to_string()));
                }
                seen_symbol = true;
                body = rest.trim_start();
            } else {
                break;
            }
        }

        if body.is_empty() {
            return Err(MoneyParseError::NotANumber(original.to_string()));
        }

        let mut parts = body.split('.');
        let int_part = parts.next().unwrap_or_default();
        let frac_part = parts.next().unwrap_or_default();
        if parts.next().is_some() {
            return Err(MoneyParseError::MultipleDecimalPoints(original.to_string()));
        }

        if int_part.starts_with(',') || int_part.ends_with(',') || int_part.contains(",,") {
            return Err(MoneyParseError::MisplacedComma(original.to_string()));
        }
        if frac_part.contains(',') {
            return Err(MoneyParseError::MisplacedComma(original.to_string()));
        }

        let mut digits_seen = 0_usize;
        let mut units: i128 = 0;
        for ch in int_part.chars() {
            if ch == ',' {
                continue;
            }
            let d = ch.to_digit(10).ok_or(MoneyParseError::InvalidCharacter {
                input: original.to_string(),
                c: ch,
            })?;
            digits_seen += 1;
            units = units
                .checked_mul(10)
                .and_then(|v| v.checked_add(i128::from(d)))
                .ok_or_else(|| MoneyParseError::Overflow(original.to_string()))?;
        }

        if frac_part.len() > 2 {
            return Err(MoneyParseError::TooManyDecimalPlaces {
                input: original.to_string(),
                found: frac_part.len(),
            });
        }
        let mut frac: i128 = 0;
        for ch in frac_part.chars() {
            let d = ch.to_digit(10).ok_or(MoneyParseError::InvalidCharacter {
                input: original.to_string(),
                c: ch,
            })?;
            digits_seen += 1;
            frac = frac * 10 + i128::from(d);
        }
        // ".5" is five *tenths* of a dollar, i.e. 50 cents.
        if frac_part.len() == 1 {
            frac *= 10;
        }

        if digits_seen == 0 {
            return Err(MoneyParseError::NotANumber(original.to_string()));
        }

        let magnitude = units
            .checked_mul(100)
            .and_then(|v| v.checked_add(frac))
            .ok_or_else(|| MoneyParseError::Overflow(original.to_string()))?;
        let signed = if negative { -magnitude } else { magnitude };
        let cents =
            i64::try_from(signed).map_err(|_| MoneyParseError::Overflow(original.to_string()))?;
        Ok(Money(cents))
    }
}

/// `1234567 -> "1,234,567"`.
fn group_thousands(mut n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let mut groups: Vec<String> = Vec::new();
    while n >= 1000 {
        groups.push(format!("{:03}", n % 1000));
        n /= 1000;
    }
    groups.push(n.to_string());
    groups.reverse();
    groups.join(",")
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Money({})", self.to_major_string())
    }
}

impl FromStr for Money {
    type Err = MoneyParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Money::parse(s)
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        self.checked_add(rhs).expect("Money addition overflowed")
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        self.checked_sub(rhs).expect("Money subtraction overflowed")
    }
}

impl Neg for Money {
    type Output = Money;
    fn neg(self) -> Money {
        Money(self.0.checked_neg().expect("Money negation overflowed"))
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, rhs: Money) {
        *self = *self + rhs;
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, rhs: Money) {
        *self = *self - rhs;
    }
}

impl Sum<Money> for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, |a, b| a + b)
    }
}

impl<'a> Sum<&'a Money> for Money {
    fn sum<I: Iterator<Item = &'a Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, |a, b| a + *b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(v: i64) -> Money {
        Money::from_cents(v)
    }

    fn total(v: &[Money]) -> Money {
        v.iter().copied().sum()
    }

    #[test]
    fn parse_accepts_the_documented_forms() {
        assert_eq!(Money::parse("12.34").unwrap(), c(1234));
        assert_eq!(Money::parse("$1,234.5").unwrap(), c(123_450));
        assert_eq!(Money::parse(".50").unwrap(), c(50));
        assert_eq!(Money::parse("1234").unwrap(), c(123_400));
        assert_eq!(Money::parse("-3.00").unwrap(), c(-300));
        assert_eq!(Money::parse("  12.34  ").unwrap(), c(1234));
        assert_eq!(Money::parse("(4.00)").unwrap(), c(-400));
        assert_eq!(Money::parse("-$3").unwrap(), c(-300));
        assert_eq!(Money::parse("$-3").unwrap(), c(-300));
        assert_eq!(Money::parse("+7.05").unwrap(), c(705));
        assert_eq!(Money::parse("0").unwrap(), Money::ZERO);
    }

    #[test]
    fn parse_rejects_junk() {
        assert!(matches!(Money::parse(""), Err(MoneyParseError::Empty)));
        assert!(matches!(Money::parse("   "), Err(MoneyParseError::Empty)));
        assert!(matches!(
            Money::parse("12.345"),
            Err(MoneyParseError::TooManyDecimalPlaces { .. })
        ));
        assert!(matches!(
            Money::parse("12.3.4"),
            Err(MoneyParseError::MultipleDecimalPoints(_))
        ));
        assert!(Money::parse("abc").is_err());
        assert!(Money::parse("12abc").is_err());
        assert!(Money::parse("$").is_err());
        assert!(Money::parse("-").is_err());
        assert!(Money::parse(".").is_err());
        assert!(matches!(
            Money::parse("(4.00"),
            Err(MoneyParseError::UnbalancedParens(_))
        ));
        assert!(matches!(
            Money::parse(",5"),
            Err(MoneyParseError::MisplacedComma(_))
        ));
        assert!(matches!(
            Money::parse("99999999999999999999"),
            Err(MoneyParseError::Overflow(_))
        ));
    }

    #[test]
    fn round_trips_through_major_string() {
        for v in [0_i64, 1, -1, 99, 100, -12_345, 123_456_789] {
            assert_eq!(Money::parse(&c(v).to_major_string()).unwrap(), c(v));
        }
    }

    #[test]
    fn display_formats() {
        assert_eq!(c(123_456).to_display_string(), "$1,234.56");
        assert_eq!(c(-123_456).to_display_string(), "-$1,234.56");
        assert_eq!(c(5).to_display_string(), "$0.05");
        assert_eq!(c(100_000_000).to_display_string(), "$1,000,000.00");
        assert_eq!(c(-5).to_major_string(), "-0.05");
    }

    #[test]
    fn ppm_rounds_half_away_from_zero() {
        // 0.5 cent exactly -> away from zero in both directions.
        assert_eq!(c(1).mul_ppm(500_000), c(1));
        assert_eq!(c(-1).mul_ppm(500_000), c(-1));
        assert_eq!(c(1).mul_ppm(499_999), Money::ZERO);
        // 8.375% of $54.32
        assert_eq!(c(5432).mul_ppm(83_750), c(455));
    }

    #[test]
    fn extract_ppm_is_the_inverse_of_mul_ppm() {
        let rate = 83_750;
        for subtotal in 1..2000_i64 {
            let s = c(subtotal);
            let t = s.mul_ppm(rate);
            let x = s + t;
            // Backing tax out of the inclusive total lands within a cent.
            let backed = x.extract_ppm(rate);
            assert!((backed - t).abs() <= c(1), "subtotal {subtotal}");
        }
    }

    #[test]
    fn allocate_always_preserves_the_total() {
        let cases: &[(i64, &[i64])] = &[
            (1000, &[1, 1, 1]),
            (-1000, &[1, 1, 1]),
            (1, &[1, 1, 1]),
            (-1, &[1, 1, 1]),
            (10_000, &[3333, 3333, 3334]),
            (100, &[0, 0, 5]),
            (100, &[0, 0, 0]),
            (0, &[7, 3]),
            (999, &[1]),
            (-7, &[5, 5, 5, 5, 5]),
            (12_345, &[1, 2, 3, 4, 5, 6, 7]),
        ];
        for (amount, weights) in cases {
            let parts = c(*amount).allocate(weights);
            assert_eq!(parts.len(), weights.len());
            assert_eq!(total(&parts), c(*amount), "{amount} over {weights:?}");
        }
    }

    #[test]
    fn allocate_is_empty_for_empty_weights() {
        assert!(c(500).allocate(&[]).is_empty());
    }

    #[test]
    fn allocate_zero_weight_rows_get_nothing() {
        let parts = c(1000).allocate(&[0, 1, 0, 1]);
        assert_eq!(parts, vec![Money::ZERO, c(500), Money::ZERO, c(500)]);
    }

    #[test]
    fn allocate_breaks_ties_by_lowest_index() {
        // $10.00 over three equal weights: 333,333,333 + 1 leftover cent,
        // all remainders equal, so index 0 wins.
        assert_eq!(c(1000).allocate(&[1, 1, 1]), vec![c(334), c(333), c(333)]);
        assert_eq!(
            c(-1000).allocate(&[1, 1, 1]),
            vec![c(-334), c(-333), c(-333)]
        );
    }

    #[test]
    fn allocate_negative_weights_are_clamped_not_panicking() {
        let parts = c(900).allocate(&[-5, 1, 2]);
        assert_eq!(total(&parts), c(900));
        assert_eq!(parts[0], Money::ZERO);
    }

    #[test]
    fn allocate_falls_back_to_even_split_when_weights_sum_to_zero() {
        assert_eq!(c(100).allocate(&[0, 0, 0]), c(100).split_evenly(3));
    }

    #[test]
    fn split_evenly_preserves_total() {
        for n in 1..12_usize {
            for amount in [-1001_i64, -7, 0, 1, 1000, 12_345] {
                let parts = c(amount).split_evenly(n);
                assert_eq!(parts.len(), n);
                assert_eq!(total(&parts), c(amount), "{amount} / {n}");
            }
        }
        assert!(c(100).split_evenly(0).is_empty());
    }

    #[test]
    fn thirty_three_thirty_three_never_loses_a_cent() {
        let parts = c(1000).allocate(&[3333, 3333, 3334]);
        assert_eq!(total(&parts), c(1000));
        assert_eq!(parts, vec![c(333), c(333), c(334)]);
    }

    #[test]
    fn sum_and_ops() {
        let v = [c(100), c(-40), c(5)];
        assert_eq!(v.iter().copied().sum::<Money>(), c(65));
        assert_eq!(v.iter().sum::<Money>(), c(65));
        assert_eq!(-c(5), c(-5));
        assert_eq!(c(5) - c(9), c(-4));
        assert_eq!(c(i64::MAX).checked_add(c(1)), None);
    }
}
