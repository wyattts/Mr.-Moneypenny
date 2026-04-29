-- Mr. Moneypenny migration 0004: add a third category kind, 'investing',
-- alongside 'fixed' and 'variable'. Seeds Savings / 401k / Investing /
-- Roth IRA as inactive-by-default investing categories users can tick
-- on if they apply.
--
-- SQLite doesn't support ALTER TABLE … ALTER CONSTRAINT, so we recreate
-- the categories table. Foreign keys from `expenses.category_id` and
-- `budgets.category_id` are preserved by:
--   1. Disabling FK enforcement for the swap.
--   2. Building the new table with the same column types.
--   3. Copying every row.
--   4. Dropping the old table and renaming.
-- Forward-only; bumps user_version to 4.

PRAGMA foreign_keys = OFF;

CREATE TABLE categories_new (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('fixed', 'variable', 'investing')),
  monthly_target_cents INTEGER CHECK (monthly_target_cents IS NULL OR monthly_target_cents >= 0),
  is_recurring INTEGER NOT NULL DEFAULT 0 CHECK (is_recurring IN (0, 1)),
  recurrence_day_of_month INTEGER CHECK (
    recurrence_day_of_month IS NULL
    OR (recurrence_day_of_month BETWEEN 1 AND 31)
  ),
  is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
  is_seed INTEGER NOT NULL DEFAULT 0 CHECK (is_seed IN (0, 1))
);

INSERT INTO categories_new (id, name, kind, monthly_target_cents, is_recurring, recurrence_day_of_month, is_active, is_seed)
SELECT id, name, kind, monthly_target_cents, is_recurring, recurrence_day_of_month, is_active, is_seed
FROM categories;

DROP TABLE categories;
ALTER TABLE categories_new RENAME TO categories;

INSERT OR IGNORE INTO categories (name, kind, is_recurring, is_active, is_seed) VALUES
  ('Savings',    'investing', 0, 0, 1),
  ('401k',       'investing', 0, 0, 1),
  ('Investing',  'investing', 0, 0, 1),
  ('Roth IRA',   'investing', 0, 0, 1);

PRAGMA foreign_keys = ON;
PRAGMA user_version = 4;
