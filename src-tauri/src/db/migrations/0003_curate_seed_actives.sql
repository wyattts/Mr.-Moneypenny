-- Mr. Moneypenny migration 0003: curate the default-active set for
-- seeded categories so existing v0.1.1 users land on the same starting
-- point as fresh v0.1.2 installs.
--
-- Forward-only. Bumps user_version to 3 on success.
--
-- Conservative deactivation rule: only flip is_active=0 for a seeded
-- category if the user has not engaged with it. "Engaged" means either
-- (a) it has at least one expense logged against it, or (b) the user
-- has set a monthly_target on it. User-created categories (is_seed=0)
-- are never touched by this migration.

UPDATE categories
SET is_active = 0
WHERE is_seed = 1
  AND name NOT IN (
    'Rent / Mortgage',
    'Renters / Home Insurance',
    'Health Insurance',
    'Auto Insurance',
    'Phone',
    'Internet',
    'Groceries',
    'Dining Out',
    'Transportation / Gas',
    'Entertainment',
    'Personal Care',
    'Clothing',
    'Household',
    'Misc'
  )
  AND monthly_target_cents IS NULL
  AND id NOT IN (
    SELECT DISTINCT category_id FROM expenses WHERE category_id IS NOT NULL
  );

PRAGMA user_version = 3;
