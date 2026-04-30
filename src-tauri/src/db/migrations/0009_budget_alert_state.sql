-- Mr. Moneypenny migration 0009: budget alert state.
--
-- Tracks per-(category, year-month, threshold) "I already alerted you"
-- so the budget_alert_sweep doesn't spam the user every hour after a
-- threshold crosses. Reset implicitly per month (year_month is part of
-- the unique key).
--
-- Forward-only; bumps user_version to 9.

CREATE TABLE budget_alert_state (
  id INTEGER PRIMARY KEY,
  category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  year_month TEXT NOT NULL,           -- 'YYYY-MM'
  threshold_pct INTEGER NOT NULL CHECK (threshold_pct IN (80, 100)),
  fired_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  UNIQUE (category_id, year_month, threshold_pct)
);

PRAGMA user_version = 9;
