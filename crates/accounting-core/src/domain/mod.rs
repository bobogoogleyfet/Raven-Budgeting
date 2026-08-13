//! Pure domain calculations.
//!
//! Every function in this module is a function of its arguments: values in,
//! values out. Nothing here takes a database handle, opens a file, or reads
//! the clock — dates arrive as parameters. That is what makes the money rules
//! testable without fixtures and identical on every frontend.

pub mod budget_calc;
pub mod reconcile;
pub mod splits;
pub mod tax;

pub use budget_calc::{
    BudgetLine, FundedStatus, OnTrack, Period, SpendStatus, contributions, cycle_containing,
    funded_balance, monthly_contribution,
};
pub use reconcile::{AmountEntry, AmountField, Amounts, ReconcileError, TaxContext, reconcile};
pub use splits::{
    LineItemDraft, ResolvedSplit, SplitAmount, SplitDraft, SplitError, SplitPlan, plan_splits,
};
pub use tax::{distribute_discount, distribute_tax, resolve_rate};
