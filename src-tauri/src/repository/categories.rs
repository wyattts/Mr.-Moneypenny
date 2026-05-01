//! Category CRUD.

use anyhow::Result;
use rusqlite::{params, Connection, Row};

use crate::domain::{Category, NewCategory};

const SELECT_COLS: &str = "id, name, kind, monthly_target_cents, is_recurring, \
     recurrence_day_of_month, is_active, is_seed";

fn map_row(row: &Row<'_>) -> rusqlite::Result<Category> {
    Ok(Category {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        monthly_target_cents: row.get(3)?,
        is_recurring: row.get::<_, i64>(4)? != 0,
        recurrence_day_of_month: row.get::<_, Option<i64>>(5)?.map(|d| d as u8),
        is_active: row.get::<_, i64>(6)? != 0,
        is_seed: row.get::<_, i64>(7)? != 0,
    })
}

pub fn insert(conn: &Connection, c: &NewCategory) -> Result<i64> {
    conn.execute(
        "INSERT INTO categories
            (name, kind, monthly_target_cents, is_recurring, recurrence_day_of_month, is_seed)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![
            c.name,
            c.kind,
            c.monthly_target_cents,
            i64::from(c.is_recurring),
            c.recurrence_day_of_month.map(i64::from),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Category>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM categories WHERE id = ?1"
    ))?;
    match stmt.query_row(params![id], map_row) {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn get_by_name(conn: &Connection, name: &str) -> Result<Option<Category>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM categories WHERE name = ?1"
    ))?;
    match stmt.query_row(params![name], map_row) {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list(conn: &Connection, include_inactive: bool) -> Result<Vec<Category>> {
    let sql = if include_inactive {
        format!("SELECT {SELECT_COLS} FROM categories ORDER BY kind DESC, name ASC")
    } else {
        format!(
            "SELECT {SELECT_COLS} FROM categories
             WHERE is_active = 1 ORDER BY kind DESC, name ASC"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn set_active(conn: &Connection, id: i64, is_active: bool) -> Result<bool> {
    let n = conn.execute(
        "UPDATE categories SET is_active = ?1 WHERE id = ?2",
        params![i64::from(is_active), id],
    )?;
    Ok(n > 0)
}

pub fn set_monthly_target(
    conn: &Connection,
    id: i64,
    monthly_target_cents: Option<i64>,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE categories SET monthly_target_cents = ?1 WHERE id = ?2",
        params![monthly_target_cents, id],
    )?;
    Ok(n > 0)
}

/// Set the user-entered current balance for an investing-kind category.
/// `as_of` is a free-form ISO date string (YYYY-MM-DD); we accept None
/// for "now" but most callers will fill it in.
pub fn set_starting_balance(
    conn: &Connection,
    id: i64,
    starting_balance_cents: Option<i64>,
    balance_as_of: Option<&str>,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE categories
         SET starting_balance_cents = ?1, balance_as_of = ?2
         WHERE id = ?3",
        params![starting_balance_cents, balance_as_of, id],
    )?;
    Ok(n > 0)
}

#[derive(Debug, Clone)]
pub struct StartingBalance {
    pub starting_balance_cents: Option<i64>,
    pub balance_as_of: Option<String>,
}

pub fn get_starting_balance(conn: &Connection, id: i64) -> Result<StartingBalance> {
    let row: (Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT starting_balance_cents, balance_as_of FROM categories WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((None, None));
    Ok(StartingBalance {
        starting_balance_cents: row.0,
        balance_as_of: row.1,
    })
}

/// Hard delete. Disallows seed-category deletion; callers should
/// `set_active(false)` for those instead.
pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    let is_seed: i64 = conn
        .query_row(
            "SELECT is_seed FROM categories WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    anyhow::ensure!(
        is_seed == 0,
        "seed categories cannot be deleted; deactivate them with set_active(false)"
    );
    let n = conn.execute("DELETE FROM categories WHERE id = ?1", params![id])?;
    Ok(n > 0)
}
