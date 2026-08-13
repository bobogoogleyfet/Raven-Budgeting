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

#![doc(html_no_source)]

pub mod db;
pub mod domain;
pub mod error;
pub mod ids;
pub mod money;
pub mod time;

pub use db::{Db, InvariantViolation, check_all, seed_defaults};
pub use error::{CoreError, Result};
pub use ids::{
    BudgetLineId, CategoryGroupId, CategoryId, LineItemId, LocationId, MerchantId, SplitId, TxnId,
};
pub use money::{Money, MoneyParseError};
pub use time::{MonthRange, ParseYearMonthError, YearMonth, month_short_name, quarter_of};
