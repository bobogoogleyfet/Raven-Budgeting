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

#![doc(html_no_source)]

pub mod db;
pub mod domain;
pub mod error;
pub mod ids;
pub mod money;
pub mod repo;
pub mod service;
pub mod time;

pub use db::{Db, InvariantViolation, check_all, seed_defaults};
pub use error::{CoreError, Result};
pub use ids::{
    BudgetLineId, CategoryGroupId, CategoryId, LineItemId, LocationId, MerchantId, SplitId, TxnId,
};
pub use money::{Money, MoneyParseError};
pub use repo::categories::{Category, CategoryGroup};
pub use repo::locations::Location;
pub use repo::merchants::Merchant;
pub use repo::transactions::{
    DetailLevel, LineItemRecord, LoadedLineItem, LoadedSplit, NewTransaction, SplitRecord, Txn,
    TxnFilter,
};
pub use service::entry::{EntryContext, TransactionDraft, TransactionPreview};
pub use time::{MonthRange, ParseYearMonthError, YearMonth, month_short_name, quarter_of};
