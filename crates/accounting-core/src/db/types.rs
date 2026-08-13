//! `ToSql` / `FromSql` for the crate's newtypes.
//!
//! Without these, every repository function would unwrap a `CategoryId` into
//! an `i64` on the way in and re-wrap it on the way out — dozens of hand
//! conversions, each one a chance to pass a `MerchantId` where a `CategoryId`
//! belongs. With them the newtypes go straight into `params![]` and come
//! straight out of `row.get()`, and the type system covers the whole path.
//!
//! [`Money`] stores as INTEGER cents, matching the schema's `*_cents` columns.
//! [`YearMonth`] stores as TEXT `'YYYY-MM'`, matching the `GLOB` checks on
//! `budget_line.period_start` and `budget_adjustment.year_month`, and sorting
//! chronologically as a string.
//!
//! These impls live here rather than in `ids.rs` / `money.rs` so that those
//! modules stay free of any storage dependency.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use crate::ids::{
    BudgetLineId, CategoryGroupId, CategoryId, LineItemId, LocationId, MerchantId, SplitId, TxnId,
};
use crate::money::Money;
use crate::time::YearMonth;

impl ToSql for Money {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.cents()))
    }
}

impl FromSql for Money {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Money::from_cents)
    }
}

impl ToSql for YearMonth {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

impl FromSql for YearMonth {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        s.parse::<YearMonth>()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

macro_rules! sql_id {
    ($($name:ident),* $(,)?) => {$(
        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.get()))
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                i64::column_result(value).map($name::new)
            }
        }
    )*};
}

sql_id!(
    TxnId,
    SplitId,
    LineItemId,
    CategoryId,
    CategoryGroupId,
    MerchantId,
    LocationId,
    BudgetLineId,
);

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn money_round_trips_as_integer_cents() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (v INTEGER NOT NULL) STRICT;")
            .unwrap();

        for cents in [0_i64, 1, -1, 123_456, -987_654, i64::MAX, i64::MIN] {
            conn.execute("DELETE FROM t", []).unwrap();
            conn.execute("INSERT INTO t (v) VALUES (?1)", [Money::from_cents(cents)])
                .unwrap();

            // Stored as a bare integer, not text and not a float: a STRICT
            // INTEGER column would have rejected anything else outright.
            let raw: i64 = conn.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
            assert_eq!(raw, cents);

            let back: Money = conn.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
            assert_eq!(back, Money::from_cents(cents));
        }
    }

    #[test]
    fn ids_round_trip_and_stay_distinct() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (a INTEGER, b INTEGER) STRICT;")
            .unwrap();
        conn.execute(
            "INSERT INTO t (a, b) VALUES (?1, ?2)",
            (CategoryId::new(7), MerchantId::new(9)),
        )
        .unwrap();

        let (cat, mer): (CategoryId, MerchantId) = conn
            .query_row("SELECT a, b FROM t", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(cat, CategoryId::new(7));
        assert_eq!(mer, MerchantId::new(9));
    }

    #[test]
    fn year_month_stores_as_sortable_text() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (ym TEXT NOT NULL) STRICT;")
            .unwrap();
        for ym in ["2026-09", "2025-12", "2026-01"] {
            conn.execute(
                "INSERT INTO t (ym) VALUES (?1)",
                [ym.parse::<YearMonth>().unwrap()],
            )
            .unwrap();
        }

        let mut stmt = conn.prepare("SELECT ym FROM t ORDER BY ym").unwrap();
        let got: Vec<YearMonth> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            got,
            vec![
                "2025-12".parse::<YearMonth>().unwrap(),
                "2026-01".parse().unwrap(),
                "2026-09".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn a_malformed_year_month_is_an_error_not_a_panic() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (ym TEXT NOT NULL) STRICT;")
            .unwrap();
        conn.execute("INSERT INTO t (ym) VALUES ('nope')", [])
            .unwrap();
        let err = conn
            .query_row("SELECT ym FROM t", [], |r| r.get::<_, YearMonth>(0))
            .unwrap_err();
        assert!(matches!(err, rusqlite::Error::FromSqlConversionFailure(..)));
    }
}
