-- Mr. Moneypenny migration 0007: scheduled_jobs.
--
-- A generic queue of wall-clock-firing jobs. The scheduler tokio task
-- wakes once per minute, queries this table for `next_due_at <= now AND
-- enabled = 1`, dispatches each by `kind`, and advances `next_due_at`
-- for the next occurrence.
--
-- Three job kinds shipped at v0.2.6:
--   * recurring_expense — fires a user-defined recurring expense rule
--     (payload contains the rule_id). One row per rule.
--   * weekly_summary — sends the weekly bot DM. Singleton row.
--   * budget_alert_sweep — evaluates 80%/100% thresholds. Singleton row.
--
-- Forward-only; bumps user_version to 7.

CREATE TABLE scheduled_jobs (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN (
    'recurring_expense', 'weekly_summary', 'budget_alert_sweep'
  )),
  payload TEXT NOT NULL DEFAULT '{}',
  next_due_at TEXT NOT NULL,
  last_fired_at TEXT,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_scheduled_jobs_due
  ON scheduled_jobs(next_due_at)
  WHERE enabled = 1;

PRAGMA user_version = 7;
