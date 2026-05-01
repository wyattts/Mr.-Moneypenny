//! CRUD for `csv_import_profiles`.
//!
//! A profile bundles the column-mapping recipe for one bank's CSV
//! export format. Saved once per bank; reused on every subsequent
//! import. Auto-detection happens via `header_signature` lookup.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Column-mapping payload. Persisted as JSON in `mapping_json` because
/// the shape is opaque to SQLite and adding new optional fields later
/// shouldn't require a migration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColumnMapping {
    pub date_col: usize,
    pub amount_col: usize,
    pub merchant_col: usize,
    pub description_col: Option<usize>,
    pub category_col: Option<usize>,
    /// e.g., "MM/DD/YYYY", "YYYY-MM-DD", "DD/MM/YYYY"
    pub date_format: String,
    pub neg_means_refund: bool,
    pub skip_rows: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CsvImportProfile {
    pub id: i64,
    pub name: String,
    pub header_signature: Option<String>,
    pub mapping: ColumnMapping,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

/// Insert a new profile. Returns the assigned id.
pub fn create(
    conn: &Connection,
    name: &str,
    header_signature: Option<&str>,
    mapping: &ColumnMapping,
    now: OffsetDateTime,
) -> Result<i64> {
    let mapping_json = serde_json::to_string(mapping)?;
    conn.execute(
        "INSERT INTO csv_import_profiles
            (name, header_signature, mapping_json, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![name, header_signature, mapping_json, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Fetch all profiles, newest-first.
pub fn list(conn: &Connection) -> Result<Vec<CsvImportProfile>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, header_signature, mapping_json, created_at, last_used_at
         FROM csv_import_profiles
         ORDER BY COALESCE(last_used_at, created_at) DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_profile)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Find a profile by header signature (used for auto-detection at
/// import time). Most recently used wins on ties.
pub fn find_by_signature(
    conn: &Connection,
    signature: &str,
) -> Result<Option<CsvImportProfile>> {
    let row = conn
        .query_row(
            "SELECT id, name, header_signature, mapping_json, created_at, last_used_at
             FROM csv_import_profiles
             WHERE header_signature = ?1
             ORDER BY COALESCE(last_used_at, created_at) DESC
             LIMIT 1",
            params![signature],
            row_to_profile,
        )
        .optional()?;
    Ok(row)
}

/// Mark a profile as just-used (bumps `last_used_at`).
pub fn touch(conn: &Connection, id: i64, now: OffsetDateTime) -> Result<()> {
    conn.execute(
        "UPDATE csv_import_profiles SET last_used_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM csv_import_profiles WHERE id = ?1", params![id])?;
    Ok(())
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<CsvImportProfile> {
    let mapping_json: String = row.get(3)?;
    let mapping: ColumnMapping = serde_json::from_str(&mapping_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })?;
    Ok(CsvImportProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        header_signature: row.get(2)?,
        mapping,
        created_at: row.get(4)?,
        last_used_at: row.get(5)?,
    })
}

/// Compute a stable hash of a CSV header row for auto-detection. The
/// row is normalized (lowercased, trimmed, joined with `|`) before
/// hashing so trivial whitespace differences don't defeat the lookup.
///
/// Uses SHA-256 truncated to 16 hex chars — collision probability is
/// negligible for the handful of profiles a single user will ever have.
pub fn header_signature(headers: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let normalized = headers
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect::<Vec<_>>()
        .join("|");
    let digest = Sha256::digest(normalized.as_bytes());
    hex_short(&digest)
}

fn hex_short(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(16);
    for b in bytes.iter().take(8) {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

const HEX: &[u8; 16] = b"0123456789abcdef";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn fresh_conn() -> Connection {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn sample_mapping() -> ColumnMapping {
        ColumnMapping {
            date_col: 1,
            amount_col: 3,
            merchant_col: 2,
            description_col: None,
            category_col: None,
            date_format: "MM/DD/YYYY".into(),
            neg_means_refund: true,
            skip_rows: 1,
        }
    }

    #[test]
    fn create_then_list_round_trips() {
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        let id = create(&conn, "Chase Checking", Some("abc123"), &sample_mapping(), now).unwrap();
        let list = list(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].name, "Chase Checking");
        assert_eq!(list[0].mapping, sample_mapping());
    }

    #[test]
    fn find_by_signature_hits_then_misses() {
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        create(&conn, "Chase", Some("sig-A"), &sample_mapping(), now).unwrap();
        create(&conn, "Amex", Some("sig-B"), &sample_mapping(), now).unwrap();
        assert!(find_by_signature(&conn, "sig-A").unwrap().is_some());
        assert!(find_by_signature(&conn, "sig-B").unwrap().is_some());
        assert!(find_by_signature(&conn, "sig-C").unwrap().is_none());
    }

    #[test]
    fn touch_updates_last_used_at() {
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        let id = create(&conn, "Chase", Some("sig"), &sample_mapping(), now).unwrap();
        let later = now + time::Duration::hours(1);
        touch(&conn, id, later).unwrap();
        let p = list(&conn).unwrap().into_iter().next().unwrap();
        assert!(p.last_used_at.unwrap() > p.created_at);
    }

    #[test]
    fn header_signature_normalizes_whitespace_and_case() {
        let a = header_signature(&[
            "Posting Date".into(),
            " Description ".into(),
            "Amount".into(),
        ]);
        let b = header_signature(&["posting date".into(), "DESCRIPTION".into(), "amount".into()]);
        assert_eq!(a, b);
    }

    #[test]
    fn header_signature_changes_on_column_order() {
        let a = header_signature(&["Date".into(), "Amount".into()]);
        let b = header_signature(&["Amount".into(), "Date".into()]);
        assert_ne!(a, b);
    }

    #[test]
    fn delete_removes_row() {
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        let id = create(&conn, "Chase", Some("sig"), &sample_mapping(), now).unwrap();
        delete(&conn, id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }
}
