//! Tests for the `Money` newtype: parsing, formatting, and the exact-cent
//! allocation primitives every other money computation is built on.

use accounting_core::money::Money;
use proptest::prelude::*;

/// Cents in $10M — the outer bound of any amount this app must handle.
const TEN_MILLION_DOLLARS: i64 = 1_000_000_000;

fn sum_cents(v: &[Money]) -> i64 {
    v.iter().map(|m| m.cents()).sum()
}

fn signum_ok(amount: i64, part: Money) -> bool {
    match amount.signum() {
        1 => part.cents() >= 0,
        -1 => part.cents() <= 0,
        _ => part.cents() == 0,
    }
}

// ---------------------------------------------------------------------------
// allocate — the load-bearing primitive
// ---------------------------------------------------------------------------

proptest! {
    /// The single highest-value invariant in the project: an allocation never
    /// creates or destroys a cent. Proportional tax, discounts, budget
    /// contributions, and even splits are all `allocate` underneath, so this
    /// one property protects all of them at once.
    #[test]
    fn allocate_conserves_every_cent_exactly(
        amount in -TEN_MILLION_DOLLARS..=TEN_MILLION_DOLLARS,
        weights in prop::collection::vec(0i64..=1_000_000, 1..=20),
    ) {
        let money = Money::from_cents(amount);
        let parts = money.allocate(&weights);

        prop_assert_eq!(parts.len(), weights.len(), "one output row per weight");
        prop_assert_eq!(
            sum_cents(&parts),
            amount,
            "allocate must be exact, never approximate: {:?} over {:?}",
            amount,
            weights
        );

        let total_weight: i64 = weights.iter().sum();
        for (i, (w, part)) in weights.iter().zip(parts.iter()).enumerate() {
            // A zero-weight row is a row that participates in nothing (e.g. a
            // $0.00 subtotal in a split); it must never collect a stray penny
            // from the largest-remainder pass. Undefined when every weight is
            // zero, since then nothing can absorb the amount.
            if *w == 0 && total_weight > 0 {
                prop_assert_eq!(
                    part.cents(), 0,
                    "zero weight at index {} received {:?}", i, part
                );
            }
            prop_assert!(
                signum_ok(amount, *part),
                "part {:?} at index {} opposes the sign of {}", part, i, amount
            );
        }
    }

    #[test]
    fn allocate_is_monotone_in_weight(
        amount in 0i64..=TEN_MILLION_DOLLARS,
        weights in prop::collection::vec(0i64..=1_000_000, 1..=12),
    ) {
        // A bigger weight can never get a smaller share; largest-remainder
        // rounding may separate two rows by at most one cent.
        let parts = Money::from_cents(amount).allocate(&weights);
        for i in 0..weights.len() {
            for j in 0..weights.len() {
                if weights[i] > weights[j] {
                    prop_assert!(
                        parts[i].cents() + 1 >= parts[j].cents(),
                        "weight {} > {} but got {:?} < {:?}",
                        weights[i], weights[j], parts[i], parts[j]
                    );
                }
            }
        }
    }
}

#[test]
fn allocate_splits_proportionally_when_it_divides_evenly() {
    let parts = Money::from_cents(1000).allocate(&[1, 1, 2]);
    assert_eq!(
        parts,
        vec![
            Money::from_cents(250),
            Money::from_cents(250),
            Money::from_cents(500)
        ]
    );
}

#[test]
fn allocate_gives_the_odd_cent_to_the_largest_remainder() {
    // 100c over [1,1,1] is 33.33c each; the three remainders tie, so the
    // tie-break must be deterministic (earliest index first).
    let parts = Money::from_cents(100).allocate(&[1, 1, 1]);
    assert_eq!(sum_cents(&parts), 100);
    assert_eq!(
        parts,
        vec![
            Money::from_cents(34),
            Money::from_cents(33),
            Money::from_cents(33)
        ]
    );
}

#[test]
fn allocate_never_gives_a_zero_weight_row_a_penny() {
    let parts = Money::from_cents(100).allocate(&[1, 0, 1, 0]);
    assert_eq!(parts[1], Money::ZERO);
    assert_eq!(parts[3], Money::ZERO);
    assert_eq!(sum_cents(&parts), 100);
}

#[test]
fn allocate_preserves_sign_for_refunds() {
    let parts = Money::from_cents(-100).allocate(&[1, 1, 1]);
    assert_eq!(sum_cents(&parts), -100);
    assert!(parts.iter().all(|p| p.cents() <= 0));
}

#[test]
fn allocate_of_zero_is_all_zero() {
    let parts = Money::ZERO.allocate(&[5, 3, 2]);
    assert_eq!(parts, vec![Money::ZERO; 3]);
}

#[test]
fn allocate_with_all_zero_weights_still_conserves_the_total() {
    // Contract conflict: "every zero weight receives ZERO" and "the parts sum
    // to the amount" cannot both hold here. Conservation wins — losing money is
    // worse than an arbitrary-looking distribution.
    let parts = Money::from_cents(100).allocate(&[0, 0, 0]);
    assert_eq!(parts.len(), 3);
    assert_eq!(sum_cents(&parts), 100);
}

// ---------------------------------------------------------------------------
// split_evenly
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn split_evenly_parts_sum_to_the_whole(
        amount in -TEN_MILLION_DOLLARS..=TEN_MILLION_DOLLARS,
        n in 1usize..=50,
    ) {
        let parts = Money::from_cents(amount).split_evenly(n);
        prop_assert_eq!(parts.len(), n);
        prop_assert_eq!(sum_cents(&parts), amount);
        // Nobody's share may differ from anybody else's by more than a cent.
        let max = parts.iter().map(|m| m.cents()).max().unwrap();
        let min = parts.iter().map(|m| m.cents()).min().unwrap();
        prop_assert!(max - min <= 1, "shares {:?} differ by more than a cent", parts);
        for p in &parts {
            prop_assert!(signum_ok(amount, *p));
        }
    }
}

#[test]
fn split_evenly_gives_the_extra_cents_to_the_earliest_shares() {
    let parts = Money::from_cents(1000).split_evenly(3);
    assert_eq!(
        parts,
        vec![
            Money::from_cents(334),
            Money::from_cents(333),
            Money::from_cents(333)
        ]
    );
}

#[test]
fn split_evenly_of_a_refund_keeps_every_share_negative() {
    let parts = Money::from_cents(-1000).split_evenly(3);
    assert_eq!(sum_cents(&parts), -1000);
    assert!(parts.iter().all(|p| p.is_negative()));
}

// ---------------------------------------------------------------------------
// parse
// ---------------------------------------------------------------------------

#[test]
fn parse_accepts_the_shapes_people_actually_type() {
    let cases = [
        ("12.34", 1234),
        ("$1,234.5", 123450),
        (".50", 50),
        ("1234", 123400),
        ("-3.00", -300),
        ("  12.34  ", 1234),
        // Accounting-style negative.
        ("(4.00)", -400),
        ("0", 0),
        ("$0.00", 0),
    ];
    for (input, cents) in cases {
        assert_eq!(
            Money::parse(input).map(|m| m.cents()),
            Ok(cents),
            "parsing {input:?}"
        );
    }
}

#[test]
fn parse_rejects_garbage_and_sub_cent_precision() {
    // "1.005" is the important one: silently rounding a third decimal is how
    // money goes missing.
    let cases = ["1.005", "12.3.4", "", "   ", "abc", "99999999999999999999"];
    for input in cases {
        assert!(
            Money::parse(input).is_err(),
            "expected {input:?} to be rejected, got {:?}",
            Money::parse(input)
        );
    }
}

#[test]
fn from_str_matches_parse() {
    let via_trait: Money = "12.34".parse().unwrap();
    assert_eq!(via_trait, Money::parse("12.34").unwrap());
    assert!("nope".parse::<Money>().is_err());
}

proptest! {
    #[test]
    fn parse_round_trips_the_major_string(cents in -TEN_MILLION_DOLLARS..=TEN_MILLION_DOLLARS) {
        let m = Money::from_cents(cents);
        prop_assert_eq!(Money::parse(&m.to_major_string()).unwrap(), m);
    }

    /// Whatever `Display` renders must be re-readable by `parse`; a UI that
    /// shows a value and reads it back must not drift.
    #[test]
    fn parse_round_trips_display(cents in -TEN_MILLION_DOLLARS..=TEN_MILLION_DOLLARS) {
        let m = Money::from_cents(cents);
        prop_assert_eq!(Money::parse(&format!("{m}")).unwrap(), m);
        prop_assert_eq!(Money::parse(&m.to_display_string()).unwrap(), m);
    }
}

// ---------------------------------------------------------------------------
// mul_ppm / extract_ppm
// ---------------------------------------------------------------------------

#[test]
fn mul_ppm_rounds_half_away_from_zero() {
    // 2.5c must become 3c, not 2c — banker's rounding would fail here.
    assert_eq!(Money::from_cents(5).mul_ppm(500_000), Money::from_cents(3));
    assert_eq!(Money::from_cents(1).mul_ppm(500_000), Money::from_cents(1));
    assert_eq!(Money::from_cents(3).mul_ppm(500_000), Money::from_cents(2));
    assert_eq!(Money::from_cents(4).mul_ppm(125_000), Money::from_cents(1));
    // Just under the half stays down.
    assert_eq!(Money::from_cents(4).mul_ppm(124_999), Money::ZERO);
}

#[test]
fn mul_ppm_is_symmetric_around_zero() {
    for cents in [1i64, 3, 5, 7, 99, 1234, 100_000] {
        let pos = Money::from_cents(cents).mul_ppm(83_750);
        let neg = Money::from_cents(-cents).mul_ppm(83_750);
        assert_eq!(neg, -pos, "asymmetric rounding at {cents}c");
    }
}

#[test]
fn mul_ppm_of_zero_or_zero_rate_is_zero() {
    assert_eq!(Money::from_cents(12345).mul_ppm(0), Money::ZERO);
    assert_eq!(Money::ZERO.mul_ppm(83_750), Money::ZERO);
}

#[test]
fn mul_ppm_uses_a_wide_intermediate_and_does_not_overflow() {
    // $10M at 8.375% would overflow an i64 intermediate multiply only for far
    // larger inputs, but the i128 path must still be exact here.
    let m = Money::from_cents(TEN_MILLION_DOLLARS);
    assert_eq!(m.mul_ppm(83_750), Money::from_cents(83_750_000));
}

#[test]
fn extract_ppm_rounds_half_away_from_zero() {
    // x * 200_000 / 1_200_000 == x / 6: 15/6 = 2.5 must round to 3.
    assert_eq!(
        Money::from_cents(15).extract_ppm(200_000),
        Money::from_cents(3)
    );
    assert_eq!(
        Money::from_cents(3).extract_ppm(200_000),
        Money::from_cents(1)
    );
    assert_eq!(
        Money::from_cents(9).extract_ppm(200_000),
        Money::from_cents(2)
    );
    assert_eq!(
        Money::from_cents(-15).extract_ppm(200_000),
        Money::from_cents(-3)
    );
}

#[test]
fn extract_ppm_backs_tax_out_of_a_tax_inclusive_price() {
    // $108.375 inclusive of 8.375% is exactly $100.00 + $8.375 -> rounds to
    // $8.38 of tax on a $108.38 ticket.
    let total = Money::from_cents(10_838);
    let tax = total.extract_ppm(83_750);
    assert_eq!(tax, Money::from_cents(838));
    assert_eq!(total - tax, Money::from_cents(10_000));
}

#[test]
fn extract_ppm_is_symmetric_around_zero() {
    for cents in [1i64, 7, 15, 99, 10_838, 1_000_000] {
        let pos = Money::from_cents(cents).extract_ppm(83_750);
        let neg = Money::from_cents(-cents).extract_ppm(83_750);
        assert_eq!(neg, -pos, "asymmetric rounding at {cents}c");
    }
}

#[test]
fn extract_then_reconstruct_stays_within_one_cent() {
    // Backing tax out of a total and re-applying the rate to the remainder is
    // lossy by at most one cent; more drift than that means the two rounding
    // helpers disagree.
    for total_cents in 1..=20_000i64 {
        let total = Money::from_cents(total_cents);
        let tax = total.extract_ppm(83_750);
        let subtotal = total - tax;
        let reconstructed = subtotal + subtotal.mul_ppm(83_750);
        let drift = (reconstructed.cents() - total_cents).abs();
        assert!(
            drift <= 1,
            "total {total_cents}c drifted {drift}c (subtotal {subtotal:?}, tax {tax:?})"
        );
    }
}

proptest! {
    #[test]
    fn extract_ppm_never_exceeds_the_amount(
        cents in 0i64..=TEN_MILLION_DOLLARS,
        ppm in 0u32..=200_000,
    ) {
        let m = Money::from_cents(cents);
        let tax = m.extract_ppm(ppm);
        prop_assert!(tax.cents() >= 0);
        prop_assert!(tax <= m, "extracted {:?} from {:?}", tax, m);
    }
}

// ---------------------------------------------------------------------------
// arithmetic
// ---------------------------------------------------------------------------

#[test]
fn checked_add_returns_none_at_the_i64_ceiling() {
    let max = Money::from_cents(i64::MAX);
    assert_eq!(max.checked_add(Money::from_cents(1)), None);
    assert_eq!(
        max.checked_add(Money::ZERO),
        Some(Money::from_cents(i64::MAX))
    );
    let min = Money::from_cents(i64::MIN);
    assert_eq!(min.checked_add(Money::from_cents(-1)), None);
}

#[test]
fn checked_sub_returns_none_at_the_i64_floor() {
    let min = Money::from_cents(i64::MIN);
    assert_eq!(min.checked_sub(Money::from_cents(1)), None);
    assert_eq!(
        min.checked_sub(Money::ZERO),
        Some(Money::from_cents(i64::MIN))
    );
    let max = Money::from_cents(i64::MAX);
    assert_eq!(max.checked_sub(Money::from_cents(-1)), None);
}

#[test]
fn add_sub_neg_and_sum_agree_with_cent_arithmetic() {
    let a = Money::from_cents(1234);
    let b = Money::from_cents(-99);
    assert_eq!(a + b, Money::from_cents(1135));
    assert_eq!(a - b, Money::from_cents(1333));
    assert_eq!(-a, Money::from_cents(-1234));
    let total: Money = [a, b, Money::from_cents(1)].into_iter().sum();
    assert_eq!(total, Money::from_cents(1136));
    let empty: Money = std::iter::empty::<Money>().sum();
    assert_eq!(empty, Money::ZERO);
}

#[test]
fn abs_is_zero_and_is_negative_agree_with_the_sign() {
    assert_eq!(Money::from_cents(-500).abs(), Money::from_cents(500));
    assert_eq!(Money::from_cents(500).abs(), Money::from_cents(500));
    assert_eq!(Money::ZERO.abs(), Money::ZERO);
    assert!(Money::ZERO.is_zero());
    assert!(!Money::ZERO.is_negative());
    assert!(Money::from_cents(-1).is_negative());
    assert!(!Money::from_cents(1).is_negative());
}

#[test]
fn money_orders_by_cents() {
    let mut v = vec![
        Money::from_cents(5),
        Money::from_cents(-5),
        Money::ZERO,
        Money::from_cents(100),
    ];
    v.sort();
    assert_eq!(
        v,
        vec![
            Money::from_cents(-5),
            Money::ZERO,
            Money::from_cents(5),
            Money::from_cents(100)
        ]
    );
}

// ---------------------------------------------------------------------------
// formatting
// ---------------------------------------------------------------------------

#[test]
fn to_major_string_has_two_decimals_and_no_separators() {
    let cases = [
        (123_456, "1234.56"),
        (-123_456, "-1234.56"),
        (0, "0.00"),
        (5, "0.05"),
        (50, "0.50"),
        (-5, "-0.05"),
        (100, "1.00"),
        (1_000_000_000, "10000000.00"),
    ];
    for (cents, want) in cases {
        assert_eq!(Money::from_cents(cents).to_major_string(), want);
    }
}

#[test]
fn to_display_string_groups_thousands_and_marks_negatives() {
    let cases = [
        (123_456, "$1,234.56"),
        (-123_456, "-$1,234.56"),
        (0, "$0.00"),
        (5, "$0.05"),
        (-5, "-$0.05"),
        (99, "$0.99"),
        (100_000, "$1,000.00"),
        (1_000_000_000, "$10,000,000.00"),
    ];
    for (cents, want) in cases {
        assert_eq!(Money::from_cents(cents).to_display_string(), want);
    }
}

#[test]
fn to_excel_f64_carries_the_exact_value() {
    // Only exactly representable values are asserted; there is no epsilon in
    // this codebase and there will not be one.
    assert_eq!(Money::from_cents(100).to_excel_f64(), 1.0);
    assert_eq!(Money::from_cents(-25).to_excel_f64(), -0.25);
    assert_eq!(Money::ZERO.to_excel_f64(), 0.0);
    assert_eq!(Money::from_cents(12_550).to_excel_f64(), 125.5);
}

#[test]
fn money_serializes_transparently_as_cents() {
    assert_eq!(
        serde_json::to_string(&Money::from_cents(1234)).unwrap(),
        "1234"
    );
    assert_eq!(
        serde_json::from_str::<Money>("-99").unwrap(),
        Money::from_cents(-99)
    );
}

#[test]
fn const_constructors_are_usable_in_const_context() {
    const A: Money = Money::from_cents(500);
    const C: i64 = A.cents();
    assert_eq!(C, 500);
    assert_eq!(Money::ZERO.cents(), 0);
}
