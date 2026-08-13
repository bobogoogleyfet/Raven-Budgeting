//! Subtotal / tax / total reconciliation.
//!
//! A receipt gives the user between one and three of the three amounts, and
//! they may disagree. [`reconcile`] turns whatever was entered into a
//! complete, internally consistent [`Amounts`] triple, and reports honestly
//! whether the tax figure is a user-supplied fact or our estimate.
//!
//! Everything here is pure: values in, values out, no clock, no database.

use crate::money::Money;

/// The tax rate in effect for a transaction, plus how prices are quoted at
/// the merchant.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TaxContext {
    /// Sales-tax rate in parts per million (8.375% == `83_750`).
    pub rate_ppm: u32,
    /// Whether shelf prices at this merchant already include tax.
    ///
    /// **Currently unused by [`reconcile`], deliberately.** It only decides
    /// how an individual *line item* price is interpreted, which arrives in a
    /// later milestone. Both branches that estimate tax from a known total
    /// back the tax out of that total identically today: a known total is
    /// tax-inclusive by definition, whatever the shelf price convention was.
    /// The flag is stored and threaded so the plumbing exists when line-item
    /// pricing lands, but nothing branches on it yet.
    pub prices_include_tax: bool,
}

impl TaxContext {
    /// A context with the given rate and tax-exclusive pricing.
    #[must_use]
    pub const fn exclusive(rate_ppm: u32) -> Self {
        TaxContext {
            rate_ppm,
            prices_include_tax: false,
        }
    }
}

/// Which of the three amounts the user touched most recently.
#[derive(Copy, Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum AmountField {
    Subtotal,
    Tax,
    Total,
}

/// What the user has typed so far.
#[derive(Default, Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct AmountEntry {
    pub subtotal: Option<Money>,
    pub tax: Option<Money>,
    pub total: Option<Money>,
    /// The field edited most recently, used to decide which value yields when
    /// all three are present and disagree.
    pub last_edited: Option<AmountField>,
}

impl AmountEntry {
    /// Entry with only a total.
    #[must_use]
    pub fn total_only(total: Money) -> Self {
        AmountEntry {
            total: Some(total),
            ..Default::default()
        }
    }

    /// Entry with only a subtotal.
    #[must_use]
    pub fn subtotal_only(subtotal: Money) -> Self {
        AmountEntry {
            subtotal: Some(subtotal),
            ..Default::default()
        }
    }
}

/// A complete, consistent amount triple: `subtotal + tax == total` always.
#[derive(Copy, Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Amounts {
    pub subtotal: Money,
    pub tax: Money,
    pub total: Money,
    /// `true` only when the tax figure was derived from a rate because the
    /// user gave us exactly one amount. If the user typed a tax value, this
    /// is `false` — always.
    pub tax_is_estimated: bool,
}

impl Amounts {
    /// Invariant check used by tests and debug assertions.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.subtotal.checked_add(self.tax) == Some(self.total)
    }
}

/// Why a set of entered amounts could not be reconciled.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ReconcileError {
    /// All three amounts were given, they disagree, and no `last_edited`
    /// field says which one should yield.
    #[error(
        "amounts are inconsistent: subtotal {subtotal} + tax {tax} != total {total} (off by {delta})"
    )]
    Inconsistent {
        subtotal: Money,
        tax: Money,
        total: Money,
        /// `total - (subtotal + tax)`.
        delta: Money,
    },
    /// Nothing was entered, or only a tax amount was — neither pins down a
    /// transaction.
    #[error("not enough information: need a subtotal or a total")]
    InsufficientInput,
    /// The arithmetic overflowed `i64` cents.
    #[error("amount arithmetic overflowed")]
    Overflow,
}

fn add(a: Money, b: Money) -> Result<Money, ReconcileError> {
    a.checked_add(b).ok_or(ReconcileError::Overflow)
}

fn sub(a: Money, b: Money) -> Result<Money, ReconcileError> {
    a.checked_sub(b).ok_or(ReconcileError::Overflow)
}

/// Complete the subtotal / tax / total triple.
///
/// | Given | Behaviour | `tax_is_estimated` |
/// |---|---|---|
/// | S, T, X and `S + T == X` | accept as entered | `false` |
/// | S, T, X inconsistent | recompute the field *not* named by `last_edited`, preferring Total; `last_edited == None` → [`ReconcileError::Inconsistent`] | `false` |
/// | S, T | `X = S + T` | `false` |
/// | S, X | `T = X - S` | `false` |
/// | T, X | `S = X - T` | `false` |
/// | X only | `T = X.extract_ppm(rate)`, `S = X - T` | **`true`** |
/// | S only | `T = S.mul_ppm(rate)`, `X = S + T` | **`true`** |
/// | T only, or nothing | [`ReconcileError::InsufficientInput`] | — |
///
/// # `tax_is_estimated` is structural
///
/// It is set to `true` in exactly two places in this function — the two
/// single-input branches — and is hard-coded `false` everywhere else. There
/// is no path on which a user-entered tax value is overwritten by a
/// rate-derived one: when all three amounts disagree and the user's last edit
/// was the *total*, we adjust the **subtotal**, never the tax.
pub fn reconcile(entry: &AmountEntry, ctx: &TaxContext) -> Result<Amounts, ReconcileError> {
    // `prices_include_tax` is intentionally not read here; see TaxContext.
    let _ = ctx.prices_include_tax;

    match (entry.subtotal, entry.tax, entry.total) {
        // ---- all three entered -------------------------------------------
        (Some(subtotal), Some(tax), Some(total)) => {
            if add(subtotal, tax)? == total {
                return Ok(Amounts {
                    subtotal,
                    tax,
                    total,
                    tax_is_estimated: false,
                });
            }
            match entry.last_edited {
                // Total is stale: the two entered components win.
                Some(AmountField::Subtotal | AmountField::Tax) => Ok(Amounts {
                    subtotal,
                    tax,
                    total: add(subtotal, tax)?,
                    tax_is_estimated: false,
                }),
                // The user just typed a new total. Tax was entered by hand, so
                // it is a fact we must not clobber; the subtotal absorbs the
                // difference.
                Some(AmountField::Total) => Ok(Amounts {
                    subtotal: sub(total, tax)?,
                    tax,
                    total,
                    tax_is_estimated: false,
                }),
                // Nothing tells us which value to trust: refuse.
                None => Err(ReconcileError::Inconsistent {
                    subtotal,
                    tax,
                    total,
                    delta: sub(total, add(subtotal, tax)?)?,
                }),
            }
        }

        // ---- two entered --------------------------------------------------
        (Some(subtotal), Some(tax), None) => Ok(Amounts {
            subtotal,
            tax,
            total: add(subtotal, tax)?,
            tax_is_estimated: false,
        }),
        (Some(subtotal), None, Some(total)) => Ok(Amounts {
            subtotal,
            tax: sub(total, subtotal)?,
            total,
            tax_is_estimated: false,
        }),
        (None, Some(tax), Some(total)) => Ok(Amounts {
            subtotal: sub(total, tax)?,
            tax,
            total,
            tax_is_estimated: false,
        }),

        // ---- one entered: the only branches that may estimate --------------
        (None, None, Some(total)) => {
            let tax = total.extract_ppm(ctx.rate_ppm);
            Ok(Amounts {
                subtotal: sub(total, tax)?,
                tax,
                total,
                tax_is_estimated: true,
            })
        }
        (Some(subtotal), None, None) => {
            let tax = subtotal.mul_ppm(ctx.rate_ppm);
            Ok(Amounts {
                subtotal,
                tax,
                total: add(subtotal, tax)?,
                tax_is_estimated: true,
            })
        }

        // ---- not enough ---------------------------------------------------
        (None, Some(_), None) | (None, None, None) => Err(ReconcileError::InsufficientInput),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 83_750; // 8.375%

    fn ctx() -> TaxContext {
        TaxContext::exclusive(RATE)
    }

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn entry(s: Option<i64>, t: Option<i64>, x: Option<i64>) -> AmountEntry {
        AmountEntry {
            subtotal: s.map(m),
            tax: t.map(m),
            total: x.map(m),
            last_edited: None,
        }
    }

    #[test]
    fn consistent_triple_is_accepted_untouched() {
        let got = reconcile(&entry(Some(1000), Some(84), Some(1084)), &ctx()).unwrap();
        assert_eq!(got.subtotal, m(1000));
        assert_eq!(got.tax, m(84));
        assert_eq!(got.total, m(1084));
        assert!(!got.tax_is_estimated);
    }

    #[test]
    fn inconsistent_triple_without_last_edited_is_an_error() {
        let err = reconcile(&entry(Some(1000), Some(84), Some(1090)), &ctx()).unwrap_err();
        assert_eq!(
            err,
            ReconcileError::Inconsistent {
                subtotal: m(1000),
                tax: m(84),
                total: m(1090),
                delta: m(6),
            }
        );
    }

    #[test]
    fn inconsistent_triple_recomputes_the_unnamed_field() {
        // last edit = subtotal -> total is recomputed
        let mut e = entry(Some(1000), Some(84), Some(1090));
        e.last_edited = Some(AmountField::Subtotal);
        let got = reconcile(&e, &ctx()).unwrap();
        assert_eq!(
            (got.subtotal, got.tax, got.total),
            (m(1000), m(84), m(1084))
        );
        assert!(!got.tax_is_estimated);

        // last edit = tax -> total is recomputed
        let mut e = entry(Some(1000), Some(90), Some(1084));
        e.last_edited = Some(AmountField::Tax);
        let got = reconcile(&e, &ctx()).unwrap();
        assert_eq!(got.total, m(1090));

        // last edit = total -> subtotal absorbs it, the entered tax survives
        let mut e = entry(Some(1000), Some(84), Some(1200));
        e.last_edited = Some(AmountField::Total);
        let got = reconcile(&e, &ctx()).unwrap();
        assert_eq!(
            (got.subtotal, got.tax, got.total),
            (m(1116), m(84), m(1200))
        );
        assert!(!got.tax_is_estimated);
    }

    #[test]
    fn pairs_fill_in_the_third_and_never_estimate() {
        for e in [
            entry(Some(1000), Some(84), None),
            entry(Some(1000), None, Some(1084)),
            entry(None, Some(84), Some(1084)),
        ] {
            let got = reconcile(&e, &ctx()).unwrap();
            assert_eq!(
                (got.subtotal, got.tax, got.total),
                (m(1000), m(84), m(1084))
            );
            assert!(!got.tax_is_estimated, "{e:?}");
            assert!(got.is_consistent());
        }
    }

    #[test]
    fn total_only_backs_tax_out_and_flags_the_estimate() {
        let got = reconcile(&entry(None, None, Some(5432)), &ctx()).unwrap();
        assert!(got.tax_is_estimated);
        assert!(got.is_consistent());
        assert_eq!(got.total, m(5432));
        assert_eq!(got.tax, m(5432).extract_ppm(RATE));
    }

    #[test]
    fn subtotal_only_adds_tax_and_flags_the_estimate() {
        let got = reconcile(&entry(Some(5000), None, None), &ctx()).unwrap();
        assert!(got.tax_is_estimated);
        assert!(got.is_consistent());
        assert_eq!(got.tax, m(419)); // 8.375% of $50.00 = 4.1875 -> 4.19
        assert_eq!(got.total, m(5419));
    }

    #[test]
    fn tax_only_or_nothing_is_insufficient() {
        assert_eq!(
            reconcile(&entry(None, Some(84), None), &ctx()).unwrap_err(),
            ReconcileError::InsufficientInput
        );
        assert_eq!(
            reconcile(&entry(None, None, None), &ctx()).unwrap_err(),
            ReconcileError::InsufficientInput
        );
    }

    #[test]
    fn a_zero_rate_estimates_zero_tax() {
        let got = reconcile(&entry(None, None, Some(5000)), &TaxContext::exclusive(0)).unwrap();
        assert_eq!(got.tax, Money::ZERO);
        assert_eq!(got.subtotal, m(5000));
        assert!(got.tax_is_estimated);
    }

    #[test]
    fn prices_include_tax_does_not_change_todays_result() {
        let e = entry(None, None, Some(5432));
        let exclusive = reconcile(
            &e,
            &TaxContext {
                rate_ppm: RATE,
                prices_include_tax: false,
            },
        )
        .unwrap();
        let inclusive = reconcile(
            &e,
            &TaxContext {
                rate_ppm: RATE,
                prices_include_tax: true,
            },
        )
        .unwrap();
        assert_eq!(exclusive, inclusive);
    }

    #[test]
    fn refunds_reconcile_too() {
        let got = reconcile(&entry(None, None, Some(-5432)), &ctx()).unwrap();
        assert!(got.is_consistent());
        assert!(got.tax.is_negative());
    }
}
