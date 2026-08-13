//! Tests for `reconcile`: filling in whichever of (subtotal, tax, total) the
//! user did not type, and never silently contradicting what they did type.

use accounting_core::domain::reconcile::{
    AmountEntry, AmountField, ReconcileError, TaxContext, reconcile,
};
use accounting_core::money::Money;

/// 8.375% — a real US combined rate whose penny rounds are awkward on purpose.
const RATE: u32 = 83_750;

/// A triple that is exact in both directions at RATE:
/// 10000c * 8.375% = 837.5c -> 838c, and extract_ppm(10838c) = 838c.
const S: i64 = 10_000;
const T: i64 = 838;
const X: i64 = 10_838;

fn ctx(prices_include_tax: bool) -> TaxContext {
    TaxContext {
        rate_ppm: RATE,
        prices_include_tax,
    }
}

fn entry(
    subtotal: Option<i64>,
    tax: Option<i64>,
    total: Option<i64>,
    last_edited: Option<AmountField>,
) -> AmountEntry {
    AmountEntry {
        subtotal: subtotal.map(Money::from_cents),
        tax: tax.map(Money::from_cents),
        total: total.map(Money::from_cents),
        last_edited,
    }
}

/// Every presence combination of (subtotal, tax, total), as booleans.
const PRESENCE: [(bool, bool, bool); 8] = [
    (false, false, false),
    (true, false, false),
    (false, true, false),
    (false, false, true),
    (true, true, false),
    (true, false, true),
    (false, true, true),
    (true, true, true),
];

/// Built fresh per call so these tests do not depend on `AmountField: Copy`.
fn last_edited(i: usize) -> Option<AmountField> {
    match i {
        0 => None,
        1 => Some(AmountField::Subtotal),
        2 => Some(AmountField::Tax),
        3 => Some(AmountField::Total),
        _ => unreachable!(),
    }
}

fn pick(present: (bool, bool, bool)) -> (Option<i64>, Option<i64>, Option<i64>) {
    (
        present.0.then_some(S),
        present.1.then_some(T),
        present.2.then_some(X),
    )
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

#[test]
fn every_presence_combination_derives_the_consistent_triple() {
    for present in PRESENCE {
        let (s, t, x) = pick(present);
        let e = entry(s, t, x, None);
        let got = reconcile(&e, &ctx(false));

        match present {
            // Nothing, or a tax with nothing to apply it to, is not enough to
            // build a transaction from.
            (false, false, false) | (false, true, false) => {
                let err = got.expect_err(&format!("{present:?} should be insufficient"));
                assert!(
                    matches!(err, ReconcileError::InsufficientInput),
                    "{present:?} gave {err:?}"
                );
            }
            _ => {
                let a = got.unwrap_or_else(|e| panic!("{present:?} failed: {e:?}"));
                assert_eq!(a.subtotal, Money::from_cents(S), "subtotal for {present:?}");
                assert_eq!(a.tax, Money::from_cents(T), "tax for {present:?}");
                assert_eq!(a.total, Money::from_cents(X), "total for {present:?}");
                assert_eq!(
                    a.subtotal + a.tax,
                    a.total,
                    "subtotal + tax must equal total for {present:?}"
                );
                // The tax is estimated from the rate only when neither a tax nor
                // a second amount pinned it down exactly.
                let expected_estimated =
                    matches!(present, (true, false, false) | (false, false, true));
                assert_eq!(
                    a.tax_is_estimated, expected_estimated,
                    "tax_is_estimated for {present:?}"
                );
            }
        }
    }
}

#[test]
fn last_edited_never_changes_a_self_consistent_entry() {
    // last_edited only breaks ties; when the numbers already agree it is inert.
    for present in PRESENCE {
        let (s, t, x) = pick(present);
        let baseline = reconcile(&entry(s, t, x, None), &ctx(false));
        for i in 0..4 {
            let got = reconcile(&entry(s, t, x, last_edited(i)), &ctx(false));
            match (&baseline, &got) {
                (Ok(b), Ok(g)) => assert_eq!(
                    (b.subtotal, b.tax, b.total, b.tax_is_estimated),
                    (g.subtotal, g.tax, g.total, g.tax_is_estimated),
                    "{present:?} changed under last_edited {:?}",
                    last_edited(i)
                ),
                (Err(_), Err(_)) => {}
                _ => panic!(
                    "{present:?} flipped ok/err under last_edited {:?}",
                    last_edited(i)
                ),
            }
        }
    }
}

#[test]
fn a_tax_the_user_typed_is_never_overwritten_or_marked_estimated() {
    // The whole point of the tax field: if the receipt says $8.38, the app says
    // $8.38 — on every code path, under every flag.
    for present in PRESENCE.iter().filter(|p| p.1) {
        for i in 0..4 {
            for incl in [false, true] {
                let (s, t, x) = pick(*present);
                let le = last_edited(i);
                if let Ok(a) = reconcile(&entry(s, t, x, le), &ctx(incl)) {
                    let le = last_edited(i);
                    assert_eq!(
                        a.tax,
                        Money::from_cents(T),
                        "tax overwritten for {present:?} last_edited={le:?} incl={incl}"
                    );
                    assert!(
                        !a.tax_is_estimated,
                        "user tax marked estimated for {present:?} last_edited={le:?} incl={incl}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_user_tax_survives_even_when_it_disagrees_with_the_rate() {
    // A hand-entered tax that the rate would never produce (a bag fee, a
    // partially exempt basket) still wins.
    let odd_tax = 1_500;
    for le in [
        Some(AmountField::Subtotal),
        Some(AmountField::Tax),
        Some(AmountField::Total),
    ] {
        let a = reconcile(&entry(Some(S), Some(odd_tax), None, le), &ctx(false)).unwrap();
        assert_eq!(a.tax, Money::from_cents(odd_tax));
        assert!(!a.tax_is_estimated);
        assert_eq!(a.total, Money::from_cents(S + odd_tax));
    }
}

// ---------------------------------------------------------------------------
// Estimation from the rate
// ---------------------------------------------------------------------------

#[test]
fn subtotal_only_applies_the_rate_and_flags_the_tax_as_estimated() {
    let a = reconcile(&entry(Some(S), None, None, None), &ctx(false)).unwrap();
    assert_eq!(a.subtotal, Money::from_cents(S));
    assert_eq!(a.tax, Money::from_cents(T));
    assert_eq!(a.total, Money::from_cents(X));
    assert!(a.tax_is_estimated);
}

#[test]
fn prices_include_tax_does_not_change_any_reconciliation_today() {
    // `prices_include_tax` describes how an individual *line item* price is to
    // be read, which arrives in a later milestone. A subtotal typed into the
    // transaction form is tax-exclusive and a total is tax-inclusive under both
    // settings, so nothing here may branch on the flag. This test is the alarm
    // that fires if that ever changes without the spec changing.
    for present in PRESENCE {
        let (s, t, x) = pick(present);
        for i in 0..4 {
            let exclusive = reconcile(&entry(s, t, x, last_edited(i)), &ctx(false));
            let inclusive = reconcile(&entry(s, t, x, last_edited(i)), &ctx(true));
            assert_eq!(
                exclusive.ok(),
                inclusive.ok(),
                "{present:?} last_edited={:?} branched on prices_include_tax",
                last_edited(i)
            );
        }
    }

    // Concretely: a lone subtotal is still grossed up, never backed out.
    let a = reconcile(&entry(Some(S), None, None, None), &ctx(true)).unwrap();
    assert_eq!(a.subtotal, Money::from_cents(S));
    assert_eq!(a.total, Money::from_cents(X));
    assert!(a.tax_is_estimated);
}

#[test]
fn total_only_backs_the_tax_out_regardless_of_the_inclusive_flag() {
    // A total is tax-inclusive by definition; the flag describes entered prices,
    // not the total.
    for incl in [false, true] {
        let a = reconcile(&entry(None, None, Some(X), None), &ctx(incl)).unwrap();
        assert_eq!(a.tax, Money::from_cents(T), "incl={incl}");
        assert_eq!(a.subtotal, Money::from_cents(S), "incl={incl}");
        assert_eq!(a.total, Money::from_cents(X), "incl={incl}");
        assert!(a.tax_is_estimated, "incl={incl}");
    }
}

#[test]
fn a_zero_rate_produces_a_zero_tax_that_is_not_estimated_away() {
    let zero_ctx = TaxContext {
        rate_ppm: 0,
        prices_include_tax: false,
    };
    let a = reconcile(&entry(Some(S), None, None, None), &zero_ctx).unwrap();
    assert_eq!(a.tax, Money::ZERO);
    assert_eq!(a.subtotal, a.total);
}

#[test]
fn a_refund_reconciles_with_negative_amounts() {
    let a = reconcile(&entry(Some(-S), None, None, None), &ctx(false)).unwrap();
    assert_eq!(a.tax, Money::from_cents(-T));
    assert_eq!(a.total, Money::from_cents(-X));
    assert_eq!(a.subtotal + a.tax, a.total);
}

// ---------------------------------------------------------------------------
// Inconsistent input
// ---------------------------------------------------------------------------

/// An entry whose three fields cannot all be true at once.
fn inconsistent(le: Option<AmountField>) -> AmountEntry {
    entry(Some(S), Some(T), Some(99_999), le)
}

#[test]
fn inconsistent_triple_without_last_edited_is_an_error() {
    // Nothing says which number to trust, so guessing would corrupt data.
    let err = reconcile(&inconsistent(None), &ctx(false)).expect_err("should not resolve");
    assert!(
        !matches!(err, ReconcileError::InsufficientInput),
        "wrong error for an inconsistent triple: {err:?}"
    );
}

#[test]
fn editing_the_subtotal_recomputes_the_total() {
    let a = reconcile(&inconsistent(Some(AmountField::Subtotal)), &ctx(false)).unwrap();
    assert_eq!(a.subtotal, Money::from_cents(S));
    assert_eq!(a.tax, Money::from_cents(T));
    assert_eq!(a.total, Money::from_cents(S + T));
    assert!(!a.tax_is_estimated);
}

#[test]
fn editing_the_tax_recomputes_the_total() {
    let a = reconcile(&inconsistent(Some(AmountField::Tax)), &ctx(false)).unwrap();
    assert_eq!(a.tax, Money::from_cents(T));
    assert_eq!(a.subtotal, Money::from_cents(S));
    assert_eq!(a.total, Money::from_cents(S + T));
    assert!(!a.tax_is_estimated);
}

#[test]
fn editing_the_total_recomputes_the_subtotal_and_keeps_the_tax() {
    // The tax is the one number a receipt states unambiguously, so the subtotal
    // absorbs the change.
    let a = reconcile(&inconsistent(Some(AmountField::Total)), &ctx(false)).unwrap();
    assert_eq!(a.total, Money::from_cents(99_999));
    assert_eq!(a.tax, Money::from_cents(T));
    assert_eq!(a.subtotal, Money::from_cents(99_999 - T));
    assert!(!a.tax_is_estimated);
}

#[test]
fn every_resolution_of_an_inconsistent_triple_still_balances() {
    for i in 1..4 {
        for incl in [false, true] {
            let a = reconcile(&inconsistent(last_edited(i)), &ctx(incl)).unwrap();
            assert_eq!(
                a.subtotal + a.tax,
                a.total,
                "unbalanced after resolving {:?} incl={incl}",
                last_edited(i)
            );
        }
    }
}

#[test]
fn empty_and_tax_only_entries_report_insufficient_input() {
    for e in [
        AmountEntry::default(),
        entry(None, Some(T), None, None),
        entry(None, Some(T), None, Some(AmountField::Tax)),
    ] {
        let err = reconcile(&e, &ctx(false)).expect_err("should be insufficient");
        assert!(
            matches!(err, ReconcileError::InsufficientInput),
            "got {err:?}"
        );
    }
}

#[test]
fn a_zero_total_is_a_valid_transaction_not_missing_input() {
    // $0.00 happens (fully discounted line, comped meal); it is not "empty".
    let a = reconcile(&entry(None, None, Some(0), None), &ctx(false)).unwrap();
    assert!(a.subtotal.is_zero() && a.tax.is_zero() && a.total.is_zero());
}
