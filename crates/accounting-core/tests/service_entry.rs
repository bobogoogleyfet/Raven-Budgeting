//! The entry service — the layer a UI actually talks to.
//!
//! Everything below the service is deterministic arithmetic. The service is
//! where that arithmetic meets remembered state (the store's usual category,
//! the location's tax rate) and where the app decides what it is *allowed* to
//! write. Three properties carry the whole design:
//!
//! * **Preview never writes.** The UI calls it on every keystroke; if it
//!   created a merchant or a location on the way, a user browsing an entry
//!   screen would silently accumulate junk they never saved.
//! * **Blockers block, warnings do not.** An unassigned remainder is money in
//!   no category and must be refused; an estimated tax figure is merely worth
//!   saying out loud.
//! * **Memory is written on save and only on save**, from a receipt that
//!   actually says something — one split says "this store is Groceries", three
//!   splits say nothing at all.
//!
//! Row-count snapshots, rather than "did the one table I was thinking about
//! change", are used for every no-write assertion.

mod common;

use accounting_core::domain::reconcile::AmountEntry;
use accounting_core::domain::splits::{LineItemDraft, SplitAmount, SplitDraft};
use accounting_core::repo::transactions::TxnFilter;
use accounting_core::repo::{categories, locations, merchants, transactions};
use accounting_core::service::entry::{self, TransactionDraft};
use accounting_core::{CategoryId, CoreError, Db, DetailLevel, Money, TxnId};
use chrono::NaiveDate;
use common::{assert_clean, row_counts, seeded_db};

/// A fixed, explicit "now". The core never reads the clock — every timestamp
/// it stores is one it was handed, which is what makes these tests
/// deterministic without any clock mocking.
const NOW: &str = "2026-08-13T12:34:56.789Z";

fn m(cents: i64) -> Money {
    Money::from_cents(cents)
}

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap_or_else(|e| panic!("date `{s}`: {e}"))
}

fn cat(db: &Db, name: &str) -> CategoryId {
    categories::resolve(db, name)
        .unwrap_or_else(|e| panic!("resolve category `{name}`: {e}"))
        .id
}

fn split_at(category: CategoryId, amount: SplitAmount) -> SplitDraft {
    SplitDraft {
        category_id: Some(category),
        ..SplitDraft::new(amount)
    }
}

/// A draft against the seeded home location, which exists and has a 0% rate.
fn draft(entry: AmountEntry, splits: Vec<SplitDraft>) -> TransactionDraft {
    TransactionDraft {
        id: None,
        txn_date: day("2026-08-13"),
        merchant: None,
        location: Some("Home".to_string()),
        entry,
        splits,
        detail_level: None,
        notes: None,
        needs_review: false,
        remember_defaults: false,
    }
}

fn merchant_named(db: &Db, name: &str) -> accounting_core::repo::merchants::Merchant {
    merchants::find_by_name(db, name)
        .expect("find_by_name")
        .unwrap_or_else(|| panic!("merchant `{name}` must exist"))
}

// ---------------------------------------------------------------------------
// entry_context
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_merchant_yields_an_empty_context_without_creating_a_row() {
    // The entry screen calls this the moment the user starts typing a store
    // name. Creating the merchant here would fill the database with every
    // abandoned keystroke and every typo.
    let db = seeded_db();
    let before = row_counts(&db);

    let ctx = entry::entry_context(&db, Some("Never Heard Of It")).expect("entry_context");

    assert!(ctx.merchant.is_none());
    assert_eq!(
        row_counts(&db),
        before,
        "entry_context must never create the merchant it was asked about"
    );

    // With nothing remembered, the fallbacks apply: the home location, its
    // rate, and grouped entry.
    assert_eq!(
        ctx.location.as_ref().map(|l| l.name.as_str()),
        Some("Home"),
        "with no merchant memory the home location is the only sensible default"
    );
    assert_eq!(
        ctx.tax.rate_ppm, 0,
        "the seeded home deliberately has no rate"
    );
    assert!(!ctx.tax.prices_include_tax);
    assert_eq!(ctx.detail_level, DetailLevel::Grouped);
    assert!(ctx.default_category.is_none());
}

#[test]
fn a_context_without_any_merchant_name_still_offers_home_and_recent_stores() {
    let db = seeded_db();
    merchants::get_or_create(&db, "Costco").expect("create");
    merchants::touch(&db, merchant_named(&db, "Costco").id, NOW).expect("touch");

    let ctx = entry::entry_context(&db, None).expect("entry_context");

    assert!(ctx.merchant.is_none());
    assert_eq!(ctx.location.as_ref().map(|l| l.name.as_str()), Some("Home"));
    assert!(
        ctx.recent_merchants.iter().any(|mm| mm.name == "Costco"),
        "the recent-store chips are the point of the context; got {:?}",
        ctx.recent_merchants
            .iter()
            .map(|mm| &mm.name)
            .collect::<Vec<_>>()
    );
    assert!(
        ctx.recent_merchants.len() <= 8,
        "the entry screen renders these as chips and cannot grow a scrollbar"
    );

    assert_clean(&db);
}

#[test]
fn a_context_inherits_the_location_and_rate_a_merchant_remembers() {
    let db = seeded_db();
    merchants::get_or_create(&db, "Costco").expect("create merchant");
    locations::get_or_create(&db, "Costco Sunnyvale", 91_250).expect("create location");
    let costco = merchant_named(&db, "Costco");
    let store = locations::find_by_name(&db, "Costco Sunnyvale")
        .unwrap()
        .expect("store");
    let groceries = cat(&db, "Groceries");
    merchants::remember(
        &db,
        costco.id,
        Some(groceries),
        Some(store.id),
        DetailLevel::Itemized,
    )
    .expect("remember");

    let ctx = entry::entry_context(&db, Some("Costco")).expect("entry_context");

    assert_eq!(ctx.merchant.as_ref().map(|mm| mm.id), Some(costco.id));
    assert_eq!(
        ctx.location.as_ref().map(|l| l.id),
        Some(store.id),
        "the remembered location must beat the home default"
    );
    // The tax rate is a property of the *store's* location, not the user's:
    // this is the entire reason locations exist.
    assert_eq!(ctx.tax.rate_ppm, 91_250);
    assert_eq!(ctx.default_category.map(|c| c.id), Some(groceries));
    assert_eq!(ctx.detail_level, DetailLevel::Itemized);

    assert_clean(&db);
}

#[test]
fn a_merchant_with_no_remembered_location_falls_back_to_home() {
    let db = seeded_db();
    merchants::get_or_create(&db, "Corner Store").expect("create merchant");

    let ctx = entry::entry_context(&db, Some("Corner Store")).expect("entry_context");

    assert!(ctx.merchant.is_some(), "the merchant itself is known");
    assert_eq!(
        ctx.location.as_ref().map(|l| l.name.as_str()),
        Some("Home"),
        "a store with no remembered location must fall back to home, not to nothing"
    );
    assert_eq!(ctx.detail_level, DetailLevel::Grouped);

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// preview_transaction
// ---------------------------------------------------------------------------

#[test]
fn previewing_a_transaction_writes_nothing_at_all() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let before = row_counts(&db);

    let mut d = draft(
        AmountEntry::total_only(m(5432)),
        vec![split_at(groceries, SplitAmount::Remainder)],
    );
    // A merchant that does not exist yet, because that is the tempting one to
    // create early — the preview needs its remembered defaults.
    d.merchant = Some("Brand New Store".to_string());

    let preview = entry::preview_transaction(&db, &d).expect("preview");
    assert!(
        preview.can_save(),
        "this draft is complete; got {:?}",
        preview.blockers
    );

    assert_eq!(
        row_counts(&db),
        before,
        "preview is called on every keystroke — it must be a pure read"
    );
}

#[test]
fn an_unassigned_remainder_blocks_saving_until_it_is_claimed() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household Supplies");

    // $100.00 entered, $60.00 assigned. The missing $40.00 belongs to no
    // category, and a saved transaction whose splits do not sum to its total
    // breaks the one invariant no constraint can catch.
    let mut d = draft(
        AmountEntry::total_only(m(10_000)),
        vec![split_at(groceries, SplitAmount::Fixed(m(6000)))],
    );

    let preview = entry::preview_transaction(&db, &d).expect("preview");
    assert_eq!(preview.plan.unassigned, m(4000));
    assert!(
        !preview.blockers.is_empty(),
        "money in no category must be a blocker"
    );
    assert!(!preview.can_save());
    assert!(
        preview.blockers.iter().any(|b| b.contains("40.00")),
        "a blocker must say how much is unassigned — the number is the whole \
         recovery path; got {:?}",
        preview.blockers
    );

    // Claiming the rest clears it.
    d.splits.push(split_at(household, SplitAmount::Remainder));
    let preview = entry::preview_transaction(&db, &d).expect("preview");
    assert_eq!(preview.plan.unassigned, Money::ZERO);
    assert!(preview.blockers.is_empty(), "got {:?}", preview.blockers);
    assert!(preview.can_save());
}

#[test]
fn a_split_with_no_category_blocks_saving() {
    // An explicitly drafted split with no category is a half-finished thought,
    // not an instruction to file it under Uncategorized: the user asked for a
    // split, so silently picking a category for them would hide the mistake.
    let db = seeded_db();
    let d = draft(
        AmountEntry::total_only(m(1000)),
        vec![SplitDraft::new(SplitAmount::Remainder)],
    );

    let preview = entry::preview_transaction(&db, &d).expect("preview");
    assert!(
        !preview.blockers.is_empty(),
        "an uncategorised split must block"
    );
    assert!(!preview.can_save());

    // And a blocked draft must be refused by the write path too, not merely
    // discouraged by the preview.
    let before = row_counts(&db);
    let err = entry::save_transaction(&db, &d, NOW).expect_err("a blocked draft must not save");
    assert!(matches!(err, CoreError::Invalid(_)), "got {err:?}");
    assert_eq!(row_counts(&db), before);
}

#[test]
fn an_empty_split_list_falls_back_to_a_single_remainder_row() {
    // The quick path: type a total, hit save. With a store that remembers a
    // category the receipt files itself; without one it lands in the inbox
    // category rather than being refused.
    let db = seeded_db();
    let uncategorized = categories::uncategorized(&db).expect("uncategorized").id;

    let d = draft(AmountEntry::total_only(m(1240)), Vec::new());
    let preview = entry::preview_transaction(&db, &d).expect("preview");

    assert_eq!(preview.plan.rows.len(), 1);
    assert_eq!(preview.plan.rows[0].category_id, Some(uncategorized));
    assert_eq!(preview.plan.rows[0].amount, m(1240));
    assert_eq!(preview.plan.unassigned, Money::ZERO);
    assert!(preview.can_save());

    // With a store memory, the same empty draft picks the remembered category.
    merchants::get_or_create(&db, "Costco").expect("create");
    let groceries = cat(&db, "Groceries");
    merchants::remember(
        &db,
        merchant_named(&db, "Costco").id,
        Some(groceries),
        None,
        DetailLevel::Grouped,
    )
    .expect("remember");
    let mut with_store = draft(AmountEntry::total_only(m(1240)), Vec::new());
    with_store.merchant = Some("Costco".to_string());

    let preview = entry::preview_transaction(&db, &with_store).expect("preview");
    assert_eq!(
        preview.plan.rows[0].category_id,
        Some(groceries),
        "the store memory exists precisely so this decision does not have to be made"
    );

    assert_clean(&db);
}

#[test]
fn an_estimated_tax_figure_is_warned_about_and_an_entered_one_is_not() {
    // `tax_is_estimated` is the honesty flag: an estimate that reads like a
    // fact is worse than no estimate. It is set structurally by `reconcile`,
    // and the service must surface it rather than quietly dropping it.
    let db = seeded_db();
    locations::get_or_create(&db, "Costco Sunnyvale", 91_250).expect("create location");
    let groceries = cat(&db, "Groceries");

    let mut estimated = draft(
        AmountEntry::total_only(m(5432)),
        vec![split_at(groceries, SplitAmount::Remainder)],
    );
    estimated.location = Some("Costco Sunnyvale".to_string());

    let preview = entry::preview_transaction(&db, &estimated).expect("preview");
    assert!(preview.amounts.tax_is_estimated);
    assert_eq!(preview.tax_rate_ppm, 91_250);
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.to_lowercase().contains("estimat")),
        "a tax figure we invented must be called out; got {:?}",
        preview.warnings
    );
    // A warning is not a blocker: the estimate is usable, it just has to be
    // labelled.
    assert!(preview.can_save());

    // The same receipt with the tax read off the paper.
    let mut entered = estimated;
    entered.entry = AmountEntry {
        subtotal: Some(m(5012)),
        tax: Some(m(420)),
        total: None,
        last_edited: None,
    };
    let preview = entry::preview_transaction(&db, &entered).expect("preview");
    assert!(!preview.amounts.tax_is_estimated);
    assert_eq!(preview.amounts.total, m(5432));
    assert!(
        !preview
            .warnings
            .iter()
            .any(|w| w.to_lowercase().contains("estimat")),
        "a tax figure the user typed is a fact; got {:?}",
        preview.warnings
    );
}

#[test]
fn an_itemized_draft_with_no_line_items_anywhere_is_warned_about() {
    // The detail level and the data have to agree, or `check_all` reports a
    // DetailLevelMismatch on a transaction the user thought they had itemised.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let mut d = draft(
        AmountEntry::total_only(m(1000)),
        vec![split_at(groceries, SplitAmount::Remainder)],
    );
    d.detail_level = Some(DetailLevel::Itemized);

    let preview = entry::preview_transaction(&db, &d).expect("preview");
    assert!(
        !preview.warnings.is_empty(),
        "claiming to be itemised with nothing itemised deserves a word"
    );
    assert!(preview.can_save(), "it is still coherent, just odd");
}

#[test]
fn a_location_name_that_does_not_exist_is_an_error_not_a_silent_zero_rate() {
    // The failure mode this prevents: a typo'd store falls back to "no
    // location", which means a 0% rate, which means every subsequent receipt
    // reconciles to a plausible but wrong tax. Loud is the only safe option.
    let db = seeded_db();
    locations::get_or_create(&db, "Costco Sunnyvale", 91_250).expect("create location");
    let groceries = cat(&db, "Groceries");

    let mut d = draft(
        AmountEntry::total_only(m(5432)),
        vec![split_at(groceries, SplitAmount::Remainder)],
    );
    d.location = Some("Costko Sunnyvale".to_string());

    let err = entry::preview_transaction(&db, &d).expect_err("a typo'd location must fail");
    assert!(
        matches!(err, CoreError::NotFound(_)),
        "an unknown location is missing, not invalid — and above all it is not \
         a 0% rate; got {err:?}"
    );
    assert!(
        err.to_string().contains("Costko"),
        "the message must echo what was typed; got: {err}"
    );
}

// ---------------------------------------------------------------------------
// save_transaction and store memory
// ---------------------------------------------------------------------------

#[test]
fn saving_a_single_split_receipt_teaches_the_store_its_category() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    locations::get_or_create(&db, "Costco Sunnyvale", 91_250).expect("create location");
    let store = locations::find_by_name(&db, "Costco Sunnyvale")
        .unwrap()
        .expect("store");

    let mut d = draft(
        AmountEntry::total_only(m(5432)),
        vec![split_at(groceries, SplitAmount::Remainder)],
    );
    d.merchant = Some("Costco".to_string());
    d.location = Some("Costco Sunnyvale".to_string());
    d.remember_defaults = true;

    let id = entry::save_transaction(&db, &d, NOW).expect("save_transaction");

    let costco = merchant_named(&db, "Costco");
    assert_eq!(
        costco.default_category_id,
        Some(groceries),
        "one split is an unambiguous statement about what this store sells"
    );
    assert_eq!(costco.default_location_id, Some(store.id));
    assert_eq!(
        costco.last_used_at.as_deref(),
        Some(NOW),
        "the core never reads the clock: `now` is exactly what the caller passed"
    );

    let saved = transactions::get(&db, id).expect("get");
    assert_eq!(saved.merchant_name.as_deref(), Some("Costco"));
    assert_eq!(saved.total, m(5432));
    assert_clean(&db);
}

#[test]
fn saving_a_three_way_split_teaches_the_store_nothing_about_categories() {
    // A receipt spread across three envelopes says nothing about what the
    // store *usually* is. Recording one of the three as the default would be
    // an invented fact that then autofills every future visit.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household Supplies");
    let personal = cat(&db, "Personal Care");
    locations::get_or_create(&db, "Costco Sunnyvale", 91_250).expect("create location");
    let store = locations::find_by_name(&db, "Costco Sunnyvale")
        .unwrap()
        .expect("store");

    let mut d = draft(
        AmountEntry::total_only(m(10_000)),
        vec![
            split_at(groceries, SplitAmount::Fixed(m(5000))),
            split_at(household, SplitAmount::Fixed(m(3000))),
            split_at(personal, SplitAmount::Remainder),
        ],
    );
    d.merchant = Some("Costco".to_string());
    d.location = Some("Costco Sunnyvale".to_string());
    d.remember_defaults = true;

    // Captured first: Costco is one of the seeded suggestions and already
    // carries a default, so the property is that a three-way split leaves it
    // *unchanged* — not that it is None. `remember` treats None as "keep what
    // you had", and clobbering a good default with a guess drawn from the
    // largest of three splits is exactly what this guards against.
    let before = merchant_named(&db, "Costco").default_category_id;

    entry::save_transaction(&db, &d, NOW).expect("save_transaction");

    let costco = merchant_named(&db, "Costco");
    assert_eq!(
        costco.default_category_id, before,
        "three splits are not a default category"
    );
    // The location, on the other hand, is unambiguous however many splits
    // there were: that is where the shop is.
    assert_eq!(costco.default_location_id, Some(store.id));
    assert_eq!(costco.last_used_at.as_deref(), Some(NOW));

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// quick_add, the inbox, and categorize
// ---------------------------------------------------------------------------

#[test]
fn quick_add_files_an_uncategorized_receipt_into_the_inbox() {
    // Three seconds and no decisions: the whole reason a fallback category
    // exists. What it must *not* do is invent a tax figure — nothing was said
    // about tax, so nothing is claimed about it.
    let db = seeded_db();
    let uncategorized = categories::uncategorized(&db).expect("uncategorized").id;

    let id = entry::quick_add(&db, day("2026-08-13"), m(1240), Some("Corner Store"), NOW)
        .expect("quick_add");

    let saved = transactions::get(&db, id).expect("get");
    assert_eq!(saved.total, m(1240));
    assert_eq!(saved.tax, Money::ZERO);
    assert_eq!(saved.subtotal, m(1240));
    assert!(
        !saved.tax_is_estimated,
        "quick-add did not estimate the tax, it declined to have an opinion — \
         claiming an estimate would put a made-up number in the export"
    );
    assert!(saved.needs_review);
    assert_eq!(saved.splits.len(), 1);
    assert_eq!(saved.splits[0].category_id, uncategorized);
    assert_eq!(saved.merchant_name.as_deref(), Some("Corner Store"));

    let inbox = entry::needs_categorizing(&db, None).expect("needs_categorizing");
    assert!(
        inbox.iter().any(|t| t.id == id),
        "a quick-added receipt must show up in the inbox, or it is simply lost"
    );

    assert_clean(&db);
}

#[test]
fn categorizing_clears_the_inbox_and_preserves_the_total_and_the_uuid() {
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household Supplies");

    let id = entry::quick_add(&db, day("2026-08-13"), m(2001), Some("Corner Store"), NOW)
        .expect("quick_add");
    let before = transactions::get(&db, id).expect("get");
    let uuid = before.uuid.clone();

    entry::categorize(
        &db,
        id,
        &[
            split_at(groceries, SplitAmount::Percent(3333)),
            split_at(household, SplitAmount::Remainder),
        ],
    )
    .expect("categorize");

    let after = transactions::get(&db, id).expect("get");
    assert_eq!(
        after.total,
        m(2001),
        "categorising re-plans the split of a known total; it must never move the total"
    );
    assert_eq!(after.subtotal, before.subtotal);
    assert_eq!(after.tax, before.tax);
    assert_eq!(
        after.uuid, uuid,
        "the receipt is the same fact it was before it was filed"
    );
    assert!(!after.needs_review, "it has been reviewed");
    assert_eq!(
        after.splits.iter().map(|s| s.amount).sum::<Money>(),
        m(2001)
    );

    let inbox = entry::needs_categorizing(&db, None).expect("needs_categorizing");
    assert!(
        !inbox.iter().any(|t| t.id == id),
        "a categorised receipt must leave the inbox, or the inbox never empties"
    );

    assert_clean(&db);
}

#[test]
fn a_quick_add_and_categorize_round_trip_keeps_every_cent() {
    // $20.01 across 33.33% / 33.33% / remainder. Rounding each share
    // independently gives 666 + 666 + 667 = 1999 and loses two cents; the
    // largest-remainder allocation hands those two cents back out, so the
    // three rows are 667 / 667 / 667 and sum to exactly 2001.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household Supplies");
    let personal = cat(&db, "Personal Care");

    let id = entry::quick_add(&db, day("2026-08-13"), m(2001), None, NOW).expect("quick_add");

    entry::categorize(
        &db,
        id,
        &[
            split_at(groceries, SplitAmount::Percent(3333)),
            split_at(household, SplitAmount::Percent(3333)),
            split_at(personal, SplitAmount::Remainder),
        ],
    )
    .expect("categorize");

    let saved = transactions::get(&db, id).expect("get");
    let shares: Vec<Money> = saved.splits.iter().map(|s| s.amount).collect();
    assert_eq!(shares, vec![m(667), m(667), m(667)]);
    assert_eq!(shares.iter().copied().sum::<Money>(), m(2001));

    assert_clean(&db);
}

#[test]
fn the_inbox_honours_its_limit() {
    let db = seeded_db();
    for i in 0..5_i64 {
        entry::quick_add(&db, day("2026-08-13"), m(100 + i), None, NOW).expect("quick_add");
    }

    let all = entry::needs_categorizing(&db, None).expect("needs_categorizing");
    assert_eq!(all.len(), 5);
    let some = entry::needs_categorizing(&db, Some(2)).expect("needs_categorizing");
    assert!(some.len() <= 2, "asked for 2, got {}", some.len());

    assert_clean(&db);
}

// ---------------------------------------------------------------------------
// repeat_last
// ---------------------------------------------------------------------------

#[test]
fn repeat_last_returns_a_draft_and_writes_absolutely_nothing() {
    // The duplicate-receipt guard. "Repeat last" is one keystroke away from
    // "save"; if it committed as it read, a user glancing at last week's shop
    // would have booked it again.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");
    let household = cat(&db, "Household Supplies");

    let mut original = draft(
        AmountEntry {
            subtotal: Some(m(5012)),
            tax: Some(m(420)),
            total: None,
            last_edited: None,
        },
        vec![
            split_at(groceries, SplitAmount::Fixed(m(4000))),
            split_at(household, SplitAmount::Remainder),
        ],
    );
    original.merchant = Some("Costco".to_string());
    original.detail_level = Some(DetailLevel::Grouped);
    let first = entry::save_transaction(&db, &original, NOW).expect("save_transaction");
    let stored = transactions::get(&db, first).expect("get");
    let before = row_counts(&db);

    let repeated = entry::repeat_last(&db, "Costco", day("2026-08-20")).expect("repeat_last");

    assert_eq!(
        row_counts(&db),
        before,
        "repeat_last hands back a draft; the user still has to press save"
    );
    assert!(
        repeated.id.is_none(),
        "the draft must be uncommitted — an id would make the next save silently \
         overwrite last week's receipt instead of adding this week's"
    );
    assert_eq!(
        repeated.txn_date,
        day("2026-08-20"),
        "on the date asked for"
    );
    assert_eq!(repeated.merchant.as_deref(), Some("Costco"));
    assert_eq!(repeated.detail_level, Some(stored.detail_level));

    let categories_repeated: Vec<Option<CategoryId>> =
        repeated.splits.iter().map(|s| s.category_id).collect();
    assert_eq!(
        categories_repeated,
        stored
            .splits
            .iter()
            .map(|s| Some(s.category_id))
            .collect::<Vec<_>>(),
        "the same categories, in the same order"
    );
    // Amounts come back as `Fixed`: a percentage or a remainder would silently
    // re-derive against whatever total the user types next, which is not what
    // "the same as last time" means.
    let amounts: Vec<Money> = repeated
        .splits
        .iter()
        .map(|s| match s.amount {
            SplitAmount::Fixed(v) => v,
            other => panic!("repeated splits must be Fixed amounts, got {other:?}"),
        })
        .collect();
    assert_eq!(amounts.iter().copied().sum::<Money>(), stored.total);

    assert_clean(&db);
}

#[test]
fn repeating_an_itemized_receipt_leaves_the_database_coherent() {
    // Itemizing the weekly grocery run and then repeating it next week is the
    // most ordinary sequence this feature has. It must not leave `pacc doctor`
    // permanently complaining: a warning produced by normal use is a warning
    // nobody reads, and "0 errors, 0 warnings" stops meaning anything.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    let line = |description: &str, cents: i64| LineItemDraft {
        description: description.to_string(),
        quantity_milli: 1_000,
        unit_price: Some(m(cents)),
        amount: m(cents),
    };
    let mut itemized_split = split_at(groceries, SplitAmount::Remainder);
    // $12.00 + $8.00 + $10.00 = $30.00.
    itemized_split.line_items = vec![
        line("Apples", 1200),
        line("Bread", 800),
        line("Cheese", 1000),
    ];

    let mut original = draft(AmountEntry::total_only(m(3000)), vec![itemized_split]);
    original.merchant = Some("Costco".to_string());
    original.detail_level = Some(DetailLevel::Itemized);
    let first = entry::save_transaction(&db, &original, NOW).expect("save the itemized receipt");
    assert_eq!(
        transactions::get(&db, first).expect("get").detail_level,
        DetailLevel::Itemized
    );
    assert_clean(&db);

    let repeated = entry::repeat_last(&db, "Costco", day("2026-08-20")).expect("repeat_last");
    let second = entry::save_transaction(&db, &repeated, NOW).expect("save the repeat");

    // The repeat carries the detail level across but not the line items, so
    // the stored transaction claims to be itemized with nothing itemized.
    // Either half of the fix is fine — repeat the line items too, or store the
    // level the data actually supports — but the two must agree.
    let stored = transactions::get(&db, second).expect("get");
    let items: usize = stored.splits.iter().map(|s| s.line_items.len()).sum();
    assert!(
        (stored.detail_level == DetailLevel::Itemized) == (items > 0),
        "a transaction's detail level must match its data: this one is {:?} with \
         {items} line items",
        stored.detail_level
    );

    assert_clean(&db);
}

#[test]
fn repeat_last_for_a_merchant_with_no_history_is_not_found() {
    let db = seeded_db();
    merchants::get_or_create(&db, "Never Shopped Here").expect("create");

    let err = entry::repeat_last(&db, "Never Shopped Here", day("2026-08-20"))
        .expect_err("no receipts to repeat");
    assert!(matches!(err, CoreError::NotFound(_)), "got {err:?}");

    // And a merchant that does not exist at all is the same answer, not a panic
    // and not an empty draft the user might save.
    let err = entry::repeat_last(&db, "Who?", day("2026-08-20")).expect_err("no such merchant");
    assert!(matches!(err, CoreError::NotFound(_)), "got {err:?}");
}

#[test]
fn saving_a_repeated_draft_creates_a_second_transaction() {
    // The other half of the guard: the draft has to be *saveable*, or "repeat
    // last" is just a read-only curiosity.
    let db = seeded_db();
    let groceries = cat(&db, "Groceries");

    let mut original = draft(
        AmountEntry::total_only(m(5432)),
        vec![split_at(groceries, SplitAmount::Remainder)],
    );
    original.merchant = Some("Costco".to_string());
    let first = entry::save_transaction(&db, &original, NOW).expect("save");

    let repeated = entry::repeat_last(&db, "Costco", day("2026-08-20")).expect("repeat_last");
    let second = entry::save_transaction(&db, &repeated, NOW).expect("save the repeat");

    assert_ne!(
        second, first,
        "repeating must add a receipt, not replace one"
    );
    let rows = transactions::list(&db, &TxnFilter::default()).expect("list");
    let found: Vec<TxnId> = rows.iter().map(|t| t.id).collect();
    assert!(found.contains(&first) && found.contains(&second));
    assert_eq!(transactions::get(&db, second).expect("get").total, m(5432));

    assert_clean(&db);
}
