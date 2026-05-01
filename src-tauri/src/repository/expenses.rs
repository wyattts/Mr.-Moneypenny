//! Expense CRUD.

use anyhow::Result;
use rusqlite::{params, Connection, Row};
use time::OffsetDateTime;

use crate::domain::{CategoryKind, Expense, ExpenseSource, NewExpense};

const SELECT_COLS: &str = "id, amount_cents, currency, category_id, description, \
     occurred_at, created_at, source, raw_message, llm_confidence, logged_by_chat_id, \
     is_refund, refund_for_expense_id";

/// Signed contribution of an expense row to aggregate totals: positive for
/// regular expenses, negative for refunds. Use this expression in any SUM()
/// over `expenses`.
pub const SIGNED_AMOUNT_SQL: &str =
    "CASE WHEN is_refund = 1 THEN -amount_cents ELSE amount_cents END";

fn map_row(row: &Row<'_>) -> rusqlite::Result<Expense> {
    Ok(Expense {
        id: row.get(0)?,
        amount_cents: row.get(1)?,
        currency: row.get(2)?,
        category_id: row.get(3)?,
        description: row.get(4)?,
        occurred_at: row.get(5)?,
        created_at: row.get(6)?,
        source: row.get(7)?,
        raw_message: row.get(8)?,
        llm_confidence: row.get(9)?,
        logged_by_chat_id: row.get(10)?,
        is_refund: row.get::<_, i64>(11)? != 0,
        refund_for_expense_id: row.get(12)?,
    })
}

pub fn insert(conn: &Connection, e: &NewExpense) -> Result<i64> {
    conn.execute(
        "INSERT INTO expenses
            (amount_cents, currency, category_id, description, occurred_at,
             source, raw_message, llm_confidence, logged_by_chat_id,
             is_refund, refund_for_expense_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            e.amount_cents,
            e.currency,
            e.category_id,
            e.description,
            e.occurred_at,
            e.source,
            e.raw_message,
            e.llm_confidence,
            e.logged_by_chat_id,
            if e.is_refund { 1i64 } else { 0i64 },
            e.refund_for_expense_id,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Expense>> {
    let mut stmt =
        conn.prepare_cached(&format!("SELECT {SELECT_COLS} FROM expenses WHERE id = ?1"))?;
    let row = stmt
        .query_row(params![id], map_row)
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(row)
}

pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute("DELETE FROM expenses WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

pub fn list_in_range(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<Vec<Expense>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM expenses
         WHERE occurred_at >= ?1 AND occurred_at < ?2
         ORDER BY occurred_at DESC, id DESC"
    ))?;
    let rows = stmt
        .query_map(params![start, end], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<Expense>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM expenses
         ORDER BY created_at DESC, id DESC
         LIMIT ?1"
    ))?;
    let rows = stmt
        .query_map(params![limit], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Net spend (signed: refunds subtract) in [start, end) whose category has
/// the given kind. Expenses without a category or with a deleted category
/// contribute 0 to either kind — pacing logic only counts categorized spend.
pub fn sum_in_range_by_kind(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
    kind: CategoryKind,
) -> Result<i64> {
    let sql = format!(
        "SELECT COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0)
         FROM expenses e
         JOIN categories c ON c.id = e.category_id
         WHERE e.occurred_at >= ?1 AND e.occurred_at < ?2 AND c.kind = ?3"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let total: i64 = stmt.query_row(params![start, end, kind], |r| r.get(0))?;
    Ok(total)
}

/// Net spend (signed: refunds subtract) across all categorized + uncategorized
/// rows in [start, end).
pub fn sum_in_range(conn: &Connection, start: OffsetDateTime, end: OffsetDateTime) -> Result<i64> {
    let sql = format!(
        "SELECT COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0)
         FROM expenses
         WHERE occurred_at >= ?1 AND occurred_at < ?2"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let total: i64 = stmt.query_row(params![start, end], |r| r.get(0))?;
    Ok(total)
}

/// Net spend (signed: refunds subtract) per calendar month for a single
/// category, going `months_back` months back from `now`. Returned vector
/// is `months_back` long, oldest first; months with no spend appear as 0.
///
/// Used by the v0.3.0 forecast tools to derive descriptive stats and
/// historical contribution rates.
pub fn monthly_totals_for_category(
    conn: &Connection,
    category_id: i64,
    now: OffsetDateTime,
    months_back: u32,
) -> Result<Vec<i64>> {
    use time::{Date, Duration, Month, Time};
    let offset = now.offset();
    // The first day of the *current* month, in the user's offset.
    let mut anchor =
        Date::from_calendar_date(now.year(), now.month(), 1).expect("day 1 always valid");
    // Step back (months_back - 1) months so the loop covers exactly
    // `months_back` calendar months ending at the current one.
    for _ in 0..months_back.saturating_sub(1) {
        anchor = previous_month_start(anchor);
    }

    let mut totals = Vec::with_capacity(months_back as usize);
    for _ in 0..months_back {
        let next = if anchor.month() == Month::December {
            Date::from_calendar_date(anchor.year() + 1, Month::January, 1)
        } else {
            Date::from_calendar_date(anchor.year(), anchor.month().next(), 1)
        }
        .expect("first of next month is valid");
        let start = anchor.with_time(Time::MIDNIGHT).assume_offset(offset);
        let end = next.with_time(Time::MIDNIGHT).assume_offset(offset);
        let sql = format!(
            "SELECT COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0)
             FROM expenses
             WHERE category_id = ?1
               AND occurred_at >= ?2 AND occurred_at < ?3"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let total: i64 = stmt.query_row(params![category_id, start, end], |r| r.get(0))?;
        totals.push(total);
        anchor = next;
        let _ = Duration::days(0); // suppress unused-import lint for Duration
    }
    Ok(totals)
}

fn previous_month_start(d: time::Date) -> time::Date {
    use time::{Date, Month};
    let (year, month) = if d.month() == Month::January {
        (d.year() - 1, Month::December)
    } else {
        (d.year(), d.month().previous())
    };
    Date::from_calendar_date(year, month, 1).expect("day 1 always valid")
}

/// Source filter helper for backfills / debugging.
pub fn list_by_source(conn: &Connection, source: ExpenseSource) -> Result<Vec<Expense>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM expenses WHERE source = ?1 ORDER BY occurred_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![source], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
