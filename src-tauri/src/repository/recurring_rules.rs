//! Recurring expense rules CRUD + pending-confirmation helpers.

use anyhow::Result;
use rusqlite::{params, Connection, Row};
use time::OffsetDateTime;

use crate::domain::recurring::{Frequency, NewRecurringRule, RecurringMode, RecurringRule};

const SELECT_COLS: &str = "id, label, amount_cents, currency, category_id, \
     frequency, anchor_day, mode, enabled, created_at";

fn map_row(row: &Row<'_>) -> rusqlite::Result<RecurringRule> {
    Ok(RecurringRule {
        id: row.get(0)?,
        label: row.get(1)?,
        amount_cents: row.get(2)?,
        currency: row.get(3)?,
        category_id: row.get(4)?,
        frequency: row.get::<_, Frequency>(5)?,
        anchor_day: row.get::<_, i64>(6)? as u16,
        mode: row.get::<_, RecurringMode>(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
    })
}

pub fn insert(conn: &Connection, rule: &NewRecurringRule) -> Result<i64> {
    conn.execute(
        "INSERT INTO recurring_rules
            (label, amount_cents, currency, category_id, frequency,
             anchor_day, mode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            rule.label,
            rule.amount_cents,
            rule.currency,
            rule.category_id,
            rule.frequency,
            rule.anchor_day as i64,
            rule.mode,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<RecurringRule>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SELECT_COLS} FROM recurring_rules WHERE id = ?1"
    ))?;
    let row = stmt
        .query_row(params![id], map_row)
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(row)
}

pub fn list(conn: &Connection, include_disabled: bool) -> Result<Vec<RecurringRule>> {
    let sql = if include_disabled {
        format!("SELECT {SELECT_COLS} FROM recurring_rules ORDER BY label")
    } else {
        format!("SELECT {SELECT_COLS} FROM recurring_rules WHERE enabled = 1 ORDER BY label")
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    // ON DELETE CASCADE on scheduled_jobs takes care of the queue row(s).
    let n = conn.execute("DELETE FROM recurring_rules WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

pub fn set_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<bool> {
    let n = conn.execute(
        "UPDATE recurring_rules SET enabled = ?2 WHERE id = ?1",
        params![id, if enabled { 1i64 } else { 0i64 }],
    )?;
    Ok(n > 0)
}

// ---------------------------------------------------------------------
// pending_recurring_confirmations
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PendingConfirmation {
    pub chat_id: i64,
    pub rule_id: i64,
    pub asked_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

pub fn insert_pending(
    conn: &Connection,
    chat_id: i64,
    rule_id: i64,
    asked_at: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> Result<()> {
    // INSERT OR REPLACE: if a stale pending row exists for this chat,
    // overwrite it (this only happens if a previous confirmation timed
    // out without being cleaned up).
    conn.execute(
        "INSERT OR REPLACE INTO pending_recurring_confirmations
            (chat_id, rule_id, asked_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![chat_id, rule_id, asked_at, expires_at],
    )?;
    Ok(())
}

pub fn get_pending(conn: &Connection, chat_id: i64) -> Result<Option<PendingConfirmation>> {
    let mut stmt = conn.prepare_cached(
        "SELECT chat_id, rule_id, asked_at, expires_at
         FROM pending_recurring_confirmations
         WHERE chat_id = ?1",
    )?;
    let row = stmt
        .query_row(params![chat_id], |r| {
            Ok(PendingConfirmation {
                chat_id: r.get(0)?,
                rule_id: r.get(1)?,
                asked_at: r.get(2)?,
                expires_at: r.get(3)?,
            })
        })
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(row)
}

pub fn delete_pending(conn: &Connection, chat_id: i64) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM pending_recurring_confirmations WHERE chat_id = ?1",
        params![chat_id],
    )?;
    Ok(n > 0)
}

/// True iff some chat already has an outstanding pending confirmation.
/// Scheduler uses this to defer firing a second confirm-mode rule for
/// the same chat.
pub fn chat_has_pending(conn: &Connection, chat_id: i64) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_recurring_confirmations WHERE chat_id = ?1",
        params![chat_id],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}
