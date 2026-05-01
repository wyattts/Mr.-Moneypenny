-- Mr. Moneypenny migration 0013: merchant categorization rules.
--
-- Bank/CC exports rarely have categories that match Mr. Moneypenny's.
-- Rather than round-trip every CSV row through the LLM (expensive),
-- the importer matches each merchant string against this rules table
-- and auto-applies the saved category.
--
-- Patterns are SQLite GLOB / LIKE-style globs. `STARBUCKS%` matches all
-- of "STARBUCKS #4521 SEATTLE WA", "STARBUCKS #6789 OAKLAND CA", etc.
-- Patterns get populated automatically by the import wizard's review
-- screen — every category-pick the user makes for an unmatched merchant
-- becomes a saved rule for the next import.
--
-- `default_is_refund` lets a rule pre-flip the refund bit (e.g., a
-- "AMAZON RETURN" rule auto-marks matching rows as refunds in addition
-- to whatever the negative-amount detection catches).
--
-- `priority` resolves first-match-wins ordering when multiple patterns
-- match a single merchant string. Higher priority wins; ties broken by
-- recency (newer rule wins).
--
-- Forward-only; bumps user_version to 13.

CREATE TABLE merchant_rules (
  id INTEGER PRIMARY KEY,
  pattern TEXT NOT NULL,
  category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  default_is_refund INTEGER NOT NULL DEFAULT 0
    CHECK (default_is_refund IN (0, 1)),
  priority INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE INDEX idx_merchant_rules_priority
  ON merchant_rules(priority DESC, created_at DESC);

CREATE INDEX idx_merchant_rules_category
  ON merchant_rules(category_id);

PRAGMA user_version = 13;
