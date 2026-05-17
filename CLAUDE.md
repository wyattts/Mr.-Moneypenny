# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Mr. Moneypenny is a Tauri 2.x desktop app (Rust backend + React/TypeScript frontend) for privacy-first personal budgeting. You log expenses by chatting with your *own* Telegram bot; the desktop app long-polls Telegram, asks an LLM (Anthropic API or local Ollama) to parse the message into a validated tool call, and writes to a local SQLite DB. Zero servers, zero copies of user data. AGPL-3.0.

## Commands

Frontend (repo root):

```bash
npm run typecheck     # tsc --noEmit for both tsconfig projects
npm run lint          # eslint . (lint:fix to autofix)
npm run build         # tsc -b && vite build -> dist/
npm run tauri:dev     # full dev loop, hot-reload frontend + Rust rebuilds
```

Rust (`src-tauri/`):

```bash
cargo test --no-default-features                                  # all domain/logic tests, no GTK needed
cargo test --no-default-features insights::period                 # single module
cargo test --no-default-features -- some_test_name                # single test
cargo fmt --all -- --check
cargo clippy --no-default-features --all-targets -- -D warnings
```

CI runs the test/clippy suite **both** with `--all-features` and with `--no-default-features`. When changing backend code, run both paths locally — the headless path is what tests and downstream consumers compile, and it drifts silently otherwise.

Release build (Linux needs `NO_STRIP=true` on binutils ≥ 2.41 — Fedora ≥ 40, Ubuntu 24.04+):

```bash
NO_STRIP=true npm run tauri:build
```

## Critical build ordering

`tauri::generate_context!()` checks at compile time that `../dist` exists. **Any `cargo` command that compiles the `desktop` feature requires `npm run build` to have produced `dist/` first.** This is why CI runs `npm run build` before any cargo step. Tests run with `--no-default-features` (no `desktop` feature) specifically to avoid this and the GTK/webkit system deps.

## Architecture

### Data flow (logging an expense)

User messages their bot → Telegram queues it → desktop's `telegram::poller` long-poll (`getUpdates`) receives it → `llm` dispatcher sends it to Anthropic/Ollama with a system prompt containing the user's categories + budget context → LLM returns a **tool call** (never raw SQL) → dispatcher validates against JSON schema → executes via the parameterized `repository` layer → Tauri event emitted (e.g. `expense:added`) → bot replies via `sendMessage`.

### Backend module layout (`src-tauri/src/`)

- `db/` — the **DbHandle actor**. As of the CC-1/Pf-4 work the SQLite connection lives on one dedicated OS thread; *all* DB access goes through `state.db_actor` (`.blocking_run(|conn| ...)`). Do not reintroduce `Arc<Mutex<Connection>>` or open the DB elsewhere. `db/migrations/*.sql` are forward-only, applied on actor spawn.
- `domain/` — `Expense`, `Category`, `Budget`, and `period.rs`. **`period.rs` is the single source of truth for budget math** — it backs both the LLM `summarize_period` tool and the GUI dashboard so they cannot disagree. Change budget math here, nowhere else.
- `repository/` — parameterized CRUD. The DB boundary is parameterized at all times; the LLM never emits SQL.
- `llm/` — provider trait + `anthropic.rs`/`ollama.rs` adapters, `dispatcher.rs` (tool-call validation/execution), `tools.rs` (schemas), `system_prompt.rs`, `pricing.rs`, `report.rs` (AI report generation, v0.4.0).
- `telegram/` — typed `client`, `poller` (long-poll loop), `router`, `auth` (6-digit household pairing codes), multi-user via authorized-chat whitelist.
- `insights/` — deterministic dashboard aggregation, forecasting, Monte Carlo simulator, debt, report engine. No LLM calls; fast/offline.
- `secrets/` — OS-keychain-backed store (service `moneypenny`) for bot token + Anthropic key. Never plaintext on disk.
- `scheduler/` — recurring rules, budget alerts, weekly summaries.
- `csv_import/` — parser, dedupe, categorize, AI-suggest.
- `commands.rs` — Tauri IPC commands exposed to React (the `invoke_handler!` list in `lib.rs`).
- `lib.rs` — Tauri builder, tray, single-instance, close-to-tray / `ExitRequested` guard, poller/scheduler launch.

### Frontend (`src/`)

`App.tsx` router → `wizard/` (setup wizard) and `views/` (Insights is the default landing view, plus Ledger, Categories, Household, Settings, CsvImport, Forecast, ReportWizard). `lib/tauri.ts` wraps IPC commands; `lib/store.ts` is the Zustand store. Charts via recharts; styling via Tailwind (theme tokens in `src/styles/`).

### Close-to-tray behavior

`tauri.conf.json` declares **no startup window**. `lib.rs` drives the event loop manually and intercepts `RunEvent::ExitRequested`: closing the window kills the WebKit process (the ~440 MB resident saving) but the app stays alive in the tray with poller + scheduler running, unless `RUN_IN_BACKGROUND=0` or the Quit menu set the `UserQuit` flag. `--silent` launch (autostart) skips building the window entirely. Be careful editing this flow.

## Privacy invariant

Outbound HTTP is restricted by the CSP allowlist in `src-tauri/tauri.conf.json`: `api.telegram.org`, `api.anthropic.com`, `localhost:11434` (Ollama), and the release-update endpoint (off in privacy mode). **Adding any new outbound endpoint requires updating the CSP and documenting it in `docs/privacy.md`** — this friction is intentional. No CDNs, fonts, analytics, or telemetry; all assets bundled.

## Conventions

- Source files carry SPDX headers (`scripts/add-spdx-headers.sh`); `#![forbid(unsafe_code)]` in the Rust crate.
- Versioning: each release is a `vX.Y.Z` commit that bumps `package.json` + `src-tauri/` versions and updates `CHANGELOG.md`. Audit-driven changes reference IDs like `CC-1`, `Pf-4`, `B-9` tracked in `docs/audit-*.md`.
- More build detail and platform prerequisites: `BUILDING.md`. Architecture deep-dive: `docs/architecture.md`. Threat model: `docs/threat-model.md`.
