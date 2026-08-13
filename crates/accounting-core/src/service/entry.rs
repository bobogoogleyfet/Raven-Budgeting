//! Transaction entry: draft → preview → saved aggregate.
//!
//! # Preview and save run the same code
//!
//! [`save_transaction`] is [`preview_transaction`] plus a write. Not "roughly
//! the same logic" — literally the same call, with the preview's numbers
//! persisted verbatim. A UI that shows one set of numbers and stores another
//! is the failure mode this arrangement makes impossible, and it is the reason
//! the preview carries `blockers` rather than the save re-deriving them.
//!
//! # Nothing here reads the clock
//!
//! `txn_date` arrives in the draft and `now` arrives as an ISO-8601 string,
//! because a frontend knows the user's timezone and the core does not — and
//! because a function that reads the clock cannot be tested without mocking
//! one.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::domain::reconcile::{AmountEntry, Amounts, TaxContext, reconcile};
use crate::domain::splits::{SplitAmount, SplitDraft, SplitPlan, plan_by_subtotal, plan_splits};
use crate::domain::tax::distribute_tax;
use crate::error::{CoreError, Result};
use crate::ids::{CategoryId, TxnId};
use crate::money::Money;
use crate::repo::categories::{self, Category};
use crate::repo::locations::{self, Location};
use crate::repo::merchants::{self, Merchant};
use crate::repo::transactions::{
    self, DetailLevel, LineItemRecord, NewTransaction, SplitRecord, Txn, TxnFilter,
};

/// How many recent merchants an entry screen offers as one-tap chips.
///
/// Eight because that is about what fits on a phone without scrolling; the
/// point of the chips is to remove a decision, and a list long enough to need
/// reading puts the decision back.
const RECENT_MERCHANTS: usize = 8;

/// Everything the entry screen needs to prefill itself before the user has
/// typed an amount.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryContext {
    /// The named merchant, if it already exists. A new name is not an error
    /// here and is not created — the merchant row appears on save, so that
    /// abandoning a half-typed entry leaves no debris.
    pub merchant: Option<Merchant>,
    /// The merchant's remembered location, else the home location.
    pub location: Option<Location>,
    /// Built from that location. Rate 0 and tax-exclusive when there is none —
    /// which is honest: nothing has told us what the rate is.
    pub tax: TaxContext,
    pub default_category: Option<Category>,
    pub detail_level: DetailLevel,
    pub recent_merchants: Vec<Merchant>,
}

/// A transaction as the user is describing it, before anything is resolved.
///
/// Names, not ids: this is what a text field or a URL query holds, and the
/// resolution rules (which merchant is "heb", which category is "groc") are
/// the service's business rather than every frontend's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionDraft {
    /// `Some` edits that transaction in place.
    pub id: Option<TxnId>,
    pub txn_date: NaiveDate,
    /// Merchant name. Created on save, never on preview.
    pub merchant: Option<String>,
    /// Location name. Must already exist — see [`preview_transaction`].
    pub location: Option<String>,
    pub entry: AmountEntry,
    /// Empty means "one split for everything", which is synthesised during
    /// preview so the user never has to say it.
    pub splits: Vec<SplitDraft>,
    /// `None` inherits the merchant's remembered preference.
    pub detail_level: Option<DetailLevel>,
    pub notes: Option<String>,
    pub needs_review: bool,
    /// Update the merchant's memory (category, location, detail level) on
    /// save.
    pub remember_defaults: bool,
}

/// The complete result of resolving a draft: the numbers that would be
/// written, and why they might not be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionPreview {
    pub amounts: Amounts,
    pub plan: SplitPlan,
    pub context: EntryContext,
    pub detail_level: DetailLevel,
    pub tax_rate_ppm: u32,
    /// Things the user should see but that do not stop a save.
    pub warnings: Vec<String>,
    /// Reasons the save is impossible.
    pub blockers: Vec<String>,
}

impl TransactionPreview {
    /// Whether [`save_transaction`] would accept this draft.
    #[must_use]
    pub fn can_save(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// What the entry screen should show before the user types anything.
///
/// The location is the merchant's remembered one, else home. That single rule
/// is most of the store-memory feature: the second receipt from a store needs
/// no location, no category and no detail-level decision.
pub fn entry_context(db: &Db, merchant_name: Option<&str>) -> Result<EntryContext> {
    resolve_context(db, merchant_name, None, None)
}

/// Resolve a draft into numbers, **without writing anything**.
///
/// This is what a UI calls on every keystroke, so it must be free of side
/// effects: a merchant named here is looked up, never created, and an
/// abandoned entry leaves the database exactly as it found it.
///
/// # A missing merchant is fine; a missing location is not
///
/// A merchant name that does not exist yet is the normal first-visit case. A
/// *location* name that does not exist is a typo, and falling back to "no
/// location" would silently switch the tax rate to 0% and produce a plausible,
/// wrong number — the single worst kind of bug this app can have. So it is
/// [`CoreError::NotFound`].
pub fn preview_transaction(db: &Db, draft: &TransactionDraft) -> Result<TransactionPreview> {
    preview_transaction_with_rate(db, draft, None)
}

/// [`preview_transaction`], with the location's tax rate overridden.
///
/// The rate normally comes from the location, because that is where a sales
/// tax rate actually lives. This exists for the one-off — `pacc add --rate
/// 8.375` at a store the user is not going to record — and it is threaded
/// through rather than pre-computed by the caller for one reason:
/// `tax_is_estimated`. A frontend that reconciled the amounts itself and
/// handed us a subtotal and a tax would produce a row claiming the tax was
/// read off the receipt. Passing the *rate* keeps the estimate flagged as an
/// estimate all the way into storage and the export.
pub fn preview_transaction_with_rate(
    db: &Db,
    draft: &TransactionDraft,
    rate_ppm: Option<u32>,
) -> Result<TransactionPreview> {
    let context = resolve_context(
        db,
        draft.merchant.as_deref(),
        draft.location.as_deref(),
        rate_ppm,
    )?;

    let amounts = reconcile(&draft.entry, &context.tax)?;
    let splits = effective_splits(db, draft, &context)?;
    let plan = plan_splits(&amounts, &splits)?;
    let requested_detail = draft.detail_level.unwrap_or(context.detail_level);
    let has_items = plan.rows.iter().any(|r| !r.line_items.is_empty());

    // "Itemized" is a claim about the data, not a preference, so it is derived
    // from the data rather than trusted. A draft can ask for it and carry no
    // line items -- `repeat_last` does exactly that, since it reuses a receipt's
    // categories but not the groceries you bought last time -- and storing the
    // claim anyway leaves a row that `check_all` reports as DetailLevelMismatch
    // forever. That is the failure mode `pacc doctor` is least able to survive:
    // a warning count that ordinary use makes permanently non-zero is a warning
    // count nobody reads. Downgrading here also keeps the preview honest, which
    // is the entire contract of a preview -- it must be what gets written.
    let detail_level = if requested_detail == DetailLevel::Itemized && !has_items {
        DetailLevel::Grouped
    } else {
        requested_detail
    };

    let mut warnings = Vec::new();
    let mut blockers = Vec::new();

    if plan.rows.is_empty() {
        blockers.push("there are no splits, so the money belongs to no category".to_string());
    }
    if !plan.unassigned.is_zero() {
        blockers.push(format!(
            "{} is unassigned; add a split or make one of them `remainder`",
            plan.unassigned.to_display_string()
        ));
    }
    for (index, row) in plan.rows.iter().enumerate() {
        if row.category_id.is_none() {
            blockers.push(format!("split {} has no category", index + 1));
        }
        if row.amount.is_negative() {
            warnings.push(format!(
                "split {} is negative ({}), which records a refund",
                index + 1,
                row.amount.to_display_string()
            ));
        }
    }
    if amounts.tax_is_estimated {
        // Worded by case. At a 0% rate the generic sentence reads "tax of
        // $0.00 was estimated at 0%", which tells the user nothing and hides
        // the actually useful fact: no rate is configured, so no tax was
        // applied to a receipt that probably had some.
        warnings.push(if context.tax.rate_ppm == 0 {
            "no tax was applied: this location has no tax rate set".to_string()
        } else {
            format!(
                "tax of {} was estimated at {}, not read off the receipt",
                amounts.tax.to_display_string(),
                format_rate(context.tax.rate_ppm)
            )
        });
    }
    if requested_detail == DetailLevel::Itemized && !has_items {
        warnings.push(
            "marked itemized but no split has line items; storing as grouped detail".to_string(),
        );
    }

    Ok(TransactionPreview {
        amounts,
        plan,
        tax_rate_ppm: context.tax.rate_ppm,
        context,
        detail_level,
        warnings,
        blockers,
    })
}

/// One category's subtotal, as copied off a receipt.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SubtotalRow {
    pub category_id: Option<CategoryId>,
    pub subtotal: Money,
    pub note: Option<String>,
}

/// A receipt entered the way it is read: a subtotal per category, no total.
///
/// The other draft type starts from a total and divides it, which suits a
/// till receipt you are copying wholesale. This one suits the common case —
/// you have the category subtotals in front of you, and asking for the total
/// as well means typing a number the app can work out and can then disagree
/// with.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SubtotalDraft {
    pub id: Option<TxnId>,
    pub txn_date: NaiveDate,
    pub merchant: Option<String>,
    /// `None` uses the merchant's remembered location, else home.
    pub location: Option<String>,
    pub rows: Vec<SubtotalRow>,
    pub notes: Option<String>,
    pub needs_review: bool,
    pub remember_defaults: bool,
}

/// Reconcile and plan a subtotal-entered receipt. Writes nothing.
///
/// Tax comes from the location's rate and is never asked for: that is the
/// whole point of recording where the store is. It is flagged estimated,
/// because it was computed rather than read off the paper.
pub fn preview_subtotals(db: &Db, draft: &SubtotalDraft) -> Result<TransactionPreview> {
    let context = resolve_context(
        db,
        draft.merchant.as_deref(),
        draft.location.as_deref(),
        None,
    )?;

    let subtotal = draft
        .rows
        .iter()
        .try_fold(Money::ZERO, |acc, r| acc.checked_add(r.subtotal))
        .ok_or_else(|| CoreError::Invalid("the subtotals overflowed".to_string()))?;

    let amounts = reconcile(&AmountEntry::subtotal_only(subtotal), &context.tax)?;
    let rows: Vec<(Option<CategoryId>, Money)> = draft
        .rows
        .iter()
        .map(|r| (r.category_id, r.subtotal))
        .collect();
    let plan = plan_by_subtotal(&amounts, &rows)?;

    let mut warnings = Vec::new();
    let mut blockers = Vec::new();

    if plan.rows.is_empty() {
        blockers.push("nothing entered yet: add a category and what it came to".to_string());
    }
    for (index, row) in plan.rows.iter().enumerate() {
        if row.category_id.is_none() {
            blockers.push(format!("row {} has no category", index + 1));
        }
        if row.amount.is_negative() {
            warnings.push(format!(
                "row {} is negative ({}), which records a refund",
                index + 1,
                row.amount.to_display_string()
            ));
        }
    }
    if context.tax.rate_ppm == 0 && !plan.rows.is_empty() {
        warnings.push(format!(
            "no tax was added: {} has no sales-tax rate set",
            context
                .location
                .as_ref()
                .map_or("this receipt", |l| l.name.as_str())
        ));
    }

    Ok(TransactionPreview {
        amounts,
        plan,
        tax_rate_ppm: context.tax.rate_ppm,
        context,
        detail_level: DetailLevel::Grouped,
        warnings,
        blockers,
    })
}

/// Persist a subtotal-entered receipt.
pub fn save_subtotals(db: &Db, draft: &SubtotalDraft, now: &str) -> Result<TxnId> {
    let preview = preview_subtotals(db, draft)?;
    if !preview.can_save() {
        return Err(CoreError::Invalid(format!(
            "this transaction cannot be saved: {}",
            preview.blockers.join("; ")
        )));
    }

    let merchant = match draft.merchant.as_deref() {
        Some(name) => Some(merchants::get_or_create(db, name)?),
        None => None,
    };
    let location_id = preview.context.location.as_ref().map(|l| l.id);

    let splits: Vec<SplitRecord> = preview
        .plan
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let category_id = row
                .category_id
                .ok_or_else(|| CoreError::Invalid(format!("row {} has no category", index + 1)))?;
            Ok(SplitRecord {
                id: None,
                uuid: None,
                category_id,
                subtotal: row.subtotal,
                tax: row.tax,
                amount: row.amount,
                sort_order: 0,
                notes: draft.rows.get(index).and_then(|r| r.note.clone()),
                line_items: Vec::new(),
            })
        })
        .collect::<Result<_>>()?;

    let new = NewTransaction {
        id: draft.id,
        txn_date: draft.txn_date,
        merchant_id: merchant.as_ref().map(|m| m.id),
        location_id,
        amounts: preview.amounts,
        tax_rate_ppm: (location_id.is_some() || preview.tax_rate_ppm != 0)
            .then_some(preview.tax_rate_ppm),
        detail_level: DetailLevel::Grouped,
        needs_review: draft.needs_review,
        notes: draft.notes.clone(),
        splits,
    };

    let id = transactions::save(db, &new)?;

    if let Some(merchant) = merchant {
        if draft.remember_defaults {
            let category = match preview.plan.rows.as_slice() {
                [only] => only.category_id,
                _ => None,
            };
            merchants::remember(db, merchant.id, category, location_id, DetailLevel::Grouped)?;
        }
        merchants::touch(db, merchant.id, now)?;
    }

    Ok(id)
}

/// Persist a draft, creating the merchant and updating its memory.
///
/// `now` is an ISO-8601 UTC timestamp supplied by the caller (the CLI formats
/// `chrono::Utc::now()` as `%Y-%m-%dT%H:%M:%S%.3fZ`, matching the schema's own
/// default). Core does not read the clock.
pub fn save_transaction(db: &Db, draft: &TransactionDraft, now: &str) -> Result<TxnId> {
    save_transaction_with_rate(db, draft, None, now)
}

/// [`save_transaction`] with the tax rate overridden, as in
/// [`preview_transaction_with_rate`].
pub fn save_transaction_with_rate(
    db: &Db,
    draft: &TransactionDraft,
    rate_ppm: Option<u32>,
    now: &str,
) -> Result<TxnId> {
    let preview = preview_transaction_with_rate(db, draft, rate_ppm)?;
    if !preview.can_save() {
        return Err(CoreError::Invalid(format!(
            "this transaction cannot be saved: {}",
            preview.blockers.join("; ")
        )));
    }

    // Created here rather than during preview: a merchant row for a receipt
    // the user abandoned is debris that shows up in their picker forever.
    let merchant = match draft.merchant.as_deref() {
        Some(name) => Some(merchants::get_or_create(db, name)?),
        None => None,
    };
    let location_id = preview.context.location.as_ref().map(|l| l.id);

    let splits = effective_splits(db, draft, &preview.context)?;
    let new = NewTransaction {
        id: draft.id,
        txn_date: draft.txn_date,
        merchant_id: merchant.as_ref().map(|m| m.id),
        location_id,
        amounts: preview.amounts,
        // Recorded only when something actually determined a rate. Storing a
        // bare 0 for a transaction that never had a tax context would claim
        // "the rate here is 0%", which is a different statement from "no rate
        // was involved" and would be exported as one.
        tax_rate_ppm: (location_id.is_some() || preview.tax_rate_ppm != 0)
            .then_some(preview.tax_rate_ppm),
        detail_level: preview.detail_level,
        needs_review: draft.needs_review,
        notes: draft.notes.clone(),
        splits: split_records(&preview.plan, &splits)?,
    };

    let id = transactions::save(db, &new)?;

    if let Some(merchant) = merchant {
        if draft.remember_defaults {
            // Only a single-split receipt has a meaningful default category: a
            // three-way split has no one category to remember, and picking the
            // largest would prefill the wrong one on the next visit.
            let category = match preview.plan.rows.as_slice() {
                [only] => only.category_id,
                _ => None,
            };
            // The *requested* level, not the stored one. These differ when a
            // draft asks to itemize and carries no line items: the transaction
            // is stored as grouped, because that is what its data is, but the
            // merchant's memory is a preference — "I itemize at this store" —
            // and one grouped receipt is not a reason to forget it. Remembering
            // the downgraded level instead would silently clear the preference
            // the first time the user was in a hurry.
            let remembered = draft.detail_level.unwrap_or(preview.context.detail_level);
            merchants::remember(db, merchant.id, category, location_id, remembered)?;
        }
        // Unconditional: `last_used_at` orders the recent-store chips, which
        // are about where you shopped, not about whether you asked us to
        // remember anything.
        merchants::touch(db, merchant.id, now)?;
    }

    Ok(id)
}

/// The checkout-line path: a total, and nothing else.
///
/// Files the whole amount under Uncategorized and flags it for review, so the
/// decision happens later from the inbox instead of in front of a queue.
///
/// Tax is **zero and not estimated**. That looks odd next to a rate-bearing
/// location, and it is deliberate: nothing here claimed a tax figure, so
/// flagging one as an estimate would be inventing a claim in order to label it
/// uncertain. The whole amount is the subtotal until someone says otherwise.
///
/// This must not fail for lack of a category, which is why Uncategorized is a
/// system category that cannot be deleted or archived.
pub fn quick_add(
    db: &Db,
    txn_date: NaiveDate,
    total: Money,
    merchant: Option<&str>,
    now: &str,
) -> Result<TxnId> {
    let uncategorized = categories::uncategorized(db)?;
    // A blank name is treated as absent rather than as an error: this path
    // exists to be impossible to fail.
    let merchant = match merchant.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => Some(merchants::get_or_create(db, name)?),
        None => None,
    };

    let new = NewTransaction {
        id: None,
        txn_date,
        merchant_id: merchant.as_ref().map(|m| m.id),
        location_id: None,
        amounts: Amounts {
            subtotal: total,
            tax: Money::ZERO,
            total,
            tax_is_estimated: false,
        },
        tax_rate_ppm: None,
        detail_level: DetailLevel::Grouped,
        needs_review: true,
        notes: None,
        splits: vec![SplitRecord {
            id: None,
            uuid: None,
            category_id: uncategorized.id,
            subtotal: total,
            tax: Money::ZERO,
            amount: total,
            sort_order: 0,
            notes: None,
            line_items: Vec::new(),
        }],
    };

    let id = transactions::save(db, &new)?;
    if let Some(merchant) = merchant {
        merchants::touch(db, merchant.id, now)?;
    }
    Ok(id)
}

/// Build a draft from the merchant's last receipt.
///
/// # This returns a draft. It does not save anything.
///
/// Read that twice before changing this function. Silently duplicating a
/// receipt is the worst bug this feature can have — it is invisible (the
/// numbers look right), it inflates every report, and the user only discovers
/// it when reconciling against a bank statement weeks later. The returned
/// draft has `id: None` and is not written until the caller passes it to
/// [`save_transaction`], which is a second, explicit act.
///
/// Carried over: the merchant, the location, the split categories and their
/// amounts (as [`SplitAmount::Fixed`]), and the detail level. Not carried
/// over: line items and notes, which describe one specific shopping trip
/// rather than a habit.
pub fn repeat_last(db: &Db, merchant_name: &str, txn_date: NaiveDate) -> Result<TransactionDraft> {
    let merchant = merchants::find_by_name(db, merchant_name)?
        .ok_or_else(|| CoreError::NotFound(format!("merchant `{merchant_name}`")))?;
    let last = transactions::last_for_merchant(db, merchant.id)?
        .ok_or_else(|| CoreError::NotFound(format!("any transaction for `{}`", merchant.name)))?;

    // If the previous tax was an estimate, re-enter only the total and let it
    // be estimated again — at today's rate, and flagged as an estimate.
    // Copying the old subtotal *and* tax would launder last month's guess into
    // a figure that claims to have been read off a receipt.
    let entry = if last.tax_is_estimated {
        AmountEntry::total_only(last.total)
    } else {
        AmountEntry {
            subtotal: Some(last.subtotal),
            tax: Some(last.tax),
            total: Some(last.total),
            last_edited: None,
        }
    };

    let splits = last
        .splits
        .iter()
        .map(|s| SplitDraft {
            // None: these are new rows on a new receipt, not edits of the old
            // one's.
            id: None,
            category_id: Some(s.category_id),
            amount: SplitAmount::Fixed(s.amount),
            note: s.notes.clone(),
            line_items: Vec::new(),
        })
        .collect();

    Ok(TransactionDraft {
        id: None,
        txn_date,
        merchant: Some(merchant.name),
        location: last.location_name,
        entry,
        splits,
        detail_level: Some(last.detail_level),
        notes: None,
        needs_review: false,
        remember_defaults: false,
    })
}

/// The needs-categorizing inbox, newest first.
pub fn needs_categorizing(db: &Db, limit: Option<usize>) -> Result<Vec<Txn>> {
    transactions::list(db, &TxnFilter::needs_review(limit))
}

/// Assign categories to an existing transaction and clear its review flag.
///
/// The amounts are the ones already stored, `tax_is_estimated` included: this
/// re-divides money that has already been recorded, and re-deriving the tax
/// from a rate here would silently overwrite a figure the user may have typed
/// off the receipt.
pub fn categorize(db: &Db, id: TxnId, splits: &[SplitDraft]) -> Result<()> {
    let txn = transactions::get(db, id)?;
    let amounts = txn.amounts();
    let plan = plan_splits(&amounts, splits)?;

    if !plan.unassigned.is_zero() {
        return Err(CoreError::Invalid(format!(
            "{} of {} is unassigned; add a split or make one of them `remainder`",
            plan.unassigned.to_display_string(),
            amounts.total.to_display_string()
        )));
    }
    if plan.rows.is_empty() {
        return Err(CoreError::Invalid(
            "no splits were given, so the money would belong to no category".to_string(),
        ));
    }
    for (index, row) in plan.rows.iter().enumerate() {
        if row.category_id.is_none() {
            return Err(CoreError::Invalid(format!(
                "split {} has no category",
                index + 1
            )));
        }
    }

    let new = NewTransaction {
        id: Some(txn.id),
        txn_date: txn.txn_date,
        merchant_id: txn.merchant_id,
        location_id: txn.location_id,
        amounts,
        tax_rate_ppm: txn.tax_rate_ppm,
        detail_level: txn.detail_level,
        // The point of the exercise: this is how the inbox empties.
        needs_review: false,
        notes: txn.notes.clone(),
        splits: split_records(&plan, splits)?,
    };
    transactions::save(db, &new)?;
    Ok(())
}

/// Resolve merchant, location, tax context and defaults.
fn resolve_context(
    db: &Db,
    merchant_name: Option<&str>,
    location_name: Option<&str>,
    rate_ppm: Option<u32>,
) -> Result<EntryContext> {
    let merchant = match merchant_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => merchants::find_by_name(db, name)?,
        None => None,
    };

    let location = match location_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => Some(
            locations::find_by_name(db, name)?
                .ok_or_else(|| CoreError::NotFound(format!("location `{name}`")))?,
        ),
        None => match merchant.as_ref().and_then(|m| m.default_location_id) {
            Some(id) => Some(locations::get(db, id)?),
            None => locations::home(db)?,
        },
    };

    let tax = TaxContext {
        rate_ppm: rate_ppm.unwrap_or_else(|| location.as_ref().map_or(0, |l| l.tax_rate_ppm)),
        prices_include_tax: location.as_ref().is_some_and(|l| l.prices_include_tax),
    };

    let default_category = match merchant.as_ref().and_then(|m| m.default_category_id) {
        Some(id) => Some(categories::get(db, id)?),
        None => None,
    };
    let detail_level = merchant
        .as_ref()
        .map_or(DetailLevel::Grouped, |m| m.default_detail_level);

    Ok(EntryContext {
        merchant,
        location,
        tax,
        default_category,
        detail_level,
        recent_merchants: merchants::recent(db, RECENT_MERCHANTS)?,
    })
}

/// The drafts actually planned: the user's, or one synthesised row that takes
/// everything.
///
/// Synthesising here rather than making the caller do it is what lets
/// `pacc quick-add` and a one-field mobile form exist at all — and it means
/// preview and save synthesise *identically*, because they call this.
fn effective_splits(
    db: &Db,
    draft: &TransactionDraft,
    context: &EntryContext,
) -> Result<Vec<SplitDraft>> {
    if !draft.splits.is_empty() {
        return Ok(draft.splits.clone());
    }
    let category = match context.default_category.as_ref() {
        Some(c) => c.id,
        None => categories::uncategorized(db)?.id,
    };
    Ok(vec![SplitDraft {
        id: None,
        category_id: Some(category),
        amount: SplitAmount::Remainder,
        note: None,
        line_items: Vec::new(),
    }])
}

/// Turn a resolved plan into rows for the write path.
///
/// `drafts` runs in lockstep with `plan.rows` — [`plan_splits`] emits one row
/// per draft, in order — and is needed only for the per-split note, which the
/// resolved row does not carry.
fn split_records(plan: &SplitPlan, drafts: &[SplitDraft]) -> Result<Vec<SplitRecord>> {
    plan.rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let category_id = row.category_id.ok_or_else(|| {
                CoreError::Invalid(format!("split {} has no category", index + 1))
            })?;
            let draft = drafts.get(index);
            Ok(SplitRecord {
                id: None,
                uuid: None,
                category_id,
                subtotal: row.subtotal,
                tax: row.tax,
                amount: row.amount,
                // 0 asks the write path for the sparse default.
                sort_order: 0,
                notes: draft.and_then(|d| d.note.clone()),
                line_items: line_item_records(row.tax, &row.line_items),
            })
        })
        .collect()
}

/// Split one split's tax across its line items.
///
/// Apportioned with [`distribute_tax`] rather than computed per item: rounding
/// each item's tax independently drifts up to half a cent per line, so a
/// twenty-line grocery receipt would miss its own split by a dime and the
/// aggregate would stop balancing. The apportionment is sum-preserving by
/// construction, so the items always add back up to exactly the split.
fn line_item_records(
    split_tax: Money,
    items: &[crate::domain::splits::LineItemDraft],
) -> Vec<LineItemRecord> {
    let amounts: Vec<Money> = items.iter().map(|li| li.amount).collect();
    let taxes = distribute_tax(split_tax, &amounts);

    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let tax = taxes.get(index).copied().unwrap_or(Money::ZERO);
            LineItemRecord {
                id: None,
                uuid: None,
                description: item.description.clone(),
                quantity_milli: item.quantity_milli,
                unit_price: item.unit_price,
                subtotal: item.amount.saturating_sub(tax),
                tax,
                amount: item.amount,
                category_id: None,
                is_taxable: true,
                sort_order: 0,
                notes: None,
            }
        })
        .collect()
}

/// `83_750 -> "8.375%"`, without touching a float.
fn format_rate(ppm: u32) -> String {
    let whole = ppm / 10_000;
    let mut decimals = format!("{:04}", ppm % 10_000);
    while decimals.ends_with('0') {
        decimals.pop();
    }
    if decimals.is_empty() {
        format!("{whole}%")
    } else {
        format!("{whole}.{decimals}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;
    use crate::domain::splits::LineItemDraft;

    const NOW: &str = "2026-08-12T18:30:00.000Z";

    fn m(cents: i64) -> Money {
        Money::from_cents(cents)
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().expect("valid date")
    }

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    fn draft(total: i64) -> TransactionDraft {
        TransactionDraft {
            id: None,
            txn_date: date("2026-08-12"),
            merchant: None,
            location: None,
            entry: AmountEntry::total_only(m(total)),
            splits: Vec::new(),
            detail_level: None,
            notes: None,
            needs_review: false,
            remember_defaults: false,
        }
    }

    fn split_for(db: &Db, category: &str, amount: SplitAmount) -> SplitDraft {
        SplitDraft {
            id: None,
            category_id: Some(categories::resolve(db, category).expect("category").id),
            amount,
            note: None,
            line_items: Vec::new(),
        }
    }

    #[test]
    fn an_empty_draft_is_categorised_to_uncategorized_and_saves() {
        let db = seeded();
        let preview = preview_transaction(&db, &draft(5432)).unwrap();
        assert!(preview.can_save(), "{:?}", preview.blockers);
        assert_eq!(preview.plan.rows.len(), 1);
        assert_eq!(preview.plan.rows[0].amount, m(5432));
        assert_eq!(preview.plan.unassigned, Money::ZERO);
        assert_eq!(
            preview.plan.rows[0].category_id,
            Some(categories::uncategorized(&db).unwrap().id)
        );

        // A preview writes nothing.
        assert_eq!(
            transactions::count(&db, &TxnFilter::default()).unwrap(),
            0,
            "preview must not write"
        );

        let id = save_transaction(&db, &draft(5432), NOW).unwrap();
        assert_eq!(transactions::get(&db, id).unwrap().total, m(5432));
    }

    #[test]
    fn the_merchants_memory_prefills_the_next_receipt() {
        let db = seeded();
        let mut d = draft(5000);
        d.merchant = Some("H-E-B".to_string());
        d.location = Some("Home".to_string());
        d.splits = vec![split_for(&db, "Groceries", SplitAmount::Remainder)];
        d.detail_level = Some(DetailLevel::Itemized);
        d.remember_defaults = true;

        // The merchant does not exist yet: previewing must not create it.
        assert!(
            preview_transaction(&db, &d)
                .unwrap()
                .context
                .merchant
                .is_none()
        );
        assert!(merchants::find_by_name(&db, "H-E-B").unwrap().is_none());

        save_transaction(&db, &d, NOW).unwrap();

        let ctx = entry_context(&db, Some("H-E-B")).unwrap();
        let merchant = ctx.merchant.expect("created on save");
        assert_eq!(merchant.last_used_at.as_deref(), Some(NOW));
        assert_eq!(
            ctx.default_category.map(|c| c.name).as_deref(),
            Some("Groceries")
        );
        assert_eq!(ctx.location.map(|l| l.name).as_deref(), Some("Home"));
        assert_eq!(ctx.detail_level, DetailLevel::Itemized);
        assert!(ctx.recent_merchants.iter().any(|m| m.id == merchant.id));
    }

    #[test]
    fn a_multi_way_split_teaches_the_merchant_nothing_about_categories() {
        let db = seeded();
        let mut d = draft(10_000);
        d.merchant = Some("Target".to_string());
        d.remember_defaults = true;
        d.splits = vec![
            split_for(&db, "Groceries", SplitAmount::Percent(5_000)),
            split_for(&db, "Household", SplitAmount::Remainder),
        ];
        save_transaction(&db, &d, NOW).unwrap();

        let merchant = merchants::find_by_name(&db, "Target").unwrap().unwrap();
        assert_eq!(
            merchant.default_category_id, None,
            "a 3-way split has no meaningful default category"
        );
        assert!(merchant.last_used_at.is_some());
    }

    #[test]
    fn a_mistyped_location_is_an_error_rather_than_a_zero_percent_fallback() {
        let db = seeded();
        let mut d = draft(5432);
        d.location = Some("Hoem".to_string());
        assert!(matches!(
            preview_transaction(&db, &d),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn unassigned_money_blocks_the_save() {
        let db = seeded();
        let mut d = draft(10_000);
        d.splits = vec![split_for(&db, "Groceries", SplitAmount::Percent(2_500))];

        let preview = preview_transaction(&db, &d).unwrap();
        assert!(!preview.can_save());
        assert!(
            preview.blockers[0].contains("$75.00"),
            "{:?}",
            preview.blockers
        );

        let err = save_transaction(&db, &d, NOW).unwrap_err();
        assert!(matches!(err, CoreError::Invalid(_)));
        assert_eq!(transactions::count(&db, &TxnFilter::default()).unwrap(), 0);
    }

    #[test]
    fn a_split_without_a_category_blocks_the_save() {
        let db = seeded();
        let mut d = draft(1000);
        d.splits = vec![SplitDraft::new(SplitAmount::Remainder)];
        let preview = preview_transaction(&db, &d).unwrap();
        assert!(!preview.can_save());
        assert!(preview.blockers.iter().any(|b| b.contains("no category")));
    }

    #[test]
    fn an_estimated_tax_warns_but_still_saves() {
        let db = seeded();
        let home = locations::home(&db).unwrap().unwrap();
        locations::set_rate(&db, home.id, 83_750).unwrap();

        let preview = preview_transaction(&db, &draft(5432)).unwrap();
        assert!(preview.amounts.tax_is_estimated);
        assert_eq!(preview.amounts.tax, m(5432).extract_ppm(83_750));
        assert!(preview.can_save());
        assert!(
            preview.warnings.iter().any(|w| w.contains("8.375%")),
            "{:?}",
            preview.warnings
        );

        let id = save_transaction(&db, &draft(5432), NOW).unwrap();
        let saved = transactions::get(&db, id).unwrap();
        assert!(saved.tax_is_estimated);
        assert_eq!(saved.tax_rate_ppm, Some(83_750));
    }

    #[test]
    fn an_entered_tax_is_never_relabelled_as_an_estimate() {
        let db = seeded();
        let home = locations::home(&db).unwrap().unwrap();
        locations::set_rate(&db, home.id, 83_750).unwrap();

        let mut d = draft(0);
        d.entry = AmountEntry {
            subtotal: Some(m(5000)),
            tax: Some(m(400)),
            total: None,
            last_edited: None,
        };
        let id = save_transaction(&db, &d, NOW).unwrap();
        let saved = transactions::get(&db, id).unwrap();
        assert!(!saved.tax_is_estimated);
        // The rate stored is the context's, but the tax is the user's figure —
        // not the 8.375% one.
        assert_eq!(saved.tax, m(400));
        assert_eq!(saved.total, m(5400));
    }

    #[test]
    fn a_rate_override_estimates_at_that_rate_and_still_says_so() {
        let db = seeded();
        let preview = preview_transaction_with_rate(&db, &draft(5432), Some(83_750)).unwrap();
        assert_eq!(preview.tax_rate_ppm, 83_750);
        assert!(preview.amounts.tax_is_estimated);

        let id = save_transaction_with_rate(&db, &draft(5432), Some(83_750), NOW).unwrap();
        let saved = transactions::get(&db, id).unwrap();
        assert_eq!(saved.tax, m(5432).extract_ppm(83_750));
        assert!(saved.tax_is_estimated, "an estimate must stay an estimate");
        assert_eq!(saved.tax_rate_ppm, Some(83_750));
    }

    #[test]
    fn line_items_are_stored_with_their_share_of_the_split_tax() {
        let db = seeded();
        let home = locations::home(&db).unwrap().unwrap();
        locations::set_rate(&db, home.id, 83_750).unwrap();

        let mut split = split_for(&db, "Groceries", SplitAmount::Remainder);
        split.line_items = vec![
            LineItemDraft {
                description: "Milk".to_string(),
                quantity_milli: 1_000,
                unit_price: Some(m(3_000)),
                amount: m(3_000),
            },
            LineItemDraft {
                description: "Bread".to_string(),
                quantity_milli: 2_000,
                unit_price: Some(m(1_216)),
                amount: m(2_432),
            },
        ];
        let mut d = draft(5432);
        d.splits = vec![split];
        d.detail_level = Some(DetailLevel::Itemized);

        let id = save_transaction(&db, &d, NOW).unwrap();
        let saved = transactions::get(&db, id).unwrap();
        assert_eq!(saved.detail_level, DetailLevel::Itemized);
        let items = &saved.splits[0].line_items;
        assert_eq!(items.len(), 2);
        // Sum-preserving: no penny is created or destroyed by the split.
        assert_eq!(
            items.iter().map(|i| i.tax).sum::<Money>(),
            saved.splits[0].tax
        );
        assert_eq!(
            items.iter().map(|i| i.amount).sum::<Money>(),
            saved.splits[0].amount
        );
        for item in items {
            assert_eq!(item.subtotal + item.tax, item.amount);
        }
        assert_eq!(items[1].quantity_milli, 2_000);
    }

    #[test]
    fn itemized_without_items_warns_and_saves() {
        let db = seeded();
        let mut d = draft(1000);
        d.detail_level = Some(DetailLevel::Itemized);
        let preview = preview_transaction(&db, &d).unwrap();
        assert!(preview.can_save());
        assert!(
            preview.warnings.iter().any(|w| w.contains("itemized")),
            "{:?}",
            preview.warnings
        );
        save_transaction(&db, &d, NOW).unwrap();
    }

    #[test]
    fn quick_add_never_needs_a_category_and_lands_in_the_inbox() {
        let db = seeded();
        let id = quick_add(&db, date("2026-08-12"), m(1240), Some("Corner Store"), NOW).unwrap();

        let saved = transactions::get(&db, id).unwrap();
        assert!(saved.needs_review);
        assert_eq!(saved.total, m(1240));
        assert_eq!(saved.subtotal, m(1240));
        assert_eq!(saved.tax, Money::ZERO);
        assert!(
            !saved.tax_is_estimated,
            "no tax was claimed, so nothing was estimated"
        );
        assert_eq!(saved.tax_rate_ppm, None);
        assert_eq!(saved.splits.len(), 1);
        assert_eq!(saved.splits[0].category_name, "Uncategorized");

        let inbox = needs_categorizing(&db, None).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].id, id);

        // Even with a rate-bearing home location and no merchant at all.
        let home = locations::home(&db).unwrap().unwrap();
        locations::set_rate(&db, home.id, 83_750).unwrap();
        let bare = quick_add(&db, date("2026-08-12"), m(500), None, NOW).unwrap();
        assert_eq!(transactions::get(&db, bare).unwrap().tax, Money::ZERO);
    }

    #[test]
    fn categorize_empties_the_inbox_without_touching_the_amounts() {
        let db = seeded();
        let id = quick_add(&db, date("2026-08-12"), m(1240), None, NOW).unwrap();
        let before = transactions::get(&db, id).unwrap();

        categorize(
            &db,
            id,
            &[
                split_for(&db, "Groceries", SplitAmount::Fixed(m(1000))),
                split_for(&db, "Household", SplitAmount::Remainder),
            ],
        )
        .unwrap();

        let after = transactions::get(&db, id).unwrap();
        assert!(!after.needs_review);
        assert_eq!(after.total, before.total);
        assert_eq!(after.tax, before.tax);
        assert_eq!(after.tax_is_estimated, before.tax_is_estimated);
        assert_eq!(after.uuid, before.uuid);
        assert_eq!(after.splits.len(), 2);
        assert_eq!(after.splits[0].amount, m(1000));
        assert_eq!(after.splits[1].amount, m(240));
        assert!(needs_categorizing(&db, None).unwrap().is_empty());
    }

    #[test]
    fn categorize_refuses_a_partial_assignment() {
        let db = seeded();
        let id = quick_add(&db, date("2026-08-12"), m(1240), None, NOW).unwrap();
        let err = categorize(
            &db,
            id,
            &[split_for(&db, "Groceries", SplitAmount::Fixed(m(1000)))],
        )
        .unwrap_err();
        assert!(err.to_string().contains("$2.40"), "{err}");
        assert!(transactions::get(&db, id).unwrap().needs_review);
    }

    #[test]
    fn repeat_last_returns_a_draft_and_saves_nothing() {
        let db = seeded();
        let mut d = draft(5000);
        d.merchant = Some("H-E-B".to_string());
        d.location = Some("Home".to_string());
        d.splits = vec![
            split_for(&db, "Groceries", SplitAmount::Fixed(m(3000))),
            split_for(&db, "Household", SplitAmount::Remainder),
        ];
        save_transaction(&db, &d, NOW).unwrap();
        let before = transactions::count(&db, &TxnFilter::default()).unwrap();

        let repeated = repeat_last(&db, "h-e-b", date("2026-09-01")).unwrap();
        assert_eq!(
            transactions::count(&db, &TxnFilter::default()).unwrap(),
            before,
            "repeat_last must not write"
        );
        assert_eq!(repeated.id, None, "a repeat is a new receipt, not an edit");
        assert_eq!(repeated.txn_date, date("2026-09-01"));
        assert_eq!(repeated.merchant.as_deref(), Some("H-E-B"));
        assert_eq!(repeated.location.as_deref(), Some("Home"));
        assert_eq!(repeated.splits.len(), 2);
        assert_eq!(repeated.splits[0].amount, SplitAmount::Fixed(m(3000)));
        assert!(repeated.splits.iter().all(|s| s.id.is_none()));

        // …and it saves as a second, independent transaction.
        let id = save_transaction(&db, &repeated, NOW).unwrap();
        assert_eq!(
            transactions::count(&db, &TxnFilter::default()).unwrap(),
            before + 1
        );
        assert_eq!(transactions::get(&db, id).unwrap().total, m(5000));
    }

    #[test]
    fn repeating_an_estimate_re_estimates_rather_than_claiming_a_fact() {
        let db = seeded();
        let home = locations::home(&db).unwrap().unwrap();
        locations::set_rate(&db, home.id, 83_750).unwrap();
        let mut d = draft(5432);
        d.merchant = Some("H-E-B".to_string());
        save_transaction(&db, &d, NOW).unwrap();

        let repeated = repeat_last(&db, "H-E-B", date("2026-09-01")).unwrap();
        assert_eq!(repeated.entry.tax, None);
        assert_eq!(repeated.entry.total, Some(m(5432)));
        let preview = preview_transaction(&db, &repeated).unwrap();
        assert!(
            preview.amounts.tax_is_estimated,
            "an estimate must not be laundered into a fact by repeating it"
        );
    }

    #[test]
    fn repeat_last_is_not_found_without_a_history() {
        let db = seeded();
        assert!(matches!(
            repeat_last(&db, "Nowhere", date("2026-09-01")),
            Err(CoreError::NotFound(_))
        ));

        merchants::get_or_create(&db, "Nowhere").unwrap();
        assert!(matches!(
            repeat_last(&db, "Nowhere", date("2026-09-01")),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn editing_a_transaction_replaces_it_rather_than_duplicating_it() {
        let db = seeded();
        let mut d = draft(5000);
        d.splits = vec![split_for(&db, "Groceries", SplitAmount::Remainder)];
        let id = save_transaction(&db, &d, NOW).unwrap();

        d.id = Some(id);
        d.entry = AmountEntry::total_only(m(6000));
        d.splits = vec![
            split_for(&db, "Groceries", SplitAmount::Fixed(m(4000))),
            split_for(&db, "Gas", SplitAmount::Remainder),
        ];
        assert_eq!(save_transaction(&db, &d, NOW).unwrap(), id);

        assert_eq!(transactions::count(&db, &TxnFilter::default()).unwrap(), 1);
        let saved = transactions::get(&db, id).unwrap();
        assert_eq!(saved.total, m(6000));
        assert_eq!(saved.splits.len(), 2);
    }

    #[test]
    fn split_notes_survive_the_round_trip() {
        let db = seeded();
        let mut split = split_for(&db, "Groceries", SplitAmount::Remainder);
        split.note = Some("the good olive oil".to_string());
        let mut d = draft(2000);
        d.splits = vec![split];
        d.notes = Some("weekly shop".to_string());

        let id = save_transaction(&db, &d, NOW).unwrap();
        let saved = transactions::get(&db, id).unwrap();
        assert_eq!(saved.notes.as_deref(), Some("weekly shop"));
        assert_eq!(saved.splits[0].notes.as_deref(), Some("the good olive oil"));
    }

    #[test]
    fn rates_format_without_floats() {
        assert_eq!(format_rate(83_750), "8.375%");
        assert_eq!(format_rate(0), "0%");
        assert_eq!(format_rate(100_000), "10%");
    }
}
