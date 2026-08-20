//! Application services: the layer every frontend calls.
//!
//! A repository knows one table. A *service* knows the workflow: resolve the
//! names the user typed, apply what we remember about the store, reconcile the
//! amounts, plan the splits, and persist the result as one aggregate. That
//! sequence is the product, and it lives here precisely so that the CLI, the
//! Flutter app on the desktop and the same app on Android are three renderings
//! of one implementation rather than three implementations.
//!
//! The division of labour with [`crate::domain`] is strict: domain functions
//! are pure arithmetic over values, services are the only place that combines
//! them with the database. Neither reads the clock — timestamps arrive as
//! parameters from the caller.

pub mod budgets;
pub mod entry;
pub mod locations;
pub mod rates;
pub mod reports;
