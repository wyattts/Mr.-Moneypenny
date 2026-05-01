//! CRUD for `merchant_rules` — pattern → category mappings the import
//! flow consults to skip the LLM.
//!
//! Patterns are SQLite GLOB strings. Real merchant-string matching uses
//! `fnmatch_glob` here in Rust rather than `LIKE` in SQL, because the
//! input merchant string is unbounded and we want full control over
//! case-folding and wildcard semantics.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize)]
pub struct MerchantRule {
    pub id: i64,
    pub pattern: String,
    pub category_id: i64,
    pub default_is_refund: bool,
    pub priority: i64,
    pub created_at: OffsetDateTime,
}

pub fn create(
    conn: &Connection,
    pattern: &str,
    category_id: i64,
    default_is_refund: bool,
    priority: i64,
    now: OffsetDateTime,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO merchant_rules
            (pattern, category_id, default_is_refund, priority, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            pattern,
            category_id,
            default_is_refund as i64,
            priority,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list(conn: &Connection) -> Result<Vec<MerchantRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, pattern, category_id, default_is_refund, priority, created_at
         FROM merchant_rules
         ORDER BY priority DESC, created_at DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_rule)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM merchant_rules WHERE id = ?1", params![id])?;
    Ok(())
}

/// Find the highest-priority rule whose pattern matches `merchant`. The
/// match is case-insensitive and uses a small glob implementation
/// supporting `*` (zero+ chars), `?` (one char), and literal text.
/// Most patterns the import wizard auto-generates look like
/// `STARBUCKS*` or `*SAFEWAY*`.
pub fn find_match<'a>(rules: &'a [MerchantRule], merchant: &str) -> Option<&'a MerchantRule> {
    let needle = merchant.to_lowercase();
    rules
        .iter()
        .find(|r| glob_match(&r.pattern.to_lowercase(), &needle))
}

fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<MerchantRule> {
    let refund_int: i64 = row.get(3)?;
    Ok(MerchantRule {
        id: row.get(0)?,
        pattern: row.get(1)?,
        category_id: row.get(2)?,
        default_is_refund: refund_int != 0,
        priority: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// Minimal recursive glob matcher: `*` = zero+ any-chars, `?` = one
/// any-char, everything else is literal. Both inputs lower-cased
/// upstream. Returns true when the entire input is consumed by the
/// pattern.
fn glob_match(pattern: &str, input: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let inp: Vec<char> = input.chars().collect();
    glob_recur(&pat, 0, &inp, 0)
}

fn glob_recur(pat: &[char], pi: usize, inp: &[char], ii: usize) -> bool {
    if pi == pat.len() {
        return ii == inp.len();
    }
    match pat[pi] {
        '*' => {
            // Greedy with backtrack: try every possible consumption length.
            for k in ii..=inp.len() {
                if glob_recur(pat, pi + 1, inp, k) {
                    return true;
                }
            }
            false
        }
        '?' => ii < inp.len() && glob_recur(pat, pi + 1, inp, ii + 1),
        c => ii < inp.len() && inp[ii] == c && glob_recur(pat, pi + 1, inp, ii + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::repository::categories;

    fn fresh_conn() -> Connection {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn first_active_category(conn: &Connection) -> i64 {
        let cats = categories::list(conn, false).unwrap();
        cats.into_iter()
            .find(|c| c.is_active)
            .expect("seeded actives exist")
            .id
    }

    #[test]
    fn glob_star_matches_suffix_noise() {
        assert!(glob_match("starbucks*", "starbucks #4521 seattle wa"));
        assert!(glob_match("*safeway*", "safeway store 1234"));
        assert!(!glob_match("starbucks*", "blue bottle coffee"));
    }

    #[test]
    fn glob_exact_pattern_must_match_fully() {
        assert!(glob_match("netflix", "netflix"));
        assert!(!glob_match("netflix", "netflix.com"));
    }

    #[test]
    fn glob_question_mark_consumes_one_char() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a?c", "abbc"));
    }

    #[test]
    fn create_list_round_trip() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        create(&conn, "STARBUCKS*", cat, false, 0, now).unwrap();
        create(&conn, "AMAZON RETURN*", cat, true, 10, now).unwrap();
        let rules = list(&conn).unwrap();
        assert_eq!(rules.len(), 2);
        // Higher priority first.
        assert_eq!(rules[0].pattern, "AMAZON RETURN*");
        assert!(rules[0].default_is_refund);
        assert_eq!(rules[1].pattern, "STARBUCKS*");
    }

    #[test]
    fn find_match_picks_highest_priority() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        create(&conn, "*", cat, false, 0, now).unwrap();
        create(&conn, "STARBUCKS*", cat, false, 100, now).unwrap();
        let rules = list(&conn).unwrap();
        let m = find_match(&rules, "STARBUCKS #4521").unwrap();
        assert_eq!(m.pattern, "STARBUCKS*");
    }

    #[test]
    fn find_match_is_case_insensitive() {
        let conn = fresh_conn();
        let cat = first_active_category(&conn);
        let now = OffsetDateTime::now_utc();
        create(&conn, "Starbucks*", cat, false, 0, now).unwrap();
        let rules = list(&conn).unwrap();
        assert!(find_match(&rules, "STARBUCKS #4521").is_some());
        assert!(find_match(&rules, "starbucks #4521").is_some());
    }

    #[test]
    fn cascade_delete_removes_rule_when_category_deleted() {
        // The categories repo's delete is soft (sets is_active=0) for
        // seeded rows but hard for user-created. Make a user category,
        // attach a rule, hard-delete the category — rule should vanish
        // via FK ON DELETE CASCADE.
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        // Use a hard insert to bypass the seed-protection in the repo.
        conn.execute(
            "INSERT INTO categories (name, kind, is_active, is_seed) VALUES ('TestCat', 'variable', 1, 0)",
            [],
        ).unwrap();
        let cat_id = conn.last_insert_rowid();
        create(&conn, "TEST*", cat_id, false, 0, now).unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);
        conn.execute("DELETE FROM categories WHERE id = ?1", params![cat_id])
            .unwrap();
        assert_eq!(list(&conn).unwrap().len(), 0);
    }
}
