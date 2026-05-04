# Mr. Moneypenny v0.3.7 — Code Audit

**Audit date:** 2026-05-04
**Scope:** Full codebase (~28k LOC: Rust backend, React/TypeScript frontend, 14 SQL migrations, build pipeline)
**Method:** Read-only audit. Ten module-focused dives in parallel, plus cross-cutting privacy / redundancy / test-coverage / supply-chain sweeps. No code modified.
**Severity scale:** Critical (ship-blocker) · High (exploitable with positioning OR broad-blast-radius bug) · Medium (defense-in-depth or limited-blast-radius bug) · Low (best-practice gap, no current exploit) · Info (observation, no remediation required)

---

## Executive summary

```
Critical: 0
High:     6
Medium:  33
Low:     47
Info:    44
─────────────
Total:  130 findings across 12 categories
```

**Posture: solid.** No Critical findings. The architecture is sound — secrets are properly encrypted at rest with a sound AEAD, all SQL is parameterized, the IPC trust boundary is small (58 commands) and uses minimal Tauri capabilities, no `unsafe` Rust, no `any` TypeScript, no raw `invoke()` outside the typed binding layer, and lockfiles + dependency hygiene are excellent. The Telegram bot token, the Anthropic API key, and the user's expense data are all kept on-device with sensible defaults.

The six **High** findings are concentrated in three structural areas:

1. **Telegram authentication is under-defended.** The 6-digit pairing code has no rate-limit/lockout (≈30% takeover probability over a few legitimate setup attempts at modest brute-force rates), and the bot token can leak into logs via `reqwest::Error::Display` on transport failures.
2. **Migrations are not transactional.** Four table-recreate migrations (0004, 0006, 0011, 0014) and the bulk CSV importer lack `BEGIN…COMMIT` wrappers — partial failure leaves the DB unrecoverable on next launch.
3. **AGPL §6 compliance and basic accessibility are gapped.** Shipped binaries carry no source-offer notice (copyleft obligation), and every range slider in the Forecast/Simulator/Debt views is missing `aria-label`/`aria-valuetext` (the entire view is unusable with a screen reader).

The bulk of the Medium and Low findings are defense-in-depth: master key not zeroized in memory; LLM tool-result echo-back as a prompt-injection vector; CSV amount parser silently corrupting European-format numbers; performance smells in the Simulator slider drag and the Insights polling intervals; and a stale `keyring` migration crate that was supposed to be dropped at v0.2.7 but is still present at v0.3.7.

### Top 5 actionable items (recommended for v0.3.8)

| # | Finding | Severity | Effort |
|---|---|---|---|
| 1 | Telegram pairing-code brute force window | High | M |
| 2 | Bot token leak via `reqwest::Error::Display` in tracing logs | High | S |
| 3 | Migrations 0004/0006/0011/0014 + `csv_import_commit` not wrapped in transactions | High | S |
| 4 | CSV amount parser silently corrupts European-format amounts | Medium | S |
| 5 | AGPL §6 source-offer notice missing from shipped binaries | High | S |

These five are the highest expected value to address. (1) is the only realistically exploitable security issue; (2) is the only realistic credential-leak path; (3) is the only realistic data-loss/corruption path; (4) is the only realistic correctness bug that quietly produces wrong financial numbers for a meaningful user segment (anyone with a non-US bank); (5) is a copyleft-license obligation, not optional. (Honorable mentions: drop the dead `keyring` crate; add `aria-label` to the `NumberSlider` helper; add a soft daily Anthropic cost ceiling.)

### Posture statement (one line per category)

| # | Category | Posture | Notable |
|---|---|---|---|
| 1 | Security | **concerns** | Pairing brute force + token-in-logs are real |
| 2 | Crypto correctness | **good** | AEAD/KDF sound; only zeroization gap |
| 3 | Privacy (PII) | **good** | No financial PII in logs; one log-PII vector confirmed |
| 4 | SQL / DB integrity | **concerns** | Migrations not transactional (partial-failure risk) |
| 5 | Concurrency / panic | **good** | No deadlock paths; `unwrap` density misreported by recon (most are test-only) |
| 6 | Performance | **fair** | Simulator slider thrash; no `spawn_blocking`; N+1 budget alerts |
| 7 | Portability | **good** | Cross-platform code minimal and well-justified |
| 8 | Compliance | **concerns** | AGPL §6 source-offer + NOTICES bundle missing |
| 9 | Redundancy / dead code | **good** | Stale `keyring` crate is the only material item |
| 10 | Test coverage + migrations | **fair** | `commands.rs` has zero unit tests; no proptest/fuzz |
| 11 | Build / release pipeline | **good** | Lockfiles, signed AppImage, `cargo audit` blocking |
| 12 | Frontend hygiene | **fair** | a11y gaps on sliders; modal lacks focus-trap; no lazy-loading |

---

## Recon corrections

Several "smells" the pre-audit recon flagged turned out to be misattributions:

- **"43 unwraps in `secrets/store.rs`":** all 35 are inside `#[cfg(test)]`. Production has zero unwraps in this file.
- **"40 unwraps in `telegram/auth.rs`":** all in `#[cfg(test)]`. Production is panic-free.
- **"36 unwraps in `db/mod.rs`":** all in `#[cfg(test)]`.
- **"21 unwraps in `scheduler/mod.rs`":** 18 are test-only; 3 are production `Mutex::lock().unwrap()` (mutex-poisoning idiom).
- **"44 unwraps in `commands.rs`":** all 44 are `state.db.lock().unwrap()` (mutex-poisoning idiom; same value).
- **"Large comment block at `lib.rs:1860`":** `lib.rs` is 300 lines. No such block exists.
- **"`rusqlite` dual-licensed AGPL OR Unlicense":** rusqlite 0.32.1 is single-licensed **MIT**. No license-compatibility risk.

Net effect: the panic surface is much smaller than the recon implied. The remaining concern is mutex-poisoning propagation (cluster of `Mutex::lock().unwrap()` calls in commands + scheduler), which is the standard idiom but worth migrating to `parking_lot::Mutex` (no poisoning) at some point.

---

## 1. Security

**Verdict.** Architecture is sound (parameterized SQL throughout, minimal capabilities, locked-down CSP, no `unsafe`), but the Telegram pairing flow is under-defended and one credential-leak path through tracing logs justifies pre-release fixes.

### High

#### S-1 · Telegram pairing-code brute force inside the 10-minute TTL with no rate limit, lockout, or attempt cap
- **Severity:** High · **Effort:** M
- **Files:** `src-tauri/src/telegram/auth.rs:23,72,92-161`, `src-tauri/src/telegram/router.rs:55-57,114-138`, `src-tauri/src/db/migrations/0001_init.sql:15-19`
- `redeem_pairing_code` accepts unlimited `/start <code>` attempts per chat with no per-chat counter, no cooldown, no global ceiling. Code space = 10⁶, TTL = 600s.
  - 30 msg/s × 600s = **18,000 attempts/window ⇒ ~1.8% hit per outstanding code**.
  - 100 msg/s (multi-account) × 600s = **60,000 attempts ⇒ ~6%/window, ~30% over a small handful of legitimate setup attempts.**
  - Bot username is publicly enumerable on Telegram; an attacker only needs to win once.
  - In a fresh-install scenario where no owner exists yet, the attacker becomes the household **OWNER**. After ownership is established, a winning attacker becomes a Member, which still authorizes them to drive the LLM agentic loop (`add_expense`, `delete_expense`, `query_expenses`, etc.).
- **Recommendation:** Add three layers in `auth.rs`: (1) per-chat-id attempt counter with exponential cooldown after 5 wrong codes in 60s, persisted in a small `telegram_redemption_attempts(chat_id, attempts, blocked_until)` table; (2) global ceiling — 10 invalid attempts/minute pause all redemptions for 30s; (3) extend code to 8–10 digits or base32 (≥40 bits entropy). Plus: harden ownership transfer by requiring the desktop UI to explicitly mark a code as "this is the household-owner code" — random-member windows should not be hijackable into ownership.

#### S-2 · Bot token leak via `reqwest::Error::Display` in tracing logs
- **Severity:** High · **Effort:** S
- **Files:** `src-tauri/src/telegram/client.rs:45-47,49-69`, `src-tauri/src/telegram/poller.rs:69-87`, `src-tauri/src/lib.rs:45`
- The bot token is in the URL path (`/bot{token}/{method}` per the Telegram Bot API standard). When `reqwest::Client::send()` fails on transport (connect refused, DNS, timeout, TLS), `reqwest::Error::Display` includes the URL on most error variants. The poller logs `error=%e` at `tracing::warn`/`tracing::error`, and `tracing_subscriber::fmt::try_init()` writes to stderr by default. Any user running from a terminal, capturing stderr in a launcher, piping to a file, or using journald will store the bot token in plaintext on disk. The 512-char body truncation in `client.rs:67` does NOT affect URL-bearing transport errors.
- **Recommendation:** Catch `reqwest::Error` and emit a sanitized error (`{method} request failed: {kind}` with no URL); or add `.map_err(scrub_token)` that strips `/bot[^/]+/` patterns. Also tighten `tracing_subscriber` to suppress `reqwest=debug`. Cross-reference: A4 in §3 (privacy sweep) flags the same site at Medium severity — promote to High to match this finding.

### Medium

#### S-3 · `redeem_pairing_code` returns "expired" vs "invalid" oracle
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/telegram/auth.rs:122-131`, `src-tauri/src/telegram/router.rs:133-136`, `src-tauri/src/telegram/formatter.rs:79-81`
- Distinct error strings ("pairing code expired" vs "invalid or expired pairing code") for matched-but-expired vs no-match give a brute-forcer feedback that a real code recently existed in the same digit-pattern. Compounds S-1.
- **Recommendation:** Collapse both variants to one user-facing string. Keep distinction in `tracing::debug` for support.

#### S-4 · `save_ollama_config` / `list_ollama_models` accept arbitrary URL with no validation
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/commands.rs:152-208`, `src-tauri/src/commands.rs:1180`
- `endpoint: String` is written to `settings::OLLAMA_ENDPOINT` and later used as an outbound `reqwest` GET target with no scheme allowlist, no length cap, no host check. CSP allowlists localhost Ollama for the *webview*, but reqwest in Rust is **not** subject to CSP — any URL the user types becomes a real outbound HTTP target. A misconfigured value silently exfiltrates merchant strings (CSV ai_suggest path).
- **Recommendation:** `url::Url::parse`, scheme ∈ {`http`, `https`}, length cap 2048, restrict host to localhost/private ranges or require an explicit "remote endpoint" opt-in.

#### S-5 · Update offset persisted *after* `handle_update`; crash window allows duplicate logs
- **Severity:** Medium · **Effort:** M
- **Files:** `src-tauri/src/telegram/poller.rs:64-79,111-118`
- If `handle_update` commits an `INSERT INTO expenses` and the process crashes/SIGKILLs before `persist_offset` finishes, on restart `getUpdates(offset = last_processed + 1)` re-fetches the same update and re-runs the LLM agentic loop — likely double-inserting the expense.
- **Recommendation:** Either use a SQLite txn that bumps `telegram_state.last_update_id` and inserts the expense atomically, or add a `processed_telegram_updates(update_id PRIMARY KEY)` idempotency table with `INSERT OR IGNORE`.

### Low

#### S-6 · `chat_id` logged at warn/error in tracing events
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/telegram/router.rs:230,250`, `src-tauri/src/telegram/poller.rs:69-74`
- Single-user laptop mitigates the impact, but conflicts with "privacy-first" framing when users export logs for support.
- **Recommendation:** Hash/truncate (`fmt_short_chat`) at warn/error levels.

#### S-7 · `handle_undo` uses SQLite `datetime('now')` ignoring injected `_now` parameter
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/telegram/router.rs:140-186`
- Breaks testability and time-travel correctness; clock drift causes silent disagreement between agent's `now` and SQL filter's `now`.
- **Recommendation:** Bind `OffsetDateTime` as a parameter; remove the `_` prefix.

#### S-8 · No upper bound on concurrently pending pairing codes
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/telegram/auth.rs:75-86,237-243`
- Local-only concern; no wire path to `generate_pairing_code`. With the brute-force window above, more concurrent codes ⇒ higher hit probability.
- **Recommendation:** Refuse new codes when pending count ≥ 5.

### Info

#### S-9 · CSP allowlists are minimal and correct
- `connect-src` exactly: `'self' ipc: api.telegram.org api.anthropic.com localhost:11434 127.0.0.1:11434 api.github.com github.com objects.githubusercontent.com`. Capabilities: `core:default + updater:default` only. No `fs:`, no `shell:`, no `dialog:`, no `http:` plugin.

#### S-10 · `script-src 'self'` is correct; no `unsafe-eval`. `style-src 'self' 'unsafe-inline'` is required by Recharts SVG inline-style injection (verified). `img-src 'self' data: asset: http://asset.localhost` is correct.

#### S-11 · `lib.rs` plugin order is correct (single-instance first) and `#![forbid(unsafe_code)]` is at crate root.

#### S-12 · Single-instance plugin makes idempotency-on-double-launch verifiable
- A second launch hands argv to the running process and exits, so two scheduler tasks in the same DB cannot occur unless the user runs two builds with different config dirs.

---

## 2. Crypto correctness

**Verdict.** Sound. AEAD nonce math safe, KDF derivation correct, file permissions enforced. Three Medium defense-in-depth gaps (zeroization, atomic-write durability, keyring migration probe).

### Medium

#### C-1 · Master key never zeroized in memory
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/secrets/store.rs:62-67`, `kdf.rs:42-56`, `cipher.rs:16-25`
- `SecretsFile::master_key: [u8; 32]` lives for process lifetime in `OnceLock<Mutex<SecretsFile>>` with default `Drop`. Intermediate KDF buffers, the cloned `Key` inside `ChaCha20Poly1305::new`, and the `out` array all rely on default Drop (no-op). On a system with swap or after a core dump, the master key — and therefore every stored secret — is recoverable. `chacha20poly1305 0.10` does not zeroize-on-drop unless its `zeroize` feature is enabled. The `zeroize` crate is not in the dep tree.
- **Recommendation:** Add `zeroize = "1"`, wrap `master_key` in `Zeroizing<[u8;32]>`, enable `chacha20poly1305/zeroize` feature.

#### C-2 · `save_atomic` swallows `sync_all` error; no parent-directory fsync
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/secrets/store.rs:160-187`
- `f.sync_all().ok()` discards errors; after `rename`, parent directory not fsync'd. POSIX `rename` is atomic w.r.t. readers but durability requires parent-dir fsync. On crash between rename and journal commit, EXT4/XFS users can be left with the old file plus a fresh `secrets.bin.tmp` containing the new contents (or worse: zero-length new file). The doc comment promises atomicity that the implementation under-delivers.
- **Recommendation:** Propagate `sync_all` with `?`; after `fs::rename`, open parent dir and `sync_all` on its handle (Unix only).

#### C-3 · Keyring migration probe runs on every miss for every key, forever
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/secrets/mod.rs:78-93`, `migration.rs:31-58`
- `retrieve()` calls `try_copy_from_keyring` whenever the disk store has no entry — for every retrieval of a missing key, forever. On Linux this is a `dbus` call into the Secret Service for each unset secret. Doc comment says "first call after upgrading from v0.2.6" but the code actually runs on every miss.
- **Recommendation:** Eagerly drain the keyring inside `SecretsFile::open` (loop both known key names), write a sentinel, never enter the migration path again. Cross-references the keyring removal in §9.

### Low

#### C-4 · `data_dir` fed to KDF via `to_string_lossy()` — non-UTF-8 paths produce non-reversible mapping
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/secrets/mod.rs:45-49`, `kdf.rs:42-56`
- Two distinct invalid Linux paths can map to the same lossy string, producing the same master key. If a future Rust update preserves bytes correctly, the derived key changes silently and stored secrets become undecryptable.
- **Recommendation:** Hash raw `OsStr` bytes (`as_bytes()` on Unix, `encode_wide()` on Windows) into the IKM.

#### C-5 · Plaintext key names ("anthropic_api_key", "telegram_bot_token") visible in JSON
- **Severity:** Low · **Effort:** M
- **Files:** `src-tauri/src/secrets/store.rs:44-58`
- Information leak (which services have secrets) but not a confidentiality break.
- **Recommendation:** Optional defense-in-depth; HMAC the key name with a name-binding subkey if hardening.

#### C-6 · `handle()` opens secrets file twice on cold-start race
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/secrets/mod.rs:54-64`
- Two callers race; both run KDF + read file, one wins `get_or_init`. Cosmetic.

#### C-7 · `encrypt`/`decrypt` allocates fresh `ChaCha20Poly1305` cipher per call
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/secrets/cipher.rs:16-41`
- Tied to C-1 zeroization fix.

### Info

#### C-8 · Nonce uniqueness math sound
- 12-byte random nonces; realistic ~2^10 lifetime encryptions per user. P(collision) ≈ 2^-76. No change needed; recording so a future contributor doesn't "fix" it with a counter.

#### C-9 · KDF input format (colon-separator) relies on machine_uid being hex-only
- Add a comment noting the constraint; non-exploitable.

#### C-10 · License compatibility of crypto deps fine
- `chacha20poly1305 0.10`, `hkdf 0.12`, `sha2 0.10`, `keyring 3`, `machine-uid 0.5` — all MIT or MIT/Apache-2.0. Compatible with AGPL-3.0-or-later.

---

## 3. Privacy (PII handling)

**Verdict.** No financial PII reaches the log stream; secret material never logged. One realistic exfiltration path (telegram bot token in reqwest URL — same as S-2 above). One structural retention concern (`expenses.raw_message` retained indefinitely with no scrub command).

### What user data leaves the device when Anthropic is selected

| Data | Sent? | Where |
|---|---|---|
| User's free-text Telegram message (verbatim) | **Yes** | router.rs:222–223 → anthropic.rs:215 |
| Expense `description` field | **Yes** (echoed via `query_expenses` tool result) | dispatcher.rs:309, 344 |
| Merchant strings (CSV import AI-suggest path) | **Yes** | csv_import/ai_suggest.rs |
| Category names | **Yes** | system_prompt.rs:107–126 (volatile block, every turn) |
| Authorized chat display name | **Yes** | system_prompt.rs:91–97 |
| Authorized chat role ("owner"/"member") | **Yes** | system_prompt.rs:91–97 |
| Household member display names | **Yes** | system_prompt.rs:99–105 |
| Currency code | **Yes** | system_prompt.rs:89 |
| Telegram `chat_id` integers | **Yes** (when `list_household_members` invoked) | dispatcher.rs:432–451 |
| Local date/time | **Yes** | system_prompt.rs:84–88 |
| Per-category monthly budget targets | **Yes** | dispatcher.rs:400 |
| Expense amounts/dates/refund flags | **Yes** | dispatcher.rs:329–334 |
| Recurring-rule labels | **Yes** | dispatcher.rs:577 |
| Anthropic API key | **Yes** (every request as `x-api-key` header) | anthropic.rs:61 |
| Raw Telegram message (`raw_message` column) | **No** (`#[serde(skip)]`) | dispatcher.rs:309 |
| Per-call LLM confidence scores | **No** (`#[serde(skip)]`) | dispatcher.rs:310 |
| File paths, OS user, hostname | **No** | not collected |

When Ollama is selected, none of this leaves localhost.

### Medium

#### P-1 · Bot token leak via `reqwest::Error::Display` (cross-reference S-2)
- Already documented at High in §1 (Security). Listed here for completeness — privacy sweep independently confirmed this is the single realistic exfiltration path.

### Low

#### P-2 · `expenses.raw_message` retained indefinitely with no scrub command
- **Severity:** Low · **Effort:** S
- **Files:** `repository/expenses.rs`, `commands.rs:510` (Ledger search reaches `raw_message LIKE`)
- For Telegram-routed messages, `raw_message: None` is correctly used (audited; verified). For CSV imports `commands.rs:1268` writes `row.merchant.clone()` — these are merchant names, not free-text. For recurring rule fires, the field stores `format!("recurring rule #{}", rule.id)` (innocuous).
- The concern is that any free-text Telegram message that DOES end up in `raw_message` (none in current code paths, but a future feature could) has no rotation/scrubbing policy. The Ledger search bar already queries the column with `LIKE`.
- **Recommendation:** (a) Document in README/SECURITY.md — "Telegram raw text would be stored as-is for ledger filtering if added"; (b) Add a one-click "scrub raw_message older than N months" maintenance command symmetrical with the existing `PRIVACY_MODE` setting.

#### P-3 · `chat_id` logged at warn/error (cross-reference S-6)
- See §1 for full description.

#### P-4 · LLM ToS / User Content posture undocumented in-app
- **Severity:** Low · **Effort:** S
- **Files:** `llm/anthropic.rs`, `llm/system_prompt.rs`, README, Settings UI
- Project markets "privacy-first" but Settings → API Provider lets users pick Anthropic without disclosing what data crosses the wire. GDPR-hygiene gap for EU/UK users.
- **Recommendation:** Add Settings disclosure paragraph: "Selecting Anthropic sends your Telegram messages, expense descriptions, category names, and household member names to api.anthropic.com over TLS. Anthropic does not train on this data per their commercial ToS, but it does leave your machine. Pick Ollama for fully local processing."

### Info

#### P-5 · `payload = %job.payload` in scheduler logs
- **Files:** `scheduler/recurring.rs:49-54`
- Today payload is `{"rule_id": N}` — system-generated. Structurally fine; flagged because future `JobKind`s could put user text into payload.
- **Recommendation:** Replace with `payload_keys = ?json_keys(...)` or drop the field.

#### P-6 · Only one `eprintln!`/`println!` in non-test code (`main.rs:14`, headless stub) — confirmed clean.

#### P-7 · localStorage stores only `moneypenny.theme` (auto/light/dark). No PII, no secrets.

#### P-8 · LLM system prompt has no hardcoded user data; only butler persona + tool-selection rules.

#### P-9 · Conversation history bounded by turn count (24) but not payload size; one tool_result can be hundreds of KB.
- **Files:** `telegram/state.rs:16-19`, `llm/anthropic.rs:194-204`
- Cache-amortizable via prompt caching; first call after a 5-min idle pays full token cost.
- **Recommendation:** Per-message size cap (8 KB) on `tool_result` content stored in conversation history.

#### P-10 · `llm_usage` table contains zero PII (provider, model, token counts, cost_micros, occurred_at).

---

## 4. SQL / DB integrity

**Verdict.** Schema and FK story are correct; all SQL is parameterized; cascade behavior verified. The one structural gap is **migrations are not transactional** — partial-failure recovery is broken.

### Migration replay table

| Migration | Idempotent (gated by `user_version`) | Self-replay safe | Forward-safe on data | FK preserved | Notes |
|---|---|---|---|---|---|
| 0001_init.sql | yes | no (CREATE TABLE) | n/a | n/a | Initial schema |
| 0002_seed_categories.sql | yes | yes (INSERT OR IGNORE) | yes | yes | |
| 0003_curate_seed_actives.sql | yes | yes | yes (engagement test preserves user-touched rows) | yes | Tested |
| 0004_investing_kind.sql | yes | **partial** (see D-1) | yes | yes | Table-recreate; **no txn** |
| 0005_seed_electric_water.sql | yes | yes | yes | yes | |
| 0006_refunds.sql | yes | partial (see D-1) | yes | yes | Table-recreate; tightens amount_cents > 0 |
| 0007_scheduled_jobs.sql | yes | no (CREATE TABLE) | yes | n/a | |
| 0008_recurring_rules.sql | yes | no | yes | yes (cascade on rule_id) | |
| 0009_budget_alert_state.sql | yes | no | yes | yes | |
| 0010_llm_usage.sql | yes | no | yes | n/a | |
| 0011_investment_balances.sql | yes | **no** (ALTER ADD COLUMN errors) | yes | yes | Two ALTERs without txn |
| 0012_csv_import_profiles.sql | yes | no | yes | n/a | |
| 0013_merchant_rules.sql | yes | no | yes | yes (cascade on category_id) | Tested |
| 0014_csv_expense_source.sql | yes | partial (see D-1) | yes | yes | Table-recreate |

### High

#### D-1 · Migrations 0004, 0006, 0011, 0014 not wrapped in transactions; partial failure leaves DB unrecoverable
- **Severity:** High · **Effort:** S
- **Files:** `src-tauri/src/db/mod.rs:108-110`, migrations `0004_investing_kind.sql`, `0006_refunds.sql`, `0011_investment_balances.sql`, `0014_csv_expense_source.sql`
- `db::migrate` calls `conn.execute_batch(sql)`. SQLite's autocommit fires per statement, so any mid-batch failure (disk full, OOM, panic) leaves the DB in a half-applied state with `user_version` unchanged. On next launch, the runner re-applies the migration, which immediately fails on `CREATE TABLE categories_new` (or `expenses_new`) — already exists. **App crashes at startup, every startup.**
- For 0011 (two `ALTER ADD COLUMN` statements), the same shape applies: first column added but second failed → next run errors on duplicate column.
- **Recommendation:** Wrap each migration in `tx = conn.transaction(); tx.execute_batch(sql); tx.commit()`. SQLite forbids `PRAGMA foreign_keys = …` inside a transaction — keep those pragmas outside the txn. `PRAGMA user_version` must be the last statement *inside* the txn so it commits atomically with the schema change.

#### D-2 · `csv_import_commit` bulk-loop has no transaction; partial import on mid-batch failure
- **Severity:** High · **Effort:** S
- **Files:** `src-tauri/src/commands.rs:1244-1296`
- Holds the global `db.lock()` then loops `expenses::insert(...)` per row. Each insert autocommits; row 387 of a 500-row import that fails leaves 386 rows on disk. The dedupe heuristic isn't strong enough to make a retry idempotent against the partial state. Same issue for the `merchant_rules` write loop and `csv_profiles::touch` that follow.
- **Recommendation:** Wrap whole commit in `conn.unchecked_transaction()` (codebase already uses this pattern in `telegram/auth.rs`).

### Medium

#### D-3 · Recurring-rule fire path is not transactional (network-then-DB ordering)
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/scheduler/recurring.rs:93-169`
- Confirm-mode: send DM (network) → on success, insert pending row. If process killed between send and `insert_pending`, user gets a message with no DB row, and the next tick sends a duplicate prompt. `INSERT OR REPLACE` masks the duplicate but silently resets the first prompt's TTL.

#### D-4 · No composite index on `(category_id, occurred_at)`
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/repository/expenses.rs:98,166`
- `expenses::list_in_range_by_category` and `monthly_totals_for_category` both filter on `category_id = ?1 AND occurred_at >= ?2 AND occurred_at < ?3`. SQLite picks `idx_expenses_category_id` and post-filters by date. Category Analyzer hot path will be noticeably slower for users with several years of expenses in a hot category.
- **Recommendation:** Add `CREATE INDEX idx_expenses_category_occurred ON expenses(category_id, occurred_at)` in migration 0015.

#### D-5 · Settings KV reads have no caching; every UI render hits SQLite
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/repository/settings.rs:11-19`
- Each `get(conn, key)` call takes the global `db.lock()`. Multiple consumers (privacy_mode, default_currency, llm_provider, ollama_endpoint) read on every command invocation. Under load — LLM dispatcher hot path — serializes against poller and scheduler.
- **Recommendation:** Read-through cache keyed by `key`, invalidate on `set`/`delete`.

#### D-6 · `categories::get_starting_balance` swallows DB errors as `None`
- **Severity:** Medium · **Effort:** M
- **Files:** `src-tauri/src/repository/categories.rs:122-134, 139-145`
- `query_row(...).unwrap_or((None, None))`. Real DB errors (locked, schema drift) silently report "no balance" rather than propagating, flowing into investment forecast. Same pattern in `delete` reading `is_seed`.
- **Recommendation:** Replace with proper error propagation; only `Err(QueryReturnedNoRows)` is "absent."

#### D-7 · `csv_import_profiles.header_signature` is non-UNIQUE
- **Severity:** Medium · **Effort:** S
- **Files:** `migrations/0012_csv_import_profiles.sql:30-37`
- Two profiles can be saved against the same header. `find_by_signature` returns most-recent — silent UX choice.

#### D-8 · `expenses.raw_message` no retention policy (cross-reference P-2)
- See §3.

### Low

#### D-9 · LLM usage rows are PII-free but include model identifiers — informational, no action.

#### D-10 · `format!`-built SQL in repository — confirmed only constants, no data interpolation
- **Files:** All `format!` sites in `repository/{expenses,categories,budgets,recurring_rules}.rs` interpolate only `const SELECT_COLS: &str` or `SIGNED_AMOUNT_SQL`. User data flows through `params![]` exclusively. Verified clean.

#### D-11 · `db::open` enables FK / WAL / `synchronous=NORMAL` per connection — confirmed.
- App uses one shared connection (`Arc<Mutex<Connection>>`), so "every connection" = "the one connection." Tests use `db::open_in_memory` which only sets `foreign_keys` (WAL is moot in-memory; `synchronous` left at FULL — minor, not correctness).

### Info

#### D-12 · `db/mod.rs` 36 unwraps all under `#[cfg(test)]` — confirmed.

#### D-13 · Repository test coverage gaps:
- No tests for `expenses::list_in_range_by_category` (Category Analyzer hot path).
- No tests for `expenses::monthly_totals_for_category`.
- No tests for `expenses::recent`.
- No migration-failure-and-recovery test (would catch D-1).

#### D-14 · Cascade behavior matrix (verified)

| Parent → Child | Action | Source |
|---|---|---|
| categories → expenses | SET NULL | 0001/0006/0014 |
| categories → budgets | CASCADE | 0001 |
| categories → recurring_rules | CASCADE | 0008 |
| categories → merchant_rules | CASCADE | 0013 |
| categories → budget_alert_state | CASCADE | 0009 |
| expenses → expenses (refund link) | SET NULL | 0006/0014 |
| telegram_authorized_chats → expenses | SET NULL | 0001 |
| telegram_authorized_chats → pending_recurring_confirmations | CASCADE | 0008 |
| recurring_rules → pending_recurring_confirmations | CASCADE | 0008 |

Refund-vs-parent: deleting an expense **does not** orphan its refund row — `is_refund` row survives with `refund_for_expense_id = NULL`.

---

## 5. Concurrency / panic surface

**Verdict.** Single-`Mutex<Connection>` model is workable for desktop but has 16 blocking-in-async sites that will bite under any concurrency. Two correctness bugs (wall-clock catch-up + recurring-rule timestamp).

### Medium

#### CC-1 · Sync rusqlite called from async tasks without `spawn_blocking`
- **Severity:** Medium · **Effort:** L
- **Files:** Scheduler — 16 sites enumerated below
- `RouterDeps.conn: Arc<Mutex<Connection>>` is `std::sync::Mutex` wrapping sync `rusqlite::Connection`. Every scheduler handler is `async fn` that calls `lock().unwrap()` and runs SQL on the Tokio worker. Locks are short and never held across `.await` (verified — see CC-4), so no deadlock, BUT while a query executes the worker thread cannot schedule any other future. The moment anything wants concurrent work (CSV import + scheduler tick + TG poller), the runtime stalls.

| # | File | Line | Operation |
|---|---|---|---|
| 1-3 | `scheduler/mod.rs` | 226, 243, 254 | `list_due`, stale-bump, post-handler reschedule |
| 4-8 | `scheduler/recurring.rs` | 60, 96, 110, 127, 155 | rule get, expense insert, owner lookup, pending check, insert |
| 9-15 | `scheduler/budget_alerts.rs` | 26, 36, 60, 82, 90, 110, 146 | settings get, owner, category list, currency, per-category SUM/COUNT in loop, INSERT alert |
| 16 | `scheduler/weekly_summary.rs` | 24, 36, 54, 58, 68, 94 | 6 sequential lock-and-query sites |

- **Recommendation:** Wrap each contiguous SQL block in `tokio::task::spawn_blocking({ let conn = Arc::clone(&deps.conn); move || {…} }).await?`. Or migrate to single-writer actor: one OS thread owns the Connection, accepts requests over bounded `mpsc`, returns via `oneshot`. Actor pattern simultaneously addresses CC-1, CC-4, CC-6.

#### CC-2 · Wall-clock time source breaks under NTP correction / manual clock change
- **Severity:** Medium · **Effort:** M
- **Files:** `src-tauri/src/scheduler/mod.rs:293`
- `OffsetDateTime::now_utc()` compared to `next_due_at` stored in DB. Forward jump > MAX_STALE_DAYS (7) silently misses every recurring expense. Backward jump that crosses boundary causes stale skip on legitimately-due job. `tokio::time::interval` is correctly monotonic for tick cadence; only the comparison is wall-clock.
- **Recommendation:** Detect clock jumps — track `last_tick_wall`; if `(now - last_tick_wall).abs() > 2 * TICK_INTERVAL`, log a warning and skip the tick.

#### CC-3 · Catch-up window collapses many recurring expenses to the same timestamp
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/src/scheduler/recurring.rs:78,84`
- After 6-day offline, all 6 daily-recurring rules fire on first tick (within MAX_STALE=7d). Auto-mode rules silently insert 6 rows, but `occurred_at = now` so they all share a single timestamp instead of their actual due dates. User sees six `$15.49 Spotify` charges all stamped `2026-05-04 09:32:11`. Pollutes spend-by-day chart and could trip budget alerts.
- **Recommendation:** Set `occurred_at = job.next_due_at` (the rule's intended due time) rather than `now`.

### Low

#### CC-4 · `std::sync::Mutex` held across `.await` is currently safe but fragile
- **Severity:** Low · **Effort:** S
- **Files:** All scheduler files
- Verified all guards confined to scope blocks that drop before next `.await`. Correct today, but `clippy::await_holding_lock` is allow-by-default — a future maintainer copying the pattern wrongly produces a deadlock under contention.
- **Recommendation:** Add `#![warn(clippy::await_holding_lock)]` to `lib.rs`. Don't migrate to `tokio::sync::Mutex` for a sync rusqlite handle — actor model is the correct fix.

#### CC-5 · No graceful shutdown wired to Tauri exit
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/app_state.rs:117`, `src-tauri/src/lib.rs`
- `AppState::shutdown_poller` exists but is never called. No `RunEvent::ExitRequested` listener flips it. Scheduler has no equivalent. On exit, both tasks are torn down by process death mid-transaction. Rusqlite WAL handles SQLite-side, but a recurring-rule insert may or may not commit, and `last_fired_at` won't update — so the same rule fires again at next launch (within MAX_STALE) or is silently dropped.
- **Recommendation:** Register `RunEvent::ExitRequested` callback that flips both shutdown flags and awaits the tasks (requires keeping `JoinHandle`s, currently dropped per `app_state.rs:50`).

#### CC-6 · 21 unwraps in `scheduler/mod.rs` classified
- 3 production-path `Mutex::lock().unwrap()` (mutex-poisoning idiom; reachable but only fail on poisoning) at lines 226, 243, 254.
- 18 test-only.
- **Recommendation:** `.lock().unwrap_or_else(|e| e.into_inner())` in the 3 production sites, or migrate to `parking_lot::Mutex`.

#### CC-7 · Budget-alert sweep is N+1 on categories
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/scheduler/budget_alerts.rs:87`
- Loop over categories with fresh `SELECT SUM` and `SELECT COUNT` per category, each grabbing the mutex. N=20 ⇒ 43 locks per hour.
- **Recommendation:** Single GROUP BY query.

#### CC-8 · `restart_poller` doesn't wait for old poller to exit
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/app_state.rs:200`
- Old task may issue one more `getUpdates` to Telegram before noticing orphan shutdown flag. Resulting 409 Conflict recovered by backoff. Documented; harmless.

### Info

#### CC-9 · Lock-acquisition order uniform; no deadlock paths observed.

#### CC-10 · No `tokio::sync::mpsc` / `crossbeam::channel` in use; no unbounded-channel OOM risk.

#### CC-11 · Single-instance plugin makes idempotency-on-double-launch verifiable. `ensure_singleton` already guards weekly_summary / budget_alert_sweep.

#### CC-12 · Scheduler `expect`s in `domain/recurring.rs` are infallibly bounded (preceded by `clamp` proving input range). Class (a) — genuinely unreachable.

---

## 6. Performance

**Verdict.** Fair. Backend hot paths are short queries on an in-process SQLite — fine for desktop. Frontend has notable slider re-render thrash and unconditional polling.

### Medium

#### Pf-1 · Slider drag triggers redundant double-fetch in Simulator
- **Severity:** Medium · **Effort:** M
- **Files:** `src/views/Forecast.tsx:300-345`
- Every slider tick re-runs the entire effect — calls `simulatorSolveRequired` *and* `simulatorHeatmap` (or probability+heatmap). Heatmap is 12×12=144 Monte Carlo cells, an order of magnitude more expensive than the solver. Dragging horizon 1y→50y at 60fps fires ~50–100 IPC pairs sequentially, each rerunning 1000-path × 144-cell sims. Cancellation flag only nulls the *result write*, not the work.
- **Recommendation:** `useDebouncedValue` (250ms) on slider state writes; or split — keep the cheap solver eager, debounce the heatmap to ~500ms after last input. Or pass a cancellation token through `invoke` (Tauri 2 supports this) and bail on the Rust side.

#### Pf-2 · Insights polls every 5s + Household polls every 2s while view mounted (no visibility check)
- **Severity:** Medium · **Effort:** S
- **Files:** `src/views/Insights.tsx:67,89`, `src/views/Household.tsx:23`
- Polling runs unconditionally as long as view is mounted; doesn't pause when window hidden. With Background-mode setting on (recommended), the app sitting in tray hammers SQLite forever for nothing. Household poll is especially wasteful — only matters during active pairing.
- **Recommendation:** Add `document.visibilityState !== "visible"` short-circuit; gate Household's poll behind `inviteCode !== null`.

#### Pf-3 · No code-splitting; Forecast.tsx (2562 LOC) + Recharts always in initial bundle
- **Severity:** Medium · **Effort:** M
- **Files:** `src/App.tsx:8-14`
- Recharts is ~380KB-min; only ever rendered on `/insights` and `/forecast`. Settings, Household, Ledger, Categories views don't need it.
- **Recommendation:** `React.lazy(() => import(…))` + `<Suspense>`; should drop ~150KB from eager bundle.

#### Pf-4 · DB mutex held across handler bodies; no `spawn_blocking` (cross-reference CC-1)
- **Severity:** Medium · **Effort:** M
- **Files:** All command handlers in `commands.rs`
- Each handler acquires `state.db.lock().unwrap()` synchronously then holds it through query execution. Heaviest paths: `csv_import_commit` (loop over thousands of rows), `list_expenses` with LIKE search, `list_investment_categories` (N+1).
- **Recommendation:** Run heavy commands inside `tokio::task::spawn_blocking`; switch to `parking_lot::Mutex` to remove poisoning surface.

#### Pf-5 · Dedupe Levenshtein search is O(N · K) with unbounded fan-out per row
- **Severity:** Medium · **Effort:** M
- **Files:** `src-tauri/src/csv_import/dedupe.rs:82-120`
- One query per imported row; candidate fan-out unbounded (power user importing 5,000 $4.95 transactions over 5y has hundreds of candidates per query, each Levenshtein-scored).
- **Recommendation:** Batch SQL (`amount_cents IN (?, ?, ...)`); fetch candidates once into in-memory bucket-by-amount map; iterate parsed rows against it. Add D-4 composite index in tandem.

#### Pf-6 · `MAX_AGENT_ITERATIONS = 5` with no per-day cost ceiling
- **Severity:** Medium · **Effort:** S (cap bump) / M (cost ceiling)
- **Files:** `src-tauri/src/telegram/router.rs:29,225-232`
- Cap of 5 hit on legitimate multi-tool flows ("how am I doing this month and oh also log $5 coffee"). User in a tight typing loop racks unbounded Anthropic spend until they notice.
- **Recommendation:** Bump to 8; add soft daily ceiling (default $1.00) read from settings; refuse next turn when exceeded with Settings link. Per-message token cap (2000 chars) on Telegram input.

### Low

#### Pf-7 · Monte Carlo Box-Muller throws away half its samples
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/insights/monte_carlo.rs:179-185`
- `sample_normal` consumes two uniforms but returns only `z0`. With 144 heatmap cells × 200 paths × 360 months, doubles RNG cost (~10M wasted samples per heatmap).
- **Recommendation:** Use `rand_distr::StandardNormal` (transitive dep) or cache `z1`.

#### Pf-8 · Box-Muller `f64::EPSILON` floor truncates extreme tails
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/insights/monte_carlo.rs:180`
- Caps |z| at ~8.5σ; fine for 1000 paths, appears with N=10k.

#### Pf-9 · Monte Carlo single-threaded; defaults are CPU-heavy
- **Severity:** Low · **Effort:** M
- **Files:** `src-tauri/src/insights/monte_carlo.rs`, `simulator.rs:267`
- Default 1000 paths × 360 months = 360k normals per simulate call. Heatmap is 144 cells × 200 × 360 = ~10M normals (1–3s blocking IPC). `rayon` would parallelize trivially.

#### Pf-10 · Recharts `isAnimationActive={false}` set on Forecast charts but NOT on Insights
- **Severity:** Low · **Effort:** S
- **Files:** `src/views/Insights.tsx:333-401, 506-555, 653-698, 720-739`
- Forecast disables animations during slider drag; Insights re-renders on every 5s poll with full animation enabled, causing brief stutter.

#### Pf-11 · `Categories.tsx` invokes `getCategoryStats` on every row expansion, never cached
- **Severity:** Low · **Effort:** S
- **Files:** `src/views/Categories.tsx:316-323`

### Info

#### Pf-12 · Forecast `least_squares` doesn't guard against degenerate single-bucket window with `n=1`
- Returns `(0.0, mean_y, 0.0)` (slope=0, "flat") which is correct but misleading; consider gating headline on `n_buckets >= 3`.

#### Pf-13 · Goal-seek bisection iteration cap is bounded (50 iters, converges to ±1¢; 14 iters in `solve_required_contribution`, ±$10). No infinite-loop risk.

---

## 7. Portability

**Verdict.** Excellent. Codebase is highly platform-neutral; only 2 files contain `#[cfg(target_os = …)]` conditionals, both defensive.

### Info

#### Po-1 · Linux Wayland workaround at `lib.rs` for WebKitGTK DMABUF (Mutter on Fedora) — current and necessary.

#### Po-2 · `secrets/store.rs` has `#[cfg(unix)]` 0o600 chmod + `#[cfg(not(unix))]` no-op — Windows fallback is documented as relying on AppData ACL inheritance.

#### Po-3 · Auto-updater signed-update path validated per platform via embedded minisign pubkey.

#### Po-4 · WebKitGTK 4.1 dep on Linux — Debian Stable still on 4.0; documented in BUILDING.md.

#### Po-5 · macOS arm64 + x86_64 + Windows MSVC + Linux x86_64 in release matrix. ARM Linux/Windows missing — scope decision, not defect (cross-reference §11).

#### Po-6 · `keyring` crate features `apple-native`, `windows-native`, `linux-native` — confirmed migration code path runs cleanly on all three but is on the chopping block (see §9).

#### Po-7 · `directories` crate handles cross-platform data dir; usage clean.

---

## 8. Compliance

**Verdict.** Concerns. AGPL §6 source-offer notice missing from shipped binaries, no third-party-license bundle, no per-file SPDX headers. README content drift.

### High

#### Co-1 · No source-offer notice in shipped binaries (AGPL §6)
- **Severity:** High · **Effort:** S
- **Files:** `src-tauri/tauri.conf.json:54` (copyright), `.github/workflows/release.yml:86-89` (releaseBody)
- AGPL-3.0 §6 requires conveyed object code be accompanied by either corresponding source or written offer for it. §13 extends this to network-interaction users. Mr. Moneypenny conveys binaries via GitHub Releases (`tauri-action` produces `.AppImage`, `.deb`, `.rpm`, `.dmg`, `.app.tar.gz`, `.msi`, `.exe`, `.nsis.zip`). Inside those artifacts there is no `LICENSE` file and no "source available at <url>" notice. Release body mentions signing status but not the source URL. `bundle.copyright` says "Licensed under AGPL-3.0-or-later" without specifying *where*.
- **Recommendation:**
  1. `tauri.conf.json` `bundle.copyright`: append `Source: https://github.com/wyattts/Mr.-Moneypenny`.
  2. Add `bundle.resources` to copy `LICENSE` into the bundle root.
  3. In-app About / Settings: display `License: AGPL-3.0-or-later — source: <repo URL>`.
  4. Append source URL to `releaseBody` template.

### Medium

#### Co-2 · No third-party-license / NOTICES bundle (downstream redistribution friction)
- **Severity:** Medium · **Effort:** S
- Static-links 400+ Rust crates, bundles 100+ npm packages — every MIT/BSD/Apache-2.0 carries a copyright/license retention obligation. No `cargo-about`, no `THIRD-PARTY-LICENSES.{html,txt}`, no `NOTICE` in repo or bundle.
- **Recommendation:** Add `cargo-about` + `license-checker` (npm) to CI; generate combined `THIRD-PARTY-LICENSES.html` artifact during build; ship via `bundle.resources`.

### Low

#### Co-3 · No license headers in source files (AGPL convention; legal robustness)
- **Severity:** Low · **Effort:** S (one-time pass)
- LICENSE at repo root is sufficient for the work as a whole, but per-file headers help downstream forks and matter if individual files are vendored.
- **Recommendation:** Add SPDX header to each source file: `// SPDX-License-Identifier: AGPL-3.0-or-later` + `// Copyright (C) 2026 Wyatt Smith and contributors`.

### Info

#### Co-4 · LICENSE present at repo root; SECURITY.md has real contact + 7/14-day SLA + coordinated-disclosure process; CODE_OF_CONDUCT.md has working enforcement contact; CONTRIBUTING.md DCO (Signed-off-by), no CLA.

#### Co-5 · README "Status" claims v0.1.0 alpha; current is v0.3.7 — drift.

#### Co-6 · `BUILDING.md` "Reproducibility" says toolchain pinned via rust-toolchain.toml — partially false (channel = stable, no version pin; cross-reference §11).

---

## 9. Redundancy / dead code

**Verdict.** Conservative codebase. The headline finding is the stale `keyring` crate (overdue for removal); the rest is justified.

### Medium

#### R-1 · Dead `keyring` crate two minor versions past declared sunset
- **Severity:** Medium · **Effort:** S
- **Files:** `src-tauri/Cargo.toml:45-48`, `src-tauri/src/secrets/migration.rs`
- Cargo.toml comment: "Will be dropped in v0.2.7 once the migration window closes." Walked back to v0.2.8 in CHANGELOG. Current version is v0.3.7 — eleven minors past v0.2.7. The crate (and transitive `dbus`, `secret-service`, `windows-sys`, `apple-native`) is dead weight on every build.
- **Verdict for v0.3.8:** **Yes — removable.** On a v0.3.7 fresh install, `try_copy_from_keyring` is provably a no-op (disk store hits, never falls through to keyring path). For v0.2.6→v0.3.x stragglers, mitigation is "re-enter API key in Settings" which writes through the disk path.
- **Recommended path:** v0.3.8 — eagerly drain the keyring once during `SecretsFile::open` (loop both known key names), write a sentinel, never enter migration path again. v0.3.9 — delete `migration.rs`, drop the `keyring` dep, remove module reference. Net delete: ~80 LOC + one Cargo.toml dep.

### Low

#### R-2 · Three "fixable" ESLint `react-hooks/exhaustive-deps` suppressions (of 12 total)
- **Severity:** Low · **Effort:** S
- **Files:** `views/Settings.tsx:742, 806`, `views/Forecast.tsx:2352`
- All three IIFE-style mount-only effects that capture `onError` from props. If a parent re-creates `onError` per render, the effect won't pick that up. Fix: include `onError` in deps; consumer wraps in `useCallback` upstream.
- The other 9 suppressions are idiomatic React mount-only-effect ("run once on mount") patterns and are correctly marked.

#### R-3 · Vestigial functions in LLM module
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/src/llm/dispatcher.rs:692-698`, `src-tauri/src/llm/anthropic.rs:30-34`, `src-tauri/src/llm/ollama.rs:25-28`
- `category_kind_to_str` and `_expense_used` are `#[allow(dead_code)]` no-ops. `AnthropicProvider::new` and `OllamaProvider::new` are convenience constructors used only by tests.
- **Recommendation:** Delete the no-ops; demote `::new` to `#[cfg(test)]`.

#### R-4 · Dead `getElementById("forecast-root")` query
- **Severity:** Low · **Effort:** S
- **Files:** `src/views/Forecast.tsx:1105`
- ID never set anywhere. `root?.scrollTo?.()` swallows the null safely; smooth-scroll relies on adjacent `window.scrollTo`. Either delete or set the id on the wrapper at line 147.

#### R-5 · `refundOverrides` state is read-only with no UI control
- **Severity:** Low · **Effort:** S
- **Files:** `src/views/CsvImport.tsx:93,266`
- Setter intentionally omitted; lookup `refundOverrides[i] ?? row.is_refund` always falls back. Either wire the UI or use `row.is_refund` directly.

### Info

#### R-6 · `ping` is exported but never imported (only truly unused export in `src/lib/tauri.ts` of 109 total).

#### R-7 · 6 Rust `#[allow(...)]` attributes — all justified.

#### R-8 · Top-5 multi-line comment blocks all confirmed as `//!` rustdoc module-level documentation, not dead code.

#### R-9 · Two `_force_use_*` no-op functions (`llm/dispatcher.rs:697`, `csv_import/categorize.rs:157`) suppress unused-import warnings on `pub use` re-exports. Could be replaced with `#[allow(unused_imports)]` on the re-export — 0-impact stylistic.

#### R-10 · No `_old.rs`, `_legacy.rs`, `_v1.rs` paired files. Clean architecture.

---

## 10. Test coverage + migration safety

**Verdict.** Fair. Insights/domain/secrets/csv_import are well-tested. Telegram and scheduler are thinly tested at unit level (compensated by integration tests). The three weakest spots: `commands.rs` has **zero unit tests**; `repository/{expenses,categories,recurring_rules,budgets}.rs` have zero in-file tests (covered transitively); `llm/dispatcher.rs` has zero in-file tests.

### Test density (top 10)

```
13  csv_import/parser.rs
12  insights/range.rs
11  telegram/auth.rs
10  insights/simulator.rs
10  insights/debt.rs
 9  insights/stats.rs
 9  insights/monte_carlo.rs
 8  secrets/store.rs
 8  llm/pricing.rs
 8  domain/recurring.rs
```

26 files with ≥1 test; 197 unit tests + 66 integration tests = 263 total.

### Module verdicts

| Module | Verdict |
|---|---|
| `secrets` | well-tested |
| `insights` | well-tested |
| `csv_import` | well-tested |
| `domain` | well-tested |
| `db` | well-tested (mod + integration_db) |
| `repository` | thinly-tested (4 of 8 modules have 0 in-file tests; transitively covered) |
| `llm` | thinly-tested (`dispatcher.rs` has 0 in-file tests; covered by `tests/integration_dispatcher.rs`) |
| `scheduler` | thinly-tested (recurring/budget_alerts/weekly_summary 0 unit tests; covered by `tests/integration_scheduler.rs`) |
| `telegram` | thinly-tested (router/poller 0 unit tests; covered by `tests/integration_telegram.rs`) |
| `commands` | **untested** (0 unit + no integration covers Tauri command surface) |

### Medium

#### T-1 · `commands.rs` (the entire IPC surface) has zero unit + zero integration coverage
- **Severity:** Medium · **Effort:** M
- **Files:** `src-tauri/src/commands.rs`, `src-tauri/tests/`
- `list_expenses` filter SQL (off-by-one on `end_date + 1 day` upper bound is classic), `csv_import_commit` column-mapping pipeline, `delete_expense` boolean (does it cascade to refunds?), and the 50 other handlers — none exercised by tests.
- **Recommendation:** Add `tests/integration_commands.rs` covering at least `list_expenses` filters, `csv_import_commit` round-trip, `delete_expense` + refund cascade, and the simulator/debt/forecast commands. ~30-50 lines per command minimum.

### Low

#### T-2 · No migration-failure-and-recovery test (would catch D-1)
- **Severity:** Low · **Effort:** M
- Inject disk-full or panic mid-batch in 0004 / 0006; assert app recovers cleanly on next launch.

#### T-3 · `llm/dispatcher.rs` has zero in-file tests
- **Severity:** Low · **Effort:** S
- Worth in-file coverage for `parse_date_or_datetime` (RFC3339 + `YYYY-MM-DD` + tz) and tool-arg validation rejection paths.

#### T-4 · `telegram/router.rs` thin coverage
- **Severity:** Low · **Effort:** S
- Missing: `MAX_AGENT_ITERATIONS` exhaustion path; pending-recurring-confirmation flow (`yes`/`no`/`skip` aliases); `send_message` failure handling in `reply` (`router.rs:448-454`).

#### T-5 · `scheduler/{recurring, budget_alerts, weekly_summary}` zero unit tests
- **Severity:** Low · **Effort:** S
- Missing: `recurring::handle` Auto vs Confirm on missing rule; `budget_alerts::handle` partial-success ordering (only inserts state AFTER successful `send_message`).

### Info

#### T-6 · No `proptest`, no `cargo-fuzz`. High-value candidates if added: `csv_import/parser.rs`, `insights/monte_carlo.rs` (GBM seed-stability), `insights/debt.rs` (goal-seek bisection convergence).

#### T-7 · Integration tests do not exercise `desktop` feature gate
- 5 integration test files use only `db, domain, repository, insights, llm, telegram, scheduler` — none transitively requires `desktop`-gated items. `commands.rs` IS desktop-gated and is also the module with no tests.
- **Recommendation:** Add CI matrix entry running `cargo test --no-default-features` to lock the contract in.

---

## 11. Build / release pipeline

**Verdict.** Good for a one-maintainer FOSS project. Lockfiles committed, `cargo audit` blocking, AppImage GPG-signed, updater bundle minisign-signed. Five Medium gaps (NOTICES bundle, dead keyring crate, no Dependabot, no SLSA attestations, manual `gh release edit --latest` footgun).

### Medium

#### B-1 · Manual `gh release edit --latest` after CI breaks the auto-updater (already happened on v0.3.4)
- **Severity:** Medium · **Effort:** S
- **Files:** `.github/workflows/release.yml:86-90`
- Release workflow creates a *draft* (`releaseDraft: true`). Updater endpoint requires `--latest` flag set on the release. If the maintainer publishes without setting `latest` (or forgets), every existing install silently goes without updates. CHANGELOG v0.3.4 entry shows this has happened.
- **Recommendation:** Add an `if: success() && github.event_name == 'push'` step at end of release.yml that promotes the draft via `gh release edit "$TAG" --draft=false --latest`. Preserves the "all-platforms-built" gate; removes the manual step.

#### B-2 · No Dependabot / Renovate
- **Severity:** Medium · **Effort:** S
- **Files:** `.github/dependabot.yml` (does not exist)
- `cargo audit` and `npm audit` detect CVEs in pinned versions but don't propose upgrades. With one maintainer, drift is the default.
- **Recommendation:** Add weekly Dependabot for `cargo`, `npm`, `github-actions`.

#### B-3 · No SLSA / GitHub artifact attestations on releases
- **Severity:** Medium · **Effort:** M
- **Files:** `.github/workflows/release.yml`
- Updater verifies via embedded minisign pubkey (good for in-app updates). But release-page binaries have no provenance chain a security-conscious downloader can verify offline.
- **Recommendation:** Add `actions/attest-build-provenance@v2` after Build step; add `id-token: write` to job permissions.

#### B-4 · No third-party-licenses / NOTICES bundle (cross-reference Co-2)
- See §8 (Compliance).

#### B-5 · `keyring` crate dead code path (cross-reference R-1, C-3)
- See §9 (Redundancy).

### Low

#### B-6 · Rust toolchain pinned only to `stable` channel — silent drift
- **Severity:** Low · **Effort:** S
- **Files:** `src-tauri/rust-toolchain.toml`, `.github/workflows/{ci,release}.yml`
- Reproducible-build claim in `BUILDING.md:160` is unverifiable in practice. A clippy upgrade can break CI on an otherwise green PR.
- **Recommendation:** Pin to specific stable release; bump deliberately in own PR.

#### B-7 · `npm audit` non-blocking; asymmetric with blocking `cargo audit`
- **Severity:** Low · **Effort:** S
- **Files:** `.github/workflows/ci.yml:84-87`
- A critical npm CVE in `vite` won't fail CI; a low-severity informational from RUSTSEC will.
- **Recommendation:** `npm audit --audit-level=critical` blocking, or migrate to `cargo deny` + `npm audit --audit-level=high --production`.

#### B-8 · macOS / Windows binaries unsigned (Gatekeeper / SmartScreen friction)
- **Severity:** Low · **Effort:** L (depends on Sponsors funding)
- Documented in BUILDING.md. Not a code defect — flagged for completeness.

#### B-9 · `bundle.targets: "all"` builds NSIS *and* MSI on Windows
- **Severity:** Low · **Effort:** S
- Two installer formats ship; doubles unsigned-binary footprint and confuses users.
- **Recommendation:** Constrain to `["app", "dmg", "appimage", "deb", "rpm", "nsis"]`.

### Info

#### B-10 · `dist/` is gitignored but appears in working tree (build output, correctly excluded).

#### B-11 · Release matrix lacks ARM Linux + ARM Windows. Not a defect; ARM Linux user demand rising.

#### B-12 · `BUILDING.md` "Reproducibility" section overstates the toolchain pin (cross-reference Co-6).

#### B-13 · Things working well: Cargo.lock + package-lock.json committed; `cargo audit` blocking; AppImage GPG-signed with detached `.asc`; updater bundles minisign-signed; CSP tight; `#![forbid(unsafe_code)]` at crate root; `concurrency:` block correctly cancels in-flight runs; `cargo test --no-default-features` runs in CI; `gpg --pinentry-mode loopback` handles both passphrase-protected and passwordless keys; DCO required, no CLA; SECURITY.md real contact + SLA + disclosure process.

---

## 12. Frontend hygiene

**Verdict.** Solid TypeScript posture (strict + `exactOptionalPropertyTypes`, no `any`, no localStorage of secrets, no `console.*` of sensitive values). One High accessibility finding (slider aria), several Medium UX/perf, performance smells in slider drag.

### High

#### F-1 · Range sliders missing `aria-label` / `aria-valuetext` (Forecast view unusable with screen reader)
- **Severity:** High · **Effort:** M
- **Files:** `src/views/Forecast.tsx:478-485` (confidence), `:2549-2558` (NumberSlider helper used 8+ places), `:2405-2412` (ScenarioTool per-category)
- Every `<input type="range">` in the app is wrapped in a `<label>` whose `<span>` text contains the live value (fine for sighted users), but the `<input>` itself has no `aria-label`, no `aria-valuetext`, no `id` linking. Screen reader announces "slider, 30" with no unit context.
- `NumberSlider` is the canonical helper used by Simulator + DebtManager for at least 8 sliders; fixing once propagates everywhere.
- **Recommendation:** Add `aria-label={label}` and `aria-valuetext={\`${value} ${unit}\`}` to NumberSlider's `<input>`; bespoke confidence slider gets `aria-label="Confidence" aria-valuetext={\`${(value*100).toFixed(0)} percent\`}`.

### Medium

#### F-2 · Token entry forms in wizard don't clear secret state after save
- **Severity:** Medium · **Effort:** S
- **Files:** `src/wizard/steps/AnthropicConfig.tsx:55-63`, `src/wizard/steps/Telegram.tsx:172-184`
- Settings.tsx clears `setVal("")` after save (good); wizard equivalents do not. Secret stays in component state until step unmounts. Low risk because next step transition unmounts; defense-in-depth gap.
- Plus: error toasts pipe `String(e)` directly from backend with no token/key regex scrub.
- **Recommendation:** Clear state after success; regex-scrub `bot\d+:[\w-]+` and `sk-ant-\w+` from error strings before display.

#### F-3 · Pill-toggle buttons lack `aria-pressed`
- **Severity:** Medium · **Effort:** S
- **Files:** `Forecast.tsx:404-423, 546-566, 1122-1143, 1215-1234`
- Visually two-state segmented pickers rendered as `<button>` with active-class swap. Screen reader hears as plain buttons; selected state invisible.
- **Recommendation:** Add `aria-pressed={…}` or migrate to `role="radiogroup"` + `role="radio"`.

#### F-4 · CSV-import modal lacks focus-trap, role="dialog", aria-modal
- **Severity:** Medium · **Effort:** M
- **Files:** `src/views/CsvImport.tsx:291-429`
- Tab can escape behind backdrop into Settings page. ESC does nothing. Close button not auto-focused on open. h2 at line 294 is a perfect `aria-labelledby` target.

#### F-5 · Slider drag triggers redundant double-fetch in Simulator (cross-reference Pf-1)
- See §6.

#### F-6 · Insights polls every 5s + Household polls every 2s (cross-reference Pf-2)
- See §6.

#### F-7 · No code-splitting (cross-reference Pf-3)
- See §6.

### Low

#### F-8 · Native `confirm()` for destructive actions
- **Severity:** Low · **Effort:** S
- **Files:** `Categories.tsx:96`, `Ledger.tsx:67`, `Household.tsx:62`
- Keyboard-trapped on Linux Tauri webview; styled inconsistently with dark theme.

#### F-9 · Recharts `isAnimationActive={false}` set on Forecast, NOT on Insights (cross-reference Pf-10)

#### F-10 · `Categories.tsx` invokes `getCategoryStats` on every row expansion (cross-reference Pf-11)

#### F-11 · Tauri-IPC payloads use `as` casts to unwrap Recharts tooltip payloads
- **Severity:** Low · **Effort:** S
- **Files:** `Forecast.tsx:680-682, 1727-1728, 1869-1870`, `Insights.tsx:488, 671`
- Recharts tooltip `payload` types are loose (`unknown[]` in newer versions). Casts narrow to runtime shape but bypass TS — schema change wouldn't surface compiler error.
- **Recommendation:** Generic helper `function getDatum<T>(p): T | undefined`.

#### F-12 · Error strings echoed verbatim into UI banners; no PII filter (cross-reference F-2 + S-2)

#### F-13 · Dead `getElementById("forecast-root")` (cross-reference R-4)

#### F-14 · `refundOverrides` state is read-only (cross-reference R-5)

#### F-15 · NumberField string→cents desync on `"1.2.3"` input
- **Severity:** Low · **Effort:** S
- **Files:** `Forecast.tsx:NumberField` callers
- Add parse-on-blur normalization or regex onChange filter.

#### F-16 · `parseInt(l.month)` accepts negative offsets, then `Math.max(0, …)` clamps — but input field accepts `-`
- **Severity:** Low · **Effort:** S
- **Files:** `Forecast.tsx:937`
- Cosmetic; user-typed `-12` becomes month 0 silently.

### Info

#### F-17 · `<img alt="">` on logo is correct (decorative; wordmark text follows).

#### F-18 · `seed: null as number | null` cast is fine; cleanest expression under `exactOptionalPropertyTypes: true`.

#### F-19 · `console.warn` calls are benign (`Categories.tsx:53`, `UpdateBanner.tsx:37`); print swallowed exception object only.

#### F-20 · `localStorage` only stores `moneypenny.theme`; no PII, no secrets.

#### F-21 · CSP `'unsafe-inline'` styles confirmed required by Recharts SVG injection; tightening would require Vite plugin nonce injection or migrating Recharts inline-style sites to className-based dynamic CSS variables. Realistic exploit path near-zero on Tauri desktop with `script-src 'self'`. Defer until Recharts ships CSP-friendly mode.

#### F-22 · `lib/format.ts:25` fallback `toFixed(2)` always assumes 2 decimals; fires only on unknown currency codes.

---

## Dependency snapshot

### Top-level Rust dependencies

| Crate | Toml pin | Resolved | License | Notes |
|---|---|---|---|---|
| `tauri` | `^2` | 2.10.3 | MIT OR Apache-2.0 | |
| `tauri-build` | `^2` | 2.5.6 | MIT OR Apache-2.0 | Build-script; `desktop`-gated |
| `tauri-plugin-autostart` | `^2` | 2.5.1 | MIT OR Apache-2.0 | |
| `tauri-plugin-updater` | `^2` | 2.10.1 | MIT OR Apache-2.0 | |
| `tauri-plugin-single-instance` | `^2` | 2.4.1 | MIT OR Apache-2.0 | |
| `serde` | `^1` | 1.0.228 | MIT OR Apache-2.0 | |
| `serde_json` | `^1` | 1.0.149 | MIT OR Apache-2.0 | |
| `rusqlite` | `^0.32` | 0.32.1 | **MIT** | Single-licensed MIT (recon's "AGPL OR Unlicense" was wrong) |
| `time` | `^0.3` | 0.3.47 | MIT OR Apache-2.0 | |
| `anyhow` | `^1` | 1.0.102 | MIT OR Apache-2.0 | |
| `thiserror` | `^2` | 2.0.18 | MIT OR Apache-2.0 | |
| `tracing` | `^0.1` | 0.1.44 | MIT | |
| `tracing-subscriber` | `^0.3` | 0.3.23 | MIT | |
| `directories` | `^5` | 5.0.1 | MIT OR Apache-2.0 | |
| `tokio` | `^1` | 1.52.1 | MIT | |
| `async-trait` | `^0.1` | 0.1.89 | MIT OR Apache-2.0 | |
| `reqwest` | `^0.12` (rustls-tls) | 0.12.28 | MIT OR Apache-2.0 | rustls only — no system OpenSSL |
| `rand` | `^0.8` | 0.8.6 | MIT OR Apache-2.0 | |
| `chacha20poly1305` | `^0.10` | 0.10.1 | Apache-2.0 OR MIT | Secrets cipher |
| `hkdf` | `^0.12` | 0.12.4 | MIT OR Apache-2.0 | |
| `sha2` | `^0.10` | 0.10.9 | MIT OR Apache-2.0 | |
| `machine-uid` | `^0.5` | 0.5.4 | MIT | |
| `base64` | `^0.22` | 0.22.1 | MIT OR Apache-2.0 | |
| `csv` | `^1` | 1.4.0 | Unlicense OR MIT | |
| `strsim` | `^0.11` | 0.11.1 | MIT | |
| `keyring` | `^3` | 3.6.3 | MIT OR Apache-2.0 | **Dead code path — see R-1** |
| `tempfile` | `^3` | (dev-only) | MIT OR Apache-2.0 | |

Selected transitive deps (no current RUSTSEC advisories at versions present): `hyper 1.9.0`, `h2 0.4.13`, `rustls 0.23.40`, `tokio-rustls 0.26.4`, `ring 0.17.14`, `regex 1.12.3`, `url 2.5.8`, `idna 1.1.0`, `webkit2gtk 2.0.2`, `wry 0.54.4`, `tao 0.34.8`. No `openssl-sys` in tree.

### Top-level npm dependencies

| Package | package.json pin | Resolved | License |
|---|---|---|---|
| `@tauri-apps/api` | `^2.1.1` | 2.10.1 | MIT OR Apache-2.0 |
| `react` | `^18.3.1` | 18.3.1 | MIT |
| `react-dom` | `^18.3.1` | 18.3.1 | MIT |
| `react-router-dom` | `^6.28.0` | 6.30.3 | MIT |
| `recharts` | `^2.13.3` | 2.15.4 | MIT |
| `zustand` | `^5.0.1` | 5.0.12 | MIT |

(Dev deps: `typescript 5.9.3` Apache-2.0, `vite 5.4.21` MIT, `eslint 9.39.4` MIT, etc. — all MIT/Apache/BSD/ISC.)

All top-level licenses are AGPL-compatible. No (L)GPL, no proprietary, no SSPL/BUSL. The Co-2 finding addresses *attribution* (NOTICES bundle), not compatibility.

---

## Recommended remediation roadmap

### v0.3.8 (security + correctness patch)

| # | Finding | Severity | Effort |
|---|---|---|---|
| 1 | S-2 / P-1: scrub bot token from `reqwest` errors before logging | High | S |
| 2 | D-1: wrap migrations 0004/0006/0011/0014 in transactions | High | S |
| 3 | D-2: wrap `csv_import_commit` in `unchecked_transaction` | High | S |
| 4 | M-1 (CSV EU-format amount corruption) | Medium | S |
| 5 | Co-1: AGPL §6 — add source URL to `bundle.copyright`, ship LICENSE in bundle | High | S |
| 6 | R-1: drain keyring eagerly on `SecretsFile::open`, add sentinel | Medium | S |
| 7 | C-1: zeroize master key (add `zeroize` dep, wrap field) | Medium | S |
| 8 | C-2: propagate `sync_all` error + parent-dir fsync | Medium | S |
| 9 | F-1: add `aria-label`/`aria-valuetext` to `NumberSlider` | High | M |
| 10 | CC-3: set `occurred_at = job.next_due_at` in recurring catch-up | Medium | S |
| 11 | B-1: auto-promote draft release with `--latest` flag | Medium | S |

### v0.3.9 (security defense-in-depth + cleanup)

| # | Finding | Severity | Effort |
|---|---|---|---|
| 12 | S-1: pairing-code rate limit + lockout + entropy bump | High | M |
| 13 | S-3: collapse expired/invalid oracle | Medium | S |
| 14 | S-4: validate Ollama endpoint URL | Medium | S |
| 15 | S-5: telegram offset + expense in same txn (or idempotency table) | Medium | M |
| 16 | LLM-1: prompt-injection mitigations on tool-result echo-back | Medium | M |
| 17 | Pf-6: bump MAX_AGENT_ITERATIONS to 8; add daily cost ceiling | Medium | S |
| 18 | Co-2: cargo-about + license-checker; ship NOTICES bundle | Medium | S |
| 19 | B-2: add Dependabot for cargo + npm + actions | Medium | S |
| 20 | Delete `keyring` dep entirely (R-1 part 2) | Low | S |
| 21 | F-2: clear wizard token state + scrub error toasts | Medium | S |
| 22 | F-3: `aria-pressed` on pill toggles | Medium | S |

### v0.4.0 (architecture)

| # | Finding | Severity | Effort |
|---|---|---|---|
| 23 | CC-1 / Pf-4: actor pattern for DB access (one OS thread, bounded mpsc) | Medium | L |
| 24 | Pf-1: debounce slider drag + cancellation token | Medium | M |
| 25 | Pf-3: lazy-load Forecast + Insights | Medium | M |
| 26 | F-4: focus-trap CSV-import modal | Medium | M |
| 27 | T-1: add integration tests for `commands.rs` | Medium | M |
| 28 | D-4: composite index `(category_id, occurred_at)` (migration 0015) | Medium | S |
| 29 | B-3: SLSA build-provenance attestations on releases | Medium | M |
| 30 | Co-3: SPDX file headers across source tree | Low | S |

### Deferred

- B-8: macOS/Windows code signing (pending Sponsors funding)
- B-9: prune `bundle.targets` to remove MSI duplicate
- F-22: CSP nonce-based migration off `'unsafe-inline'` styles (depends on Recharts upstream)
- T-6: proptest / cargo-fuzz adoption
- R-3 / R-4 / R-5 / Pf-7..11 / various Info: opportunistic cleanup

---

## Out of scope

- Live exploitation / fuzzing / penetration testing of the Telegram pairing endpoint against the real Bot API.
- Migrating to React 19 / cr-sqlite / iroh / Tauri Mobile (roadmap).
- Distro packaging review (Fedora COPR, Debian PPA — separate effort).
- Performance benchmarking with synthetic load (audit reads for perf smells; does not benchmark).
- Apple notarization, Windows Authenticode (B-8 — pending Sponsors funding).

---

## Appendix: source materials

Per-category staging files were produced during the audit and are kept in `/tmp/moneypenny-audit/` (not committed):

- `01-secrets.md` — line-by-line crypto review
- `02-telegram.md` — pairing brute-force quantification + token leak path
- `03-llm.md` — Anthropic vs Ollama PII flow tables; prompt-injection analysis
- `04-concurrency.md` — 16 blocking-in-async sites enumerated; unwrap classification
- `05-db.md` — full migration replay table; cascade matrix verified
- `06-commands.md` — IPC surface counts; CSP justifications
- `07-insights-csv.md` — numerical correctness reproductions; CSV crash inputs table
- `08-frontend.md` — eslint-suppression verdicts per site; type-cast inventory
- `09-build-deps.md` — full dependency snapshot; CI workflow analysis
- `10-cross-cutting.md` — privacy log sweep (26 sites reviewed, 1 flagged); test-coverage distribution

These contain the verbose reasoning behind each finding compiled here. Treat the staging files as the primary record; this document is the executive view.
