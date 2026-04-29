# Changelog

All notable changes to Mr. Moneypenny are documented here. The format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.4] - 2026-04-29

### Added

- **New `investing` category kind** alongside `fixed` and `variable`. Seeds four inactive-by-default investing categories — Savings, 401k, Investing, Roth IRA — that you can tick on under a new "Saving / Investing" group in the Categories view. Investing categories accept a monthly target like the others (e.g., "$500/month into Roth IRA").
- **Per-category bar chart on the Insights dashboard** — one horizontal bar per category that had spend in the selected range, regardless of kind. Coloring rules:
  - **Fixed / Variable** — graphite by default; turns **orange** when `spent > monthly_target_cents` (over budget).
  - **Investing** — light forest green by default; turns **deep forest green** when `spent >= monthly_target_cents` (savings goal met or exceeded).
  - Categories without a monthly target stay at the default tone for their kind.

### Internal

- Migration `0004_investing_kind.sql` recreates the `categories` table with `'investing'` admitted by the `kind` CHECK constraint, then seeds the four investing categories. SQLite doesn't support `ALTER TABLE … ADD CONSTRAINT`, so the migration disables foreign keys, copies the table, drops the old, and renames — all rows + schema invariants preserved.
- `CategoryTotal` now carries `monthly_target_cents` so the bar chart can decide over/under-budget per row without a second query.

## [0.1.3] - 2026-04-29

### Fixed

- **Telegram token rotation now actually rotates the running poller.** v0.1.2 saved the new token to the keychain and called `ensure_poller_running`, which is idempotent — the old poller kept running with the old `TelegramClient` (and therefore the old credentials) captured at startup. After rotating to a new bot, `/start <code>` messages landed in the new bot's update queue but were never read. Saving a new token now tears down the old poll loop and spawns a fresh one against the new token. Old loop self-terminates within ~30s; brief overlap is harmless because the two pollers target different Telegram endpoints.

### Internal

- New `AppState::restart_poller()` helper. The `save_telegram_token` command now calls it instead of `ensure_poller_running`.

## [0.1.2] - 2026-04-29

### Changed

- **Telegram token rotation now offers a pairing-code workflow.** After saving a new token in Settings → "Telegram bot token", the UI walks you through generating a fresh 6-digit code and re-pairing — previously it only confirmed the new token but left you with no way to re-authenticate. An optional **"clear all authorized chats"** checkbox during rotation performs a factory reset of the household whitelist (useful when paired to a brand-new bot); the first chat to redeem the next pairing code becomes the new owner.
- **Curated default-active categories.** Fresh installs now ship with 14 commonly-used categories enabled (Rent / Mortgage, Renters / Home Insurance, Health Insurance, Auto Insurance, Phone, Internet, Groceries, Dining Out, Transportation / Gas, Entertainment, Personal Care, Clothing, Household, Misc); the remaining 15 seeded categories ship inactive and are one click away in the Categories view. Existing v0.1.1 installs are migrated by `0003_curate_seed_actives.sql` — but only seeded categories with **zero expenses logged and no monthly target set** are flipped off, so any category you have already engaged with stays exactly as it was.
- **CI now signs the Linux AppImage automatically** even when the GPG signing key has no passphrase. v0.1.0 and v0.1.1 required local signing because the workflow passed `--passphrase ""` and gpg refused; the workflow now branches on whether `GPG_PASSPHRASE` is set.

## [0.1.1] - 2026-04-29

### Changed

- Default Anthropic model is now `claude-haiku-4-5-20251001` (was `claude-sonnet-4-6`). Cuts API cost ~4–5× for typical workloads. Existing v0.1.0 installs auto-pick-up Haiku on next launch unless they've explicitly set `anthropic_model`. Users who prefer Sonnet's heavier reasoning can override via the `anthropic_model` setting key (Settings UI control planned for v0.2.0).

## [0.1.0] - early-alpha

First end-to-end working build. Smoke-tested on Fedora 43 (GNOME / Wayland).

### Added

- **Telegram bot** — long-polling against your own BotFather bot. The desktop app holds an open `getUpdates` connection; no relay, no inbound port. Multi-user pairing via 6-digit codes with 10-minute TTL. First chat to redeem becomes household owner; subsequent are members.
- **LLM tool-use** — Anthropic Claude (default model `claude-haiku-4-5`, ~4–5× cheaper than Sonnet at this workload; prompt caching enabled) or local Ollama. Seven tools: `add_expense`, `delete_expense`, `query_expenses`, `summarize_period`, `list_categories`, `set_budget`, `list_household_members`. The dispatcher strictly type-checks every input before any DB access; the LLM never sees or generates SQL.
- **Period pacing** — `compute_snapshot()` powers both the bot's "how am I doing this month" and the dashboard's KPI strip from the same math. Fixed expenses do not affect the variable-pacing flag, so paying rent on the 2nd never makes the user look "over."
- **Insights dashboard** — KPI strip (variable remaining / daily allowance / total spent / on-pace status), category donut (top 8 + Other), daily-trend line (variable solid + fixed dashed), per-household-member spend bars (only when ≥ 2 chats), top-5 expenses, over-budget table, upcoming-fixed table, month-over-month delta. Time-range picker (week / month / quarter / year / YTD). 5-second auto-refresh.
- **Ledger** view — filter by category, search description, paginated, inline delete.
- **Categories / Budgets / Household / Settings** views.
- **Setup wizard** — 8 steps, GUI-only on the Anthropic path (no terminal). Persists progress so you can resume after a crash.
- **System tray + close-to-tray + auto-start** — bot stays online when you close the window. Auto-start enabled by default on macOS / Windows; opt-in on Linux because GNOME tray support requires the AppIndicator extension.
- **Privacy posture** — outbound HTTPS allowlist enforced via Tauri CSP: only `api.telegram.org`, `api.anthropic.com`, and a user-configured Ollama endpoint. No analytics, no telemetry, no auto-uploaded crash reports. Secrets in OS keychain (Keychain / Credential Manager / libsecret).
- **AGPL-3.0** license, Contributor Covenant 2.1 CoC, contributing guide, security disclosure policy.
- **Linux release artifacts** — `.AppImage`, `.deb`, `.rpm`. macOS `.dmg` / Windows `.msi` produced unsigned by CI.

### Known limitations

- macOS and Windows artifacts are unsigned. Gatekeeper / SmartScreen warnings are bypassable; instructions in [`docs/distribution.md`](docs/distribution.md). Signing pending project sponsorship.
- AppImage requires `NO_STRIP=true` at build time on hosts with binutils ≥ 2.41 because the bundled `linuxdeploy` ships an older `strip`.
- GNOME tray icons require the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/).
- Auto-update mechanism not yet wired up (binary is unsigned anyway). Plan: opt-in `tauri-plugin-updater` against GitHub Releases.
- Frontend bundle is ~633 KB (Recharts) — code-splitting deferred.
- Single host machine — the bot runs on whichever computer holds the database; multi-host sync is out of scope for v1.
- Only English UI strings.
- Branding placeholder; final logo and palette pending.
