//! Tax rate resolution and proportional distribution of transaction-level
//! amounts across splits.

use std::collections::HashMap;

use crate::ids::CategoryId;
use crate::money::Money;

/// Pick the tax rate (in ppm) that applies to one split.
///
/// A per-category override always wins over the location's rate — that is how
/// "groceries are exempt here" or "prepared food is 10%" is modelled. A split
/// with no category falls back to the location rate.
#[must_use]
pub fn resolve_rate(
    location_rate_ppm: u32,
    overrides: &HashMap<CategoryId, u32>,
    category: Option<CategoryId>,
) -> u32 {
    category
        .and_then(|c| overrides.get(&c).copied())
        .unwrap_or(location_rate_ppm)
}

/// Which of a location's published rates applies to a category.
///
/// Three, not two. A state taxes classes of goods differently in *both*
/// directions: Utah charges a combined 3% on unprepared food, roughly 7.45% on
/// ordinary goods, and 8.45% on prepared food, where a county has levied its
/// Restaurant Tax (Utah Code 59-12-603) on top of the combined rate. Which
/// figure applies is a property of the *category*, not of the receipt or the
/// store.
///
/// An enum rather than the two schema booleans because the booleans admit a
/// state that has no meaning — a category cannot be both unprepared and
/// prepared food — and every caller would otherwise have to decide what that
/// means for itself. Resolving it once, in [`RateClass::from_flags`], is what
/// keeps them agreeing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RateClass {
    /// Taxed at the location's ordinary combined rate.
    General,
    /// Unprepared food, where the state publishes a reduced rate.
    ReducedFood,
    /// Prepared food, where the county adds a restaurant tax on top.
    PreparedFood,
}

impl RateClass {
    /// Resolve the schema's two flags (`uses_food_rate`,
    /// `uses_restaurant_rate`) into one class.
    ///
    /// The flags are mutually exclusive in any sane data set, and the seed
    /// keeps them that way. If both are somehow set, prepared food wins: it is
    /// the higher of the two rates, and overstating tax produces a total the
    /// user can see is wrong against the receipt in their hand, while
    /// understating it looks entirely plausible and reconciles nowhere. Same
    /// principle as `tax_is_estimated` — a wrong number that looks like a fact
    /// is the worse failure.
    #[must_use]
    pub const fn from_flags(uses_food_rate: bool, uses_restaurant_rate: bool) -> Self {
        if uses_restaurant_rate {
            Self::PreparedFood
        } else if uses_food_rate {
            Self::ReducedFood
        } else {
            Self::General
        }
    }
}

/// The rate a category is taxed at, given what this location charges.
///
/// `food_ppm` and `restaurant_ppm` of `None` mean this place publishes no such
/// rate, which is the correct reading for most of the US. A missing rate falls
/// back to the general one rather than to zero — "no reduced rate here" and
/// "free here" are very different claims, and only one of them is ever true.
///
/// It lives here, alone, because it is asked in two places that must never
/// disagree: the entry form labels each row with a rate, and the tax
/// calculation buckets rows by one. A second copy would drift, and the symptom
/// would be a screen showing 3% beside tax charged at 7.45%.
#[must_use]
pub const fn category_rate(
    general_ppm: u32,
    food_ppm: Option<u32>,
    restaurant_ppm: Option<u32>,
    class: RateClass,
    taxable: bool,
) -> u32 {
    if !taxable {
        // Not "0% is the rate here" but "no sales tax is added to this at
        // all": rent, a phone bill, a tank of fuel. The amount the user types
        // for those is the amount they paid, taxes and fees already inside it,
        // and adding a rate on top would invent money nobody was charged.
        return 0;
    }
    match class {
        RateClass::ReducedFood => match food_ppm {
            Some(reduced) => reduced,
            None => general_ppm,
        },
        RateClass::PreparedFood => match restaurant_ppm {
            Some(prepared) => prepared,
            None => general_ppm,
        },
        RateClass::General => general_ppm,
    }
}

/// Spread the transaction's **single** tax figure across splits, weighted by
/// their subtotals.
///
/// # Why not compute tax per split?
///
/// Because `sum(mul_ppm(subtotal_i))` is not `mul_ppm(sum(subtotal_i))`.
/// Rounding each split's tax independently drifts up to half a cent per
/// split, so a five-way split can miss the receipt total by two or three
/// cents and the transaction stops balancing. The receipt's tax line is the
/// ground truth; we take that number and apportion it with
/// [`Money::allocate`], which is sum-preserving by construction. The split
/// taxes therefore always add back up to exactly the transaction tax.
///
/// Weights are the **magnitudes** of the subtotals: on a refund every
/// subtotal is negative, and the sign of the result comes from `total_tax`
/// itself, not from the weights.
#[must_use]
pub fn distribute_tax(total_tax: Money, split_subtotals: &[Money]) -> Vec<Money> {
    total_tax.allocate(&magnitudes(split_subtotals))
}

/// Spread a transaction-level discount across splits, weighted by their
/// subtotals. Same reasoning (and same sum-preserving guarantee) as
/// [`distribute_tax`].
#[must_use]
pub fn distribute_discount(discount: Money, split_subtotals: &[Money]) -> Vec<Money> {
    discount.allocate(&magnitudes(split_subtotals))
}

fn magnitudes(amounts: &[Money]) -> Vec<i64> {
    amounts.iter().map(|m| m.abs().cents()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn sum(v: &[Money]) -> Money {
        v.iter().copied().sum()
    }

    #[test]
    fn category_override_beats_location_rate() {
        let mut overrides = HashMap::new();
        overrides.insert(CategoryId::new(7), 0);
        assert_eq!(
            resolve_rate(83_750, &overrides, Some(CategoryId::new(7))),
            0
        );
        assert_eq!(
            resolve_rate(83_750, &overrides, Some(CategoryId::new(8))),
            83_750
        );
        assert_eq!(resolve_rate(83_750, &overrides, None), 83_750);
    }

    #[test]
    fn distributed_tax_always_sums_to_the_receipt_tax() {
        let subtotals = [m(1000), m(1000), m(1000), m(1000), m(1000)];
        let taxes = distribute_tax(m(419), &subtotals);
        assert_eq!(sum(&taxes), m(419));
        assert_eq!(taxes.len(), 5);
    }

    #[test]
    fn per_split_rounding_would_have_drifted() {
        // Five $10.00 splits at 8.375%: per-split rounding gives 5 * 84 = 420,
        // but the receipt says 419. Distribution keeps the receipt honest.
        let per_split: Money = (0..5).map(|_| m(1000).mul_ppm(83_750)).sum();
        assert_eq!(per_split, m(420));
        let receipt_tax = m(5000).mul_ppm(83_750);
        assert_eq!(receipt_tax, m(419));
        assert_eq!(sum(&distribute_tax(receipt_tax, &[m(1000); 5])), m(419));
    }

    #[test]
    fn refund_subtotals_are_weighted_by_magnitude() {
        let taxes = distribute_tax(m(-100), &[m(-750), m(-250)]);
        assert_eq!(taxes, vec![m(-75), m(-25)]);
        assert_eq!(sum(&taxes), m(-100));
    }

    #[test]
    fn zero_subtotals_fall_back_to_an_even_split() {
        let taxes = distribute_tax(m(100), &[Money::ZERO, Money::ZERO]);
        assert_eq!(sum(&taxes), m(100));
        assert_eq!(taxes, vec![m(50), m(50)]);
    }

    #[test]
    fn discounts_distribute_the_same_way() {
        let d = distribute_discount(m(-1000), &[m(3333), m(3333), m(3334)]);
        assert_eq!(sum(&d), m(-1000));
    }

    #[test]
    fn empty_splits_produce_no_rows() {
        assert!(distribute_tax(m(419), &[]).is_empty());
    }
}
