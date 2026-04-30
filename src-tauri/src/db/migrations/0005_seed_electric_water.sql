-- Mr. Moneypenny migration 0005: add Electric + Water as inactive-by-
-- default fixed-cost seed categories. Fresh installs already see them
-- via the updated 0002; existing v0.2.0 installs need this insert.
--
-- INSERT OR IGNORE skips rows whose names already exist, so re-runs are
-- safe and the migration is a no-op for users who manually created
-- categories with these exact names before the seed shipped.
-- Forward-only; bumps user_version to 5.

INSERT OR IGNORE INTO categories (name, kind, is_recurring, is_active, is_seed) VALUES
  ('Electric', 'fixed', 1, 0, 1),
  ('Water',    'fixed', 1, 0, 1);

PRAGMA user_version = 5;
