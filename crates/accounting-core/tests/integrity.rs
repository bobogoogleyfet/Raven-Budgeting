//! Integrity-checker tests.
//!
//! `SUM(txn_split) == txn.total` and `SUM(line_item) == split.total` cannot be
//! CHECK constraints — SQLite has no cross-row constraints — so they are
//! enforced by a single write path and verified after the fact by
//! `check_all()`. That makes `check_all()` itself load-bearing: if it silently
//! checks nothing, every "0 violations" the app has ever printed was a lie.
//!
//! Hence the shape of this file: every "clean database reports nothing" test is
//! paired with a deliberate corruption that must be caught. A checker that
//! returns an empty vec unconditionally passes the first half and fails here.

mod common;

use accounting_core::db::integrity::{InvariantViolation, Severity, check_all};
use common::{
    insert_balanced_aggregate, insert_group_with_category, insert_split, insert_txn, raw, seeded_db,
};

/// Debug-format the violations for failure messages.
fn rendered(violations: &[InvariantViolation]) -> String {
    format!("{violations:?}")
}

/// A violation is only actionable if it says *which* row is wrong — "something
/// does not add up" is not a bug report. The typed id's `Debug` (`TxnId(7)`) is
/// what is matched, so a bare `7` appearing in some money field cannot make
/// this pass by accident.
#[track_caller]
fn assert_names(violations: &[InvariantViolation], typed_id: &str) {
    let text = rendered(violations);
    assert!(
        text.contains(typed_id),
        "a violation must name the offending row {typed_id}; got: {text}"
    );
}

#[track_caller]
fn assert_names_txn(violations: &[InvariantViolation], id: i64) {
    assert_names(violations, &format!("TxnId({id})"));
}

#[track_caller]
fn assert_names_split(violations: &[InvariantViolation], id: i64) {
    assert_names(violations, &format!("SplitId({id})"));
}

// ---------------------------------------------------------------------------
// The clean baseline
// ---------------------------------------------------------------------------

#[test]
fn a_freshly_seeded_database_reports_no_violations() {
    let db = seeded_db();
    let violations = check_all(&db).expect("check_all");
    assert!(
        violations.is_empty(),
        "a seeded database must be clean; got {}",
        rendered(&violations)
    );
}

#[test]
fn a_consistent_aggregate_reports_no_violations() {
    let db = seeded_db();
    insert_balanced_aggregate(raw(&db));
    let violations = check_all(&db).expect("check_all");
    assert!(
        violations.is_empty(),
        "a balanced txn/split/line_item aggregate must be clean; got {}",
        rendered(&violations)
    );
}

// ---------------------------------------------------------------------------
// Corruption — the reason this module exists
// ---------------------------------------------------------------------------

#[test]
fn a_txn_total_that_no_longer_matches_its_splits_is_reported() {
    let db = seeded_db();
    let agg = insert_balanced_aggregate(raw(&db));

    // One cent. Nothing rounds it away, and no row-local CHECK sees it: the
    // txn's own subtotal+tax=total still holds because we move both sides.
    raw(&db)
        .execute(
            "UPDATE txn SET total_cents = total_cents + 1, tax_cents = tax_cents + 1
             WHERE id = ?1",
            [agg.txn_id],
        )
        .expect("corrupt txn total");

    let violations = check_all(&db).expect("check_all");
    assert!(
        !violations.is_empty(),
        "a txn whose total exceeds the sum of its splits by one cent must be reported"
    );
    assert_names_txn(&violations, agg.txn_id);
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, InvariantViolation::SplitSumMismatch { .. })),
        "expected a SplitSumMismatch, got {}",
        rendered(&violations)
    );
    assert_eq!(violations[0].severity(), Severity::Error);
}

#[test]
fn a_split_total_that_no_longer_matches_its_line_items_is_reported() {
    let db = seeded_db();
    let agg = insert_balanced_aggregate(raw(&db));

    // Both columns, so the item's own `subtotal + tax = amount` CHECK still
    // holds. This is exactly the damage a row-local constraint cannot see.
    raw(&db)
        .execute(
            "UPDATE line_item
                SET subtotal_cents = subtotal_cents - 1, amount_cents = amount_cents - 1
              WHERE split_id = ?1 AND description = 'first'",
            [agg.split_id],
        )
        .expect("corrupt line item");

    let violations = check_all(&db).expect("check_all");
    assert!(
        !violations.is_empty(),
        "line items that no longer sum to their split must be reported"
    );
    assert_names_split(&violations, agg.split_id);
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, InvariantViolation::LineItemSumMismatch { .. })),
        "expected a LineItemSumMismatch, got {}",
        rendered(&violations)
    );
}

#[test]
fn a_txn_with_no_splits_at_all_is_reported() {
    // Every dollar must land in some category. An unsplit txn is money that
    // exists on the transaction list and in no report.
    let db = seeded_db();
    let txn_id = insert_txn(raw(&db), 1000, 84);

    let violations = check_all(&db).expect("check_all");
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, InvariantViolation::TxnWithoutSplits { .. })),
        "a transaction with zero splits must be reported, got {}",
        rendered(&violations)
    );
    assert_names_txn(&violations, txn_id);
}

#[test]
fn a_zero_dollar_txn_with_no_splits_is_still_reported() {
    // The one the SUM check cannot catch: `0 = 0` balances, so only the
    // explicit "has no splits" check sees it.
    let db = seeded_db();
    let txn_id = insert_txn(raw(&db), 0, 0);

    let violations = check_all(&db).expect("check_all");
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, InvariantViolation::TxnWithoutSplits { .. })),
        "a $0.00 unsplit transaction must still be reported, got {}",
        rendered(&violations)
    );
    assert_names_txn(&violations, txn_id);
}

#[test]
fn splits_that_sum_to_less_than_their_txn_are_reported() {
    let db = seeded_db();
    let c = raw(&db);
    let (_, category_id) = insert_group_with_category(c, "Partial", "Partial");
    let txn_id = insert_txn(c, 10_000, 0);
    insert_split(c, txn_id, category_id, 4_000, 0);

    let violations = check_all(&db).expect("check_all");
    assert!(
        !violations.is_empty(),
        "an under-assigned transaction must be reported"
    );
    assert_names_txn(&violations, txn_id);
}

#[test]
fn a_grouped_split_with_no_line_items_is_not_a_violation() {
    // Grouped and itemized are one model at two detail levels: a split with NO
    // line items is grouped and legal. Only a split with SOME line items that
    // fail to sum is wrong. Getting this backwards would flag every ordinary
    // receipt.
    let db = seeded_db();
    let c = raw(&db);
    let (_, category_id) = insert_group_with_category(c, "Grouped", "Grouped");
    let txn_id = insert_txn(c, 2_500, 0);
    insert_split(c, txn_id, category_id, 2_500, 0);

    let violations = check_all(&db).expect("check_all");
    assert!(
        violations.is_empty(),
        "a split with no line items is a grouped receipt, not a violation; got {}",
        rendered(&violations)
    );
}

#[test]
fn several_independent_corruptions_are_all_reported() {
    // The checker must not stop at the first problem — `pacc doctor` is meant
    // to give a complete picture in one run.
    let db = seeded_db();
    let c = raw(&db);
    let first = insert_balanced_aggregate(c);
    let orphan_txn = insert_txn(c, 500, 0);

    c.execute(
        "UPDATE txn SET total_cents = total_cents + 100, tax_cents = tax_cents + 100
         WHERE id = ?1",
        [first.txn_id],
    )
    .expect("corrupt first txn");

    let violations = check_all(&db).expect("check_all");
    assert!(
        violations.len() >= 2,
        "expected at least two violations, got {}",
        rendered(&violations)
    );
    assert_names_txn(&violations, first.txn_id);
    assert_names_txn(&violations, orphan_txn);
}

#[test]
fn errors_are_reported_before_warnings() {
    // `pacc doctor` output is read top-down; the thing that breaks the
    // arithmetic must not be buried under advisory notes.
    let db = seeded_db();
    let c = raw(&db);
    let agg = insert_balanced_aggregate(c);

    // An Error: the txn total no longer matches its splits.
    c.execute(
        "UPDATE txn SET total_cents = total_cents + 1, tax_cents = tax_cents + 1 WHERE id = ?1",
        [agg.txn_id],
    )
    .expect("corrupt total");
    // A Warning: the split's category has been archived.
    c.execute(
        "UPDATE category SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
        [agg.category_id],
    )
    .expect("archive category");

    let violations = check_all(&db).expect("check_all");
    let severities: Vec<Severity> = violations
        .iter()
        .map(InvariantViolation::severity)
        .collect();
    assert!(
        severities.contains(&Severity::Error) && severities.contains(&Severity::Warning),
        "expected both severities, got {}",
        rendered(&violations)
    );
    assert!(
        severities.windows(2).all(|w| w[0] <= w[1]),
        "errors must sort before warnings, got {severities:?}"
    );
}
