//! Entering a receipt.
//!
//! The shape the form uses: a subtotal per category, and no total. Tax comes
//! from the location's rate, and the total is the sum plus that tax — asking
//! for a number the core can work out means the two can disagree, which is
//! then the user's problem to adjudicate.
//!
//! Preview and save run the same core code, so the numbers on screen are not
//! this layer's reading of the receipt: they *are* what will be stored.

use accounting_core::repo::{locations, merchants, tax_areas};
use accounting_core::service::entry::{self, SubtotalDraft, SubtotalRow};
use accounting_core::{CategoryId, Money, TxnId};

use crate::api::db;
use crate::error::to_message;

/// One category's subtotal, as copied off a receipt.
pub struct SubtotalRowDto {
    pub category_id: Option<i64>,
    pub subtotal: i64,
    /// The tax on this row when the user has corrected it off the receipt;
    /// null leaves it to the rate.
    pub tax: Option<i64>,
    pub note: Option<String>,
}

/// A receipt entered the way it is read.
pub struct SubtotalDraftDto {
    pub id: Option<i64>,
    /// `YYYY-MM-DD`. The frontend owns "today" — it knows the user's timezone
    /// and may read the clock; the core may not.
    pub txn_date: String,
    pub merchant: Option<String>,
    /// Null uses the merchant's remembered location, else home.
    pub location: Option<String>,
    pub rows: Vec<SubtotalRowDto>,
    pub notes: Option<String>,
    pub needs_review: bool,
    pub remember_defaults: bool,
}

pub struct SplitRowDto {
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub subtotal: i64,
    pub tax: i64,
    pub amount: i64,
}

/// What would be written, and why it might not be.
pub struct PreviewDto {
    pub subtotal: i64,
    pub tax: i64,
    pub total: i64,
    pub tax_is_estimated: bool,
    /// The location's general rate — what the store charges on most things.
    pub tax_rate_ppm: u32,
    /// The rate this receipt was actually taxed at, or null when it straddles
    /// two. Never label a tax figure with `tax_rate_ppm`: on a grocery receipt
    /// that prints a percentage which does not match the money beside it.
    pub applied_rate_ppm: Option<u32>,
    pub unassigned: i64,
    pub detail_level: String,
    pub rows: Vec<SplitRowDto>,
    /// Worth showing; saving is still allowed.
    pub warnings: Vec<String>,
    /// Saving is impossible until these are empty.
    pub blockers: Vec<String>,
}

/// What each category is taxed at here, so the form can label a row the moment
/// a category is picked. The rate is a property of the category, not of the
/// store: at one till, groceries are 3% and shampoo is 7.45%.
pub struct CategoryRateDto {
    pub category_id: i64,
    pub rate_ppm: u32,
}

/// What the app already knows about a store, so the next receipt prefills.
pub struct ContextDto {
    pub merchant_name: Option<String>,
    pub location_name: Option<String>,
    pub tax_rate_ppm: u32,
    /// The location's reduced rate on unprepared food, where it has one.
    pub food_rate_ppm: Option<u32>,
    pub category_rates: Vec<CategoryRateDto>,
    pub default_category_id: Option<i64>,
    pub detail_level: String,
    pub recent_merchants: Vec<String>,
}

pub struct LocationDto {
    pub id: i64,
    pub name: String,
    pub tax_rate_ppm: u32,
    pub is_home: bool,
}

pub struct MerchantDto {
    pub id: i64,
    pub name: String,
    pub default_category_id: Option<i64>,
}

pub struct AreaDto {
    /// The publisher's jurisdiction code, so a location made from this area
    /// can follow it through later revisions.
    pub code: String,
    pub name: String,
    pub tax_rate_ppm: u32,
    /// None where the state taxes food like everything else.
    pub food_rate_ppm: Option<u32>,
}

fn to_draft(dto: SubtotalDraftDto) -> Result<SubtotalDraft, String> {
    Ok(SubtotalDraft {
        id: dto.id.map(TxnId::new),
        txn_date: dto
            .txn_date
            .parse::<chrono::NaiveDate>()
            .map_err(|e| format!("{} is not a YYYY-MM-DD date: {e}", dto.txn_date))?,
        merchant: dto.merchant,
        location: dto.location,
        rows: dto
            .rows
            .into_iter()
            .map(|r| SubtotalRow {
                category_id: r.category_id.map(CategoryId::new),
                subtotal: Money::from_cents(r.subtotal),
                tax: r.tax.map(Money::from_cents),
                note: r.note,
            })
            .collect(),
        notes: dto.notes,
        needs_review: dto.needs_review,
        remember_defaults: dto.remember_defaults,
    })
}

fn to_preview_dto(
    preview: &entry::TransactionPreview,
    names: &[accounting_core::Category],
) -> PreviewDto {
    PreviewDto {
        subtotal: preview.amounts.subtotal.cents(),
        tax: preview.amounts.tax.cents(),
        total: preview.amounts.total.cents(),
        tax_is_estimated: preview.amounts.tax_is_estimated,
        tax_rate_ppm: preview.tax_rate_ppm,
        applied_rate_ppm: preview.applied_rate_ppm,
        unassigned: preview.plan.unassigned.cents(),
        detail_level: preview.detail_level.to_string(),
        rows: preview
            .plan
            .rows
            .iter()
            .map(|row| SplitRowDto {
                category_id: row.category_id.map(CategoryId::get),
                category_name: row
                    .category_id
                    .and_then(|id| names.iter().find(|c| c.id == id).map(|c| c.name.clone())),
                subtotal: row.subtotal.cents(),
                tax: row.tax.cents(),
                amount: row.amount.cents(),
            })
            .collect(),
        warnings: preview.warnings.clone(),
        blockers: preview.blockers.clone(),
    }
}

/// Reconcile a subtotal-entered receipt without writing anything.
///
/// Safe to call on every keystroke, and the only way the UI learns what a
/// receipt's numbers are. Because saving runs this same function first, the UI
/// cannot display one set of numbers and store another.
pub fn preview_subtotals(draft: SubtotalDraftDto) -> Result<PreviewDto, String> {
    let draft = to_draft(draft)?;
    let db = db::locked()?;
    let preview = entry::preview_subtotals(&db, &draft).map_err(to_message)?;
    // `list_all`, not `list`: the preview's rows can name an archived category
    // and it still has to be labelled.
    let names = accounting_core::repo::categories::list_all(&db).map_err(to_message)?;
    Ok(to_preview_dto(&preview, &names))
}

/// Write the receipt. Returns the transaction's id.
pub fn save_subtotals(draft: SubtotalDraftDto) -> Result<i64, String> {
    let draft = to_draft(draft)?;
    let db = db::locked()?;
    // The timestamp is this layer's to supply — the core never reads the clock
    // — but "now" for a `last_used_at` is genuinely this instant, so it is
    // taken here rather than round-tripped through Dart.
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    entry::save_subtotals(&db, &draft, &now)
        .map(TxnId::get)
        .map_err(to_message)
}

/// What the app knows about a store.
///
/// Naming a *new* merchant is not an error and creates nothing — the row
/// appears on save, so abandoning a half-typed receipt leaves no debris in the
/// picker.
pub fn entry_context(merchant: Option<String>) -> Result<ContextDto, String> {
    let db = db::locked()?;
    let ctx = entry::entry_context(&db, merchant.as_deref()).map_err(to_message)?;
    Ok(ContextDto {
        merchant_name: ctx.merchant.as_ref().map(|m| m.name.clone()),
        location_name: ctx.location.as_ref().map(|l| l.name.clone()),
        tax_rate_ppm: ctx.tax.rate_ppm,
        food_rate_ppm: ctx.location.as_ref().and_then(|l| l.food_rate_ppm),
        category_rates: entry::category_rates(&db, &ctx)
            .map_err(to_message)?
            .into_iter()
            .map(|r| CategoryRateDto {
                category_id: r.category_id.get(),
                rate_ppm: r.rate_ppm,
            })
            .collect(),
        default_category_id: ctx.default_category.as_ref().map(|c| c.id.get()),
        detail_level: ctx.detail_level.to_string(),
        recent_merchants: ctx.recent_merchants.iter().map(|m| m.name.clone()).collect(),
    })
}

/// Make sure a location exists by that name, creating it from the published
/// rate table if it does not.
///
/// The core refuses a location it does not know — deliberately, because
/// falling back to "no location" would switch the rate to 0% and produce a
/// plausible, wrong number. A name the table does not know is created at 0%:
/// no rate has been supplied, and inventing one would be worse than a visible
/// zero.
pub fn ensure_location(name: String) -> Result<LocationDto, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a location needs a name".to_string());
    }

    let db = db::locked()?;
    let published = tax_areas::find_by_name(&db, trimmed).map_err(to_message)?;
    let rate = published.as_ref().map_or(0, |a| a.combined_rate_ppm);
    let location = locations::get_or_create(&db, trimmed, rate).map_err(to_message)?;

    if let (Some(area), None) = (published.as_ref(), location.area_code.as_ref()) {
        // Also on a location that already existed: it may predate the rate
        // table, and following the publication is how a quarterly refresh
        // reaches this location at all.
        locations::adopt_area(
            &db,
            location.id,
            &area.code,
            area.combined_rate_ppm,
            area.food_rate_ppm,
        )
        .map_err(to_message)?;
    }

    let fresh = locations::get(&db, location.id).map_err(to_message)?;
    Ok(LocationDto {
        id: fresh.id.get(),
        name: fresh.name,
        tax_rate_ppm: fresh.tax_rate_ppm,
        is_home: fresh.is_home,
    })
}

/// The home location and its rate.
///
/// `None`, or a rate of zero, means the app has not been told where the user
/// lives — and a receipt silently recording $0.00 of tax is wrong in a way
/// nothing else surfaces.
pub fn home_location() -> Result<Option<LocationDto>, String> {
    let db = db::locked()?;
    Ok(locations::home(&db).map_err(to_message)?.map(|l| LocationDto {
        id: l.id.get(),
        name: l.name,
        tax_rate_ppm: l.tax_rate_ppm,
        is_home: l.is_home,
    }))
}

/// Every known store, most-recently-used first, so the places you go keep
/// floating to the top of the picker with the untouched suggestions behind.
pub fn list_merchants() -> Result<Vec<MerchantDto>, String> {
    let db = db::locked()?;
    let all = merchants::list(&db).map_err(to_message)?;
    Ok(merchants::recent(&db, all.len())
        .map_err(to_message)?
        .into_iter()
        .map(|m| MerchantDto {
            id: m.id.get(),
            name: m.name,
            default_category_id: m.default_category_id.map(CategoryId::get),
        })
        .collect())
}

pub fn list_locations() -> Result<Vec<LocationDto>, String> {
    let db = db::locked()?;
    Ok(locations::list(&db)
        .map_err(to_message)?
        .into_iter()
        .map(|l| LocationDto {
            id: l.id.get(),
            name: l.name,
            tax_rate_ppm: l.tax_rate_ppm,
            is_home: l.is_home,
        })
        .collect())
}

/// Every jurisdiction the state publishes, so a trip to another town can be
/// taxed at that town's rate. Still starting points — a receipt's own tax
/// always wins.
pub fn list_areas() -> Result<Vec<AreaDto>, String> {
    let db = db::locked()?;
    Ok(tax_areas::list_places(&db)
        .map_err(to_message)?
        .into_iter()
        .map(|a| AreaDto {
            code: a.code,
            name: a.name,
            tax_rate_ppm: a.combined_rate_ppm,
            food_rate_ppm: a.food_rate_ppm,
        })
        .collect())
}
