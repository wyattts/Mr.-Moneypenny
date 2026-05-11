-- Mr. Moneypenny migration 0016: composite index on
-- expenses(category_id, occurred_at).
--
-- `expenses::list_in_range_by_category` and
-- `monthly_totals_for_category` both filter on
--   category_id = ?1 AND occurred_at >= ?2 AND occurred_at < ?3
--
-- Before this index, SQLite picked `idx_expenses_category_id` and
-- post-filtered each match by date — fine when a category has a few
-- dozen rows, noticeable when one has years of activity. The composite
-- index satisfies both predicates with a single range scan (audit D-4
-- in docs/audit-v0.3.7.md).
--
-- The pre-existing single-column indexes are left in place; the query
-- planner picks whichever one fits each statement.

CREATE INDEX idx_expenses_category_occurred
  ON expenses(category_id, occurred_at);

PRAGMA user_version = 16;
