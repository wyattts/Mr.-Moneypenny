//! Budget CRUD.

use anyhow::Result;
use rusqlite::{params, Connection, Row};
use time::OffsetDateTime;

use crate::domain::{Budget, NewBudget};

const SELECT_COLS: &str = "id, category_id, amount_cents, period, effective_from, effective_to";

fn map_row(row: &Row<'_>) -> rusqlite::Result<Budget> {
    Ok(Budget {
        id: row.get(0)?,
        category_id: row.get(1)?,
        amount_cents: row.get(2)?,
        period: row.get(3)?,
        effective_from: row.get(4)?,
        effective_to: row.get(5)?,
    })
}

pub fn insert(conn: &Connection, b: &NewBudget) -> Result<i64> {
    conn.execute(
        "INSERT INTO budgets (category_id, amount_cents, period, effective_from, effective_to)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            b.category_id,
            b.amount_cents,
            b.period,
            b.effective_from,
            b.effective_to,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_for_category(conn: &Connection, category_id: i64) -> Result<Vec<Budget>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM budgets
         WHERE category_id = ?1 ORDER BY effective_from DESC"
    ))?;
    let rows = stmt
        .query_map(params![category_id], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Budget that's effective at the given moment, or `None`.
pub fn effective_at(
    conn: &Connection,
    category_id: i64,
    at: OffsetDateTime,
) -> Result<Option<Budget>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM budgets
         WHERE category_id = ?1
           AND effective_from <= ?2
           AND (effective_to IS NULL OR effective_to > ?2)
         ORDER BY effective_from DESC LIMIT 1"
    ))?;
    match stmt.query_row(params![category_id, at], map_row) {
        Ok(b) => Ok(Some(b)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    let n = conn.execute("DELETE FROM budgets WHERE id = ?1", params![id])?;
    Ok(n > 0)
}
