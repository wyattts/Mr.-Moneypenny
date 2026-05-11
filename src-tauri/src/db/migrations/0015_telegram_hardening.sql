-- Mr. Moneypenny migration 0015: Telegram pairing-code hardening +
-- update-processing idempotency.
--
-- Three concerns, one migration, one txn (wrapped by the runner):
--
--   1. Per-chat redemption rate limit. Tracks how many wrong codes a
--      given chat_id has tried in the current 60s window and applies an
--      exponential cooldown (30s -> 1min -> 5min -> 30min, capped) once
--      they exceed the threshold. Row is cleared on a successful
--      redemption.
--
--   2. Pairing-code "kind" (owner|member). Ownership can no longer be
--      claimed by whoever happens to redeem first on a fresh install --
--      only a code explicitly issued as kind='owner' grants Owner.
--      Existing pending rows (if any) default to 'member'.
--
--   3. Update-processing idempotency. The poller's previous
--      handle_update -> persist_offset sequence had a crash window that
--      duplicated expenses on restart. The new table is the source of
--      truth for "already-processed" so the poller can skip work it has
--      already done.

CREATE TABLE telegram_redemption_attempts (
  chat_id INTEGER PRIMARY KEY,
  attempts INTEGER NOT NULL DEFAULT 0,
  window_start TEXT NOT NULL,
  blocked_until TEXT,
  -- 0 = no cooldown applied yet, increments each time the threshold is
  -- tripped. Indexes into the PER_CHAT_COOLDOWNS ladder in auth.rs.
  -- Resets implicitly when the row is deleted on a successful redeem.
  cooldown_level INTEGER NOT NULL DEFAULT 0
);

-- Single-row counter for the global per-minute ceiling. id=1 is the
-- only allowed row (see CHECK). `window_start` rolls forward each
-- minute; `blocked_until` non-NULL means redemptions are paused for
-- everyone.
CREATE TABLE telegram_redemption_global (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  failures_in_window INTEGER NOT NULL DEFAULT 0,
  window_start TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  blocked_until TEXT
);
INSERT INTO telegram_redemption_global (id) VALUES (1);

-- Add the kind column to existing pending pairings. Default 'member'
-- keeps legacy pending rows from accidentally granting Owner.
ALTER TABLE telegram_pending_pairings
  ADD COLUMN kind TEXT NOT NULL DEFAULT 'member'
  CHECK (kind IN ('owner', 'member'));

CREATE TABLE processed_telegram_updates (
  update_id INTEGER PRIMARY KEY,
  processed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_processed_telegram_updates_processed_at
  ON processed_telegram_updates(processed_at);

PRAGMA user_version = 15;
