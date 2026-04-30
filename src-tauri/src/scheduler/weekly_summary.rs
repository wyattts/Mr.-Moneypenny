//! Weekly summary job handler.
//!
//! Singleton job. Sends a Telegram DM to the household owner once per
//! week summarizing the last 7 days. Toggleable via the
//! `weekly_summary_enabled` setting (default ON). When disabled, the
//! handler short-circuits but keeps rescheduling so flipping it back on
//! resumes seamlessly.

use anyhow::Result;
use rusqlite::params;
use time::{Duration, OffsetDateTime};

use crate::repository::{expenses, settings};
use crate::telegram::router::RouterDeps;

use super::{Job, JobOutcome};

/// How far apart consecutive summaries fire.
const SUMMARY_INTERVAL_DAYS: i64 = 7;

pub async fn handle(deps: &RouterDeps, _job: &Job, now: OffsetDateTime) -> Result<JobOutcome> {
    // Toggle.
    let enabled = {
        let conn = deps.conn.lock().unwrap();
        settings::get_or_default(&conn, settings::keys::WEEKLY_SUMMARY_ENABLED, "1")?
    };
    if enabled != "1" {
        return Ok(JobOutcome::Reschedule(
            now + Duration::days(SUMMARY_INTERVAL_DAYS),
        ));
    }

    // Owner chat — without one we can't send anything; just slip the
    // schedule forward and try again next week.
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
            now + Duration::days(SUMMARY_INTERVAL_DAYS),
        ));
    };

    // 7-day window ending at `now`.
    let window_start = now - Duration::days(7);
    let total_cents = {
        let conn = deps.conn.lock().unwrap();
        expenses::sum_in_range(&conn, window_start, now)?
    };
    let count: i64 = {
        let conn = deps.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM expenses \
             WHERE occurred_at >= ?1 AND occurred_at < ?2",
            params![window_start, now],
            |r| r.get(0),
        )?
    };

    let top_categories: Vec<(String, i64)> = {
        let conn = deps.conn.lock().unwrap();
        let sql = format!(
            "SELECT c.name, COALESCE(SUM({}), 0) AS total
             FROM expenses e
             LEFT JOIN categories c ON c.id = e.category_id
             WHERE e.occurred_at >= ?1 AND e.occurred_at < ?2
             GROUP BY c.name
             HAVING total > 0
             ORDER BY total DESC
             LIMIT 3",
            crate::repository::expenses::SIGNED_AMOUNT_SQL,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![window_start, now], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?
                        .unwrap_or_else(|| "(uncategorized)".into()),
                    r.get::<_, i64>(1)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let currency = {
        let conn = deps.conn.lock().unwrap();
        settings::get_or_default(&conn, settings::keys::DEFAULT_CURRENCY, "USD")?
    };

    let total_str = crate::telegram::formatter::format_money(total_cents, &currency);
    let mut msg = format!("Last 7 days: {total_str} across {count} expenses.");
    if !top_categories.is_empty() {
        let parts: Vec<String> = top_categories
            .iter()
            .map(|(name, cents)| {
                format!(
                    "{name} {amt}",
                    amt = crate::telegram::formatter::format_money(*cents, &currency)
                )
            })
            .collect();
        msg.push_str(&format!("\nTop: {}", parts.join(", ")));
    }
    msg.push_str("\n\nDisable weekly summaries in Settings → Notifications.");

    if let Err(e) = deps.client.send_message(chat_id, &msg).await {
        tracing::warn!(
            target: "scheduler::weekly_summary",
            error = %e,
            "failed to send weekly summary — will retry next tick"
        );
        return Ok(JobOutcome::Retry);
    }

    Ok(JobOutcome::Reschedule(
        now + Duration::days(SUMMARY_INTERVAL_DAYS),
    ))
}
