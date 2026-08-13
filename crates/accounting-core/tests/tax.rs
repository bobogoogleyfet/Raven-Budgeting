//! Tests for sales-tax rate resolution and for spreading a transaction-level
//! tax or discount across splits without creating or destroying a penny.

use std::collections::HashMap;

use accounting_core::domain::reconcile::{AmountEntry, TaxContext, reconcile};
use accounting_core::domain::tax::{distribute_discount, distribute_tax, resolve_rate};
use accounting_core::ids::CategoryId;
use accounting_core::money::Money;

/// 8.375% — a real US combined rate chosen because its pennies are awkward.
const RATE: u32 = 83_750;

const GROCERIES: CategoryId = CategoryId::new(1);
const DINING: CategoryId = CategoryId::new(2);
const FUEL: CategoryId = CategoryId::new(3);

fn cents(v: &[Money]) -> Vec<i64> {
    v.iter().map(|m| m.cents()).collect()
}

fn sum(v: &[Money]) -> i64 {
    v.iter().map(|m| m.cents()).sum()
}

// ---------------------------------------------------------------------------
// Rate sweep
// ---------------------------------------------------------------------------

#[test]
fn every_total_from_one_cent_to_two_hundred_dollars_reconciles_exactly() {
    // Exhaustive, not sampled: a single total in this range that fails to
    // balance is a wrong number on a real receipt.
    let ctx = TaxContext {
        rate_ppm: RATE,
        prices_include_tax: false,
    };
    for total_cents in 1..=20_000i64 {
        let entry = AmountEntry {
            total: Some(Money::from_cents(total_cents)),
            ..Default::default()
        };
        let a = reconcile(&entry, &ctx).unwrap();
        assert_eq!(
            a.subtotal + a.tax,
            a.total,
            "total {total_cents}c: {:?} + {:?} != {:?}",
            a.subtotal,
            a.tax,
            a.total
        );
        assert_eq!(a.total.cents(), total_cents);
        assert!(a.tax.cents() >= 0, "negative tax on a positive total");
    }
}

#[test]
fn every_subtotal_from_one_cent_to_two_hundred_dollars_reconciles_exactly() {
    let ctx = TaxContext {
        rate_ppm: RATE,
        prices_include_tax: false,
    };
    for subtotal_cents in 1..=20_000i64 {
        let entry = AmountEntry {
            subtotal: Some(Money::from_cents(subtotal_cents)),
            ..Default::default()
        };
        let a = reconcile(&entry, &ctx).unwrap();
        assert_eq!(a.subtotal + a.tax, a.total, "subtotal {subtotal_cents}c");
        assert_eq!(a.subtotal.cents(), subtotal_cents);
    }
}

#[test]
fn reconciling_a_total_then_re_adding_the_rate_drifts_at_most_one_cent() {
    let ctx = TaxContext {
        rate_ppm: RATE,
        prices_include_tax: false,
    };
    for total_cents in 1..=20_000i64 {
        let a = reconcile(
            &AmountEntry {
                total: Some(Money::from_cents(total_cents)),
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
        let b = reconcile(
            &AmountEntry {
                subtotal: Some(a.subtotal),
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
        let drift = (b.total.cents() - total_cents).abs();
        assert!(
            drift <= 1,
            "total {total_cents}c round-tripped to {:?}",
            b.total
        );
    }
}

// ---------------------------------------------------------------------------
// resolve_rate
// ---------------------------------------------------------------------------

#[test]
fn a_category_override_beats_the_location_rate() {
    let mut overrides = HashMap::new();
    overrides.insert(GROCERIES, 0);
    overrides.insert(DINING, 100_000);
    assert_eq!(resolve_rate(RATE, &overrides, Some(DINING)), 100_000);
}

#[test]
fn an_uncategorized_amount_falls_back_to_the_location_rate() {
    let mut overrides = HashMap::new();
    overrides.insert(GROCERIES, 0);
    assert_eq!(resolve_rate(RATE, &overrides, None), RATE);
    assert_eq!(resolve_rate(RATE, &HashMap::new(), Some(DINING)), RATE);
}

#[test]
fn groceries_at_zero_percent_stay_untaxed_inside_a_taxed_basket() {
    // Many states exempt food; the exemption must survive being one row of an
    // otherwise-taxable receipt.
    let mut overrides = HashMap::new();
    overrides.insert(GROCERIES, 0);
    assert_eq!(resolve_rate(RATE, &overrides, Some(GROCERIES)), 0);
    assert_eq!(resolve_rate(RATE, &overrides, Some(FUEL)), RATE);

    let groceries = Money::from_cents(4_000);
    let untaxed = groceries.mul_ppm(resolve_rate(RATE, &overrides, Some(GROCERIES)));
    assert_eq!(untaxed, Money::ZERO);
}

// ---------------------------------------------------------------------------
// distribute_tax
// ---------------------------------------------------------------------------

#[test]
fn tax_distributes_proportionally_and_sums_to_the_transaction_tax() {
    let subtotals = [
        Money::from_cents(333),
        Money::from_cents(333),
        Money::from_cents(334),
    ];
    let parts = distribute_tax(Money::from_cents(100), &subtotals);
    assert_eq!(sum(&parts), 100, "components must sum to exactly $1.00");
    // Deterministic tie-break: the odd cent goes to the largest remainder, and
    // ties there go to the lowest index.
    assert_eq!(cents(&parts), vec![33, 33, 34]);
}

#[test]
fn distribute_tax_closes_the_per_split_rounding_gap() {
    // Three $7.00 splits: 7.00 * 8.375% = 58.625c -> 59c each = 177c, but the
    // transaction's own tax on $21.00 is 175.875c -> 176c. Per-split mul_ppm
    // drifts a penny per split; distribute_tax must close the gap exactly.
    let split = Money::from_cents(700);
    let naive: i64 = (0..3).map(|_| split.mul_ppm(RATE).cents()).sum();
    let txn_tax = Money::from_cents(2_100).mul_ppm(RATE);
    assert_eq!(naive, 177);
    assert_eq!(txn_tax.cents(), 176);

    let parts = distribute_tax(txn_tax, &[split, split, split]);
    assert_eq!(sum(&parts), 176, "distribute_tax must not invent a penny");
    assert_eq!(cents(&parts), vec![59, 59, 58]);
}

#[test]
fn a_zero_subtotal_split_never_receives_a_stray_penny_of_tax() {
    // A $0.00 row (a comped item, a placeholder split) is not taxable; the
    // largest-remainder pass must never hand it the leftover cent.
    let subtotals = [
        Money::from_cents(1_000),
        Money::ZERO,
        Money::from_cents(1_000),
    ];
    for tax in [0i64, 1, 100, 101, 837, 12_345] {
        let parts = distribute_tax(Money::from_cents(tax), &subtotals);
        assert_eq!(sum(&parts), tax, "tax {tax}c not conserved");
        assert_eq!(
            parts[1],
            Money::ZERO,
            "zero-subtotal split got {:?} of a {tax}c tax",
            parts[1]
        );
    }
}

#[test]
fn distribute_tax_over_a_refund_keeps_every_component_negative() {
    let subtotals = [
        Money::from_cents(-333),
        Money::from_cents(-333),
        Money::from_cents(-334),
    ];
    let parts = distribute_tax(Money::from_cents(-100), &subtotals);
    assert_eq!(sum(&parts), -100);
    assert!(parts.iter().all(|p| p.cents() <= 0), "{:?}", cents(&parts));
}

#[test]
fn distribute_tax_over_a_single_split_gives_it_everything() {
    let parts = distribute_tax(Money::from_cents(837), &[Money::from_cents(10_000)]);
    assert_eq!(cents(&parts), vec![837]);
}

#[test]
fn distributing_a_zero_tax_yields_all_zeros() {
    let parts = distribute_tax(Money::ZERO, &[Money::from_cents(500), Money::from_cents(1)]);
    assert_eq!(cents(&parts), vec![0, 0]);
}

// ---------------------------------------------------------------------------
// distribute_discount
// ---------------------------------------------------------------------------

#[test]
fn a_discount_sums_to_exactly_the_discount() {
    let subtotals = [
        Money::from_cents(1_000),
        Money::from_cents(2_000),
        Money::from_cents(3_333),
    ];
    for discount in [-1_000i64, -1, 0, 1, 7, 1_000, 6_333] {
        let parts = distribute_discount(Money::from_cents(discount), &subtotals);
        assert_eq!(parts.len(), 3);
        assert_eq!(
            sum(&parts),
            discount,
            "discount {discount}c became {:?}",
            cents(&parts)
        );
    }
}

#[test]
fn a_five_dollar_off_coupon_splits_by_subtotal_share() {
    let subtotals = [Money::from_cents(2_500), Money::from_cents(7_500)];
    let parts = distribute_discount(Money::from_cents(-500), &subtotals);
    assert_eq!(cents(&parts), vec![-125, -375]);
}

#[test]
fn a_discount_never_lands_on_a_zero_subtotal_split() {
    let subtotals = [Money::from_cents(999), Money::ZERO];
    let parts = distribute_discount(Money::from_cents(-333), &subtotals);
    assert_eq!(cents(&parts), vec![-333, 0]);
}

#[test]
fn distributing_across_no_splits_yields_no_components() {
    assert!(distribute_tax(Money::from_cents(100), &[]).is_empty());
    assert!(distribute_discount(Money::from_cents(-100), &[]).is_empty());
}
