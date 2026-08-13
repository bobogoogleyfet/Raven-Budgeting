//! Turning a user's split draft into resolved, penny-exact rows.

use serde::{Deserialize, Serialize};

use crate::domain::reconcile::Amounts;
use crate::domain::tax::distribute_tax;
use crate::ids::{CategoryId, SplitId};
use crate::money::Money;

/// One hundred percent, in basis points.
pub const ONE_HUNDRED_PERCENT_BP: u32 = 10_000;

/// How much of the transaction a split claims.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SplitAmount {
    /// An exact amount.
    Fixed(Money),
    /// A share of the transaction total in **basis points** (33.33% == `3333`).
    Percent(u32),
    /// Whatever the other rows did not claim. At most one per transaction.
    Remainder,
}

/// A split as the user drafted it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SplitDraft {
    /// `None` for a row that has not been persisted yet.
    pub id: Option<SplitId>,
    pub category_id: Option<CategoryId>,
    pub amount: SplitAmount,
    pub note: Option<String>,
    pub line_items: Vec<LineItemDraft>,
}

impl SplitDraft {
    /// A bare draft with just an amount — convenient in tests and the CLI.
    #[must_use]
    pub fn new(amount: SplitAmount) -> Self {
        SplitDraft {
            id: None,
            category_id: None,
            amount,
            note: None,
            line_items: Vec::new(),
        }
    }
}

/// One line on a receipt, inside a split.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LineItemDraft {
    pub description: String,
    /// Quantity in thousandths, so `1.5 kg` is `1_500`. Never a float.
    pub quantity_milli: i64,
    pub unit_price: Option<Money>,
    /// The extended amount for this line.
    pub amount: Money,
}

/// A split after resolution: an exact amount, its share of the transaction
/// tax, and the subtotal implied by the two.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ResolvedSplit {
    pub category_id: Option<CategoryId>,
    pub subtotal: Money,
    pub tax: Money,
    /// `subtotal + tax`.
    pub amount: Money,
    pub line_items: Vec<LineItemDraft>,
}

/// The resolved rows plus whatever share of the transaction nobody claimed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SplitPlan {
    pub rows: Vec<ResolvedSplit>,
    /// `total - sum(row amounts)`. Always zero when a `Remainder` row exists.
    pub unassigned: Money,
}

impl SplitPlan {
    /// Sum of the resolved row amounts.
    #[must_use]
    pub fn assigned(&self) -> Money {
        self.rows.iter().map(|r| r.amount).sum()
    }
}

/// Why a draft could not be resolved.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum SplitError {
    /// More than one row asked for the remainder; the split is ambiguous.
    #[error("only one split may take the remainder, found {count}")]
    MultipleRemainders { count: usize },
    /// The percentage rows add up to more than 100%.
    #[error("percentage splits total {total_bp} basis points, more than 100%")]
    PercentsExceedOneHundred { total_bp: u32 },
    /// A row's line items do not add up to the row.
    #[error("split {split_index}: line items total {found} but the split is {expected}")]
    LineItemMismatch {
        split_index: usize,
        expected: Money,
        found: Money,
    },
    /// The arithmetic overflowed `i64` cents.
    #[error("split arithmetic overflowed")]
    Overflow,
}

/// Resolve a set of split drafts against reconciled transaction amounts.
///
/// 1. `Fixed` rows take their stated amount.
/// 2. All `Percent` rows are apportioned **together**, in one
///    [`Money::allocate`] call over the full 10 000 basis points (with a
///    synthetic bucket absorbing any unclaimed percentage). This is what
///    makes 33.33 / 33.33 / 33.34 of `$10.00` consume exactly `$10.00`
///    instead of `$9.99` — the leftover cent is handed out by
///    largest-remainder rather than lost to three independent roundings.
/// 3. At most one `Remainder` row, which takes `total - assigned`.
/// 4. The transaction's tax is apportioned across the rows with
///    [`distribute_tax`], and each row's subtotal is `amount - tax`. (The
///    weights are the resolved row *amounts*: the per-row subtotals are
///    derived from the tax, so they cannot also be its input. For a single
///    tax rate the two weightings are proportional and give the same answer.)
/// 5. Rows carrying line items must have those line items add up to the row.
pub fn plan_splits(amounts: &Amounts, drafts: &[SplitDraft]) -> Result<SplitPlan, SplitError> {
    let total = amounts.total;

    let remainder_rows: Vec<usize> = drafts
        .iter()
        .enumerate()
        .filter(|(_, d)| matches!(d.amount, SplitAmount::Remainder))
        .map(|(i, _)| i)
        .collect();
    if remainder_rows.len() > 1 {
        return Err(SplitError::MultipleRemainders {
            count: remainder_rows.len(),
        });
    }

    // --- percentage rows, apportioned as a single group -------------------
    let percent_rows: Vec<(usize, u32)> = drafts
        .iter()
        .enumerate()
        .filter_map(|(i, d)| match d.amount {
            SplitAmount::Percent(bp) => Some((i, bp)),
            _ => None,
        })
        .collect();
    let total_bp = percent_rows
        .iter()
        .try_fold(0_u32, |acc, (_, bp)| acc.checked_add(*bp))
        .ok_or(SplitError::Overflow)?;
    if total_bp > ONE_HUNDRED_PERCENT_BP {
        return Err(SplitError::PercentsExceedOneHundred { total_bp });
    }

    let mut resolved: Vec<Option<Money>> = vec![None; drafts.len()];
    if !percent_rows.is_empty() {
        let mut weights: Vec<i64> = percent_rows.iter().map(|(_, bp)| i64::from(*bp)).collect();
        // A final bucket for the percentage nobody claimed, so that rows
        // summing to less than 100% are not silently scaled up to 100%.
        weights.push(i64::from(ONE_HUNDRED_PERCENT_BP - total_bp));
        let shares = total.allocate(&weights);
        for (slot, (row, _)) in percent_rows.iter().enumerate() {
            resolved[*row] = Some(shares[slot]);
        }
    }

    // --- fixed rows --------------------------------------------------------
    for (i, d) in drafts.iter().enumerate() {
        if let SplitAmount::Fixed(m) = d.amount {
            resolved[i] = Some(m);
        }
    }

    // --- the remainder row -------------------------------------------------
    let claimed = checked_sum(resolved.iter().flatten().copied())?;
    if let Some(&row) = remainder_rows.first() {
        resolved[row] = Some(total.checked_sub(claimed).ok_or(SplitError::Overflow)?);
    }

    let row_amounts: Vec<Money> = resolved.iter().map(|r| r.unwrap_or(Money::ZERO)).collect();
    let assigned = checked_sum(row_amounts.iter().copied())?;
    let unassigned = total.checked_sub(assigned).ok_or(SplitError::Overflow)?;

    // --- tax, apportioned once across the rows ----------------------------
    let taxes = distribute_tax(amounts.tax, &row_amounts);

    let mut rows = Vec::with_capacity(drafts.len());
    for (i, draft) in drafts.iter().enumerate() {
        let amount = row_amounts[i];
        let tax = taxes.get(i).copied().unwrap_or(Money::ZERO);
        let subtotal = amount.checked_sub(tax).ok_or(SplitError::Overflow)?;

        if !draft.line_items.is_empty() {
            let found = checked_sum(draft.line_items.iter().map(|li| li.amount))?;
            if found != amount {
                return Err(SplitError::LineItemMismatch {
                    split_index: i,
                    expected: amount,
                    found,
                });
            }
        }

        rows.push(ResolvedSplit {
            category_id: draft.category_id,
            subtotal,
            tax,
            amount,
            line_items: draft.line_items.clone(),
        });
    }

    Ok(SplitPlan { rows, unassigned })
}

/// Build a plan from a **subtotal typed per category**, the way a receipt is
/// actually read.
///
/// The other entry mode starts from a total and divides it. This one starts
/// from the parts: you copy each category's subtotal off the receipt, tax is
/// computed from the location's rate, and the total falls out. Nobody has to
/// type a number the app can work out, and nothing has to be divided.
///
/// Tax is apportioned across the rows by **subtotal**, which is the correct
/// weight here and is exact: [`distribute_tax`] is [`Money::allocate`], so the
/// per-row taxes sum to `amounts.tax` to the cent with no drift. Each row is
/// then `subtotal + its share of tax`, and the rows sum to `amounts.total` by
/// construction — which is why `unassigned` is always zero in this mode.
///
/// `amounts.subtotal` must equal the sum of `subtotals`; the caller derives
/// both from the same list, so a mismatch is a bug rather than user input.
pub fn plan_by_subtotal(
    amounts: &Amounts,
    rows: &[(Option<CategoryId>, Money)],
) -> Result<SplitPlan, SplitError> {
    let subtotals: Vec<Money> = rows.iter().map(|(_, s)| *s).collect();
    let summed = subtotals
        .iter()
        .try_fold(Money::ZERO, |acc, m| acc.checked_add(*m))
        .ok_or(SplitError::Overflow)?;
    debug_assert_eq!(
        summed, amounts.subtotal,
        "plan_by_subtotal was given amounts that do not match its own rows"
    );

    let taxes = distribute_tax(amounts.tax, &subtotals);
    let mut resolved = Vec::with_capacity(rows.len());
    for (index, (category_id, subtotal)) in rows.iter().enumerate() {
        let tax = taxes.get(index).copied().unwrap_or(Money::ZERO);
        resolved.push(ResolvedSplit {
            category_id: *category_id,
            subtotal: *subtotal,
            tax,
            amount: subtotal.checked_add(tax).ok_or(SplitError::Overflow)?,
            line_items: Vec::new(),
        });
    }

    let assigned = resolved
        .iter()
        .try_fold(Money::ZERO, |acc, r| acc.checked_add(r.amount))
        .ok_or(SplitError::Overflow)?;
    Ok(SplitPlan {
        rows: resolved,
        // Zero by construction: the rows *are* the transaction here, rather
        // than claims against a total entered separately.
        unassigned: amounts
            .total
            .checked_sub(assigned)
            .ok_or(SplitError::Overflow)?,
    })
}

fn checked_sum(iter: impl IntoIterator<Item = Money>) -> Result<Money, SplitError> {
    iter.into_iter().try_fold(Money::ZERO, |acc, m| {
        acc.checked_add(m).ok_or(SplitError::Overflow)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::reconcile::AmountEntry;
    use crate::domain::reconcile::{TaxContext, reconcile};

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn amounts(subtotal: i64, tax: i64) -> Amounts {
        Amounts {
            subtotal: m(subtotal),
            tax: m(tax),
            total: m(subtotal + tax),
            tax_is_estimated: false,
        }
    }

    fn sum_rows(plan: &SplitPlan) -> Money {
        plan.rows.iter().map(|r| r.amount).sum()
    }

    #[test]
    fn percent_rows_consume_the_total_exactly() {
        let a = amounts(1000, 0);
        let plan = plan_splits(
            &a,
            &[
                SplitDraft::new(SplitAmount::Percent(3333)),
                SplitDraft::new(SplitAmount::Percent(3333)),
                SplitDraft::new(SplitAmount::Percent(3334)),
            ],
        )
        .unwrap();
        assert_eq!(sum_rows(&plan), m(1000));
        assert_eq!(plan.unassigned, Money::ZERO);
        assert_eq!(
            plan.rows.iter().map(|r| r.amount).collect::<Vec<_>>(),
            vec![m(333), m(333), m(334)]
        );
    }

    #[test]
    fn partial_percentages_leave_the_rest_unassigned() {
        let a = amounts(1000, 0);
        let plan = plan_splits(&a, &[SplitDraft::new(SplitAmount::Percent(2500))]).unwrap();
        assert_eq!(plan.rows[0].amount, m(250));
        assert_eq!(plan.unassigned, m(750));
    }

    #[test]
    fn percentages_over_one_hundred_are_rejected() {
        let a = amounts(1000, 0);
        let err = plan_splits(
            &a,
            &[
                SplitDraft::new(SplitAmount::Percent(6000)),
                SplitDraft::new(SplitAmount::Percent(6000)),
            ],
        )
        .unwrap_err();
        assert_eq!(
            err,
            SplitError::PercentsExceedOneHundred { total_bp: 12_000 }
        );
    }

    #[test]
    fn remainder_absorbs_everything_left() {
        let a = amounts(10_000, 0);
        let plan = plan_splits(
            &a,
            &[
                SplitDraft::new(SplitAmount::Fixed(m(3333))),
                SplitDraft::new(SplitAmount::Percent(2500)),
                SplitDraft::new(SplitAmount::Remainder),
            ],
        )
        .unwrap();
        assert_eq!(plan.rows[1].amount, m(2500));
        assert_eq!(plan.rows[2].amount, m(10_000 - 3333 - 2500));
        assert_eq!(plan.unassigned, Money::ZERO);
        assert_eq!(sum_rows(&plan), a.total);
    }

    #[test]
    fn two_remainders_are_ambiguous() {
        let a = amounts(1000, 0);
        let err = plan_splits(
            &a,
            &[
                SplitDraft::new(SplitAmount::Remainder),
                SplitDraft::new(SplitAmount::Remainder),
            ],
        )
        .unwrap_err();
        assert_eq!(err, SplitError::MultipleRemainders { count: 2 });
    }

    #[test]
    fn tax_is_apportioned_and_subtotals_add_back_up() {
        let a = reconcile(
            &AmountEntry::total_only(m(5432)),
            &TaxContext::exclusive(83_750),
        )
        .unwrap();
        let plan = plan_splits(
            &a,
            &[
                SplitDraft::new(SplitAmount::Percent(3333)),
                SplitDraft::new(SplitAmount::Percent(3333)),
                SplitDraft::new(SplitAmount::Remainder),
            ],
        )
        .unwrap();
        let tax: Money = plan.rows.iter().map(|r| r.tax).sum();
        let subtotal: Money = plan.rows.iter().map(|r| r.subtotal).sum();
        assert_eq!(tax, a.tax);
        assert_eq!(subtotal, a.subtotal);
        assert_eq!(sum_rows(&plan), a.total);
        for row in &plan.rows {
            assert_eq!(row.subtotal + row.tax, row.amount);
        }
    }

    #[test]
    fn line_items_must_match_their_split() {
        let a = amounts(1000, 0);
        let li = |cents| LineItemDraft {
            description: "thing".to_string(),
            quantity_milli: 1_000,
            unit_price: Some(m(cents)),
            amount: m(cents),
        };
        let mut draft = SplitDraft::new(SplitAmount::Fixed(m(1000)));
        draft.line_items = vec![li(600), li(400)];
        assert!(plan_splits(&a, std::slice::from_ref(&draft)).is_ok());

        draft.line_items = vec![li(600), li(300)];
        let err = plan_splits(&a, &[draft]).unwrap_err();
        assert_eq!(
            err,
            SplitError::LineItemMismatch {
                split_index: 0,
                expected: m(1000),
                found: m(900),
            }
        );
    }

    #[test]
    fn no_drafts_leaves_everything_unassigned() {
        let a = amounts(1000, 84);
        let plan = plan_splits(&a, &[]).unwrap();
        assert!(plan.rows.is_empty());
        assert_eq!(plan.unassigned, m(1084));
    }

    #[test]
    fn refunds_split_by_percentage_too() {
        let a = amounts(-1000, 0);
        let plan = plan_splits(
            &a,
            &[
                SplitDraft::new(SplitAmount::Percent(3333)),
                SplitDraft::new(SplitAmount::Percent(3333)),
                SplitDraft::new(SplitAmount::Percent(3334)),
            ],
        )
        .unwrap();
        assert_eq!(sum_rows(&plan), m(-1000));
    }
}
