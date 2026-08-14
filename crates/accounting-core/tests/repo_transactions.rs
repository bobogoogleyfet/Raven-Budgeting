//! The transaction write path — the most consequential code in the crate.
//!
//! A transaction is an *aggregate*: the `txn` row, its splits, and their line
//! items are one indivisible fact, and the two invariants that say so
//! (`SUM(split.amount) = txn.total`, `SUM(item.amount) = split.amount`) cannot
//! be expressed as constraints. They are held instead by a single write path
//! that validates first and then replaces the whole aggregate inside one SQLite
//! transaction.
//!
//! That design puts the weight on three things, which is what this file tests:
//!
//! 1. **Validation happens before the write.** Every rejection must surface as
//!    `CoreError::Invalid`, never as a `CoreError::Sqlite` from a CHECK that
//!    happened to catch it — a schema constraint firing means malformed rows
//!    reached the database, and only luck decided whether one was there to
//!    catch it.
//! 2. **A rejected save changes nothing.** Not the txn, not the splits, and not
//!    the merchant or location that a helpful implementation might have created
//!    on the way in.
//! 3. **A re-save replaces, rather than accumulates.** Nothing else in M3
//!    notices an orphaned split, and an orphan is money counted twice.
//!
//! Every test that writes ends with `assert_clean`.

mod common;

use accounting_core::domain::reconcile::Amounts;
use accounting_core::repo::transactions::{
    LineItemRecord, NewTransaction, SplitRecord, Txn, TxnFilter, TxnSort,
};
use accounting_core::repo::{categories, locations, merchants, transactions};
use accounting_core::{
    CategoryId, CoreError, Db, DetailLevel, LocationId, MerchantId, Money, TxnId,
};
use chrono::{Days, NaiveDate};
use common::{assert_clean, count, raw, row_counts, seeded_db};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn m(cents: i64) -> Money {
    Money::from_cents(cents)
}

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap_or_else(|e| panic!("date `{s}`: {e}"))
}

/// A consistent amount triple, built from the two independent numbers.
fn amounts(subtotal: i64, tax: i64) -> Amounts {
    Amounts {
        subtotal: m(subtotal),
        tax: m(tax),
        total: m(subtotal + tax),
        tax_is_estimated: false,
    }
}

/// A split with `sort_order` left at 0, i.e. "you choose".
fn split(category_id: CategoryId, subtotal: i64, tax: i64) -> SplitRecord {
    SplitRecord {
        id: None,
        uuid: None,
        category_id,
        subtotal: m(subtotal),
        tax: m(tax),
        amount: m(subtotal + tax),
        sort_order: 0,
        notes: None,
        line_items: Vec::new(),
    }
}

/// One tax-free line: `subtotal == amount`, quantity 1.
fn item(description: &str, amount: i64) -> LineItemRecord {
    LineItemRecord {
        id: None,
        uuid: None,
        description: description.to_string(),
        quantity_milli: 1_000,
        unit_price: Some(m(amount)),
        subtotal: m(amount),
        tax: Money::ZERO,
        amount: m(amount),
        category_id: None,
        is_taxable: true,
        sort_order: 0,
        notes: None,
    }
}

fn txn(txn_date: NaiveDate, amounts: Amounts, splits: Vec<SplitRecord>) -> NewTransaction {
    NewTransaction {
        id: None,
        txn_date,
        merchant_id: None,
        location_id: None,
        amounts,
        tax_rate_ppm: None,
        detail_level: DetailLevel::Grouped,
        needs_review: false,
        notes: None,
        splits,
    }
}

fn cat(db: &Db, name: &str) -> CategoryId {
    categories::resolve(db, name)
        .unwrap_or_else(|e| panic!("resolve category `{name}`: {e}"))
        .id
}

fn merchant(db: &Db, name: &str) -> MerchantId {
    merchants::get_or_create(db, name).unwrap_or_else(|e| panic!("merchant `{name}`: {e}"));
    merchants::find_by_name(db, name)
        .expect("find_by_name")
        .unwrap_or_else(|| panic!("merchant `{name}` must exist after get_or_create"))
        .id
}

fn location(db: &Db, name: &str, rate_ppm: u32) -> LocationId {
    locations::get_or_create(db, name, rate_ppm)
        .unwrap_or_else(|e| panic!("location `{name}`: {e}"));
    locations::find_by_name(db, name)
        .expect("find_by_name")
        .unwrap_or_else(|| panic!("location `{name}` must exist after get_or_create"))
        .id
}

/// The splits of one stored transaction, summed.
fn split_total(db: &Db, id: TxnId) -> Money {
    transactions::get(db, id)
        .expect("get")
        .splits
        .iter()
        .map(|s| s.amount)
        .sum()
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

#[test]
fn a_grouped_single_category_receipt_round_trips_unchanged() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let costco = merchant(&db, "Costco");
    let store = location(&db, "Costco Sunnyvale", 91_250);

    // $50.12 + $4.20 tax = $54.32, read straight off the receipt: nothing here
    // is estimated, so `tax_is_estimated` must survive as false.
    let mut new = txn(
        day("2026-08-13"),
        amounts(5012, 420),
        vec![split(groceries, 5012, 420)],
    );
    new.merchant_id = Some(costco);
    new.location_id = Some(store);
    new.tax_rate_ppm = Some(91_250);
    new.notes = Some("weekly shop".to_string());

    let id = transactions::save(&db, &new).expect("save");
    let got = transactions::get(&db, id).expect("get");

    assert_eq!(got.id, id);
    assert_eq!(got.txn_date, day("2026-08-13"));
    assert_eq!(got.subtotal, m(5012));
    assert_eq!(got.tax, m(420));
    assert_eq!(got.total, m(5432));
    assert!(!got.tax_is_estimated);
    assert_eq!(got.tax_rate_ppm, Some(91_250));
    assert_eq!(got.detail_level, DetailLevel::Grouped);
    assert!(!got.needs_review);
    assert!(!got.archived);
    assert_eq!(got.notes.as_deref(), Some("weekly shop"));
    assert!(
        !got.uuid.is_empty(),
        "every row carries a stable cross-device uuid"
    );

    // The joined-in display names are the whole reason `get` returns a `Txn`
    // rather than the raw row: no caller should have to re-query for them.
    assert_eq!(got.merchant_id, Some(costco));
    assert_eq!(got.merchant_name.as_deref(), Some("Costco"));
    assert_eq!(got.location_id, Some(store));
    assert_eq!(got.location_name.as_deref(), Some("Costco Sunnyvale"));

    let only = match got.splits.as_slice() {
        [one] => one,
        other => panic!("a single-category receipt must round-trip as one split, got {other:?}"),
    };
    assert_eq!(only.category_id, groceries);
    assert_eq!(only.category_name, "Groceries");
    assert_eq!(only.group_name, "Frequent");
    assert_eq!(only.subtotal, m(5012));
    assert_eq!(only.tax, m(420));
    assert_eq!(only.amount, m(5432));

    assert_clean(&db);
}

#[test]
fn a_three_way_split_round_trips_in_sort_order() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");
    let personal = cat(&db, "Toiletries");

    // $90.00 + $10.00 tax = $100.00, divided so that each column adds up
    // independently:
    //   subtotal 45.00 + 30.00 + 15.00 = 90.00
    //   tax       5.00 +  3.33 +  1.67 = 10.00
    //   amount   50.00 + 33.33 + 16.67 = 100.00
    let mut new = txn(
        day("2026-08-13"),
        amounts(9000, 1000),
        vec![
            split(groceries, 4500, 500),
            split(household, 3000, 333),
            split(personal, 1500, 167),
        ],
    );
    // Supplied deliberately out of order: the stored order is what `sort_order`
    // says, not the order the rows happened to be inserted in.
    new.splits[0].sort_order = 300;
    new.splits[1].sort_order = 100;
    new.splits[2].sort_order = 200;

    let id = transactions::save(&db, &new).expect("save");
    let got = transactions::get(&db, id).expect("get");

    let names: Vec<&str> = got
        .splits
        .iter()
        .map(|s| s.category_name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Household", "Toiletries", "Groceries"],
        "splits must come back in sort_order, not insertion order"
    );
    assert!(
        got.splits
            .windows(2)
            .all(|w| w[0].sort_order < w[1].sort_order),
        "sort_order must be preserved as given; got {:?}",
        got.splits.iter().map(|s| s.sort_order).collect::<Vec<_>>()
    );

    let summed: Money = got.splits.iter().map(|s| s.amount).sum();
    assert_eq!(
        summed, got.total,
        "every cent of the receipt must land in a split"
    );
    assert_eq!(
        got.splits.iter().map(|s| s.tax).sum::<Money>(),
        got.tax,
        "and the tax column must add up on its own — a split can offset an \
         overstated subtotal against an understated tax and still balance"
    );
    assert_eq!(
        got.splits.iter().map(|s| s.subtotal).sum::<Money>(),
        got.subtotal
    );

    assert_clean(&db);
}

#[test]
fn split_sort_order_defaults_to_sparse_hundreds() {
    // Sparse ordering (100, 200, 300 …) is what makes a later drag-reorder a
    // one-row write instead of a renumbering of every sibling.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");

    let new = txn(
        day("2026-08-13"),
        amounts(1000, 0),
        vec![split(groceries, 600, 0), split(household, 400, 0)],
    );
    let id = transactions::save(&db, &new).expect("save");

    let orders: Vec<i64> = transactions::get(&db, id)
        .expect("get")
        .splits
        .iter()
        .map(|s| s.sort_order)
        .collect();
    assert_eq!(
        orders,
        vec![100, 200],
        "an unset sort_order must become (index + 1) * 100"
    );

    assert_clean(&db);
}

#[test]
fn an_itemized_receipt_round_trips_with_its_line_items_in_order() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    // $12.00 + $8.00 + $10.00 = $30.00. Tax-free, so each item's own
    // subtotal + tax = amount holds without inventing a per-item tax rule.
    let mut only = split(groceries, 3000, 0);
    only.line_items = vec![
        item("Apples", 1200),
        item("Bread", 800),
        item("Cheese", 1000),
    ];
    only.line_items[0].quantity_milli = 1_500; // 1.5 lb
    only.line_items[0].unit_price = Some(m(800)); // $8.00/lb -> $12.00
    for (i, li) in only.line_items.iter_mut().enumerate() {
        li.sort_order = i64::try_from(i + 1).expect("small index") * 100;
    }

    let mut new = txn(day("2026-08-13"), amounts(3000, 0), vec![only]);
    new.detail_level = DetailLevel::Itemized;

    let id = transactions::save(&db, &new).expect("save");
    let got = transactions::get(&db, id).expect("get");

    assert_eq!(got.detail_level, DetailLevel::Itemized);
    let stored = &got.splits[0].line_items;
    let descriptions: Vec<&str> = stored.iter().map(|li| li.description.as_str()).collect();
    assert_eq!(descriptions, vec!["Apples", "Bread", "Cheese"]);
    assert_eq!(stored[0].quantity_milli, 1_500);
    assert_eq!(stored[0].unit_price, Some(m(800)));
    assert_eq!(stored[0].amount, m(1200));
    assert_eq!(
        stored.iter().map(|li| li.amount).sum::<Money>(),
        got.splits[0].amount,
        "the line items are the split, itemised — they must still add up to it"
    );

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A valid $10.00 grouped receipt. Each rejection case below changes exactly
/// one thing about it, so a case that is *not* rejected points at exactly one
/// missing check.
fn valid(groceries: CategoryId) -> NewTransaction {
    txn(
        day("2026-08-13"),
        amounts(1000, 0),
        vec![split(groceries, 1000, 0)],
    )
}

#[test]
fn every_validation_branch_rejects_before_touching_the_database() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");
    let before = row_counts(&db);

    let cases: Vec<(&str, NewTransaction)> = vec![
        (
            "a transaction with no splits at all — its money would belong to no category",
            NewTransaction {
                splits: Vec::new(),
                ..valid(groceries)
            },
        ),
        (
            "amounts whose subtotal and tax do not add up to the total",
            NewTransaction {
                amounts: Amounts {
                    subtotal: m(1000),
                    tax: m(100),
                    total: m(1200),
                    tax_is_estimated: false,
                },
                splits: vec![split(groceries, 1000, 100)],
                ..valid(groceries)
            },
        ),
        (
            "splits that sum to less than the transaction",
            NewTransaction {
                amounts: amounts(1000, 100),
                splits: vec![split(groceries, 900, 100)],
                ..valid(groceries)
            },
        ),
        // The remaining sum rule — splits that divide the total differently
        // while still summing to it — has a test of its own below, because the
        // assertion worth making about it is about the *message*.
        (
            "a split whose subtotal and tax do not add up to its own amount",
            NewTransaction {
                amounts: amounts(900, 0),
                splits: vec![SplitRecord {
                    subtotal: m(1000),
                    tax: Money::ZERO,
                    amount: m(900),
                    ..split(groceries, 900, 0)
                }],
                ..valid(groceries)
            },
        ),
        (
            "line items that do not sum to their split",
            NewTransaction {
                detail_level: DetailLevel::Itemized,
                splits: vec![SplitRecord {
                    line_items: vec![item("Apples", 600), item("Bread", 300)],
                    ..split(groceries, 1000, 0)
                }],
                ..valid(groceries)
            },
        ),
        (
            "a line item with no quantity — every line is at least one of something",
            NewTransaction {
                detail_level: DetailLevel::Itemized,
                splits: vec![SplitRecord {
                    line_items: vec![
                        LineItemRecord {
                            quantity_milli: 0,
                            ..item("Apples", 600)
                        },
                        item("Bread", 400),
                    ],
                    ..split(groceries, 1000, 0)
                }],
                ..valid(groceries)
            },
        ),
        (
            "a line item with an empty description",
            NewTransaction {
                detail_level: DetailLevel::Itemized,
                splits: vec![SplitRecord {
                    line_items: vec![item("", 600), item("Bread", 400)],
                    ..split(groceries, 1000, 0)
                }],
                ..valid(groceries)
            },
        ),
        (
            "a tax rate above 100%",
            NewTransaction {
                tax_rate_ppm: Some(1_000_001),
                ..valid(groceries)
            },
        ),
    ];

    for (label, candidate) in &cases {
        let err = match transactions::save(&db, candidate) {
            Ok(id) => panic!("save must reject {label}, but it stored {id:?}"),
            Err(e) => e,
        };
        assert!(
            !matches!(err, CoreError::Sqlite(_)),
            "{label}: this must be caught by validation before the write, not by a \
             schema constraint on the way in — a constraint firing means malformed \
             rows reached SQLite. Got: {err}"
        );
    }

    assert_eq!(
        row_counts(&db),
        before,
        "a rejected save must leave every table exactly as it found it"
    );

    // The negative control: without it, a `save` that rejected everything
    // unconditionally would pass every case above.
    transactions::save(&db, &valid(groceries)).expect("the base case must be accepted");
    let two_way = txn(
        day("2026-08-13"),
        amounts(1000, 0),
        vec![split(groceries, 600, 0), split(household, 400, 0)],
    );
    transactions::save(&db, &two_way).expect("a balanced two-way split must be accepted");

    assert_clean(&db);
}

#[test]
fn splits_that_divide_the_total_differently_are_rejected_by_column_not_by_total() {
    // The case a total-only check cannot see, built deliberately. The split
    // moves $1.00 out of subtotal and into tax:
    //
    //   split   subtotal  9.00 + tax 2.00 = amount 11.00   (row CHECK holds)
    //   txn     subtotal 10.00 + tax 1.00 = total  11.00   (row CHECK holds)
    //   sum(amount) 11.00 == total 11.00                   (totals agree)
    //
    // An implementation that compares only `Σ amount == total` accepts this and
    // stores a receipt whose per-category tax is wrong forever — exactly the
    // damage `InvariantViolation::SplitTaxSumMismatch` exists to detect after
    // the fact. Rejecting it means the comparison is done column by column, and
    // the message has to name a column for the user to have any idea what to
    // fix.
    //
    // Note that the subtotal, tax and total checks cannot be exercised
    // *independently* through this API: per-split `subtotal + tax = amount` is
    // also enforced, and given that, any two of the three sums agreeing implies
    // the third. So this asserts the property that matters — a column-wise
    // rejection — rather than which of the three checks fired.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    let skewed = NewTransaction {
        amounts: amounts(1000, 100),
        splits: vec![split(groceries, 900, 200)],
        ..valid(groceries)
    };

    let err = transactions::save(&db, &skewed).expect_err("a skewed tax division must be refused");
    assert!(matches!(err, CoreError::Invalid(_)), "got {err:?}");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("subtotal") || msg.contains("tax"),
        "the totals agree, so the rejection must name the column that does not; got: {err}"
    );

    assert_clean(&db);
}

#[test]
fn a_split_on_a_category_that_does_not_exist_is_rejected_and_rolled_back() {
    // Not one of the validated rules — a category id can only come from the
    // repository in the first place, so this is a programming error rather
    // than user input, and it is SQLite's `REFERENCES` clause that catches it.
    // What matters is that catching it *late* is still safe: the whole write
    // is inside one transaction, so the rejection costs a confusing message
    // and nothing else.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    transactions::save(&db, &valid(groceries)).expect("save a real one first");
    let before = row_counts(&db);

    let orphan = txn(
        day("2026-08-13"),
        amounts(1000, 0),
        vec![split(CategoryId::new(999_999), 1000, 0)],
    );
    transactions::save(&db, &orphan).expect_err("no such category");

    assert_eq!(
        row_counts(&db),
        before,
        "a foreign-key rejection must roll the whole aggregate back, not leave \
         the txn row behind with no splits"
    );
    assert_clean(&db);
}

#[test]
fn a_rejected_save_leaves_the_database_untouched() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let costco = merchant(&db, "Costco");
    let store = location(&db, "Costco Sunnyvale", 91_250);

    // A transaction already on file, so the test is about *no change* rather
    // than about an empty database staying empty.
    let mut good = valid(groceries);
    good.merchant_id = Some(costco);
    good.location_id = Some(store);
    transactions::save(&db, &good).expect("save the good one");
    let before = row_counts(&db);

    // Broken only in the splits, and only in the last of them: an
    // implementation that inserts as it validates has already written the txn
    // row and the first split by the time it notices.
    let mut bad = txn(
        day("2026-08-14"),
        amounts(5000, 0),
        vec![split(groceries, 3000, 0), split(groceries, 1000, 0)],
    );
    bad.merchant_id = Some(costco);
    bad.location_id = Some(store);
    let err = transactions::save(&db, &bad).expect_err("splits that do not cover the total");
    assert!(matches!(err, CoreError::Invalid(_)), "got {err:?}");

    assert_eq!(
        row_counts(&db),
        before,
        "a failed save must roll back completely — a half-written aggregate is \
         money in no category"
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Re-save
// ---------------------------------------------------------------------------

#[test]
fn re_saving_replaces_the_old_splits_and_keeps_the_uuid() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");
    let personal = cat(&db, "Toiletries");

    // Three splits, two of them itemised, so the re-save has both splits and
    // line items to clean up. $30.00 total, no tax.
    let mut first = split(groceries, 1500, 0);
    first.line_items = vec![item("Apples", 900), item("Bread", 600)];
    let mut second = split(household, 1000, 0);
    second.line_items = vec![item("Soap", 1000)];
    let third = split(personal, 500, 0);

    let mut new = txn(
        day("2026-08-13"),
        amounts(3000, 0),
        vec![first, second, third],
    );
    new.detail_level = DetailLevel::Itemized;
    let id = transactions::save(&db, &new).expect("save");

    let original = transactions::get(&db, id).expect("get");
    let old_uuid = original.uuid.clone();
    // Identity is tracked by uuid, not by rowid. The write path deletes every
    // split and re-inserts, and SQLite hands a fresh row the lowest free
    // rowid — so the replacement splits come back as ids 1 and 2, exactly the
    // ids the old ones had. A rowid-based check here would "pass" on a write
    // path that left the originals in place and fail on one that did the right
    // thing; only the uuid distinguishes the two.
    let old_split_uuids: Vec<String> = original.splits.iter().map(|s| s.uuid.clone()).collect();
    let old_item_uuids: Vec<String> = original
        .splits
        .iter()
        .flat_map(|s| s.line_items.iter().map(|li| li.uuid.clone()))
        .collect();
    assert_eq!(old_split_uuids.len(), 3);
    assert_eq!(old_item_uuids.len(), 3);

    // The correction: same receipt, now two categories, still $30.00.
    let mut replacement = txn(
        day("2026-08-13"),
        amounts(3000, 0),
        vec![split(groceries, 2000, 0), split(household, 1000, 0)],
    );
    replacement.id = Some(id);
    let same_id = transactions::save(&db, &replacement).expect("re-save");
    assert_eq!(
        same_id, id,
        "re-saving must update in place, not insert a copy"
    );

    assert_eq!(
        count(raw(&db), "SELECT COUNT(*) FROM txn"),
        1,
        "re-saving must not leave the old transaction row behind"
    );

    // uuids are v7 and generated per row, so quoting them into the SQL is
    // safe; there is no user input anywhere near this string.
    let quoted = |uuids: &[String]| {
        uuids
            .iter()
            .map(|u| format!("'{u}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    assert_eq!(
        count(
            raw(&db),
            &format!(
                "SELECT COUNT(*) FROM txn_split WHERE uuid IN ({})",
                quoted(&old_split_uuids)
            )
        ),
        0,
        "every split of the previous version must be gone — a leftover split is \
         money counted twice, and nothing else in the system looks for one"
    );
    assert_eq!(
        count(
            raw(&db),
            &format!(
                "SELECT COUNT(*) FROM line_item WHERE uuid IN ({})",
                quoted(&old_item_uuids)
            )
        ),
        0,
        "deleting a split must cascade to its line items"
    );
    assert_eq!(
        count(raw(&db), "SELECT COUNT(*) FROM txn_split"),
        2,
        "exactly the two new splits must remain"
    );
    assert_eq!(
        count(raw(&db), "SELECT COUNT(*) FROM line_item"),
        0,
        "the replacement is grouped, so no line item may survive it"
    );

    let updated = transactions::get(&db, id).expect("get after re-save");
    assert_eq!(
        updated.uuid, old_uuid,
        "the uuid is the row's cross-device identity: a re-save is an edit, not a new fact"
    );
    assert_eq!(updated.detail_level, DetailLevel::Grouped);
    assert_eq!(split_total(&db, id), m(3000));

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// Six transactions across two merchants, four dates and two categories, one
/// archived and one needing review. Every filter test runs against this same
/// shape, so the filters can be compared against one another rather than each
/// being handed data that only it could pass.
struct Listing {
    db: Db,
    costco: MerchantId,
    shell: MerchantId,
    groceries: CategoryId,
    fuel: CategoryId,
    june_1: TxnId,
    june_15_review: TxnId,
    july_1_fuel: TxnId,
    july_1_groceries: TxnId,
    august_1_no_merchant: TxnId,
    archived: TxnId,
}

fn listing() -> Listing {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let fuel = cat(&db, "Gas");
    let costco = merchant(&db, "Costco");
    let shell = merchant(&db, "Shell");

    let save = |date: &str, who: Option<MerchantId>, what: CategoryId, cents: i64, review: bool| {
        let mut new = txn(day(date), amounts(cents, 0), vec![split(what, cents, 0)]);
        new.merchant_id = who;
        new.needs_review = review;
        transactions::save(&db, &new).unwrap_or_else(|e| panic!("save {date}: {e}"))
    };

    let june_1 = save("2026-06-01", Some(costco), groceries, 1000, false);
    let june_15_review = save("2026-06-15", Some(shell), fuel, 2000, true);
    let july_1_fuel = save("2026-07-01", Some(costco), fuel, 3000, false);
    let july_1_groceries = save("2026-07-01", Some(costco), groceries, 4000, false);
    let august_1_no_merchant = save("2026-08-01", None, groceries, 5000, false);
    let archived = save("2026-05-01", Some(shell), groceries, 6000, false);
    transactions::archive(&db, archived).expect("archive");

    let built = Listing {
        db,
        costco,
        shell,
        groceries,
        fuel,
        june_1,
        june_15_review,
        july_1_fuel,
        july_1_groceries,
        august_1_no_merchant,
        archived,
    };
    // The fixture is itself a write, and every filter test below inherits it,
    // so the invariants are audited once here rather than in each of them.
    assert_clean(&built.db);
    built
}

fn ids(rows: &[Txn]) -> Vec<TxnId> {
    rows.iter().map(|t| t.id).collect()
}

#[test]
fn list_filters_by_date_range() {
    let f = listing();
    let rows = transactions::list(
        &f.db,
        &TxnFilter {
            from: Some(day("2026-06-01")),
            to: Some(day("2026-07-01")),
            ..Default::default()
        },
    )
    .expect("list");

    assert!(
        rows.iter()
            .all(|t| t.txn_date >= day("2026-06-01") && t.txn_date <= day("2026-07-01")),
        "every row must fall inside the range; got {:?}",
        rows.iter().map(|t| t.txn_date).collect::<Vec<_>>()
    );
    // Both ends are inclusive: a month report that dropped the 1st or the 31st
    // would disagree with the bank statement.
    let found = ids(&rows);
    for expected in [
        f.june_1,
        f.june_15_review,
        f.july_1_fuel,
        f.july_1_groceries,
    ] {
        assert!(
            found.contains(&expected),
            "{expected:?} missing from {found:?}"
        );
    }
    assert!(!found.contains(&f.august_1_no_merchant));

    assert_clean(&f.db);
}

#[test]
fn list_filters_by_merchant() {
    let f = listing();
    let rows = transactions::list(
        &f.db,
        &TxnFilter {
            merchant_ids: vec![f.costco],
            ..Default::default()
        },
    )
    .expect("list");

    assert!(
        rows.iter().all(|t| t.merchant_id == Some(f.costco)),
        "every row must belong to the requested merchant"
    );
    let found = ids(&rows);
    assert!(found.contains(&f.june_1));
    assert!(found.contains(&f.july_1_groceries));
    assert!(!found.contains(&f.june_15_review), "that one is Shell's");
    assert!(
        !found.contains(&f.august_1_no_merchant),
        "a transaction with no merchant matches no merchant filter"
    );
}

#[test]
fn list_filters_by_category() {
    let f = listing();
    let rows = transactions::list(
        &f.db,
        &TxnFilter {
            category_ids: vec![f.groceries],
            ..Default::default()
        },
    )
    .expect("list");

    assert!(
        rows.iter()
            .all(|t| t.splits.iter().any(|s| s.category_id == f.groceries)),
        "a category filter matches a transaction through its splits"
    );
    let found = ids(&rows);
    assert!(found.contains(&f.june_1));
    assert!(found.contains(&f.august_1_no_merchant));
    assert!(
        !found.contains(&f.july_1_fuel),
        "a fuel-only receipt must not appear under Groceries"
    );

    // And the other category, so the filter cannot be passing by returning
    // everything.
    let fuel_rows = transactions::list(
        &f.db,
        &TxnFilter {
            category_ids: vec![f.fuel],
            ..Default::default()
        },
    )
    .expect("list");
    assert!(ids(&fuel_rows).contains(&f.july_1_fuel));
    assert!(!ids(&fuel_rows).contains(&f.june_1));
}

#[test]
fn list_filters_by_needs_review() {
    let f = listing();

    let inbox = transactions::list(
        &f.db,
        &TxnFilter {
            needs_review: Some(true),
            ..Default::default()
        },
    )
    .expect("list");
    assert!(inbox.iter().all(|t| t.needs_review));
    assert!(ids(&inbox).contains(&f.june_15_review));

    let settled = transactions::list(
        &f.db,
        &TxnFilter {
            needs_review: Some(false),
            ..Default::default()
        },
    )
    .expect("list");
    assert!(settled.iter().all(|t| !t.needs_review));
    assert!(ids(&settled).contains(&f.june_1));
    assert!(!ids(&settled).contains(&f.june_15_review));
}

#[test]
fn list_hides_archived_transactions_unless_they_are_asked_for() {
    let f = listing();

    let live = transactions::list(&f.db, &TxnFilter::default()).expect("list");
    assert!(
        !ids(&live).contains(&f.archived),
        "an archived transaction is deleted as far as the user is concerned"
    );
    assert!(live.iter().all(|t| !t.archived));

    let everything = transactions::list(
        &f.db,
        &TxnFilter {
            include_archived: true,
            ..Default::default()
        },
    )
    .expect("list");
    let restored = everything
        .iter()
        .find(|t| t.id == f.archived)
        .expect("include_archived must bring the archived row back");
    assert!(restored.archived, "and it must be labelled as archived");
    assert!(
        ids(&everything).contains(&f.june_1),
        "include_archived widens the result, it does not replace it"
    );

    // Archiving does not cascade to the splits, and the invariant checker
    // deliberately ignores archived rows — so this must still be clean.
    assert_clean(&f.db);
}

#[test]
fn list_honours_its_limit() {
    let f = listing();
    let rows = transactions::list(
        &f.db,
        &TxnFilter {
            limit: Some(2),
            ..Default::default()
        },
    )
    .expect("list");
    assert!(rows.len() <= 2, "asked for 2, got {}", rows.len());
    // The limit takes the *newest*, which is what a "recent transactions"
    // panel needs; taking the oldest two would be useless and equally "2 rows".
    assert_eq!(
        ids(&rows).first().copied(),
        Some(f.august_1_no_merchant),
        "a limited list must still start at the most recent row"
    );
}

#[test]
fn list_is_ordered_by_date_then_id_descending() {
    let f = listing();
    let rows = transactions::list(&f.db, &TxnFilter::default()).expect("list");

    let keys: Vec<(NaiveDate, TxnId)> = rows.iter().map(|t| (t.txn_date, t.id)).collect();
    assert!(
        keys.windows(2).all(|w| w[0] > w[1]),
        "the transaction list must be newest-first, ties broken by the newest \
         entry; got {keys:?}"
    );
    // The tie-break is the point: two receipts on the same day must show the
    // one entered second at the top, because that is the one just typed.
    let found = ids(&rows);
    let at = |id: TxnId| found.iter().position(|x| *x == id).expect("present");
    assert!(at(f.july_1_groceries) < at(f.july_1_fuel));
}

#[test]
fn count_agrees_with_the_list_it_describes() {
    let f = listing();

    for filter in [
        TxnFilter::default(),
        TxnFilter {
            merchant_ids: vec![f.shell],
            ..Default::default()
        },
        TxnFilter {
            include_archived: true,
            ..Default::default()
        },
        TxnFilter {
            needs_review: Some(true),
            ..Default::default()
        },
    ] {
        let listed = transactions::list(&f.db, &filter).expect("list").len();
        let counted = transactions::count(&f.db, &filter).expect("count");
        assert_eq!(
            counted, listed,
            "count and list must apply the same filter; {filter:?}"
        );
    }
}

#[test]
fn last_for_merchant_returns_the_most_recent_live_transaction() {
    let f = listing();

    let latest = transactions::last_for_merchant(&f.db, f.costco)
        .expect("last_for_merchant")
        .expect("Costco has transactions");
    assert_eq!(
        latest.id, f.july_1_groceries,
        "of two receipts on the same day, the most recently entered wins"
    );

    // Shell's newest receipt is archived, so "repeat last" must reach past it
    // rather than offering to duplicate something the user deleted.
    let shell_latest = transactions::last_for_merchant(&f.db, f.shell)
        .expect("last_for_merchant")
        .expect("Shell has a live transaction");
    assert_eq!(shell_latest.id, f.june_15_review);
    assert_ne!(shell_latest.id, f.archived);

    let unknown = merchant(&f.db, "Never Shopped Here");
    assert!(
        transactions::last_for_merchant(&f.db, unknown)
            .expect("last_for_merchant")
            .is_none(),
        "a merchant with no history has no last receipt"
    );

    assert_clean(&f.db);
}

#[test]
fn archiving_hides_a_transaction_without_disturbing_its_splits() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let id = transactions::save(&db, &valid(groceries)).expect("save");

    transactions::archive(&db, id).expect("archive");

    assert!(
        !ids(&transactions::list(&db, &TxnFilter::default()).expect("list")).contains(&id),
        "an archived transaction must leave the default list"
    );
    assert!(
        transactions::get(&db, id).expect("get").archived,
        "archiving is a soft delete: the row is still there, and still readable"
    );
    assert_eq!(
        count(raw(&db), "SELECT COUNT(*) FROM txn_split"),
        1,
        "archiving must not cascade — the historical split is the record"
    );

    assert_clean(&db);
}

#[test]
fn fifty_transactions_all_load_with_their_splits_intact() {
    // Loading a list of transactions with their splits is the classic N+1
    // query: one query for the list, then one per row for the splits, then one
    // per split for the line items. A test process cannot count the queries
    // SQLite executed, and a timing assertion would flake on a loaded CI box —
    // so this asserts *correctness at a size where the naive implementation
    // stops being acceptable*, and leaves the query shape to review.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");
    let base = day("2026-01-01");

    for i in 0..50_i64 {
        let date = base
            .checked_add_days(Days::new(u64::try_from(i).expect("non-negative")))
            .expect("still 2026");
        // Distinct totals, and two splits each, so a row picking up another
        // row's splits is visible rather than plausible.
        let cents = 1000 + i;
        let new = txn(
            date,
            amounts(cents, 0),
            vec![split(groceries, 600, 0), split(household, cents - 600, 0)],
        );
        transactions::save(&db, &new).unwrap_or_else(|e| panic!("save {i}: {e}"));
    }

    let rows = transactions::list(&db, &TxnFilter::default()).expect("list");
    assert_eq!(rows.len(), 50);
    assert_eq!(
        transactions::count(&db, &TxnFilter::default()).expect("count"),
        50
    );

    for row in &rows {
        assert_eq!(
            row.splits.len(),
            2,
            "txn {:?} came back with {} splits",
            row.id,
            row.splits.len()
        );
        assert_eq!(
            row.splits.iter().map(|s| s.amount).sum::<Money>(),
            row.total,
            "txn {:?} was assembled from the wrong splits",
            row.id
        );
    }
    let keys: Vec<(NaiveDate, TxnId)> = rows.iter().map(|t| (t.txn_date, t.id)).collect();
    assert!(
        keys.windows(2).all(|w| w[0] > w[1]),
        "ordering must survive at size"
    );

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

#[test]
fn deleting_a_transaction_takes_its_splits_and_line_items_with_it() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household");

    let mut new = txn(
        day("2026-08-13"),
        amounts(5000, 400),
        vec![split(groceries, 3000, 240), split(household, 2000, 160)],
    );
    // The items must come to the split's *amount* — $30.00 plus its $2.40 of
    // tax — because that invariant is what `save` checks before writing.
    new.splits[0].line_items = vec![item("Milk", 1240), item("Bread", 2000)];
    let id = transactions::save(&db, &new).expect("save");
    let before = row_counts(&db);
    assert_eq!(before.splits, 2);
    assert_eq!(before.line_items, 2);

    transactions::delete(&db, id).expect("delete");

    let after = row_counts(&db);
    assert_eq!(after.txns, 0);
    // Through ON DELETE CASCADE, twice over: an orphaned split is money that
    // belongs to no receipt, and the integrity check would rightly call it a
    // violation.
    assert_eq!(after.splits, 0, "splits outlived their transaction");
    assert_eq!(after.line_items, 0, "line items outlived their split");
    assert_clean(&db);
}

#[test]
fn deleting_leaves_the_other_receipts_alone() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    let keep = transactions::save(
        &db,
        &txn(
            day("2026-08-12"),
            amounts(1000, 80),
            vec![split(groceries, 1000, 80)],
        ),
    )
    .expect("save");
    let doomed = transactions::save(
        &db,
        &txn(
            day("2026-08-13"),
            amounts(2000, 160),
            vec![split(groceries, 2000, 160)],
        ),
    )
    .expect("save");

    transactions::delete(&db, doomed).expect("delete");

    assert_eq!(row_counts(&db).txns, 1);
    let survivor = transactions::get(&db, keep).expect("the other receipt survives");
    assert_eq!(survivor.total, m(1080));
    assert_clean(&db);
}

#[test]
fn deleting_a_receipt_that_is_already_gone_says_so() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let id = transactions::save(
        &db,
        &txn(
            day("2026-08-13"),
            amounts(1000, 80),
            vec![split(groceries, 1000, 80)],
        ),
    )
    .expect("save");

    transactions::delete(&db, id).expect("first delete");

    // Two windows open on one database is ordinary, and a silent success would
    // tell the second one it removed something it did not.
    let error = transactions::delete(&db, id).expect_err("second delete");
    assert!(
        matches!(error, CoreError::NotFound(_)),
        "expected NotFound, got {error:?}"
    );
    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// Filtering and ordering the list
// ---------------------------------------------------------------------------

/// Three expenses on three days, with amounts that do not follow the dates —
/// so a date sort and a total sort cannot accidentally agree.
fn three_expenses(db: &Db) -> (TxnId, TxnId, TxnId) {
    let groceries = cat(db, "Groceries");
    let household = cat(db, "Household");
    let costco = merchant(db, "Costco");
    let target = merchant(db, "Target");

    let mut small = txn(
        day("2026-08-10"),
        amounts(1_000, 0),
        vec![split(groceries, 1_000, 0)],
    );
    small.merchant_id = Some(costco);
    let mut large = txn(
        day("2026-08-11"),
        amounts(9_000, 0),
        vec![split(household, 9_000, 0)],
    );
    large.merchant_id = Some(target);
    let mut middle = txn(
        day("2026-08-12"),
        amounts(5_000, 0),
        vec![split(groceries, 5_000, 0)],
    );
    middle.merchant_id = Some(costco);

    (
        transactions::save(db, &small).expect("save"),
        transactions::save(db, &large).expect("save"),
        transactions::save(db, &middle).expect("save"),
    )
}

fn listed(db: &Db, filter: &TxnFilter) -> Vec<TxnId> {
    transactions::list(db, filter)
        .expect("list")
        .into_iter()
        .map(|t| t.id)
        .collect()
}

#[test]
fn the_default_order_is_newest_first() {
    let db = seeded_db();
    let (small, large, middle) = three_expenses(&db);

    assert_eq!(
        listed(&db, &TxnFilter::default()),
        vec![middle, large, small],
        "a list of expenses is read newest first"
    );
    assert_clean(&db);
}

#[test]
fn every_order_is_available_and_total() {
    let db = seeded_db();
    let (small, large, middle) = three_expenses(&db);
    let sorted = |sort| {
        listed(
            &db,
            &TxnFilter {
                sort,
                ..TxnFilter::default()
            },
        )
    };

    assert_eq!(sorted(TxnSort::DateAsc), vec![small, large, middle]);
    assert_eq!(sorted(TxnSort::TotalDesc), vec![large, middle, small]);
    assert_eq!(sorted(TxnSort::TotalAsc), vec![small, middle, large]);
    assert_clean(&db);
}

#[test]
fn ties_do_not_reshuffle_between_queries() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    // Same day, same amount, three times over: without a tiebreak the order
    // would be whatever SQLite felt like, and a list that reorders itself under
    // the user looks like the data changing.
    let same = |_| {
        transactions::save(
            &db,
            &txn(
                day("2026-08-13"),
                amounts(2_500, 0),
                vec![split(groceries, 2_500, 0)],
            ),
        )
        .expect("save")
    };
    let saved: Vec<TxnId> = (0..3).map(same).collect();

    for sort in [
        TxnSort::DateDesc,
        TxnSort::DateAsc,
        TxnSort::TotalDesc,
        TxnSort::TotalAsc,
    ] {
        let filter = TxnFilter {
            sort,
            ..TxnFilter::default()
        };
        let first = listed(&db, &filter);
        assert_eq!(first, listed(&db, &filter), "{sort:?} is not stable");
        assert_eq!(first.len(), saved.len());
    }
    assert_clean(&db);
}

#[test]
fn filtering_by_store_and_by_category_narrows_the_list() {
    let db = seeded_db();
    let (small, _large, middle) = three_expenses(&db);

    let by_store = listed(
        &db,
        &TxnFilter {
            merchant_ids: vec![merchant(&db, "Costco")],
            ..TxnFilter::default()
        },
    );
    assert_eq!(by_store, vec![middle, small], "Costco's two, newest first");

    let by_category = listed(
        &db,
        &TxnFilter {
            category_ids: vec![cat(&db, "Household")],
            ..TxnFilter::default()
        },
    );
    assert_eq!(by_category.len(), 1);

    // Both at once is an AND, not an OR: Target sold the household goods, so
    // Costco plus Household is nothing rather than everything.
    let both = listed(
        &db,
        &TxnFilter {
            merchant_ids: vec![merchant(&db, "Costco")],
            category_ids: vec![cat(&db, "Household")],
            ..TxnFilter::default()
        },
    );
    assert!(both.is_empty(), "{both:?}");
    assert_clean(&db);
}

#[test]
fn several_stores_or_categories_can_be_shown_at_once() {
    let db = seeded_db();
    let (small, large, middle) = three_expenses(&db);

    // The list's filter is a column of checkboxes, so "these two shops" is the
    // ordinary request — not something to ask for one query at a time.
    let both_stores = listed(
        &db,
        &TxnFilter {
            merchant_ids: vec![merchant(&db, "Costco"), merchant(&db, "Target")],
            ..TxnFilter::default()
        },
    );
    assert_eq!(both_stores, vec![middle, large, small]);

    let two_categories = listed(
        &db,
        &TxnFilter {
            category_ids: vec![cat(&db, "Groceries"), cat(&db, "Household")],
            ..TxnFilter::default()
        },
    );
    assert_eq!(two_categories.len(), 3);
    assert_clean(&db);
}

#[test]
fn an_empty_filter_shows_everything_rather_than_nothing() {
    let db = seeded_db();
    three_expenses(&db);

    // Nothing ticked is "every store", not "no stores". The other reading
    // would empty the list the moment the filter was opened.
    let all = listed(
        &db,
        &TxnFilter {
            merchant_ids: Vec::new(),
            category_ids: Vec::new(),
            ..TxnFilter::default()
        },
    );
    assert_eq!(all.len(), 3);
    assert_clean(&db);
}

#[test]
fn a_storeless_expense_can_be_kept_in_view() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let costco = merchant(&db, "Costco");

    let mut shopped = txn(
        day("2026-08-10"),
        amounts(1_000, 0),
        vec![split(groceries, 1_000, 0)],
    );
    shopped.merchant_id = Some(costco);
    let shopped = transactions::save(&db, &shopped).expect("save");
    // What `quick_add` leaves behind: an amount, no store, sort it out later.
    let anonymous = transactions::save(
        &db,
        &txn(
            day("2026-08-11"),
            amounts(2_000, 0),
            vec![split(groceries, 2_000, 0)],
        ),
    )
    .expect("save");

    let named_only = TxnFilter {
        merchant_ids: vec![costco],
        ..TxnFilter::default()
    };
    assert_eq!(listed(&db, &named_only), vec![shopped]);

    // Without this, touching the store filter would silently hide every
    // quick-added expense — the kind most in need of being found again.
    let with_blanks = TxnFilter {
        merchant_ids: vec![costco],
        include_no_merchant: true,
        ..TxnFilter::default()
    };
    assert_eq!(listed(&db, &with_blanks), vec![anonymous, shopped]);
    assert_clean(&db);
}

#[test]
fn the_filter_menus_offer_only_what_the_columns_hold() {
    let db = seeded_db();
    three_expenses(&db);

    let values = transactions::filter_values(&db).expect("filter values");

    // The seed installs two dozen suggested shops. A menu built from those is
    // mostly rows that match nothing, and ticking one off does nothing
    // visible — which reads as a broken control.
    let stores: Vec<&str> = values.merchants.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(stores, vec!["Costco", "Target"]);
    let cats: Vec<&str> = values.categories.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cats, vec!["Groceries", "Household"]);
    assert!(!values.any_without_merchant);
    assert_clean(&db);
}

#[test]
fn a_storeless_expense_is_reported_so_it_can_be_offered() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    transactions::save(
        &db,
        &txn(
            day("2026-08-13"),
            amounts(1_000, 0),
            vec![split(groceries, 1_000, 0)],
        ),
    )
    .expect("save");

    let values = transactions::filter_values(&db).expect("filter values");
    assert!(values.merchants.is_empty());
    // Without this the menu would have no box for them, and touching the store
    // filter would hide every quick-added expense with no way to get it back.
    assert!(values.any_without_merchant);
    assert_clean(&db);
}

#[test]
fn a_deleted_expense_stops_appearing_in_the_menus() {
    let db = seeded_db();
    let (small, _large, _middle) = three_expenses(&db);
    transactions::delete(&db, small).expect("delete");

    // Costco still has the third expense, so it stays. What must not happen is
    // a menu row for a shop with nothing left behind it.
    let values = transactions::filter_values(&db).expect("filter values");
    let stores: Vec<&str> = values.merchants.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(stores, vec!["Costco", "Target"]);

    transactions::delete(&db, _middle).expect("delete");
    let values = transactions::filter_values(&db).expect("filter values");
    let stores: Vec<&str> = values.merchants.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(stores, vec!["Target"], "Costco has nothing left");
    assert_clean(&db);
}
