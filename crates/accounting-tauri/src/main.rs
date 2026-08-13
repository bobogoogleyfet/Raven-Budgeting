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
//! `accounting-core` never discovers a path — see its crate docs. Here the path
//! comes from `app.path().app_data_dir()`, which is the only API that returns
//! the right answer on every platform. On Android the real data directory is
//! obtainable only through the Java `Context`, so a core that called `dirs` or
//! read `$HOME` would have to be rewritten for the second platform.

// Stops a second, console window from opening alongside the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use accounting_core::db::{Db, seed_defaults};
use accounting_core::domain::reconcile::AmountEntry;
use accounting_core::domain::splits::{SplitAmount, SplitDraft};
use accounting_core::repo::transactions::{DetailLevel, TxnFilter};
use accounting_core::repo::{categories, transactions};
use accounting_core::service::entry::{self, TransactionDraft};
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

    Ok(PreviewDto {
        subtotal: preview.amounts.subtotal.cents(),
        tax: preview.amounts.tax.cents(),
        total: preview.amounts.total.cents(),
        tax_is_estimated: preview.amounts.tax_is_estimated,
        tax_rate_ppm: preview.tax_rate_ppm,
        unassigned: preview.plan.unassigned.cents(),
        detail_level: preview.detail_level.to_string(),
        rows,
        warnings: preview.warnings,
        blockers: preview.blockers,
    })
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

// ---------------------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // The one place a path is decided. See the module docs.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;

            // Same filename the CLI uses, so `pacc doctor`, `pacc export` and
            // `pacc backup` all still work against the GUI's database once the
            // user points `--db` at it.
            let db = Db::open(&dir.join("pacc.db"))?;
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
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}
