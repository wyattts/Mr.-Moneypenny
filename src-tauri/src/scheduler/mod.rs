//! Scheduler — a wall-clock-driven job dispatcher.
//!
//! Wakes every `TICK_INTERVAL_SECS`, queries `scheduled_jobs` for due
//! rows, dispatches each by `kind`, and advances `next_due_at` for the
//! next occurrence. Jobs whose `next_due_at` is older than `MAX_STALE`
//! (machine was off for a long time) are skipped without firing —
//! safer than silently inserting an expense that may not have actually
//! happened.
//!
//! Three kinds shipped at v0.2.6:
//!   - `recurring_expense` — fires a user-defined recurring rule.
//!   - `weekly_summary` — sends the weekly bot DM.
//!   - `budget_alert_sweep` — re-evaluates 80% / 100% thresholds.
//!
//! The dispatcher functions live in their own modules; this file owns
//! the loop, queue helpers, and shared types.

pub mod budget_alerts;
pub mod recurring;
pub mod weekly_summary;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::telegram::router::RouterDeps;

/// Scheduler wake interval. Once per minute is enough for the granularity
/// the v0.2.6 features need (daily / weekly / monthly recurring +
/// hourly alert sweep).
pub const TICK_INTERVAL_SECS: u64 = 60;

/// Maximum staleness before a job is skipped on catch-up. If the user's
/// machine was off for longer than this, we don't silently fire jobs —
/// the user can re-create them or we'll fire them next cycle.
pub const MAX_STALE_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    RecurringExpense,
    WeeklySummary,
    BudgetAlertSweep,
}

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::RecurringExpense => "recurring_expense",
            JobKind::WeeklySummary => "weekly_summary",
            JobKind::BudgetAlertSweep => "budget_alert_sweep",
        }
    }
}

impl std::str::FromStr for JobKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "recurring_expense" => JobKind::RecurringExpense,
            "weekly_summary" => JobKind::WeeklySummary,
            "budget_alert_sweep" => JobKind::BudgetAlertSweep,
            other => anyhow::bail!("unknown scheduler job kind: {other}"),
        })
    }
}

impl rusqlite::types::ToSql for JobKind {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.as_str()))
    }
}

impl rusqlite::types::FromSql for JobKind {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: anyhow::Error| rusqlite::types::FromSqlError::Other(e.into()))
    }
}

/// One row out of `scheduled_jobs`.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub kind: JobKind,
    pub payload: String,
    pub next_due_at: OffsetDateTime,
    pub last_fired_at: Option<OffsetDateTime>,
}

/// What the scheduler should do with a job after the handler ran.
#[derive(Debug)]
pub enum JobOutcome {
    /// Reschedule for the given `next_due_at`.
    Reschedule(OffsetDateTime),
    /// Job is finished (one-shot, or its underlying rule was deleted).
    /// Disable the row so it doesn't run again.
    Done,
    /// Handler had a transient failure; leave `next_due_at` alone so the
    /// next tick retries.
    Retry,
}

// ---------------------------------------------------------------------
// Queue helpers (sync, take a Connection — keep the hot lock short).
// ---------------------------------------------------------------------

/// Insert a new job. Returns the new row id.
pub fn enqueue(
    conn: &Connection,
    kind: JobKind,
    payload: &str,
    next_due_at: OffsetDateTime,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO scheduled_jobs (kind, payload, next_due_at) VALUES (?1, ?2, ?3)",
        params![kind, payload, next_due_at],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Delete every job of the given kind whose payload JSON matches the
/// given equality predicate (e.g., `payload->>'rule_id' = ?`). Used when
/// a recurring rule is deleted.
pub fn delete_jobs_for_recurring_rule(conn: &Connection, rule_id: i64) -> Result<usize> {
    let n = conn.execute(
        "DELETE FROM scheduled_jobs
         WHERE kind = 'recurring_expense'
           AND json_extract(payload, '$.rule_id') = ?1",
        params![rule_id],
    )?;
    Ok(n)
}

/// Ensure exactly one enabled job of the given (singleton) kind exists.
/// Used at startup for `weekly_summary` and `budget_alert_sweep` so
/// upgrading users get the new schedules without manual setup.
pub fn ensure_singleton(
    conn: &Connection,
    kind: JobKind,
    initial_due_at: OffsetDateTime,
) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM scheduled_jobs WHERE kind = ?1 LIMIT 1",
            params![kind],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = existing {
        // Make sure it's enabled; leave its next_due_at alone so we don't
        // double-fire at every launch.
        conn.execute(
            "UPDATE scheduled_jobs SET enabled = 1 WHERE id = ?1",
            params![id],
        )?;
        return Ok(id);
    }
    enqueue(conn, kind, "{}", initial_due_at)
}

/// Fetch all jobs with `next_due_at <= now AND enabled = 1`, oldest-due
/// first.
pub fn list_due(conn: &Connection, now: OffsetDateTime) -> Result<Vec<Job>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, kind, payload, next_due_at, last_fired_at
         FROM scheduled_jobs
         WHERE enabled = 1 AND next_due_at <= ?1
         ORDER BY next_due_at ASC, id ASC",
    )?;
    let rows = stmt
        .query_map(params![now], |r| {
            Ok(Job {
                id: r.get(0)?,
                kind: r.get(1)?,
                payload: r.get(2)?,
                next_due_at: r.get(3)?,
                last_fired_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn update_next_due(
    conn: &Connection,
    job_id: i64,
    next_due_at: OffsetDateTime,
    last_fired_at: Option<OffsetDateTime>,
) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_jobs SET next_due_at = ?2, last_fired_at = ?3 WHERE id = ?1",
        params![job_id, next_due_at, last_fired_at],
    )?;
    Ok(())
}

pub fn disable_job(conn: &Connection, job_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE scheduled_jobs SET enabled = 0 WHERE id = ?1",
        params![job_id],
    )?;
    Ok(())
}

/// True when `next_due_at` is more than `MAX_STALE_DAYS` overdue.
pub fn is_stale(job: &Job, now: OffsetDateTime) -> bool {
    let stale_secs = (now - job.next_due_at).whole_seconds();
    stale_secs > MAX_STALE_DAYS * 86_400
}

// ---------------------------------------------------------------------
// Tick — the public entry point used by the spawned task and tests.
// ---------------------------------------------------------------------

/// Process all due jobs once. Used by the scheduler loop AND by tests.
pub async fn tick(deps: &RouterDeps, now: OffsetDateTime) -> Result<usize> {
    let due = {
        let conn = deps.conn.lock().unwrap();
        list_due(&conn, now)?
    };
    let mut fired = 0;
    for job in due {
        if is_stale(&job, now) {
            tracing::warn!(
                target: "scheduler",
                job_id = job.id,
                kind = job.kind.as_str(),
                stale_days = (now - job.next_due_at).whole_days(),
                "skipping stale job"
            );
            // Bump next_due_at forward so we stop logging this every tick.
            // For singleton jobs (weekly_summary / budget_alert_sweep)
            // the handler-specific schedule kicks in on the next real tick.
            let new_due = now + Duration::from_secs((MAX_STALE_DAYS as u64) * 86_400);
            let conn = deps.conn.lock().unwrap();
            update_next_due(&conn, job.id, new_due, None)?;
            continue;
        }

        let outcome = match job.kind {
            JobKind::RecurringExpense => recurring::handle(deps, &job, now).await,
            JobKind::WeeklySummary => weekly_summary::handle(deps, &job, now).await,
            JobKind::BudgetAlertSweep => budget_alerts::handle(deps, &job, now).await,
        };

        let conn = deps.conn.lock().unwrap();
        match outcome {
            Ok(JobOutcome::Reschedule(next)) => {
                update_next_due(&conn, job.id, next, Some(now))?;
                fired += 1;
            }
            Ok(JobOutcome::Done) => {
                disable_job(&conn, job.id)?;
                fired += 1;
            }
            Ok(JobOutcome::Retry) => {
                // Leave next_due_at alone; we'll try again on the next tick.
            }
            Err(e) => {
                tracing::warn!(
                    target: "scheduler",
                    job_id = job.id,
                    kind = job.kind.as_str(),
                    error = %e,
                    "handler errored — leaving for retry"
                );
            }
        }
    }
    Ok(fired)
}

/// Long-running task: tick forever, until `shutdown` flips true.
pub async fn run(deps: RouterDeps, shutdown: Arc<AtomicBool>) {
    tracing::info!(target: "scheduler", "scheduler task started");
    let mut interval = tokio::time::interval(Duration::from_secs(TICK_INTERVAL_SECS));
    // Do an initial tick immediately so any jobs that came due while the
    // app was off get fired right after launch (subject to MAX_STALE).
    interval.tick().await;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            tracing::info!(target: "scheduler", "scheduler task shutting down");
            break;
        }
        let now = OffsetDateTime::now_utc();
        match tick(&deps, now).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(target: "scheduler", fired = n, "scheduler tick fired jobs"),
            Err(e) => tracing::warn!(target: "scheduler", error = %e, "scheduler tick failed"),
        }
        interval.tick().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use time::macros::datetime;

    fn fresh_db() -> Connection {
        let conn = db::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn enqueue_and_list_due_round_trips() {
        let conn = fresh_db();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        let id = enqueue(
            &conn,
            JobKind::WeeklySummary,
            "{}",
            datetime!(2026-04-15 11:00:00 UTC),
        )
        .unwrap();
        let due = list_due(&conn, now).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
        assert_eq!(due[0].kind, JobKind::WeeklySummary);
    }

    #[test]
    fn list_due_excludes_future_and_disabled() {
        let conn = fresh_db();
        let now = datetime!(2026-04-15 12:00:00 UTC);
        // Future job — not due.
        enqueue(
            &conn,
            JobKind::WeeklySummary,
            "{}",
            datetime!(2026-04-16 11:00:00 UTC),
        )
        .unwrap();
        // Disabled job — not returned even when due.
        let id = enqueue(
            &conn,
            JobKind::BudgetAlertSweep,
            "{}",
            datetime!(2026-04-15 10:00:00 UTC),
        )
        .unwrap();
        disable_job(&conn, id).unwrap();
        let due = list_due(&conn, now).unwrap();
        assert!(due.is_empty());
    }

    #[test]
    fn ensure_singleton_creates_then_reuses() {
        let conn = fresh_db();
        let due = datetime!(2026-04-15 12:00:00 UTC);
        let id1 = ensure_singleton(&conn, JobKind::WeeklySummary, due).unwrap();
        let id2 = ensure_singleton(&conn, JobKind::WeeklySummary, due).unwrap();
        assert_eq!(id1, id2, "second call must reuse the existing row");
    }

    #[test]
    fn ensure_singleton_re_enables_disabled_row() {
        let conn = fresh_db();
        let due = datetime!(2026-04-15 12:00:00 UTC);
        let id = ensure_singleton(&conn, JobKind::WeeklySummary, due).unwrap();
        disable_job(&conn, id).unwrap();
        let id2 = ensure_singleton(&conn, JobKind::WeeklySummary, due).unwrap();
        assert_eq!(id, id2);
        let enabled: i64 = conn
            .query_row(
                "SELECT enabled FROM scheduled_jobs WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(enabled, 1, "singleton must be re-enabled");
    }

    #[test]
    fn is_stale_detects_old_jobs() {
        let now = datetime!(2026-04-15 12:00:00 UTC);
        let recent = Job {
            id: 1,
            kind: JobKind::WeeklySummary,
            payload: "{}".into(),
            next_due_at: datetime!(2026-04-14 12:00:00 UTC), // 1 day overdue
            last_fired_at: None,
        };
        let old = Job {
            id: 2,
            kind: JobKind::WeeklySummary,
            payload: "{}".into(),
            next_due_at: datetime!(2026-04-01 12:00:00 UTC), // 14 days overdue
            last_fired_at: None,
        };
        assert!(!is_stale(&recent, now));
        assert!(is_stale(&old, now));
    }

    #[test]
    fn delete_jobs_for_recurring_rule_only_drops_matching_rule() {
        let conn = fresh_db();
        let due = datetime!(2026-04-15 12:00:00 UTC);
        let id1 = enqueue(&conn, JobKind::RecurringExpense, r#"{"rule_id": 1}"#, due).unwrap();
        let id2 = enqueue(&conn, JobKind::RecurringExpense, r#"{"rule_id": 2}"#, due).unwrap();
        let n = delete_jobs_for_recurring_rule(&conn, 1).unwrap();
        assert_eq!(n, 1);
        let rest: Vec<i64> = list_due(&conn, due)
            .unwrap()
            .into_iter()
            .map(|j| j.id)
            .collect();
        assert_eq!(rest, vec![id2]);
        let _ = id1;
    }
}
