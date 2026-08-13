//! The desktop shell.
//!
//! # This file contains no business logic, on purpose
//!
//! Every command below is the same three steps: turn a JSON payload into core
//! types, call one `accounting-core` function, turn the answer back into JSON.
//! There is no arithmetic, no reconciliation, and no decision about what a
//! receipt means — all of that already exists in the core, is already tested
//! there, and is already exercised by `pacc`.
//!
//! That is the whole point of the milestone. If this shell needed logic of its
//! own, the Android and iOS shells would need the same logic written again, and
//! the three copies would drift. The plan's phrase for it: a second frontend
//! should be a rendering exercise, not a re-implementation.
//!
//! It is also why there are no tests here. A wrapper thin enough to be obvious
//! has nothing to test; one worth testing has too much logic in it and the
//! logic belongs in the core.
//!
//! # The database path comes from Tauri, not from the core
//!
//! `accounting-core` never discovers a path — see its crate docs. The decision
//! is made once, in [`database_path`], which resolves the *same file `pacc`
//! uses* on the desktop and Tauri's per-app directory on mobile. A core that
//! called `dirs` or read `$HOME` would have to be rewritten for the second
//! platform, because on Android the real data directory is reachable only
//! through the Java `Context`.

// Stops a second, console window from opening alongside the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use accounting_core::db::{Db, seed_defaults};
use accounting_core::domain::budget_calc::OnTrack;
use accounting_core::domain::reconcile::AmountEntry;
use accounting_core::domain::splits::{SplitAmount, SplitDraft};
use accounting_core::repo::transactions::{DetailLevel, TxnFilter};
use accounting_core::repo::{categories, locations, merchants, transactions};
use accounting_core::service::budgets;
use accounting_core::service::entry::{self, SubtotalDraft, SubtotalRow, TransactionDraft};
use accounting_core::{CategoryId, Money, TxnId};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

/// The open database, behind a mutex.
///
/// `rusqlite::Connection` is `Send` but not `Sync`, which is correct rather
/// than inconvenient: SQLite serialises at the file level anyway, and the core
/// is fully synchronous. One connection guarded by a mutex matches what the
/// storage layer actually is.
struct AppState {
    db: Mutex<Db>,
}

/// Commands return this so a failure reaches the UI as a sentence.
///
/// `CoreError`'s messages are written to be read by a person — "$3.40 is
/// unassigned; add a split or make one of them `remainder`" — so passing the
/// string through is better than inventing a code the frontend would have to
/// translate back into the same sentence.
type CmdResult<T> = Result<T, String>;

fn to_message(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------------------
// Wire types
//
// These mirror `ui/src/api.ts` field for field. They are separate from the
// core's own types for one reason: the core's types are free to change shape
// for the core's reasons, and an IPC payload is a compatibility surface. The
// conversion below is the seam where that is absorbed.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryDto {
    id: i64,
    name: String,
    group_name: String,
    is_budgeted: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum SplitAmountDto {
    Fixed {
        cents: i64,
    },
    /// Basis points: 25% is `2500`.
    Percent {
        bp: u32,
    },
    Remainder,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SplitDraftDto {
    category_id: Option<i64>,
    amount: SplitAmountDto,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftDto {
    id: Option<i64>,
    /// `YYYY-MM-DD`. The frontend owns "today" — it knows the user's timezone
    /// and may read the clock; the core may not.
    txn_date: String,
    merchant: Option<String>,
    location: Option<String>,
    subtotal: Option<i64>,
    tax: Option<i64>,
    total: Option<i64>,
    splits: Vec<SplitDraftDto>,
    detail_level: Option<String>,
    notes: Option<String>,
    needs_review: bool,
    remember_defaults: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SplitRowDto {
    category_id: Option<i64>,
    category_name: Option<String>,
    subtotal: i64,
    tax: i64,
    amount: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewDto {
    subtotal: i64,
    tax: i64,
    total: i64,
    tax_is_estimated: bool,
    tax_rate_ppm: u32,
    unassigned: i64,
    detail_level: String,
    rows: Vec<SplitRowDto>,
    warnings: Vec<String>,
    blockers: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextDto {
    merchant_name: Option<String>,
    location_name: Option<String>,
    tax_rate_ppm: u32,
    default_category_id: Option<i64>,
    detail_level: String,
    recent_merchants: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubtotalRowDto {
    category_id: Option<i64>,
    subtotal: i64,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubtotalDraftDto {
    id: Option<i64>,
    txn_date: String,
    merchant: Option<String>,
    location: Option<String>,
    rows: Vec<SubtotalRowDto>,
    notes: Option<String>,
    needs_review: bool,
    remember_defaults: bool,
}

impl TryFrom<SubtotalDraftDto> for SubtotalDraft {
    type Error = String;

    fn try_from(dto: SubtotalDraftDto) -> Result<SubtotalDraft, String> {
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
                    note: r.note,
                })
                .collect(),
            notes: dto.notes,
            needs_review: dto.needs_review,
            remember_defaults: dto.remember_defaults,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MerchantDto {
    id: i64,
    name: String,
    default_category_id: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocationDto {
    id: i64,
    name: String,
    tax_rate_ppm: u32,
    is_home: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BudgetRowDto {
    category_id: i64,
    category_name: String,
    group_name: String,
    /// `None` throughout when the category has no budget line.
    budgeted: Option<i64>,
    per_month: Option<i64>,
    funded: Option<i64>,
    spent_cycle: Option<i64>,
    overspent_by: Option<i64>,
    spent_this_month: i64,
    /// `"ahead" | "on track" | "behind" | "none"`.
    funding: String,
    funding_by: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TxnSummaryDto {
    id: i64,
    txn_date: String,
    merchant_name: Option<String>,
    total: i64,
    tax_is_estimated: bool,
    needs_review: bool,
    categories: Vec<String>,
}

impl TryFrom<SplitAmountDto> for SplitAmount {
    type Error = String;

    fn try_from(dto: SplitAmountDto) -> Result<SplitAmount, String> {
        Ok(match dto {
            SplitAmountDto::Fixed { cents } => SplitAmount::Fixed(Money::from_cents(cents)),
            SplitAmountDto::Percent { bp } => SplitAmount::Percent(bp),
            SplitAmountDto::Remainder => SplitAmount::Remainder,
        })
    }
}

impl TryFrom<DraftDto> for TransactionDraft {
    type Error = String;

    fn try_from(dto: DraftDto) -> Result<TransactionDraft, String> {
        let txn_date = dto
            .txn_date
            .parse::<chrono::NaiveDate>()
            .map_err(|e| format!("{} is not a YYYY-MM-DD date: {e}", dto.txn_date))?;

        let detail_level = match dto.detail_level.as_deref() {
            Some(s) => Some(s.parse::<DetailLevel>().map_err(to_message)?),
            None => None,
        };

        let mut splits = Vec::with_capacity(dto.splits.len());
        for split in dto.splits {
            splits.push(SplitDraft {
                id: None,
                category_id: split.category_id.map(CategoryId::new),
                amount: split.amount.try_into()?,
                note: split.note,
                line_items: Vec::new(),
            });
        }

        Ok(TransactionDraft {
            id: dto.id.map(TxnId::new),
            txn_date,
            merchant: dto.merchant,
            location: dto.location,
            entry: AmountEntry {
                subtotal: dto.subtotal.map(Money::from_cents),
                tax: dto.tax.map(Money::from_cents),
                total: dto.total.map(Money::from_cents),
                // The frontend sends whole amounts rather than keystrokes, so
                // there is no "last edited" field to break a three-way tie.
                // `reconcile` then refuses an inconsistent triple outright,
                // which is the safe direction: it asks rather than guesses.
                last_edited: None,
            },
            splits,
            detail_level,
            notes: dto.notes,
            needs_review: dto.needs_review,
            remember_defaults: dto.remember_defaults,
        })
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_categories(state: State<'_, AppState>) -> CmdResult<Vec<CategoryDto>> {
    let db = state.db.lock().map_err(to_message)?;
    let rows = categories::list(&db).map_err(to_message)?;
    Ok(rows
        .into_iter()
        .map(|c| CategoryDto {
            id: c.id.get(),
            name: c.name,
            group_name: c.group_name,
            is_budgeted: c.is_budgeted,
        })
        .collect())
}

/// Reconcile and plan a draft **without writing anything**.
///
/// Safe to call on every keystroke, and the only way the UI learns what a
/// receipt's numbers are. Because `save_transaction` runs this same function
/// before persisting, the UI cannot display one set of numbers and store
/// another.
#[tauri::command]
fn preview_transaction(state: State<'_, AppState>, draft: DraftDto) -> CmdResult<PreviewDto> {
    let db = state.db.lock().map_err(to_message)?;
    let draft: TransactionDraft = draft.try_into()?;
    let preview = entry::preview_transaction(&db, &draft).map_err(to_message)?;
    // Resolve category names once here rather than per row: the picker already
    // has them, but the preview's rows may include a synthesised split the UI
    // never sent, and it still has to be labelled.
    let names = categories::list_all(&db).map_err(to_message)?;
    Ok(to_preview_dto(&preview, &names))
}

/// Shared by both entry modes, so the two can never render differently.
fn to_preview_dto(
    preview: &entry::TransactionPreview,
    names: &[accounting_core::Category],
) -> PreviewDto {
    let rows = preview
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
        .collect();

    PreviewDto {
        subtotal: preview.amounts.subtotal.cents(),
        tax: preview.amounts.tax.cents(),
        total: preview.amounts.total.cents(),
        tax_is_estimated: preview.amounts.tax_is_estimated,
        tax_rate_ppm: preview.tax_rate_ppm,
        unassigned: preview.plan.unassigned.cents(),
        detail_level: preview.detail_level.to_string(),
        rows,
        warnings: preview.warnings.clone(),
        blockers: preview.blockers.clone(),
    }
}

#[tauri::command]
fn save_transaction(state: State<'_, AppState>, draft: DraftDto) -> CmdResult<i64> {
    let db = state.db.lock().map_err(to_message)?;
    let draft: TransactionDraft = draft.try_into()?;
    // The timestamp is the frontend's to supply — the core never reads the
    // clock — but "now" for a `last_used_at` is genuinely this instant, so it
    // is taken here rather than round-tripped through the UI.
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    entry::save_transaction(&db, &draft, &now)
        .map(TxnId::get)
        .map_err(to_message)
}

fn summarise(txn: &accounting_core::repo::transactions::Txn) -> TxnSummaryDto {
    TxnSummaryDto {
        id: txn.id.get(),
        txn_date: txn.txn_date.to_string(),
        merchant_name: txn.merchant_name.clone(),
        total: txn.total.cents(),
        tax_is_estimated: txn.tax_is_estimated,
        needs_review: txn.needs_review,
        categories: txn.splits.iter().map(|s| s.category_name.clone()).collect(),
    }
}

#[tauri::command]
fn list_transactions(state: State<'_, AppState>, limit: usize) -> CmdResult<Vec<TxnSummaryDto>> {
    let db = state.db.lock().map_err(to_message)?;
    let filter = TxnFilter {
        limit: Some(limit),
        ..TxnFilter::default()
    };
    let rows = transactions::list(&db, &filter).map_err(to_message)?;
    Ok(rows.iter().map(summarise).collect())
}

#[tauri::command]
fn needs_categorizing(state: State<'_, AppState>) -> CmdResult<Vec<TxnSummaryDto>> {
    let db = state.db.lock().map_err(to_message)?;
    let rows = entry::needs_categorizing(&db, None).map_err(to_message)?;
    Ok(rows.iter().map(summarise).collect())
}

/// What the app already knows about a store, so the next receipt prefills.
///
/// This is the store-memory feature, and it is the reason the entry flow is
/// short: naming a merchant recalls its usual category, its location and
/// whether you itemize there, which removes three decisions from every visit.
/// Naming a *new* merchant is not an error and creates nothing — the row
/// appears on save, so abandoning a half-typed receipt leaves no debris in the
/// picker.
#[tauri::command]
fn entry_context(state: State<'_, AppState>, merchant: Option<String>) -> CmdResult<ContextDto> {
    let db = state.db.lock().map_err(to_message)?;
    let ctx = entry::entry_context(&db, merchant.as_deref()).map_err(to_message)?;
    Ok(ContextDto {
        merchant_name: ctx.merchant.as_ref().map(|m| m.name.clone()),
        location_name: ctx.location.as_ref().map(|l| l.name.clone()),
        tax_rate_ppm: ctx.tax.rate_ppm,
        default_category_id: ctx.default_category.as_ref().map(|c| c.id.get()),
        detail_level: ctx.detail_level.to_string(),
        recent_merchants: ctx
            .recent_merchants
            .iter()
            .map(|m| m.name.clone())
            .collect(),
    })
}

/// Every known store, for the entry form's picker.
///
/// Includes the seeded suggestions and anything the user has actually shopped
/// at. Ordered most-recently-used first so the places you go keep floating to
/// the top of the list, with the untouched suggestions behind them.
/// Reconcile a subtotal-entered receipt without writing anything.
///
/// The tax is worked out from the location's rate, so the form never asks for
/// it — nor for a total, which is the sum of the parts plus that tax.
#[tauri::command]
fn preview_subtotals(state: State<'_, AppState>, draft: SubtotalDraftDto) -> CmdResult<PreviewDto> {
    let db = state.db.lock().map_err(to_message)?;
    let draft: SubtotalDraft = draft.try_into()?;
    let preview = entry::preview_subtotals(&db, &draft).map_err(to_message)?;
    let names = categories::list_all(&db).map_err(to_message)?;
    Ok(to_preview_dto(&preview, &names))
}

#[tauri::command]
fn save_subtotals(state: State<'_, AppState>, draft: SubtotalDraftDto) -> CmdResult<i64> {
    let db = state.db.lock().map_err(to_message)?;
    let draft: SubtotalDraft = draft.try_into()?;
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    entry::save_subtotals(&db, &draft, &now)
        .map(TxnId::get)
        .map_err(to_message)
}

/// The home location and its rate, for the first-run prompt.
#[tauri::command]
fn home_location(state: State<'_, AppState>) -> CmdResult<Option<LocationDto>> {
    let db = state.db.lock().map_err(to_message)?;
    Ok(locations::home(&db)
        .map_err(to_message)?
        .map(|l| LocationDto {
            id: l.id.get(),
            name: l.name,
            tax_rate_ppm: l.tax_rate_ppm,
            is_home: l.is_home,
        }))
}

/// Name the place the user lives and set its sales-tax rate.
///
/// Asked once, on first run, because every later receipt without an explicit
/// location inherits this rate — and a receipt silently recording $0.00 of tax
/// is wrong in a way nothing else surfaces.
#[tauri::command]
fn set_home_location(state: State<'_, AppState>, name: String, tax_rate_ppm: u32) -> CmdResult<()> {
    let db = state.db.lock().map_err(to_message)?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a location needs a name".to_string());
    }
    if tax_rate_ppm > 1_000_000 {
        return Err("a tax rate above 100% is not a rate".to_string());
    }

    // Renames the existing home rather than adding a second one: the schema
    // permits exactly one, and the user is telling us where home *is*, not
    // adding a place they visited.
    match locations::home(&db).map_err(to_message)? {
        Some(home) => {
            locations::rename(&db, home.id, trimmed).map_err(to_message)?;
            locations::set_rate(&db, home.id, tax_rate_ppm).map_err(to_message)?;
        }
        None => {
            let created =
                locations::get_or_create(&db, trimmed, tax_rate_ppm).map_err(to_message)?;
            locations::set_home(&db, created.id).map_err(to_message)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn list_merchants(state: State<'_, AppState>) -> CmdResult<Vec<MerchantDto>> {
    let db = state.db.lock().map_err(to_message)?;
    let all = merchants::list(&db).map_err(to_message)?;
    let recent = merchants::recent(&db, all.len()).map_err(to_message)?;
    Ok(recent
        .into_iter()
        .map(|m| MerchantDto {
            id: m.id.get(),
            name: m.name,
            default_category_id: m.default_category_id.map(CategoryId::get),
        })
        .collect())
}

#[tauri::command]
fn list_locations(state: State<'_, AppState>) -> CmdResult<Vec<LocationDto>> {
    let db = state.db.lock().map_err(to_message)?;
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

/// Assign categories to a quick-added receipt and clear its review flag.
///
/// The amounts are already on disk and are not re-entered; this only decides
/// which envelopes they came out of, which is why it takes splits and nothing
/// else.
#[tauri::command]
fn categorize(state: State<'_, AppState>, id: i64, splits: Vec<SplitDraftDto>) -> CmdResult<()> {
    let db = state.db.lock().map_err(to_message)?;
    let mut drafts = Vec::with_capacity(splits.len());
    for split in splits {
        drafts.push(SplitDraft {
            id: None,
            category_id: split.category_id.map(CategoryId::new),
            amount: split.amount.try_into()?,
            note: split.note,
            line_items: Vec::new(),
        });
    }
    entry::categorize(&db, TxnId::new(id), &drafts).map_err(to_message)
}

/// The budget screen for one month.
///
/// `funding` and `overspent` are separate fields on purpose. They answer
/// different questions and disagree in normal use: an envelope spent exactly
/// to plan is behind on *funding* (the money was meant to be banked and is
/// not) while being perfectly fine on *spending*. Rendering the first as "over
/// budget" would make the screen cry wolf on every well-run category.
#[tauri::command]
fn budget_status(state: State<'_, AppState>, month: String) -> CmdResult<Vec<BudgetRowDto>> {
    let db = state.db.lock().map_err(to_message)?;
    let ym = month
        .parse::<accounting_core::YearMonth>()
        .map_err(to_message)?;
    let rows = budgets::budget_status(&db, ym).map_err(to_message)?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let (funding, funding_by) = match row.status.as_ref().map(|s| s.on_track) {
                Some(OnTrack::Ahead(by)) => ("ahead", by.cents()),
                Some(OnTrack::OnTrack) => ("on track", 0),
                Some(OnTrack::Behind(by)) => ("behind", by.cents()),
                None => ("none", 0),
            };
            BudgetRowDto {
                category_id: row.category_id.get(),
                category_name: row.category_name,
                group_name: row.group_name,
                budgeted: row.status.as_ref().map(|s| s.spend.budgeted.cents()),
                per_month: row.status.as_ref().map(|s| s.monthly_contribution.cents()),
                funded: row.status.as_ref().map(|s| s.funded_balance.cents()),
                spent_cycle: row.status.as_ref().map(|s| s.spend.spent.cents()),
                overspent_by: row.status.as_ref().map(|s| s.spend.overspent_by.cents()),
                spent_this_month: row.spent_this_month.cents(),
                funding: funding.to_string(),
                funding_by,
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------

/// Where the database lives — **the same file `pacc` uses**.
///
/// The obvious implementation is `app_data_dir().join("pacc.db")`, and it is
/// wrong on the desktop. Tauri derives that directory from the bundle
/// identifier, so it resolves to `~/.local/share/com.tannerhathaway.accounting`
/// while the CLI resolves `~/.local/share/personal-accounting`. Two files, one
/// per frontend: receipts entered with `pacc add` would simply not be in the
/// window, and nothing would report an error — the GUI would just look empty,
/// which reads as data loss.
///
/// So on desktop this mirrors [`accounting-cli`'s `default_db_path`] exactly,
/// `$PACC_DB` override included, and the two frontends share one database.
///
/// On Android and iOS the app cannot write to a shared location at all: the
/// data directory is per-app and only obtainable through the platform (the
/// Java `Context` on Android), which is precisely what `app_data_dir` wraps.
/// There is no CLI on those platforms to share with, so there is nothing to
/// reconcile and the per-app directory is simply correct.
fn database_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Ok(app.path().app_data_dir()?.join("pacc.db"));
    }

    if let Some(explicit) = std::env::var_os("PACC_DB").filter(|v| !v.is_empty()) {
        return Ok(std::path::PathBuf::from(explicit));
    }

    // `data_dir()` is `$XDG_DATA_HOME`, falling back to `~/.local/share` —
    // the same two rules the CLI applies, in the same order.
    Ok(app
        .path()
        .data_dir()?
        .join("personal-accounting")
        .join("pacc.db"))
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // The one place a path is decided. See the module docs.
            let path = database_path(app.handle())?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let db = Db::open(&path)?;
            seed_defaults(&db)?;

            app.manage(AppState { db: Mutex::new(db) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_categories,
            preview_transaction,
            save_transaction,
            list_transactions,
            needs_categorizing,
            entry_context,
            preview_subtotals,
            save_subtotals,
            home_location,
            set_home_location,
            list_merchants,
            list_locations,
            categorize,
            budget_status,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}
