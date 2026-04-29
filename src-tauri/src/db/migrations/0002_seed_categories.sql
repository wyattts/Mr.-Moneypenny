-- Mr. Moneypenny migration 0002: seed default categories.
-- All seed rows have is_seed=1 so the GUI can mark them and so the user
-- can deactivate (but typically not delete) them. INSERT OR IGNORE skips
-- categories with names that already exist (e.g., on a re-applied seed).
--
-- is_active is set explicitly per row. The "default-on" set is the
-- 14 categories most households actually use; everything else seeds
-- inactive and is one click away in the Categories view. See migration
-- 0003 for the matching upgrade path for v0.1.1 installs.

INSERT OR IGNORE INTO categories (name, kind, is_recurring, is_active, is_seed) VALUES
  -- Fixed (recurring monthly) ----------------------------------------
  ('Rent / Mortgage',           'fixed', 1, 1, 1),
  ('Renters / Home Insurance',  'fixed', 1, 1, 1),
  ('Health Insurance',          'fixed', 1, 1, 1),
  ('Auto Insurance',            'fixed', 1, 1, 1),
  ('Phone',                     'fixed', 1, 1, 1),
  ('Internet',                  'fixed', 1, 1, 1),
  ('Streaming Subscriptions',   'fixed', 1, 0, 1),
  ('Software Subscriptions',    'fixed', 1, 0, 1),
  ('Gym Membership',            'fixed', 1, 0, 1),
  ('Loan Payments',             'fixed', 1, 0, 1),
  ('Childcare',                 'fixed', 1, 0, 1),
  ('Tuition',                   'fixed', 1, 0, 1),
  -- Variable (discretionary) -----------------------------------------
  ('Groceries',                 'variable', 0, 1, 1),
  ('Dining Out',                'variable', 0, 1, 1),
  ('Coffee',                    'variable', 0, 0, 1),
  ('Transportation / Gas',      'variable', 0, 1, 1),
  ('Rideshare',                 'variable', 0, 0, 1),
  ('Public Transit',            'variable', 0, 0, 1),
  ('Entertainment',             'variable', 0, 1, 1),
  ('Personal Care',             'variable', 0, 1, 1),
  ('Clothing',                  'variable', 0, 1, 1),
  ('Household',                 'variable', 0, 1, 1),
  ('Gifts',                     'variable', 0, 0, 1),
  ('Travel',                    'variable', 0, 0, 1),
  ('Healthcare Out-of-Pocket',  'variable', 0, 0, 1),
  ('Pets',                      'variable', 0, 0, 1),
  ('Hobbies',                   'variable', 0, 0, 1),
  ('Charity',                   'variable', 0, 0, 1),
  ('Misc',                      'variable', 0, 1, 1);

PRAGMA user_version = 2;
