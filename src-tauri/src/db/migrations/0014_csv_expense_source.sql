-- Mr. Moneypenny migration 0014: allow 'csv' as an expense source.
--
-- v0.3.2 introduces the CSV importer. Imported expenses get
-- `source = 'csv'` for traceability (the user can later filter or
-- bulk-delete them by source if they want to redo an import).
--
-- The expenses table currently enforces `source IN ('telegram',
-- 'manual')`. SQLite can't ALTER a CHECK constraint in place, so we
-- repeat the table-recreate dance from migration 0006.
--
-- Forward-only; bumps user_version to 14.

PRAGMA foreign_keys = OFF;

CREATE TABLE expenses_new (
  id INTEGER PRIMARY KEY,
  amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
  currency TEXT NOT NULL DEFAULT 'USD',
  category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
  description TEXT,
  occurred_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  source TEXT NOT NULL CHECK (source IN ('telegram', 'manual', 'csv')),
  raw_message TEXT,
  llm_confidence REAL CHECK (
    llm_confidence IS NULL
    OR (llm_confidence >= 0.0 AND llm_confidence <= 1.0)
  ),
  logged_by_chat_id INTEGER REFERENCES telegram_authorized_chats(chat_id) ON DELETE SET NULL,
  is_refund INTEGER NOT NULL DEFAULT 0 CHECK (is_refund IN (0, 1)),
  refund_for_expense_id INTEGER REFERENCES expenses(id) ON DELETE SET NULL
);

INSERT INTO expenses_new (
  id, amount_cents, currency, category_id, description, occurred_at,
  created_at, source, raw_message, llm_confidence, logged_by_chat_id,
  is_refund, refund_for_expense_id
)
SELECT
  id, amount_cents, currency, category_id, description, occurred_at,
  created_at, source, raw_message, llm_confidence, logged_by_chat_id,
  is_refund, refund_for_expense_id
FROM expenses;

DROP TABLE expenses;
ALTER TABLE expenses_new RENAME TO expenses;

CREATE INDEX idx_expenses_occurred_at ON expenses(occurred_at);
CREATE INDEX idx_expenses_category_id ON expenses(category_id);
CREATE INDEX idx_expenses_logged_by_chat_id ON expenses(logged_by_chat_id);
CREATE INDEX idx_expenses_refund_for ON expenses(refund_for_expense_id)
  WHERE refund_for_expense_id IS NOT NULL;

PRAGMA foreign_keys = ON;
PRAGMA user_version = 14;
