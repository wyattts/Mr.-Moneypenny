-- Mr. Moneypenny migration 0008: user-defined recurring expense rules.
--
-- Each row is a user spec ("Netflix $15.49 monthly on the 7th"). The
-- scheduler creates a corresponding row in `scheduled_jobs` of kind
-- `recurring_expense` with payload `{"rule_id": <id>}` to fire it.
-- Deleting a rule cascades to its job(s).
--
-- `mode = 'confirm'` (default): when the rule fires, the bot DMs the
-- owner asking yes/no/skip. The user's next message is captured by the
-- router via `pending_recurring_confirmations` and either inserts an
-- expense or skips.
--
-- `mode = 'auto'`: the rule fires silently — used for true auto-pay
-- expenses (rent on draft, etc.) the user has already validated.
--
-- Forward-only; bumps user_version to 8.

CREATE TABLE recurring_rules (
  id INTEGER PRIMARY KEY,
  label TEXT NOT NULL,
  amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
  currency TEXT NOT NULL DEFAULT 'USD',
  category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  frequency TEXT NOT NULL CHECK (frequency IN ('monthly', 'weekly', 'yearly')),
  -- monthly: 1-31 (clamped to last day for short months)
  -- weekly:  1-7 (Mon=1 .. Sun=7)
  -- yearly:  1-366 (day-of-year)
  anchor_day INTEGER NOT NULL CHECK (anchor_day BETWEEN 1 AND 366),
  mode TEXT NOT NULL CHECK (mode IN ('confirm', 'auto')) DEFAULT 'confirm',
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_recurring_rules_category ON recurring_rules(category_id);

-- One pending confirmation per chat at a time. If a second rule fires
-- while one is outstanding, the scheduler returns Retry and tries again
-- next tick (after the user has answered).
CREATE TABLE pending_recurring_confirmations (
  chat_id INTEGER PRIMARY KEY
    REFERENCES telegram_authorized_chats(chat_id) ON DELETE CASCADE,
  rule_id INTEGER NOT NULL REFERENCES recurring_rules(id) ON DELETE CASCADE,
  asked_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

PRAGMA user_version = 8;
