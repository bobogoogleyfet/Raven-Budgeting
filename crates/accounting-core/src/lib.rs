//! Domain model, persistence, and export for the personal accounting app.
//!
//! # Boundary rules
//!
//! This crate is the product. Everything else (CLI, Tauri desktop, a future
//! Android UI) is a rendering layer over it. Four rules keep it that way, and
//! each one maps to a concrete mobile failure mode:
//!
//! 1. **No UI, no path discovery.** This crate never names `tauri`, `tokio`,
//!    `dirs`, or `directories`. [`db::Db::open`] takes a path it is *given* —
//!    the caller supplies it (from `app.path().app_data_dir()` on Tauri, or a
//!    flag on the CLI). Path-discovery crates return meaningless results on
//!    Android.
//! 2. **Fully synchronous.** SQLite is a local file; there is nothing to await.
//!    Async would drag in `tokio`/`mio` for no benefit.
//! 3. **Never reads the clock.** Every date-dependent function takes
//!    `today: NaiveDate` explicitly. This avoids `chrono`'s `iana-time-zone`
//!    JNI dependency on Android *and* makes every test deterministic without
//!    clock mocking.
//! 4. **All money math lives here.** UIs send a draft and render the returned
//!    preview; they never do arithmetic. The moment a frontend computes
//!    `total - subtotal`, floats are back and the logic has to be rewritten
//!    per platform.

//!
//! # Milestone 1 surface
//!
//! [`Money`] (i64 cents, never a float), typed [`ids`], calendar-month
//! [`time`] types, and the pure [`domain`] calculations built on them.
//!
//! # Milestone 2 surface
//!
//! [`db`] — a `STRICT` SQLite schema, `user_version` migrations, the default
//! chart of accounts, and [`db::check_all`] for the two aggregate invariants
//! that no constraint in SQLite can express.
//!
//! # Milestone 3 surface
//!
//! [`repo`] — typed reads, and the **single write path**
//! [`repo::transactions::save`], which replaces a whole transaction aggregate
//! inside one SQLite transaction. That is what holds the aggregate invariants
//! true; [`db::check_all`] only audits them.
//!
//! [`service::entry`] — the workflow every frontend calls: resolve the names
//! the user typed, apply what we remember about the store, reconcile the
//! amounts, plan the splits, persist. [`service::entry::preview_transaction`]
//! is the same code as [`service::entry::save_transaction`] minus the write,
//! so a UI cannot display one set of numbers and store another.
//!
//! # Milestone 4 surface
//!
//! [`export`] — the transaction aggregate flattened to one row per split, and
//! rendered as either an `.xlsx` workbook ([`export_xlsx`]) or a CSV file
//! ([`export_csv`]). Both return `Vec<u8>`; **neither opens a path**. Rule 1
//! again: on Android and iOS the user picks the destination through a system
//! document picker and the app is handed a stream, so file handling has to be
//! the frontend's job or it is nobody's.
//!
//! The 15 columns are defined exactly once, in [`COLUMNS`], and drive the
//! header row, the cell writes and the CSV alike — Excel "repairs" (blanks) a
//! workbook whose table definition disagrees with its header cells, and two
//! hand-maintained lists of the same names diverge at the first edit.
//!
//! # Milestone 5 surface
//!
//! [`repo::budgets`] — budget lines and adjustments. A budget is a historical
//! record, not a setting: every line carries the months it applies to, so
//! changing next year's grocery budget cannot rewrite last year's report.
//! [`repo::budgets::upsert`] refuses to leave two live lines covering one
//! month, which no SQLite constraint can express and which would silently
//! **double** every figure for the months they share.

#![doc(html_no_source)]

pub mod db;
pub mod domain;
pub mod error;
pub mod export;
pub mod ids;
pub mod money;
pub mod repo;
pub mod service;
pub mod time;

pub use db::{Db, InvariantViolation, check_all, seed_defaults};
pub use domain::budget_calc::{BudgetLine, FundedStatus, OnTrack, Period, SpendStatus};
pub use error::{CoreError, Result};
pub use export::csv::{csv_bytes, export_csv};
pub use export::xlsx::{export_xlsx, workbook_bytes};
pub use export::{COLUMNS, CellValue, Column, ColumnKind, ExportRow};
pub use ids::{
    BudgetLineId, CategoryGroupId, CategoryId, LineItemId, LocationId, MerchantId, SplitId, TxnId,
};
pub use money::{Money, MoneyParseError};
pub use repo::budgets::{Adjustment, BudgetLineRecord, StoredBudgetLine, Superseded};
pub use repo::categories::{Category, CategoryGroup};
pub use repo::locations::Location;
pub use repo::merchants::Merchant;
pub use repo::transactions::{
    DetailLevel, LineItemRecord, LoadedLineItem, LoadedSplit, NewTransaction, SplitRecord, Txn,
    TxnFilter,
};
pub use service::entry::{EntryContext, TransactionDraft, TransactionPreview};
pub use time::{MonthRange, ParseYearMonthError, YearMonth, month_short_name, quarter_of};
