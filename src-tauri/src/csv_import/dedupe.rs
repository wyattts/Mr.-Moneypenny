//! Probable-duplicate detection.
//!
//! Two passes:
//!
//! 1. **Within-CSV.** Two rows in the same import are duplicates if
//!    they share `(date_day, amount_cents, merchant_lower)`. This
//!    catches merged exports / accidental double-saves of the same
//!    file.
//! 2. **Against the existing DB.** A parsed row is "probable duplicate"
//!    of an existing expense when:
//!      - dates within ±2 days
//!      - `amount_cents` exactly equal
//!      - merchant string Levenshtein-distance < 3 from the existing
//!        expense's `description`
//!
//! All decisions are surfaced to the user via the review screen — the
//! importer never silently drops rows.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use time::Duration;

use super::parser::ParsedRow;

/// One probable-duplicate finding.
#[derive(Debug, Clone, Serialize)]
pub struct DuplicateMatch {
    /// Index into the parsed-row vec the importer is committing.
    pub row_index: usize,
    /// "csv" if matched another row in the same CSV, or "db" if matched
    /// an existing expense.
    pub kind: &'static str,
    /// For db matches, the existing expense id. None for csv-only.
    pub existing_expense_id: Option<i64>,
    /// Short reason string for the review UI.
    pub reason: String,
}

/// Run both passes and collect findings. The vec returned is *not* a
/// list of rows to drop — the UI presents these to the user, who
/// decides per-row.
pub fn find_probable_duplicates(
    conn: &Connection,
    rows: &[ParsedRow],
) -> Result<Vec<DuplicateMatch>> {
    let mut out = Vec::new();
    out.extend(within_csv(rows));
    out.extend(against_db(conn, rows)?);
    Ok(out)
}

/// Within-import pass: bucket rows by their fingerprint. Any bucket
/// with ≥2 rows produces (n-1) findings on the later occurrences.
fn within_csv(rows: &[ParsedRow]) -> Vec<DuplicateMatch> {
    use std::collections::HashMap;
    let mut seen: HashMap<(time::Date, i64, String), usize> = HashMap::new();
    let mut out = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let key = (
            r.occurred_at.date(),
            r.amount_cents,
            r.merchant.to_lowercase(),
        );
        if let Some(&first) = seen.get(&key) {
            out.push(DuplicateMatch {
                row_index: i,
                kind: "csv",
                existing_expense_id: None,
                reason: format!("matches earlier CSV row {first}"),
            });
        } else {
            seen.insert(key, i);
        }
    }
    out
}

/// DB pass: for each parsed row, check the expenses table for any
/// expense within ±2 days at the exact same amount whose description
/// is Levenshtein < 3 from the merchant string.
fn against_db(conn: &Connection, rows: &[ParsedRow]) -> Result<Vec<DuplicateMatch>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare_cached(
        "SELECT id, occurred_at, amount_cents, description
         FROM expenses
         WHERE amount_cents = ?1
           AND occurred_at >= ?2
           AND occurred_at <= ?3",
    )?;
    for (i, r) in rows.iter().enumerate() {
        let lo = r.occurred_at - Duration::days(2);
        let hi = r.occurred_at + Duration::days(2);
        let candidates = stmt.query_map(
            params![r.amount_cents, lo, hi],
            |row| -> rusqlite::Result<(i64, String)> {
                let id: i64 = row.get(0)?;
                let desc: Option<String> = row.get(3)?;
                Ok((id, desc.unwrap_or_default()))
            },
        )?;
        for cand in candidates {
            let (id, desc) = cand?;
            if desc.is_empty() {
                continue;
            }
            let dist = strsim::levenshtein(&desc.to_lowercase(), &r.merchant.to_lowercase());
            if dist < 3 {
                out.push(DuplicateMatch {
                    row_index: i,
                    kind: "db",
                    existing_expense_id: Some(id),
                    reason: format!("matches expense #{id} ({desc})"),
                });
                break; // one finding per row is enough
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::domain::{ExpenseSource, NewExpense};
    use crate::repository::{categories, expenses};
    use time::OffsetDateTime;

    fn fresh_conn() -> Connection {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn parsed(date: &str, amount_cents: i64, merchant: &str) -> ParsedRow {
        ParsedRow {
            source_row_index: 0,
            occurred_at: super::super::parser::parse_date(date, "MM/DD/YYYY").unwrap(),
            amount_cents,
            merchant: merchant.into(),
            description: None,
            raw_category: None,
            is_refund: false,
        }
    }

    #[test]
    fn within_csv_catches_exact_repeats() {
        let rows = vec![
            parsed("01/15/2026", 1250, "STARBUCKS"),
            parsed("01/15/2026", 1250, "STARBUCKS"),
        ];
        let dups = within_csv(&rows);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].row_index, 1);
        assert_eq!(dups[0].kind, "csv");
    }

    #[test]
    fn within_csv_ignores_close_but_not_exact() {
        let rows = vec![
            parsed("01/15/2026", 1250, "STARBUCKS"),
            parsed("01/16/2026", 1250, "STARBUCKS"),
            parsed("01/15/2026", 1251, "STARBUCKS"),
        ];
        let dups = within_csv(&rows);
        assert!(dups.is_empty());
    }

    fn first_active_category(conn: &Connection) -> i64 {
        categories::list(conn, false)
            .unwrap()
            .into_iter()
            .find(|c| c.is_active)
            .unwrap()
            .id
    }

    #[test]
    fn against_db_flags_levenshtein_match_within_window() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let when = super::super::parser::parse_date("01/15/2026", "MM/DD/YYYY").unwrap();
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 1250,
                currency: "USD".into(),
                category_id: Some(cat),
                description: Some("Starbucks".into()),
                occurred_at: when,
                source: ExpenseSource::Telegram,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: false,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
        let rows = vec![parsed("01/16/2026", 1250, "Starbuks")]; // 1 edit away
        let dups = against_db(&conn, &rows).unwrap();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].kind, "db");
    }

    #[test]
    fn against_db_does_not_flag_far_dates() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let when = super::super::parser::parse_date("01/15/2026", "MM/DD/YYYY").unwrap();
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 1250,
                currency: "USD".into(),
                category_id: Some(cat),
                description: Some("Starbucks".into()),
                occurred_at: when,
                source: ExpenseSource::Telegram,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: false,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
        let rows = vec![parsed("01/20/2026", 1250, "Starbucks")]; // 5d off
        let dups = against_db(&conn, &rows).unwrap();
        assert!(dups.is_empty());
    }

    #[test]
    fn against_db_does_not_flag_different_amounts() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let when = super::super::parser::parse_date("01/15/2026", "MM/DD/YYYY").unwrap();
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 1250,
                currency: "USD".into(),
                category_id: Some(cat),
                description: Some("Starbucks".into()),
                occurred_at: when,
                source: ExpenseSource::Telegram,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: false,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
        let rows = vec![parsed("01/15/2026", 1300, "Starbucks")];
        let dups = against_db(&conn, &rows).unwrap();
        assert!(dups.is_empty());
    }

    #[test]
    fn against_db_rejects_far_string_distance() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let when = super::super::parser::parse_date("01/15/2026", "MM/DD/YYYY").unwrap();
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 1250,
                currency: "USD".into(),
                category_id: Some(cat),
                description: Some("Starbucks".into()),
                occurred_at: when,
                source: ExpenseSource::Telegram,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: false,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
        let rows = vec![parsed("01/15/2026", 1250, "Whole Foods")];
        let dups = against_db(&conn, &rows).unwrap();
        assert!(dups.is_empty());
    }
}
