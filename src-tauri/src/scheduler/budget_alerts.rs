//! Budget threshold alerts.
//!
//! Singleton job that wakes hourly and evaluates each active variable
//! category against its monthly target. Fires a bot DM when net spend
//! crosses 80% or 100% of the target, *once per threshold per calendar
//! month* (tracked in `budget_alert_state`). Investing-kind categories
//! are excluded — those are savings goals, not spending caps.
//!
//! Toggleable via `budget_alerts_enabled` setting (default ON).

use anyhow::Result;
use rusqlite::params;
use time::{Duration, OffsetDateTime};

use crate::domain::current_month_bounds;
use crate::repository::settings;
use crate::telegram::router::RouterDeps;

use super::{Job, JobOutcome};

const SWEEP_INTERVAL_MINUTES: i64 = 60;
const THRESHOLDS: &[i64] = &[80, 100];

pub async fn handle(deps: &RouterDeps, _job: &Job, now: OffsetDateTime) -> Result<JobOutcome> {
    let enabled = {
        let conn = deps.conn.lock().unwrap();
        settings::get_or_default(&conn, settings::keys::BUDGET_ALERTS_ENABLED, "1")?
    };
    if enabled != "1" {
        return Ok(JobOutcome::Reschedule(
            now + Duration::minutes(SWEEP_INTERVAL_MINUTES),
        ));
    }

    let owner: Option<i64> = {
        let conn = deps.conn.lock().unwrap();
        conn.query_row(
            "SELECT chat_id FROM telegram_authorized_chats \
             WHERE role = 'owner' ORDER BY chat_id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok()
    };
    let Some(chat_id) = owner else {
        return Ok(JobOutcome::Reschedule(
            now + Duration::minutes(SWEEP_INTERVAL_MINUTES),
        ));
    };

    let (month_start, month_end) = current_month_bounds(now);
    let year_month = format!(
        "{:04}-{:02}",
        month_start.year(),
        u8::from(month_start.month())
    );

    // Gather active variable categories with a monthly target.
    let categories: Vec<(i64, String, i64)> = {
        let conn = deps.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, monthly_target_cents
             FROM categories
             WHERE kind = 'variable'
               AND is_active = 1
               AND monthly_target_cents IS NOT NULL
               AND monthly_target_cents > 0",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let currency = {
        let conn = deps.conn.lock().unwrap();
        settings::get_or_default(&conn, settings::keys::DEFAULT_CURRENCY, "USD")?
    };

    let mut alerts_to_send: Vec<(i64, i64, String)> = Vec::new();
    for (cat_id, cat_name, target_cents) in categories {
        // Net spend this month.
        let spent: i64 = {
            let conn = deps.conn.lock().unwrap();
            let sql = format!(
                "SELECT COALESCE(SUM({}), 0)
                 FROM expenses
                 WHERE category_id = ?1
                   AND occurred_at >= ?2 AND occurred_at < ?3",
                crate::repository::expenses::SIGNED_AMOUNT_SQL,
            );
            conn.query_row(&sql, params![cat_id, month_start, month_end], |r| r.get(0))?
        };
        if spent <= 0 {
            continue;
        }
        for &pct in THRESHOLDS {
            let trigger = (target_cents * pct) / 100;
            if spent < trigger {
                continue;
            }
            // Already alerted for this (category, month, threshold)?
            let already: i64 = {
                let conn = deps.conn.lock().unwrap();
                conn.query_row(
                    "SELECT COUNT(*) FROM budget_alert_state
                     WHERE category_id = ?1 AND year_month = ?2 AND threshold_pct = ?3",
                    params![cat_id, year_month, pct],
                    |r| r.get(0),
                )?
            };
            if already > 0 {
                continue;
            }
            // Compose alert.
            let amount_str = crate::telegram::formatter::format_money(spent, &currency);
            let target_str = crate::telegram::formatter::format_money(target_cents, &currency);
            let warning = if pct == 100 {
                format!("{cat_name} is OVER your {target_str} monthly budget — {amount_str} spent.")
            } else {
                format!(
                    "{cat_name} is at {pct}% of your {target_str} monthly budget ({amount_str} spent)."
                )
            };
            alerts_to_send.push((cat_id, pct, warning));
        }
    }

    // Fire alerts and persist state. We do this AFTER collecting so the
    // DB lock isn't held across await.
    for (cat_id, pct, msg) in alerts_to_send {
        if let Err(e) = deps.client.send_message(chat_id, &msg).await {
            tracing::warn!(
                target: "scheduler::budget_alerts",
                cat_id, pct, error = %e,
                "send_message failed; not recording state — will retry next sweep"
            );
            continue;
        }
        let conn = deps.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO budget_alert_state (category_id, year_month, threshold_pct)
             VALUES (?1, ?2, ?3)",
            params![cat_id, year_month, pct],
        )?;
    }

    Ok(JobOutcome::Reschedule(
        now + Duration::minutes(SWEEP_INTERVAL_MINUTES),
    ))
}
