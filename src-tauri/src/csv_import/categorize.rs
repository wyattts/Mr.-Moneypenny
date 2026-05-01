//! Categorization pipeline.
//!
//! Three layers, applied in order to each parsed row:
//!
//! 1. Saved `merchant_rules` table — pattern→category match.
//! 2. Fuzzy match against existing `expenses.description` history (if
//!    a past expense has Levenshtein-distance < 3 from the merchant
//!    string AND a non-null `category_id`, propose its category).
//! 3. Otherwise: leave for the manual review screen. The UI then
//!    optionally calls the AI-suggest helper for batched LLM
//!    categorization.

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use super::parser::ParsedRow;
use crate::repository::merchant_rules::{self, MerchantRule};

/// Per-row categorization decision after layers 1 & 2.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub row_index: usize,
    /// `rule` (from merchant_rules), `history` (fuzzy match), or
    /// `unmatched` (user must categorize).
    pub source: &'static str,
    pub category_id: Option<i64>,
    /// Override the row's `is_refund` (only set by layer 1 when the
    /// matched rule's `default_is_refund` is true).
    pub override_is_refund: Option<bool>,
}

pub fn categorize_all(conn: &Connection, rows: &[ParsedRow]) -> Result<Vec<Decision>> {
    let rules = merchant_rules::list(conn)?;
    let mut decisions = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        // Layer 1: merchant_rules.
        if let Some(rule) = merchant_rules::find_match(&rules, &r.merchant) {
            decisions.push(Decision {
                row_index: i,
                source: "rule",
                category_id: Some(rule.category_id),
                override_is_refund: if rule.default_is_refund {
                    Some(true)
                } else {
                    None
                },
            });
            continue;
        }
        // Layer 2: history fuzzy match.
        if let Some(cat) = match_history(conn, &r.merchant)? {
            decisions.push(Decision {
                row_index: i,
                source: "history",
                category_id: Some(cat),
                override_is_refund: None,
            });
            continue;
        }
        decisions.push(Decision {
            row_index: i,
            source: "unmatched",
            category_id: None,
            override_is_refund: None,
        });
    }
    Ok(decisions)
}

/// Look at the `expenses` table for the most-frequent category among
/// rows whose description is Levenshtein-close to `merchant`. Returns
/// the category_id of the modal match, or None if no descriptions are
/// close enough.
///
/// Conservative implementation: pull recent (last 365 days)
/// distinct (description, category_id) pairs, score by string distance,
/// pick the closest non-null match below threshold 3.
fn match_history(conn: &Connection, merchant: &str) -> Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT description, category_id, COUNT(*) AS n
         FROM expenses
         WHERE description IS NOT NULL
           AND category_id IS NOT NULL
           AND occurred_at >= datetime('now', '-365 days')
         GROUP BY description, category_id
         ORDER BY n DESC
         LIMIT 500",
    )?;
    let mut rows = stmt.query([])?;
    let needle = merchant.to_lowercase();
    let mut best: Option<(usize, i64)> = None;
    while let Some(r) = rows.next()? {
        let desc: Option<String> = r.get(0)?;
        let cat: Option<i64> = r.get(1)?;
        let (Some(desc), Some(cat)) = (desc, cat) else {
            continue;
        };
        let dist = strsim::levenshtein(&desc.to_lowercase(), &needle);
        if dist < 3 && best.map(|(d, _)| dist < d).unwrap_or(true) {
            best = Some((dist, cat));
        }
    }
    Ok(best.map(|(_, cat)| cat))
}

/// Helper exposed for the IPC layer's review-screen "save this rule"
/// action. Adds a new merchant rule for the given pattern + category.
pub fn record_rule_from_review(
    conn: &Connection,
    pattern: &str,
    category_id: i64,
    default_is_refund: bool,
    now: time::OffsetDateTime,
) -> Result<i64> {
    merchant_rules::create(conn, pattern, category_id, default_is_refund, 0, now)
}

/// Convenience: turn a raw merchant string from the review screen into
/// a `STARBUCKS*`-shaped pattern. Heuristic: take the first
/// alphanumeric "word", uppercase it, append `*`. The user can edit the
/// pattern in the UI before saving if they want something different.
pub fn suggest_pattern(merchant: &str) -> String {
    let first_word: String = merchant
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_uppercase();
    if first_word.is_empty() {
        merchant.trim().to_uppercase()
    } else {
        format!("{first_word}*")
    }
}

/// _Unused but kept_: surface counts of unmatched rows to the UI for
/// the "n more to categorize" badge.
pub fn count_decisions(decisions: &[Decision]) -> (usize, usize, usize) {
    let mut rule = 0usize;
    let mut history = 0usize;
    let mut unmatched = 0usize;
    for d in decisions {
        match d.source {
            "rule" => rule += 1,
            "history" => history += 1,
            _ => unmatched += 1,
        }
    }
    (rule, history, unmatched)
}

/// Re-export the rule type for callers that want to enumerate rules.
pub use crate::repository::merchant_rules::MerchantRule as Rule;

// silence unused-import warning when MerchantRule is only re-exported
// (and not referenced inside this file).
#[allow(dead_code)]
fn _force_use_merchant_rule(_: &MerchantRule) {}

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

    fn first_active_category(conn: &Connection) -> i64 {
        categories::list(conn, false)
            .unwrap()
            .into_iter()
            .find(|c| c.is_active)
            .unwrap()
            .id
    }

    fn parsed(merchant: &str) -> ParsedRow {
        ParsedRow {
            source_row_index: 0,
            occurred_at: OffsetDateTime::now_utc(),
            amount_cents: 1000,
            merchant: merchant.into(),
            description: None,
            raw_category: None,
            is_refund: false,
        }
    }

    #[test]
    fn rule_match_takes_precedence_over_history() {
        let conn = fresh_conn();
        let cat_a = first_active_category(&conn);
        let cat_b = categories::list(&conn, false).unwrap()[1].id;
        let now = OffsetDateTime::now_utc();
        // Set up history pointing at cat_b for "STARBUCKS"
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 1000,
                currency: "USD".into(),
                category_id: Some(cat_b),
                description: Some("Starbucks".into()),
                occurred_at: now,
                source: ExpenseSource::Telegram,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: false,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
        // Rule pointing at cat_a for "STARBUCKS*"
        merchant_rules::create(&conn, "STARBUCKS*", cat_a, false, 0, now).unwrap();
        let decisions = categorize_all(&conn, &[parsed("STARBUCKS #4521")]).unwrap();
        assert_eq!(decisions[0].source, "rule");
        assert_eq!(decisions[0].category_id, Some(cat_a));
    }

    #[test]
    fn history_match_when_no_rule() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        expenses::insert(
            &conn,
            &NewExpense {
                amount_cents: 1000,
                currency: "USD".into(),
                category_id: Some(cat),
                description: Some("Starbucks".into()),
                occurred_at: now,
                source: ExpenseSource::Telegram,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund: false,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
        let decisions = categorize_all(&conn, &[parsed("Starbuks")]).unwrap();
        assert_eq!(decisions[0].source, "history");
        assert_eq!(decisions[0].category_id, Some(cat));
    }

    #[test]
    fn unmatched_when_no_rule_or_history() {
        let conn = fresh_conn();
        let decisions = categorize_all(&conn, &[parsed("Some Brand New Place")]).unwrap();
        assert_eq!(decisions[0].source, "unmatched");
        assert_eq!(decisions[0].category_id, None);
    }

    #[test]
    fn rule_default_is_refund_propagates() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        merchant_rules::create(&conn, "AMAZON RETURN*", cat, true, 0, now).unwrap();
        let decisions = categorize_all(&conn, &[parsed("AMAZON RETURN PROCESSED")]).unwrap();
        assert_eq!(decisions[0].source, "rule");
        assert_eq!(decisions[0].override_is_refund, Some(true));
    }

    #[test]
    fn suggest_pattern_extracts_first_word() {
        assert_eq!(suggest_pattern("STARBUCKS #4521 SEATTLE"), "STARBUCKS*");
        assert_eq!(suggest_pattern("amazon.com"), "AMAZON*");
        assert_eq!(suggest_pattern("Whole Foods"), "WHOLE*");
        // Leading non-word chars → first_word is empty → fall back to
        // uppercased trim of the whole input.
        assert_eq!(suggest_pattern(".. weird ##"), ".. WEIRD ##");
    }

    #[test]
    fn count_decisions_breakdown() {
        let ds = vec![
            Decision {
                row_index: 0,
                source: "rule",
                category_id: Some(1),
                override_is_refund: None,
            },
            Decision {
                row_index: 1,
                source: "history",
                category_id: Some(1),
                override_is_refund: None,
            },
            Decision {
                row_index: 2,
                source: "unmatched",
                category_id: None,
                override_is_refund: None,
            },
            Decision {
                row_index: 3,
                source: "unmatched",
                category_id: None,
                override_is_refund: None,
            },
        ];
        let (r, h, u) = count_decisions(&ds);
        assert_eq!((r, h, u), (1, 1, 2));
    }
}
