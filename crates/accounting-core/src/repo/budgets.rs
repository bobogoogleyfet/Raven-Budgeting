//! Budget lines and one-off adjustments.
//!
//! # A budget line is a historical record, not a setting
//!
//! Every line carries `period_start` and an optional `period_end`, so changing
//! next year's grocery budget cannot rewrite last year's report. Editing a line
//! in place is still possible ([`upsert`] with an id) and is the right move for
//! a typo; superseding it — closing the old line and opening a new one — is the
//! right move for an actual change of plan, and the caller decides which.
//!
//! # Overlap is refused here, because the schema cannot refuse it
//!
//! `ux_budget_line_period (category_id, period_start)` catches only two live
//! lines that *start* in the same month. Two Groceries lines covering
//! `2026-01..2026-06` and `2026-04..` are both live in April, May and June, and
//! every budget figure for those months would silently **double count**: two
//! contributions, two targets, two envelopes. A fully general non-overlap
//! constraint cannot be expressed as an index, so [`upsert`] enforces it, in
//! the same transaction as the write — a check outside the transaction races
//! itself, and two concurrent writers would each see a clean database and both
//! insert.
//!
//! # Period shapes
//!
//! [`Period`] maps onto three columns: `period_type`, `period_interval` and
//! `due_date`. The schema CHECKs the combinations, but this module validates
//! them first so the user gets a sentence naming the problem instead of
//! `CHECK constraint failed: budget_line`.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rusqlite::{Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::db::{Db, new_uuid};
use crate::domain::budget_calc::{BudgetLine, Period};
use crate::error::{CoreError, Result};
use crate::ids::{BudgetLineId, CategoryId};
use crate::money::Money;
use crate::repo::{categories, not_found};
use crate::time::{MonthRange, YearMonth};

/// A budget line as handed to [`upsert`].
///
/// Names nothing: resolution of "the category the user typed" belongs to
/// [`crate::repo::categories::resolve`], one layer up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLineRecord {
    /// `None` inserts. `Some(id)` updates that line in place, keeping its uuid
    /// and `created_at` — it is the same budget, restated.
    pub id: Option<BudgetLineId>,
    pub category_id: CategoryId,
    pub period: Period,
    /// First month the line is in force, and the anchor every cycle counts
    /// from.
    pub period_start: YearMonth,
    /// Last month in force; `None` means "still in force".
    pub period_end: Option<YearMonth>,
    /// The target amount for one period — **not** per month. A `Yearly` line of
    /// `$1,200.00` contributes about `$100.00` a month.
    pub amount: Money,
    pub rollover: bool,
    pub notes: Option<String>,
}

/// A budget line as loaded, with the domain value ready for
/// [`crate::domain::budget_calc`] and the names needed to render it.
///
/// `line` is the exact type the calculations take, so a caller never rebuilds
/// one field by field and never gets a chance to rebuild it wrong.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredBudgetLine {
    pub line: BudgetLine,
    pub category_name: String,
    pub group_name: String,
    pub notes: Option<String>,
}

impl StoredBudgetLine {
    /// `"Group/Name"`, the unambiguous rendering of the category.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        format!("{}/{}", self.group_name, self.category_name)
    }
}

/// One manual correction to a category's envelope for one month.
///
/// Moving money between envelopes, or writing off an overspend so it stops
/// propagating through every later rollover balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjustment {
    pub category_id: CategoryId,
    pub year_month: YearMonth,
    pub amount: Money,
    pub reason: Option<String>,
}

/// A stored `period_type` triple that does not describe any [`Period`].
///
/// The schema's CHECKs make this unreachable through this crate; seeing one
/// means the file was written by something else, and rendering it as the wrong
/// cadence would hide that. Kept private and `String`-shaped so it is `Send +
/// Sync`, which `rusqlite::Error::FromSqlConversionFailure` requires.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct BadPeriod(String);

const SELECT_LINE: &str = "SELECT b.id, b.category_id, b.period_type, b.period_interval,
                                  b.due_date, b.period_start, b.period_end, b.amount_cents,
                                  b.rollover, b.notes, c.name, g.name
                             FROM budget_line b
                             JOIN category c ON c.id = b.category_id
                             JOIN category_group g ON g.id = c.group_id";

/// The three columns a [`Period`] occupies.
fn period_columns(period: Period) -> (&'static str, Option<i64>, Option<NaiveDate>) {
    match period {
        Period::Monthly => ("monthly", None, None),
        Period::Yearly => ("yearly", None, None),
        Period::EveryNMonths(n) => ("every_n_months", Some(i64::from(n)), None),
        Period::ByDate { due } => ("by_date", None, Some(due)),
    }
}

fn period_from_columns(
    kind: &str,
    interval: Option<i64>,
    due: Option<NaiveDate>,
) -> std::result::Result<Period, BadPeriod> {
    match kind {
        "monthly" => Ok(Period::Monthly),
        "yearly" => Ok(Period::Yearly),
        "every_n_months" => {
            let n = interval.ok_or_else(|| {
                BadPeriod("an `every N months` budget line has no interval".to_string())
            })?;
            let n = u32::try_from(n)
                .map_err(|_| BadPeriod(format!("budget interval {n} is not a month count")))?;
            Ok(Period::EveryNMonths(n))
        }
        "by_date" => due
            .map(|due| Period::ByDate { due })
            .ok_or_else(|| BadPeriod("a `by date` budget line has no due date".to_string())),
        other => Err(BadPeriod(format!(
            "`{other}` is not a budget period type (expected monthly, yearly, \
             every_n_months or by_date)"
        ))),
    }
}

fn map_line(row: &Row<'_>) -> rusqlite::Result<StoredBudgetLine> {
    let kind: String = row.get(2)?;
    let interval: Option<i64> = row.get(3)?;
    let due: Option<NaiveDate> = row.get(4)?;
    let period = period_from_columns(&kind, interval, due).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(StoredBudgetLine {
        line: BudgetLine {
            id: row.get(0)?,
            category_id: row.get(1)?,
            period,
            period_start: row.get(5)?,
            period_end: row.get(6)?,
            amount: row.get(7)?,
            rollover: row.get(8)?,
        },
        notes: row.get(9)?,
        category_name: row.get(10)?,
        group_name: row.get(11)?,
    })
}

/// Everything checkable about a record, checked before a row is touched.
///
/// Public so that a caller which is about to make *two* writes (close the old
/// line, then open the new one) can reject a bad record before it has changed
/// anything — otherwise a typo'd `--every 1` would leave the old budget closed
/// and no new one in its place.
pub fn validate(rec: &BudgetLineRecord) -> Result<()> {
    if let Some(end) = rec.period_end
        && end < rec.period_start
    {
        return Err(CoreError::Invalid(format!(
            "a budget from {} to {} ends before it starts",
            rec.period_start, end
        )));
    }

    match rec.period {
        Period::EveryNMonths(n) if !(2..=120).contains(&n) => {
            return Err(CoreError::Invalid(format!(
                "an `every N months` budget needs N between 2 and 120, not {n}\
                 {}",
                match n {
                    0 => " (a 0-month cycle would never come due)",
                    1 => " (every 1 month is a monthly budget)",
                    _ => " (10 years is the longest cycle supported)",
                }
            )));
        }
        Period::ByDate { due } if YearMonth::from_date(due) < rec.period_start => {
            return Err(CoreError::Invalid(format!(
                "a budget due {due} cannot start saving in {}, which is after it",
                rec.period_start
            )));
        }
        _ => {}
    }
    Ok(())
}

/// Insert a budget line, or update one in place.
///
/// # Refuses to overlap a live line for the same category
///
/// See the module docs: two live lines covering the same month double every
/// figure derived from them, and the schema's unique index only catches the
/// special case of an identical start month. The check runs inside the write's
/// own transaction, so it cannot race another writer, and the error names the
/// span of the line it collided with — "there is already a budget" is not
/// actionable, "there is already one covering 2026-01..2026-06" is.
///
/// An archived line never conflicts: archiving is how a budget is retired, and
/// a retired budget must not block its replacement.
pub fn upsert(db: &Db, rec: &BudgetLineRecord) -> Result<BudgetLineId> {
    validate(rec)?;
    db.with_tx(|tx| {
        ensure_no_overlap(tx, rec)?;
        write(tx, rec)
    })
}

/// What [`supersede`] did, so a frontend can say so out loud.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Superseded {
    /// The line now in force from `period_start`.
    pub id: BudgetLineId,
    /// The line that was closed to make room, and the last month it now
    /// applies to.
    pub closed: Option<(BudgetLineId, YearMonth)>,
    /// `true` when a line already starting in that month was restated in place
    /// rather than a new one being opened.
    pub restated: bool,
}

/// Set a category's budget from `rec.period_start` onward, in one transaction.
///
/// This is what "change my grocery budget" means, and it is three cases:
///
/// * Nothing in force → a new line.
/// * A line already **starts** that month → restated in place. The user is
///   correcting a figure they just entered, and rewriting it is what they
///   asked for.
/// * A line covers that month but started earlier → **closed** the month
///   before, and a new line opens. Editing the old line instead would rewrite
///   every report back to its start month, which is exactly what
///   `period_start`/`period_end` exist to prevent: a budget is a historical
///   record, not a setting.
///
/// # Why this is not two calls to [`upsert`]
///
/// It cannot be. Closing the old line and inserting the new one are two writes,
/// and the new one can still be refused — by validation, or by *another* live
/// line further out that it overlaps. Done from outside, a refusal leaves the
/// category with its old budget closed and nothing in its place, which is worse
/// than either outcome the user was choosing between. One transaction makes the
/// failure a no-op, and it lets the overlap check see the truncated line rather
/// than the original.
pub fn supersede(db: &Db, rec: &BudgetLineRecord) -> Result<Superseded> {
    validate(rec)?;
    db.with_tx(|tx| {
        let covering = covering_line(tx, rec.category_id, rec.period_start)?;
        let mut rec = rec.clone();
        let mut outcome = Superseded {
            id: BudgetLineId::new(0),
            closed: None,
            restated: false,
        };

        match covering {
            Some(existing) if existing.line.period_start == rec.period_start => {
                rec.id = Some(existing.line.id);
                outcome.restated = true;
            }
            Some(existing) => {
                let last = rec.period_start.prev();
                write(
                    tx,
                    &BudgetLineRecord {
                        period_end: Some(last),
                        ..record_of(&existing)
                    },
                )?;
                outcome.closed = Some((existing.line.id, last));
            }
            None => {}
        }

        // After the truncation, so the line that was just closed is not the
        // conflict — and still before the write, so a collision with some other
        // line rolls the truncation back with it.
        ensure_no_overlap(tx, &rec)?;
        outcome.id = write(tx, &rec)?;
        Ok(outcome)
    })
}

/// A stored line, back in the shape a write takes.
fn record_of(stored: &StoredBudgetLine) -> BudgetLineRecord {
    BudgetLineRecord {
        id: Some(stored.line.id),
        category_id: stored.line.category_id,
        period: stored.line.period,
        period_start: stored.line.period_start,
        period_end: stored.line.period_end,
        amount: stored.line.amount,
        rollover: stored.line.rollover,
        notes: stored.notes.clone(),
    }
}

/// The live line covering `ym`, read inside a transaction.
///
/// The same query as [`for_category`], against the transaction rather than the
/// connection, so that what it sees and what the following write does are one
/// atomic unit.
fn covering_line(
    tx: &Transaction<'_>,
    category_id: CategoryId,
    ym: YearMonth,
) -> Result<Option<StoredBudgetLine>> {
    let sql = format!(
        "{SELECT_LINE} WHERE b.archived_at IS NULL AND b.category_id = ?1
                         AND b.period_start <= ?2
                         AND (b.period_end IS NULL OR b.period_end >= ?2)
                       ORDER BY b.period_start DESC LIMIT 1"
    );
    let mut stmt = tx.prepare(&sql)?;
    let mut rows = stmt.query_map(params![category_id, ym], map_line)?;
    rows.next().transpose().map_err(CoreError::Sqlite)
}

/// The insert/update itself, with no checking of its own.
///
/// Private, and every caller runs [`validate`] and [`ensure_no_overlap`] first:
/// this is the half that must be shared between [`upsert`] and [`supersede`]
/// without the checks being shared into the wrong order.
fn write(tx: &Transaction<'_>, rec: &BudgetLineRecord) -> Result<BudgetLineId> {
    let (kind, interval, due) = period_columns(rec.period);
    let notes = rec
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());

    match rec.id {
        Some(id) => {
            // `uuid` and `created_at` are deliberately absent from the SET
            // list: this is the same budget, restated, and its cross-device
            // identity must survive an edit.
            let changed = tx.execute(
                "UPDATE budget_line
                    SET category_id     = ?2,
                        period_type     = ?3,
                        period_interval = ?4,
                        due_date        = ?5,
                        period_start    = ?6,
                        period_end      = ?7,
                        amount_cents    = ?8,
                        rollover        = ?9,
                        notes           = ?10,
                        updated_at      = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                  WHERE id = ?1",
                params![
                    id,
                    rec.category_id,
                    kind,
                    interval,
                    due,
                    rec.period_start,
                    rec.period_end,
                    rec.amount,
                    rec.rollover,
                    notes,
                ],
            )?;
            if changed == 0 {
                return Err(CoreError::NotFound(format!("budget line {id}")));
            }
            Ok(id)
        }
        None => {
            tx.execute(
                "INSERT INTO budget_line (uuid, category_id, period_type, period_interval,
                                          due_date, period_start, period_end, amount_cents,
                                          rollover, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    new_uuid(),
                    rec.category_id,
                    kind,
                    interval,
                    due,
                    rec.period_start,
                    rec.period_end,
                    rec.amount,
                    rec.rollover,
                    notes,
                ],
            )?;
            Ok(BudgetLineId::new(tx.last_insert_rowid()))
        }
    }
}

/// The general interval-intersection test the schema cannot express.
///
/// Two closed-open intervals `[a1, a2]` and `[b1, b2]` (either end possibly
/// unbounded) intersect when `a1 <= b2 && b1 <= a2`. `IS NOT` rather than `<>`
/// for the id, because `<>` against a NULL parameter — an insert, which has no
/// id yet — is NULL, i.e. not true, and would silently match nothing.
fn ensure_no_overlap(tx: &Transaction<'_>, rec: &BudgetLineRecord) -> Result<()> {
    let conflict: Option<(YearMonth, Option<YearMonth>, String, String)> = tx
        .query_row(
            "SELECT b.period_start, b.period_end, c.name, g.name
               FROM budget_line b
               JOIN category c ON c.id = b.category_id
               JOIN category_group g ON g.id = c.group_id
              WHERE b.category_id = ?1
                AND b.archived_at IS NULL
                AND b.id IS NOT ?2
                AND (?3 IS NULL OR b.period_start <= ?3)
                AND (b.period_end IS NULL OR b.period_end >= ?4)
              ORDER BY b.period_start
              LIMIT 1",
            params![rec.category_id, rec.id, rec.period_end, rec.period_start],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(CoreError::Sqlite(other)),
        })?;

    let Some((start, end, category, group)) = conflict else {
        return Ok(());
    };
    Err(CoreError::Invalid(format!(
        "{group}/{category} already has a budget covering {}, which overlaps the {} you asked \
         for; archive that line or change the months so only one applies at a time",
        span(start, end),
        span(rec.period_start, rec.period_end)
    )))
}

/// `"2026-01..2026-06"`, or `"2026-01 onward"` for an open-ended line.
fn span(start: YearMonth, end: Option<YearMonth>) -> String {
    match end {
        Some(end) => format!("{start}..{end}"),
        None => format!("{start} onward"),
    }
}

/// One budget line by id, archived or not.
pub fn get(db: &Db, id: BudgetLineId) -> Result<StoredBudgetLine> {
    let sql = format!("{SELECT_LINE} WHERE b.id = ?1");
    db.conn()
        .query_row(&sql, params![id], map_line)
        .map_err(|e| not_found(e, &format!("budget line {id}")))
}

/// Live budget lines, in display order: group, then category, then start month.
///
/// `active_in` narrows to the lines in force in that month — the ones whose
/// window contains it. Note that "in force" is the stored window, not whether
/// the line still contributes: a `ByDate` line past its due month is still
/// listed until it is archived or given an end month, because it is still the
/// line that owns that category's budget history.
pub fn list(db: &Db, active_in: Option<YearMonth>) -> Result<Vec<StoredBudgetLine>> {
    let mut sql = format!("{SELECT_LINE} WHERE b.archived_at IS NULL");
    if active_in.is_some() {
        sql.push_str(" AND b.period_start <= ?1 AND (b.period_end IS NULL OR b.period_end >= ?1)");
    }
    sql.push_str(" ORDER BY g.sort_order, c.sort_order, c.name, b.period_start");

    let mut stmt = db.conn().prepare(&sql)?;
    let rows: rusqlite::Result<Vec<StoredBudgetLine>> = match active_in {
        Some(ym) => stmt.query_map(params![ym], map_line)?.collect(),
        None => stmt.query_map([], map_line)?.collect(),
    };
    Ok(rows?)
}

/// The live line that owns `ym` for this category, if any.
///
/// At most one can exist — [`upsert`] refuses overlaps — but the query is
/// written to survive a database edited by hand: it takes the latest-starting
/// one, which is the most recent statement of intent.
pub fn for_category(
    db: &Db,
    category_id: CategoryId,
    ym: YearMonth,
) -> Result<Option<StoredBudgetLine>> {
    let sql = format!(
        "{SELECT_LINE} WHERE b.archived_at IS NULL AND b.category_id = ?1
                         AND b.period_start <= ?2
                         AND (b.period_end IS NULL OR b.period_end >= ?2)
                       ORDER BY b.period_start DESC LIMIT 1"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query_map(params![category_id, ym], map_line)?;
    rows.next().transpose().map_err(CoreError::Sqlite)
}

/// Retire a budget line without deleting it.
///
/// Idempotent, and the original archive time survives a second call: "when did
/// this budget stop applying" must not drift every time someone re-runs a
/// script. Nothing user-facing is hard-deleted — last year's report still needs
/// last year's budget.
pub fn archive(db: &Db, id: BudgetLineId) -> Result<()> {
    // Checked first, so a typo'd id is "not found" rather than a silent no-op
    // that reads as success.
    let _ = get(db, id)?;
    db.conn().execute(
        "UPDATE budget_line
            SET archived_at = COALESCE(archived_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at  = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Record a manual correction to one category's envelope for one month.
///
/// Adjustments accumulate rather than replace: moving $50 out of Groceries in
/// March and another $20 back in are two facts, and collapsing them into one
/// row would lose both reasons.
pub fn add_adjustment(
    db: &Db,
    category_id: CategoryId,
    ym: YearMonth,
    amount: Money,
    reason: Option<&str>,
) -> Result<()> {
    // Resolved explicitly so an unknown category reads as "not found" instead
    // of a bare FOREIGN KEY failure, which tells the user nothing.
    let _ = categories::get(db, category_id)?;
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());

    db.conn().execute(
        "INSERT INTO budget_adjustment (uuid, category_id, year_month, amount_cents, reason)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![new_uuid(), category_id, ym, amount, reason],
    )?;
    Ok(())
}

/// Every live adjustment for one category, summed per month.
///
/// Summed in SQL, keyed by month: this is the shape
/// [`crate::domain::budget_calc::funded_balance`] takes, and months with no
/// adjustment are simply absent rather than present as zero.
pub fn adjustments_for(
    db: &Db,
    category_id: CategoryId,
    range: MonthRange,
) -> Result<BTreeMap<YearMonth, Money>> {
    if range.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut stmt = db.conn().prepare(
        "SELECT year_month, SUM(amount_cents)
           FROM budget_adjustment
          WHERE category_id = ?1 AND archived_at IS NULL
            AND year_month >= ?2 AND year_month <= ?3
          GROUP BY year_month",
    )?;
    let rows = stmt
        .query_map(params![category_id, range.start, range.end], |r| {
            Ok((r.get::<_, YearMonth>(0)?, r.get::<_, Money>(1)?))
        })?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    Ok(rows)
}

/// Every live adjustment in `range`, for **every** category, summed per month.
///
/// One query for the whole budget screen. The per-category alternative issues a
/// query per row, which turns a twenty-category budget into twenty round trips
/// for data that is one `GROUP BY` away.
pub fn all_adjustments(
    db: &Db,
    range: MonthRange,
) -> Result<BTreeMap<CategoryId, BTreeMap<YearMonth, Money>>> {
    if range.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut stmt = db.conn().prepare(
        "SELECT category_id, year_month, SUM(amount_cents)
           FROM budget_adjustment
          WHERE archived_at IS NULL AND year_month >= ?1 AND year_month <= ?2
          GROUP BY category_id, year_month",
    )?;
    let rows = stmt
        .query_map(params![range.start, range.end], |r| {
            Ok((
                r.get::<_, CategoryId>(0)?,
                r.get::<_, YearMonth>(1)?,
                r.get::<_, Money>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out: BTreeMap<CategoryId, BTreeMap<YearMonth, Money>> = BTreeMap::new();
    for (category_id, ym, amount) in rows {
        out.entry(category_id).or_default().insert(ym, amount);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::seed_defaults;

    fn seeded() -> Db {
        let db = Db::open_in_memory().expect("in-memory db");
        seed_defaults(&db).expect("seed");
        db
    }

    fn ym(y: i32, m: u32) -> YearMonth {
        YearMonth::new(y, m).expect("valid month")
    }

    fn category(db: &Db, name: &str) -> CategoryId {
        categories::resolve(db, name).expect("category").id
    }

    fn record(category_id: CategoryId, start: YearMonth) -> BudgetLineRecord {
        BudgetLineRecord {
            id: None,
            category_id,
            period: Period::Monthly,
            period_start: start,
            period_end: None,
            amount: Money::from_dollars(400),
            rollover: false,
            notes: None,
        }
    }

    #[test]
    fn a_saved_line_loads_back_as_the_domain_type() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let mut rec = record(groceries, ym(2026, 1));
        rec.period = Period::EveryNMonths(3);
        rec.rollover = true;
        rec.notes = Some("  quarterly  ".to_string());

        let id = upsert(&db, &rec).unwrap();
        let stored = get(&db, id).unwrap();
        assert_eq!(stored.line.id, id);
        assert_eq!(stored.line.period, Period::EveryNMonths(3));
        assert_eq!(stored.line.period_start, ym(2026, 1));
        assert_eq!(stored.line.period_end, None);
        assert_eq!(stored.line.amount, Money::from_dollars(400));
        assert!(stored.line.rollover);
        assert_eq!(stored.category_name, "Groceries");
        assert_eq!(stored.group_name, "Frequent");
        assert_eq!(stored.notes.as_deref(), Some("quarterly"));
        assert_eq!(stored.qualified_name(), "Frequent/Groceries");
    }

    #[test]
    fn every_period_shape_round_trips() {
        let db = seeded();
        let shapes = [
            ("Groceries", Period::Monthly),
            ("Car Insurance", Period::Yearly),
            ("Car Repair", Period::EveryNMonths(6)),
            (
                "Travel",
                Period::ByDate {
                    due: NaiveDate::from_ymd_opt(2026, 7, 4).unwrap(),
                },
            ),
        ];
        for (name, period) in shapes {
            let mut rec = record(category(&db, name), ym(2026, 1));
            rec.period = period;
            let id = upsert(&db, &rec).unwrap();
            assert_eq!(get(&db, id).unwrap().line.period, period, "{name}");
        }
    }

    #[test]
    fn an_overlapping_line_is_refused_and_names_the_conflict() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let mut first = record(groceries, ym(2026, 1));
        first.period_end = Some(ym(2026, 6));
        upsert(&db, &first).unwrap();

        // April–June would be covered twice, and every figure for those months
        // would double. The unique index cannot see this: the start months
        // differ.
        let overlapping = record(groceries, ym(2026, 4));
        let err = upsert(&db, &overlapping).unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, CoreError::Invalid(_)), "{msg}");
        assert!(msg.contains("2026-01..2026-06"), "{msg}");
        assert!(msg.contains("2026-04 onward"), "{msg}");
        assert_eq!(list(&db, None).unwrap().len(), 1, "nothing was written");

        // Abutting is not overlapping: July is free.
        let after = record(groceries, ym(2026, 7));
        assert!(upsert(&db, &after).is_ok());
        // …and a different category is never a conflict.
        assert!(upsert(&db, &record(category(&db, "Gas"), ym(2026, 4))).is_ok());
    }

    #[test]
    fn updating_a_line_does_not_conflict_with_itself() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let id = upsert(&db, &record(groceries, ym(2026, 1))).unwrap();

        let mut edited = record(groceries, ym(2026, 1));
        edited.id = Some(id);
        edited.amount = Money::from_dollars(500);
        assert_eq!(upsert(&db, &edited).unwrap(), id);
        assert_eq!(get(&db, id).unwrap().line.amount, Money::from_dollars(500));
        assert_eq!(list(&db, None).unwrap().len(), 1);
    }

    #[test]
    fn supersede_closes_the_old_line_and_opens_a_new_one() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let first = upsert(&db, &record(groceries, ym(2026, 1))).unwrap();

        let mut raised = record(groceries, ym(2026, 7));
        raised.amount = Money::from_dollars(500);
        let outcome = supersede(&db, &raised).unwrap();
        assert_eq!(outcome.closed, Some((first, ym(2026, 6))));
        assert!(!outcome.restated);
        assert_ne!(outcome.id, first, "history is kept, not rewritten");

        assert_eq!(get(&db, first).unwrap().line.period_end, Some(ym(2026, 6)));
        // March still reads the old budget; August reads the new one.
        assert_eq!(
            for_category(&db, groceries, ym(2026, 3))
                .unwrap()
                .map(|l| l.line.amount),
            Some(Money::from_dollars(400))
        );
        assert_eq!(
            for_category(&db, groceries, ym(2026, 8))
                .unwrap()
                .map(|l| l.line.amount),
            Some(Money::from_dollars(500))
        );

        // Setting the same month again restates that line rather than stacking
        // a third one on top.
        let mut corrected = record(groceries, ym(2026, 7));
        corrected.amount = Money::from_dollars(550);
        let again = supersede(&db, &corrected).unwrap();
        assert!(again.restated);
        assert_eq!(again.id, outcome.id);
        assert_eq!(again.closed, None);
        assert_eq!(list(&db, None).unwrap().len(), 2);
    }

    #[test]
    fn a_refused_supersede_leaves_the_old_budget_alone() {
        // The failure this exists to prevent: closing the old line, then having
        // the new one refused, and leaving the category with no budget at all.
        // Here the requested span collides with a *third* line further out.
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let first = upsert(&db, &record(groceries, ym(2026, 1))).unwrap();
        let mut later = record(groceries, ym(2026, 9));
        later.amount = Money::from_dollars(600);
        supersede(&db, &later).unwrap();

        let mut clashing = record(groceries, ym(2026, 4));
        clashing.period_end = Some(ym(2026, 10));
        let err = supersede(&db, &clashing).unwrap_err();
        assert!(matches!(err, CoreError::Invalid(_)), "{err}");

        // Everything is exactly as it was: the first line still runs to August.
        assert_eq!(get(&db, first).unwrap().line.period_end, Some(ym(2026, 8)));
        assert_eq!(list(&db, None).unwrap().len(), 2);
        assert!(for_category(&db, groceries, ym(2026, 4)).unwrap().is_some());
    }

    #[test]
    fn an_archived_line_never_blocks_its_replacement() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let id = upsert(&db, &record(groceries, ym(2026, 1))).unwrap();
        archive(&db, id).unwrap();

        assert!(upsert(&db, &record(groceries, ym(2026, 1))).is_ok());
        assert_eq!(list(&db, None).unwrap().len(), 1);
        // Archiving twice keeps the first time.
        let first: String = db
            .conn()
            .query_row(
                "SELECT archived_at FROM budget_line WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        archive(&db, id).unwrap();
        let again: String = db
            .conn()
            .query_row(
                "SELECT archived_at FROM budget_line WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first, again);
        assert!(get(&db, id).is_ok(), "an archived line still loads");
    }

    #[test]
    fn impossible_periods_are_refused_in_words() {
        let db = seeded();
        let groceries = category(&db, "Groceries");

        for (n, needle) in [(0_u32, "0-month"), (1, "monthly budget"), (121, "10 years")] {
            let mut rec = record(groceries, ym(2026, 1));
            rec.period = Period::EveryNMonths(n);
            let err = upsert(&db, &rec).unwrap_err();
            assert!(err.to_string().contains(needle), "{n}: {err}");
        }

        let mut backwards = record(groceries, ym(2026, 6));
        backwards.period_end = Some(ym(2026, 1));
        assert!(
            upsert(&db, &backwards)
                .unwrap_err()
                .to_string()
                .contains("ends before it starts")
        );

        let mut past_due = record(groceries, ym(2026, 6));
        past_due.period = Period::ByDate {
            due: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        };
        assert!(
            upsert(&db, &past_due)
                .unwrap_err()
                .to_string()
                .contains("after it")
        );

        assert!(list(&db, None).unwrap().is_empty());
    }

    #[test]
    fn list_filters_to_the_month_and_orders_by_group() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let insurance = category(&db, "Car Insurance");

        let mut ended = record(groceries, ym(2026, 1));
        ended.period_end = Some(ym(2026, 3));
        upsert(&db, &ended).unwrap();
        upsert(&db, &record(insurance, ym(2026, 1))).unwrap();

        // Bills sorts before Frequent.
        let all = list(&db, None).unwrap();
        assert_eq!(
            all.iter()
                .map(|l| l.category_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Car Insurance", "Groceries"]
        );

        let april = list(&db, Some(ym(2026, 4))).unwrap();
        assert_eq!(april.len(), 1);
        assert_eq!(april[0].category_name, "Car Insurance");
        assert!(list(&db, Some(ym(2025, 12))).unwrap().is_empty());

        assert_eq!(
            for_category(&db, groceries, ym(2026, 2))
                .unwrap()
                .map(|l| l.line.id),
            Some(all[1].line.id)
        );
        assert_eq!(for_category(&db, groceries, ym(2026, 4)).unwrap(), None);
    }

    #[test]
    fn adjustments_sum_per_month_and_stay_in_range() {
        let db = seeded();
        let groceries = category(&db, "Groceries");
        let fuel = category(&db, "Gas");

        add_adjustment(&db, groceries, ym(2026, 3), Money::from_dollars(50), None).unwrap();
        add_adjustment(
            &db,
            groceries,
            ym(2026, 3),
            Money::from_dollars(-20),
            Some("  moved to fuel  "),
        )
        .unwrap();
        add_adjustment(&db, groceries, ym(2026, 9), Money::from_dollars(5), None).unwrap();
        add_adjustment(&db, fuel, ym(2026, 3), Money::from_dollars(20), None).unwrap();

        let window = MonthRange::new(ym(2026, 1), ym(2026, 6));
        let got = adjustments_for(&db, groceries, window).unwrap();
        assert_eq!(got.len(), 1, "September is outside the window");
        assert_eq!(got[&ym(2026, 3)], Money::from_dollars(30));

        let all = all_adjustments(&db, window).unwrap();
        assert_eq!(all[&groceries][&ym(2026, 3)], Money::from_dollars(30));
        assert_eq!(all[&fuel][&ym(2026, 3)], Money::from_dollars(20));

        // An empty range is empty, not an error.
        let backwards = MonthRange::new(ym(2026, 6), ym(2026, 1));
        assert!(
            adjustments_for(&db, groceries, backwards)
                .unwrap()
                .is_empty()
        );
        assert!(all_adjustments(&db, backwards).unwrap().is_empty());

        let reason: String = db
            .conn()
            .query_row(
                "SELECT reason FROM budget_adjustment WHERE reason IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reason, "moved to fuel");
    }

    #[test]
    fn missing_rows_are_not_found_rather_than_silence() {
        let db = seeded();
        assert!(matches!(
            get(&db, BudgetLineId::new(999)),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            archive(&db, BudgetLineId::new(999)),
            Err(CoreError::NotFound(_))
        ));
        assert!(matches!(
            add_adjustment(
                &db,
                CategoryId::new(999),
                ym(2026, 1),
                Money::from_dollars(1),
                None
            ),
            Err(CoreError::NotFound(_))
        ));

        let mut ghost = record(category(&db, "Groceries"), ym(2026, 1));
        ghost.id = Some(BudgetLineId::new(999));
        assert!(matches!(upsert(&db, &ghost), Err(CoreError::NotFound(_))));
    }
}
