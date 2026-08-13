//! Tests for `plan_splits`: turning a user's mix of fixed amounts, percentages
//! and "the rest" into rows that add back up to the transaction, to the cent.

use accounting_core::domain::reconcile::Amounts;
use accounting_core::domain::splits::{
    LineItemDraft, SplitAmount, SplitDraft, SplitError, plan_splits,
};
use accounting_core::ids::CategoryId;
use accounting_core::money::Money;
use proptest::prelude::*;

const GROCERIES: CategoryId = CategoryId::new(1);
const DINING: CategoryId = CategoryId::new(2);

/// A transaction with no tax, so a row's `amount` and `subtotal` coincide and
/// the structural assertions are about allocation only.
fn untaxed(total_cents: i64) -> Amounts {
    Amounts {
        subtotal: Money::from_cents(total_cents),
        tax: Money::ZERO,
        total: Money::from_cents(total_cents),
        tax_is_estimated: false,
    }
}

fn taxed(subtotal_cents: i64, tax_cents: i64) -> Amounts {
    Amounts {
        subtotal: Money::from_cents(subtotal_cents),
        tax: Money::from_cents(tax_cents),
        total: Money::from_cents(subtotal_cents + tax_cents),
        tax_is_estimated: false,
    }
}

fn draft(amount: SplitAmount) -> SplitDraft {
    SplitDraft {
        id: None,
        category_id: None,
        amount,
        note: None,
        line_items: Vec::new(),
    }
}

fn fixed(cents: i64) -> SplitDraft {
    draft(SplitAmount::Fixed(Money::from_cents(cents)))
}

fn percent(bps: u32) -> SplitDraft {
    draft(SplitAmount::Percent(bps))
}

fn remainder() -> SplitDraft {
    draft(SplitAmount::Remainder)
}

fn amounts_of(rows: &[accounting_core::domain::splits::ResolvedSplit]) -> Vec<i64> {
    rows.iter().map(|r| r.amount.cents()).collect()
}

fn sum_amounts(rows: &[accounting_core::domain::splits::ResolvedSplit]) -> i64 {
    rows.iter().map(|r| r.amount.cents()).sum()
}

// ---------------------------------------------------------------------------
// Allocation
// ---------------------------------------------------------------------------

#[test]
fn a_remainder_row_absorbs_the_odd_cent() {
    let plan = plan_splits(&untaxed(10_000), &[fixed(3_333), fixed(3_333), remainder()]).unwrap();
    assert_eq!(amounts_of(&plan.rows), vec![3_333, 3_333, 3_334]);
    assert_eq!(plan.unassigned, Money::ZERO);
}

#[test]
fn percentages_in_basis_points_consume_the_whole_total() {
    // 33.33% + 33.33% + 33.34% of $10.00 must be exactly $10.00, not $9.99.
    let plan = plan_splits(
        &untaxed(1_000),
        &[percent(3_333), percent(3_333), percent(3_334)],
    )
    .unwrap();
    assert_eq!(sum_amounts(&plan.rows), 1_000);
    assert_eq!(amounts_of(&plan.rows), vec![333, 333, 334]);
    assert_eq!(plan.unassigned, Money::ZERO);
}

#[test]
fn a_fixed_percent_and_remainder_mix_still_adds_up() {
    // Percentages are taken against the transaction total, not against what is
    // left after the fixed rows.
    let plan = plan_splits(
        &untaxed(10_000),
        &[fixed(2_000), percent(2_500), remainder()],
    )
    .unwrap();
    assert_eq!(amounts_of(&plan.rows), vec![2_000, 2_500, 5_500]);
    assert_eq!(sum_amounts(&plan.rows), 10_000);
    assert_eq!(plan.unassigned, Money::ZERO);
}

#[test]
fn two_remainder_rows_are_rejected() {
    // "The rest" is only meaningful once; two of them have no defined answer.
    let err = plan_splits(&untaxed(10_000), &[remainder(), remainder()])
        .expect_err("two remainders must not resolve");
    assert!(
        matches!(err, SplitError::MultipleRemainders { .. }),
        "got {err:?}"
    );
}

#[test]
fn rows_that_do_not_cover_the_total_leave_the_gap_unassigned() {
    let plan = plan_splits(&untaxed(10_000), &[fixed(2_500), fixed(2_500)]).unwrap();
    assert_eq!(sum_amounts(&plan.rows), 5_000);
    assert_eq!(plan.unassigned, Money::from_cents(5_000));
}

#[test]
fn a_single_remainder_row_takes_the_entire_transaction() {
    let plan = plan_splits(&untaxed(4_299), &[remainder()]).unwrap();
    assert_eq!(amounts_of(&plan.rows), vec![4_299]);
    assert_eq!(plan.unassigned, Money::ZERO);
}

#[test]
fn no_rows_leaves_the_whole_transaction_unassigned() {
    let plan = plan_splits(&untaxed(4_299), &[]).unwrap();
    assert!(plan.rows.is_empty());
    assert_eq!(plan.unassigned, Money::from_cents(4_299));
}

#[test]
fn the_category_on_each_draft_is_carried_through() {
    let mut a = fixed(2_500);
    a.category_id = Some(GROCERIES);
    let mut b = remainder();
    b.category_id = Some(DINING);
    let plan = plan_splits(&untaxed(10_000), &[a, b]).unwrap();
    assert_eq!(plan.rows[0].category_id, Some(GROCERIES));
    assert_eq!(plan.rows[1].category_id, Some(DINING));
}

// ---------------------------------------------------------------------------
// Tax within a plan
// ---------------------------------------------------------------------------

#[test]
fn tax_is_spread_across_rows_and_sums_to_the_transaction_tax() {
    let amounts = taxed(10_000, 838);
    let plan = plan_splits(&amounts, &[fixed(5_000), remainder()]).unwrap();
    assert_eq!(sum_amounts(&plan.rows), amounts.total.cents());
    let tax: Money = plan.rows.iter().map(|r| r.tax).sum();
    let subtotal: Money = plan.rows.iter().map(|r| r.subtotal).sum();
    assert_eq!(
        tax, amounts.tax,
        "row taxes must sum to the transaction tax"
    );
    assert_eq!(subtotal, amounts.subtotal);
    for row in &plan.rows {
        assert_eq!(
            row.subtotal + row.tax,
            row.amount,
            "row must balance: {row:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Line items
// ---------------------------------------------------------------------------

fn line_item(description: &str, amount_cents: i64) -> LineItemDraft {
    LineItemDraft {
        description: description.to_string(),
        quantity_milli: 1_000,
        unit_price: Some(Money::from_cents(amount_cents)),
        amount: Money::from_cents(amount_cents),
    }
}

#[test]
fn line_items_that_sum_to_their_split_are_accepted() {
    let mut split = fixed(1_250);
    split.line_items = vec![line_item("milk", 400), line_item("bread", 850)];
    let plan = plan_splits(&untaxed(1_250), &[split]).unwrap();
    assert_eq!(plan.rows[0].line_items.len(), 2);
    assert_eq!(sum_amounts(&plan.rows), 1_250);
}

#[test]
fn line_items_that_disagree_with_their_split_are_rejected() {
    // A basket that does not add up to the split is a data-entry error, not
    // something to silently absorb.
    let mut split = fixed(1_250);
    split.line_items = vec![line_item("milk", 400), line_item("bread", 851)];
    let err = plan_splits(&untaxed(1_250), &[split]).expect_err("mismatch must be rejected");
    assert!(
        matches!(err, SplitError::LineItemMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_split_with_no_line_items_is_fine() {
    let plan = plan_splits(&untaxed(1_250), &[fixed(1_250)]).unwrap();
    assert!(plan.rows[0].line_items.is_empty());
}

// ---------------------------------------------------------------------------
// Refunds
// ---------------------------------------------------------------------------

#[test]
fn a_refund_splits_into_negative_rows_that_sum_to_the_refund() {
    let plan = plan_splits(
        &untaxed(-10_000),
        &[fixed(-3_333), fixed(-3_333), remainder()],
    )
    .unwrap();
    assert_eq!(amounts_of(&plan.rows), vec![-3_333, -3_333, -3_334]);
    assert_eq!(plan.unassigned, Money::ZERO);
}

#[test]
fn a_refund_splits_by_percentage_without_flipping_sign() {
    let plan = plan_splits(
        &untaxed(-1_000),
        &[percent(3_333), percent(3_333), percent(3_334)],
    )
    .unwrap();
    assert_eq!(sum_amounts(&plan.rows), -1_000);
    assert!(plan.rows.iter().all(|r| r.amount.cents() <= 0));
}

#[test]
fn a_taxed_refund_keeps_every_component_negative_and_exact() {
    let amounts = taxed(-10_000, -838);
    let plan = plan_splits(&amounts, &[fixed(-5_419), remainder()]).unwrap();
    assert_eq!(sum_amounts(&plan.rows), amounts.total.cents());
    let tax: Money = plan.rows.iter().map(|r| r.tax).sum();
    assert_eq!(tax, amounts.tax);
    assert!(plan.rows.iter().all(|r| r.amount.cents() <= 0));
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    /// With a remainder row present, a plan is a partition of the transaction:
    /// nothing is unassigned and every column adds back up exactly.
    #[test]
    fn a_plan_with_a_remainder_row_partitions_the_transaction(
        subtotal in -1_000_000i64..=1_000_000,
        tax in -100_000i64..=100_000,
        fixed_rows in prop::collection::vec(-100_000i64..=100_000, 0..=8),
    ) {
        let amounts = taxed(subtotal, tax);
        let mut drafts: Vec<SplitDraft> = fixed_rows.iter().map(|c| fixed(*c)).collect();
        drafts.push(remainder());

        let plan = plan_splits(&amounts, &drafts).unwrap();
        prop_assert_eq!(plan.rows.len(), drafts.len());
        prop_assert_eq!(plan.unassigned, Money::ZERO);
        prop_assert_eq!(
            Money::from_cents(sum_amounts(&plan.rows)),
            amounts.total,
            "row amounts must sum to the total"
        );
        let row_tax: Money = plan.rows.iter().map(|r| r.tax).sum();
        let row_subtotal: Money = plan.rows.iter().map(|r| r.subtotal).sum();
        prop_assert_eq!(row_tax, amounts.tax, "row taxes must sum to the tax");
        prop_assert_eq!(row_subtotal, amounts.subtotal, "row subtotals must sum to the subtotal");
        for r in &plan.rows {
            prop_assert_eq!(r.subtotal + r.tax, r.amount);
        }
    }

    /// Without a remainder row, whatever the rows do not cover is reported as
    /// `unassigned` — never quietly dropped.
    #[test]
    fn unassigned_always_closes_the_books(
        total in -1_000_000i64..=1_000_000,
        fixed_rows in prop::collection::vec(-100_000i64..=100_000, 0..=8),
    ) {
        let amounts = untaxed(total);
        let drafts: Vec<SplitDraft> = fixed_rows.iter().map(|c| fixed(*c)).collect();
        let plan = plan_splits(&amounts, &drafts).unwrap();
        prop_assert_eq!(
            Money::from_cents(sum_amounts(&plan.rows)) + plan.unassigned,
            amounts.total
        );
    }
}
