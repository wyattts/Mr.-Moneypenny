-- Mr. Moneypenny migration 0010: LLM API usage log.
--
-- One row per successful LLM chat() call. The cost is computed at insert
-- time from a hardcoded price table (see `llm/pricing.rs`) so historical
-- totals don't drift if pricing changes later.
--
-- `cost_micros` is integer micro-dollars (1 USD = 1,000,000 micros).
-- This gives us four extra decimal places past cents — enough that even
-- per-token sub-cent costs (cache reads on Haiku are $0.10/Mtok, i.e.
-- 0.1 micro-dollars per token) round meaningfully when summed.
--
-- `provider` distinguishes Anthropic (paid, real cost_micros) from Ollama
-- (free local inference, cost_micros = 0; rows still useful for "calls
-- today" counts).
--
-- Forward-only; bumps user_version to 10.

CREATE TABLE llm_usage (
  id INTEGER PRIMARY KEY,
  provider TEXT NOT NULL,                   -- "anthropic" | "ollama"
  model TEXT NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,   -- micro-dollars, 1 USD = 1_000_000
  occurred_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_llm_usage_occurred ON llm_usage(occurred_at);

PRAGMA user_version = 10;
