-- Mr. Moneypenny migration 0001: initial schema.
-- Forward-only. Bumps user_version to 1 on success.
--
-- Money is stored in integer cents. Never floats.
-- Dates are ISO8601 strings in UTC. The application converts to local
-- time only for display.

CREATE TABLE telegram_authorized_chats (
  chat_id INTEGER PRIMARY KEY,
  display_name TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'member')),
  added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE telegram_pending_pairings (
  pairing_code TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('fixed', 'variable')),
  monthly_target_cents INTEGER CHECK (monthly_target_cents IS NULL OR monthly_target_cents >= 0),
  is_recurring INTEGER NOT NULL DEFAULT 0 CHECK (is_recurring IN (0, 1)),
  recurrence_day_of_month INTEGER CHECK (
    recurrence_day_of_month IS NULL
    OR (recurrence_day_of_month BETWEEN 1 AND 31)
  ),
  is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
  is_seed INTEGER NOT NULL DEFAULT 0 CHECK (is_seed IN (0, 1))
);

CREATE TABLE expenses (
  id INTEGER PRIMARY KEY,
  amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
  currency TEXT NOT NULL DEFAULT 'USD',
  category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
  description TEXT,
  occurred_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  source TEXT NOT NULL CHECK (source IN ('telegram', 'manual')),
  raw_message TEXT,
  llm_confidence REAL CHECK (
    llm_confidence IS NULL
    OR (llm_confidence >= 0.0 AND llm_confidence <= 1.0)
  ),
  logged_by_chat_id INTEGER REFERENCES telegram_authorized_chats(chat_id) ON DELETE SET NULL
);

CREATE INDEX idx_expenses_occurred_at ON expenses(occurred_at);
CREATE INDEX idx_expenses_category_id ON expenses(category_id);
CREATE INDEX idx_expenses_logged_by_chat_id ON expenses(logged_by_chat_id);

CREATE TABLE budgets (
  id INTEGER PRIMARY KEY,
  category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
  period TEXT NOT NULL CHECK (period IN ('weekly', 'monthly', 'yearly')),
  effective_from TEXT NOT NULL,
  effective_to TEXT
);

CREATE INDEX idx_budgets_category_id ON budgets(category_id);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE telegram_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  last_update_id INTEGER NOT NULL DEFAULT 0
);

INSERT INTO telegram_state (id) VALUES (1);

PRAGMA user_version = 1;
