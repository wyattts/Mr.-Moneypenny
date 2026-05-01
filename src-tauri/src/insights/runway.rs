//! Survivability / runway calculator.
//!
//! Question answered: **"If your income stops today, how many months
//! can you sustain your current spend?"**
//!
//! Inputs, all derived from existing data:
//! - **Monthly burn** = recent 3-month average of fixed + variable
//!   spend (refunds netted via the v0.2.6 signed-sum convention).
//! - **Drawable balance** = sum of investing-kind
//!   `starting_balance_cents` (the user-entered current account
//!   balances) + last-12-month investing contributions (proxy for
//!   short-horizon savings the user has been actively accumulating).
//!
//! ## Honest about assumptions
//!
//! - All investing balances are treated as fully drawable. Real-life:
//!   401(k) and traditional IRA early withdrawals incur 10% penalty
//!   plus income tax. The headline number ignores both.
//! - Recent burn rate is assumed to continue. Holiday months,
//!   one-off big spends, etc. are smoothed by the 3-month window but
//!   not eliminated.
//! - No income whatsoever assumed. (That's the whole point of "if
//!   income stops.")
//!
//! The UI surfaces a "What this assumes" disclosure; this module just
//! does the math.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use time::{Duration, OffsetDateTime};

use crate::repository::expenses::SIGNED_AMOUNT_SQL;

#[derive(Debug, Clone, Serialize)]
pub struct RunwayResult {
    /// Recent 3-month average fixed spend, in cents/month.
    pub fixed_per_month_cents: i64,
    /// Recent 3-month average variable spend, in cents/month.
    pub variable_per_month_cents: i64,
    /// Sum of the two (the "burn" that defines runway).
    pub total_burn_per_month_cents: i64,
    /// Sum of investing-kind starting balances + last 12 months of
    /// investing contributions.
    pub drawable_balance_cents: i64,
    /// Months the drawable balance lasts at the recent burn rate.
    /// Stored as f64 because partial months matter for the headline
    /// (e.g., "9.4 months" reads better than rounding to 9).
    pub months_at_recent_burn: f64,
    /// Stress-test mode: variable spend trimmed to its 25th percentile
    /// over the last 12 months. None when there isn't enough history
    /// to compute a P25.
    pub variable_p25_cents: Option<i64>,
    pub months_at_p25_variable: Option<f64>,
}

/// Compute the runway from real data in the live DB.
pub fn compute(conn: &Connection, now: OffsetDateTime) -> Result<RunwayResult> {
    let three_mo_ago = now - Duration::days(90);
    let twelve_mo_ago = now - Duration::days(365);

    let fixed_three_mo = sum_kind(conn, "fixed", three_mo_ago, now)?;
    let variable_three_mo = sum_kind(conn, "variable", three_mo_ago, now)?;

    // Average over 3 months. We round to whole cents at the end.
    let fixed_per_month = fixed_three_mo / 3;
    let variable_per_month = variable_three_mo / 3;
    let total_burn = fixed_per_month + variable_per_month;

    let drawable = drawable_balance(conn, twelve_mo_ago, now)?;

    let months_at_recent_burn = if total_burn > 0 {
        drawable as f64 / total_burn as f64
    } else {
        f64::INFINITY
    };

    let variable_p25 = variable_p25_last_12mo(conn, twelve_mo_ago, now)?;
    let months_at_p25 = match variable_p25 {
        Some(p25) => {
            let stressed_burn = fixed_per_month + p25;
            if stressed_burn > 0 {
                Some(drawable as f64 / stressed_burn as f64)
            } else {
                Some(f64::INFINITY)
            }
        }
        None => None,
    };

    Ok(RunwayResult {
        fixed_per_month_cents: fixed_per_month,
        variable_per_month_cents: variable_per_month,
        total_burn_per_month_cents: total_burn,
        drawable_balance_cents: drawable,
        months_at_recent_burn,
        variable_p25_cents: variable_p25,
        months_at_p25_variable: months_at_p25,
    })
}

fn sum_kind(
    conn: &Connection,
    kind: &str,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<i64> {
    let total: i64 = conn.query_row(
        &format!(
            "SELECT COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0)
             FROM expenses e
             JOIN categories c ON c.id = e.category_id
             WHERE c.kind = ?1
               AND e.occurred_at >= ?2 AND e.occurred_at < ?3"
        ),
        params![kind, start, end],
        |r| r.get(0),
    )?;
    Ok(total)
}

fn drawable_balance(
    conn: &Connection,
    twelve_mo_ago: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<i64> {
    let starting: i64 = conn.query_row(
        "SELECT COALESCE(SUM(starting_balance_cents), 0)
         FROM categories
         WHERE kind = 'investing' AND is_active = 1",
        [],
        |r| r.get(0),
    )?;
    // Investing contributions don't have refunds (no negative
    // 'investing' rows in practice), but we still apply the signed-sum
    // for safety so an accidental refund row wouldn't double-count.
    let contributed: i64 = conn.query_row(
        &format!(
            "SELECT COALESCE(SUM({SIGNED_AMOUNT_SQL}), 0)
             FROM expenses e
             JOIN categories c ON c.id = e.category_id
             WHERE c.kind = 'investing'
               AND e.occurred_at >= ?1 AND e.occurred_at < ?2"
        ),
        params![twelve_mo_ago, now],
        |r| r.get(0),
    )?;
    Ok(starting + contributed)
}

fn variable_p25_last_12mo(
    conn: &Connection,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Result<Option<i64>> {
    // Per-month variable totals over the last 12 months. Need at least
    // 4 months for a meaningful P25.
    let mut stmt = conn.prepare(&format!(
        "SELECT strftime('%Y-%m', e.occurred_at) AS ym,
                    SUM({SIGNED_AMOUNT_SQL}) AS total
             FROM expenses e
             JOIN categories c ON c.id = e.category_id
             WHERE c.kind = 'variable'
               AND e.occurred_at >= ?1 AND e.occurred_at < ?2
             GROUP BY ym
             ORDER BY ym ASC"
    ))?;
    let totals: Vec<i64> = stmt
        .query_map(params![start, end], |r| r.get::<_, i64>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if totals.len() < 4 {
        return Ok(None);
    }
    let mut sorted = totals.clone();
    sorted.sort_unstable();
    Ok(Some(crate::insights::stats::percentile(&sorted, 25.0)))
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

    fn first_kind_category(conn: &Connection, kind: &str) -> i64 {
        let cats = categories::list(conn, false).unwrap();
        cats.into_iter()
            .find(|c| c.is_active && format!("{:?}", c.kind).to_lowercase() == kind)
            .expect("seeded actives include each kind")
            .id
    }

    fn insert(conn: &Connection, cat: i64, amount: i64, when: OffsetDateTime, is_refund: bool) {
        expenses::insert(
            conn,
            &NewExpense {
                amount_cents: amount,
                currency: "USD".into(),
                category_id: Some(cat),
                description: None,
                occurred_at: when,
                source: ExpenseSource::Manual,
                raw_message: None,
                llm_confidence: None,
                logged_by_chat_id: None,
                is_refund,
                refund_for_expense_id: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn no_data_yields_zero_burn_and_infinite_runway() {
        let conn = fresh_conn();
        let r = compute(&conn, OffsetDateTime::now_utc()).unwrap();
        assert_eq!(r.fixed_per_month_cents, 0);
        assert_eq!(r.variable_per_month_cents, 0);
        assert_eq!(r.drawable_balance_cents, 0);
        assert!(r.months_at_recent_burn.is_infinite());
    }

    #[test]
    fn three_month_avg_burn_and_drawable_balance() {
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        let fixed_cat = first_kind_category(&conn, "fixed");
        let variable_cat = first_kind_category(&conn, "variable");
        // include_inactive=true because investing kinds default to inactive.
        let investing_cat = {
            let cats = categories::list(&conn, true).unwrap();
            cats.iter()
                .find(|c| format!("{:?}", c.kind).to_lowercase() == "investing")
                .expect("at least one investing seed")
                .id
        };
        categories::set_active(&conn, investing_cat, true).unwrap();
        // Seed 30 days ago: $1000 rent, $300 dining, $200 401k.
        let when = now - Duration::days(30);
        insert(&conn, fixed_cat, 100_000, when, false);
        insert(&conn, variable_cat, 30_000, when, false);
        insert(&conn, investing_cat, 20_000, when, false);
        // 60 days ago: same.
        let when = now - Duration::days(60);
        insert(&conn, fixed_cat, 100_000, when, false);
        insert(&conn, variable_cat, 30_000, when, false);
        insert(&conn, investing_cat, 20_000, when, false);
        // Set a starting balance on the investing category.
        categories::set_starting_balance(
            &conn,
            investing_cat,
            Some(500_000), // $5000
            Some("2026-01-01"),
        )
        .unwrap();
        let r = compute(&conn, now).unwrap();
        // 3-mo total fixed = $2000 → /3 = $666.67/mo (rounds to integer cents).
        // The integer-division rounds down: $2000 = 200000 cents, /3 = 66666.
        assert_eq!(r.fixed_per_month_cents, 66666);
        assert_eq!(r.variable_per_month_cents, 20000);
        assert_eq!(r.total_burn_per_month_cents, 86666);
        // drawable = $5000 starting + $400 contributions in last 12mo
        assert_eq!(r.drawable_balance_cents, 540_000);
        // Months: 540000 / 86666 ≈ 6.23
        assert!((r.months_at_recent_burn - 6.23).abs() < 0.05);
    }

    #[test]
    fn refunds_net_out_of_burn() {
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        let variable_cat = first_kind_category(&conn, "variable");
        let when = now - Duration::days(30);
        // $100 expense + $30 refund = $70 net for this month.
        insert(&conn, variable_cat, 10_000, when, false);
        insert(&conn, variable_cat, 3_000, when, true);
        let r = compute(&conn, now).unwrap();
        // 3-month avg of $70 / 3 = ~$23.33 → 2333 cents
        assert_eq!(r.variable_per_month_cents, 2333);
    }

    #[test]
    fn p25_stress_mode_is_set_when_history_exists() {
        // We rely on the SQL strftime('%Y-%m') grouping, so the goal
        // here is just to confirm: with ≥4 months of data, P25 is
        // populated. We DON'T assert P25 < variable_per_month because
        // calendar-boundary collisions (two inserts in the same month
        // when 30-day offsets straddle a month edge) make that
        // numerically fragile across run dates.
        let conn = fresh_conn();
        let now = OffsetDateTime::now_utc();
        let variable_cat = first_kind_category(&conn, "variable");
        let amounts = [10_000, 12_000, 8_000, 15_000, 11_000, 9_000];
        for (i, amt) in amounts.iter().enumerate() {
            // Use 35-day stride so months don't collide on most dates.
            let when = now - Duration::days(35 * (i as i64 + 1));
            insert(&conn, variable_cat, *amt, when, false);
        }
        let r = compute(&conn, now).unwrap();
        assert!(r.variable_p25_cents.is_some());
        if r.total_burn_per_month_cents > 0 {
            assert!(r.months_at_p25_variable.is_some());
        }
    }
}
