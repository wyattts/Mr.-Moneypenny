# Changelog

All notable changes to Mr. Moneypenny are documented here. The format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1] - 2026-05-17

### Changed

- **Import and export now live at the top of the Ledger tab.** The CSV importer moved out of Settings, and the exporter moved from the bottom of the Ledger to the top, so the two bulk data flows — in and out — sit together where the ledger they act on is. Both panels are now self-contained components (`src/components/CsvImportPanel.tsx`, `DataExportPanel.tsx`) with their own status lines. Settings no longer has a "CSV import" section. No behavior or data-format changes.

## [0.5.0] - 2026-05-17

**The open hub — data export.** Your data is yours, and now that ownership is exercisable: a complete, faithful, one-way copy of the ledger to a file you choose. A local, user-initiated *snapshot* (no server, no live feed, no re-import), desktop-only (the Telegram bot never carries the file), reading a stable published view so the internal schema can evolve without breaking exports.

### Added

- **Ledger → Export your data.** Pick one format — ordinary **spreadsheet (CSV)**, **programmer file (JSON Lines)**, or **plain-text accounting (Beancount)** for the Fava ecosystem — and a timeframe: **All time** (the full portability copy, and the default) or this **year / quarter / month** for taxes / an accountant. Money stays exact (integer cents plus a signed decimal); refunds are signed negative. Timeframe bounds reuse the same `DateRange` logic as the dashboard and bot, so an exported "this year" matches what the app shows. The panel lives at the bottom of the Ledger tab.
  - **CSV** carries derived `date` (YYYY-MM-DD) and `month` (YYYY-MM) columns beside the exact timestamp, so a monthly pivot is one drag with zero formulas.
  - **Beancount** is balanced double-entry with every account opened, refunds tagged `#refund`, and `expense_id` / `source` / `logged_by` attached as metadata for Fava filtering and reconciliation. A self-describing provenance header (app version, schema, timeframe, generated-at, row count) is embedded as comments so the file stays understandable years from now — CSV and JSON Lines stay pure (provenance lives in the filename + docs) so their parsers don't choke.
- **`v_ledger_v1` view (DB migration 0017).** The published, versioned export contract — expenses resolved with category name/kind, budget context, refund sign, source, and who logged it. Exports read only this view, never the raw tables; a future breaking change would add `v_ledger_v2` and keep `v1`.

### Privacy

- Export is documented in `docs/privacy.md` and `docs/threat-model.md` as a deliberate, user-initiated, local-only egress that adds no new outbound path or attack surface. The stale "encrypted JSON or CSV" backup note was corrected to match the shipped feature.

## [0.4.2] - 2026-05-17

**Telegram bot budget answers.** "How am I doing this month" could come back as a raw transaction dump that asked the user for a budget the app already had, and category questions ("how's my gas budget") sometimes invented the wrong timeframe. Two underlying bugs were fixed and the bot's status/category handling was made consistent.

### Fixed

- **Bot now reliably sees the variable budget on status questions.** `summarize_period`'s `period` was a required enum with no default; when the model omitted it (or sent an empty string) the call errored or fell to a non-monthly window, and the budget — which loads only for monthly ranges — silently went to zero, so the bot asked the user for a figure it already had. `period` now defaults to `this_month` at the schema, the typed input (`#[serde(default)]`), and the dispatcher (empty string → `ThisMonth`), and the system prompt hard-routes every "how am I doing / how's my budget / am I on track" phrasing to `summarize_period`, never `query_expenses`.
- **Inactive-but-budgeted categories no longer vanish from the budget.** `query_active_targets` filtered `is_active = 1` while the spend query (`query_category_totals`) did not, so a deactivated category with a monthly target had its spend paced against a budget that excluded its allowance — making the user look worse than reality (reachable normally: `set_monthly_target` never touches `is_active`, and migration 0003 deactivates non-whitelisted seeds). The target query now covers the same category set as the spend query.

### Added

- **Category-scoped summaries.** `summarize_period` accepts an optional `category`; the result carries a server-built `category_focus` block and a headline scoped to just that category ("Dining Out: $75.00 spent this month of a $120.00 monthly budget — $45.00 left."), with the spend window and monthly-budget framing baked in so the model can't misreport the timeframe. Investing categories are phrased as goal progress rather than overspend.
- **Savings-goal compliment.** When period contributions meet or exceed the sum of investing targets, the headline gains a short "🎯 Savings goal met" note.

### Changed

- **More aggressive intent inference.** New operating principle: infer the likely intent and answer rather than asking a clarifying question, except on literal word salad or a genuinely ambiguous spend amount. All money remains server-formatted — the bot never does its own arithmetic.

### Notes

- 12 new regression tests (period-default routing, the inactive-budgeted asymmetry pinned at $700 active + $200 inactive, category-focus under/over/investing/no-budget, the monthly-window timeframe fix). Full headless suite green (281 tests), clippy `-D warnings` clean, fmt clean.

## [0.4.1] - 2026-05-17

**Telegram bot money accuracy.** The bot would sometimes misstate figures it had to convert from cents itself — e.g. reporting "$5.52/day" of variable budget left when the correct figure was "$55.24/day". The per-day math in `domain/period.rs` was always correct; the language model (Haiku) was doing the cents→dollars division in its head and getting it wrong. The fix removes model-side money arithmetic entirely.

### Fixed

- **Bot no longer computes money — the server formats it.** Every tool result now carries, for each `*_cents` integer, a sibling `*_display` string pre-formatted in the user's currency via the same `format_money` used in Telegram replies (`enrich_money_display` in `llm/dispatcher.rs`, applied centrally to all tool outputs and recursing through nested objects/arrays). Raw `*_cents` remain for the model's reasoning; null targets stay null rather than rendering "$0.00"; existing `*_display` keys are never overwritten.
- **`summarize_period` returns a ready-to-speak `headline`** assembled server-side (e.g. "On pace this month — $1712.44 variable left, about $55.24/day for 31 more days."), so the bot relays a string instead of recomposing numbers.

### Changed

- **System prompt and `summarize_period` tool description** now instruct the model to quote `*_display`/`headline` verbatim and never divide `*_cents` or otherwise do money math, and to open summaries with `headline` and stop reciting the full snapshot (tighter, faster replies on Haiku). The shared `DashboardSnapshot` Rust struct and the GUI/Tauri path are unchanged — enrichment happens only on the bot's JSON result.

### Notes

- 3 new regression tests, including `enrich_fixes_the_5524_cents_bug` pinning `5524 → "$55.24"` (not "$5.52"). Full headless suite green (274 tests), clippy `-D warnings` clean, fmt clean.

## [0.4.0] - 2026-05-15

**AI Report Wizard.** A new section at the bottom of the Insights tab generates a written analysis of a chosen period. Every number in a report is computed deterministically in Rust from your local data; the language model only narrates and advises over those figures, so the quantitative content is reproducible and cannot be hallucinated. Off the critical path — the rest of Insights is unchanged until you use it.

### Added

- **Deterministic report engine** (`insights/report.rs`). `ReportTimeframe` (Last week / Last month / Last quarter / Last year → the *previous complete* calendar period; or a custom date range). Pure, read-only computation of eight sections: spending rebalance (actual vs. prorated budget, with a suggested revised target), variable spend cycles (by weekday and early/mid/late month), suggested cuts, a subscription/recurring radar, savings rate (when a monthly income is supplied), anomalies vs. the prior equal-length window, wins, and an optional goals input. 10 unit tests.
- **LLM report generation** (`llm/report.rs` + `report_estimate` / `report_generate` commands). Constrained-JSON output contract (the UI renders structured fields, never raw model markup; the PDF layout is deterministic). The optional financial-situation blurb is wrapped in `<user_data>` tags and the system prompt treats it — and user-defined labels in the figures — as data, never instructions (the LLM-1 prompt-injection stance). Anthropic generation is fixed to Claude **Sonnet**; Ollama users generate locally with their configured model at no cost. A **pre-flight cost estimate** is shown and confirmed before any API call; report spend is logged to `llm_usage` and refused server-side once the existing daily cost cap is reached, exactly like the bot.
- **Report Wizard UI** (`views/ReportWizard.tsx`). Timeframe presets + custom range, section checkboxes, optional monthly-income field, and an enable-gated financial-situation note. Provider-aware: the merchant-names opt-in (off by default, with a disclaimer that names would be sent to Anthropic) is hidden and auto-enabled under Ollama since nothing leaves the machine; the blurb disclaimer and the cost summary likewise adapt (Ollama reads "no API cost"). Pre-flight estimate → confirm → render, with a local-figures provenance footer.
- **PDF export.** Client-side `pdf-lib` renders the report (paginated A4, word-wrapped headings/bullets, API-use cost summary, provenance footer); `@tauri-apps/plugin-dialog`'s save dialog picks the path; the bytes are persisted through a new typed `report_save_pdf` command rather than granting the webview a broad filesystem capability. New permission: `dialog:allow-save`.

### Notes

- Carry-over deferred audit items remain external-gated (B-8 code signing — funding; F-22 CSP nonce — Recharts upstream; T-6 proptest/fuzz — separate effort).

New Simulator feature: an optional **deterministic mode** for users who want a single straight-line investment projection instead of the Monte Carlo probability framing. Off by default — the probability view is unchanged unless you turn this on.

### Added

- **Deterministic mode toggle in the Simulator** (off by default). A new `deterministic` flag on `SimulatorCommonInputs` (serde-default `false`, so every existing caller and the IPC wire format are unaffected). When on, the backend forces volatility to 0 and simulates a single path, which collapses the existing Monte Carlo engine to an exact closed-form compound-growth projection — no new math, the GBM path simply degenerates. Effects when enabled:
  - **Backend** (`insights/simulator.rs`): two centralized helpers (`det_volatility`, `det_n_paths`) applied at all three `PathInput` build sites (`mc_input`, the `solve_required_contribution` final run, the `compute_probability` run). The 144-cell heatmap drops from 144 × n_paths sims to 144 single-path sims and becomes a sharp reach/short feasibility map. New unit test `deterministic_mode_collapses_variance` pins the guarantee: zero-width band, probability ∈ {0, 1}, and final value within $1 of the closed-form annuity FV — with a non-deterministic control asserting the band still spreads when the flag is off.
  - **Frontend** (`views/Forecast.tsx`): the now-meaningless controls hide when the toggle is on — the volatility override disclosure, the confidence slider/presets (replaced with a one-line note in "Find required" mode), the zero-width chart band + its legend, and the final-value distribution histogram. In "Show probability" mode the 0%/100% percentage is replaced with a **"Projected final value"** readout plus a "Reaches target with $X to spare" / "Short of target by $X" status line. The "Find required" sub-line drops the confidence/σ phrasing and reads "exactly reach … (deterministic — no variance)". Chart caption switches to "deterministic projection".

## [0.3.25] - 2026-05-15

Audit cleanup: the last Deferred-bucket item that could be closed without external dependencies. B-9 — `bundle.targets` was `"all"`, which built **both** NSIS and MSI installers on Windows, doubling the unsigned-binary footprint on the release page and confusing users about which to download.

### Changed

- **`bundle.targets` constrained to `["app", "dmg", "appimage", "deb", "rpm", "nsis"]`** (audit B-9). Windows now ships a single installer (NSIS `.exe`) instead of NSIS + MSI; macOS still gets `.app`/`.dmg`, Linux still gets AppImage/deb/rpm. The release-attestation `subject-path` glob dropped its now-dead `msi/*.msi` line accordingly. No change to the in-app auto-updater (driven by the minisign-signed `.AppImage.tar.gz`/`.app.tar.gz`/`.nsis.zip` bundles, all still produced).

## [0.3.24] - 2026-05-14

The last four audit-flagged Mediums ship together: SLSA build-provenance attestations on releases (B-3), focus-trap + ARIA dialog semantics on the CSV import modal (F-4), 200 ms slider debouncing on the Simulator + Debt Manager views (Pf-1), and an integration-test file for the Tauri IPC layer (T-1). With v0.3.24 every Medium-severity finding from `docs/audit-v0.3.7.md` is shipped.

### Added

- **SLSA build-provenance attestations on every release artifact** (audit B-3). The release matrix now runs `actions/attest-build-provenance@v2` after the artifact-upload step; each leg signs its own AppImage/deb/rpm/dmg/.app.tar.gz/msi/exe via Sigstore's keyless OIDC flow and registers the bundle with GitHub's attestation API. Users verify with `gh attestation verify <file> -R wyattts/Mr.-Moneypenny`. Complements the existing minisign updater signing (in-app auto-updates) and GPG-signed AppImage (manual downloads) — provenance defends against post-build tampering of the release page itself. Job permissions gained `id-token: write` + `attestations: write`.
- **Focus-trap + ARIA dialog semantics on the CSV import modal** (audit F-4). New `useDialogFocusTrap` hook in `views/CsvImport.tsx` does the standard a11y move: on open, capture the previously-focused element and shift focus into the dialog; on Tab/Shift+Tab at the boundary, cycle within the modal so users can't fall back into the underlying app; on Escape, close; on close, restore the prior focus. The panel itself gets `role="dialog"`, `aria-modal="true"`, and `aria-labelledby` wired to the h2's new id. ~80 LOC, no new dependency.
- **`tests/integration_commands.rs`** (audit T-1) covering the highest-value commands the audit called out: the `list_expenses` date-window filter (end-date inclusive of same-day rows; off-by-one regression test); `list_expenses` search + limit-clamp + ordering; `csv_import_commit` happy path (rules + profile-touch + insert) and rollback on mid-batch parse error; refund cascade behavior (the FK is `ON DELETE SET NULL`, not cascade, which the test now pins). 8 new tests.

### Changed

- **Slider-driven Simulator + Debt Manager recomputes are debounced 200 ms** (audit Pf-1). Dragging a slider used to fire one Tauri command per tick — at 60 fps a 1-second horizon-slider drag spawned ~60 Monte Carlo + heatmap pairs, each running to completion on the Rust side regardless of whether the JS-side cancellation flag dropped its result. New `useDebouncedValue` hook (in `src/lib/debounce.ts`) wraps the inputs to the three recompute effects (Simulator, single-debt, debt portfolio); the effect re-keys on the debounced values so a continuous drag fires one compute at the end instead of one per tick.
- **`commands.rs::list_expenses` and `csv_import_commit`** each gained a plain-helper twin (`list_expenses_query(conn, &filters, tz)` and `csv_import_commit_inner(conn, &input, now)`). The `#[tauri::command]` wrapper is now a one-liner that forwards through the DbHandle actor. The split enables the T-1 tests above; no behavior change.

### Internal

- `release.yml` matrix job permissions go from `contents: write` only to `contents: write` + `id-token: write` + `attestations: write`. The new step is keyed on the platform-specific bundle paths so each leg attests its own artifacts.
- `useDialogFocusTrap` is inline-implemented (no `focus-trap` npm dep) — the modal has one entry point and 40 lines of focus management are auditable.
- Test-only `_shut_up_unused` helper in `integration_commands.rs` keeps a few imports referenced even when individual tests evolve. (Defensive — drops if anything actually needs them later.)

After v0.3.24 the audit roadmap is fully shipped: every High and every Medium from `docs/audit-v0.3.7.md` is in `main`. The Low and Info findings remain as opportunistic cleanup.

## [0.3.23] - 2026-05-14

Phase 4 of the audit CC-1/Pf-4 fix: the legacy `Arc<Mutex<Connection>>` field is gone. `DbHandle` is now the only DB-access primitive in the codebase. The migration that began with v0.3.19's phase 1 (actor abstraction + smoke-test handler) is complete.

### Added

- **`DbHandle::blocking_run`** for sync callers — Tauri's `setup()` hook, the `RunEvent::ExitRequested` callback, and the `ensure_poller_running` / `ensure_scheduler_running` startup helpers all live outside the Tokio runtime and now go through this. Internally uses a `std::sync::mpsc::sync_channel` for the reply so it works without a Tokio context. New unit test (`blocking_run_returns_closure_value_without_a_tokio_runtime`) verifies it runs from a plain `#[test]`.

### Removed

- **`AppState.db: Arc<Mutex<Connection>>`** — gone. `AppState::new(db_actor: DbHandle)` is the new signature.
- **`RouterDeps.conn: Arc<Mutex<Connection>>`** — gone. All telegram/scheduler/router code paths reach the DB via `deps.db_actor`.

### Changed

- **`lib.rs setup()`** — the `setup_complete` check and `apply_initial_defaults`'s settings reads/writes now route through `state.db_actor.blocking_run(...)`. The first-run defaults pass folds the bg-mode + autostart read/write into one actor round-trip so the two settings stay consistent across crashes mid-init.
- **`lib.rs ExitRequested` handler** — `RUN_IN_BACKGROUND` lookup goes through the actor. On any DB error the handler falls back to `bg_mode = true` (keep tray alive), which is the safer default than letting the process exit silently.
- **`app_state::ensure_poller_running` / `ensure_scheduler_running`** — each is one actor round-trip that builds the LLM provider, reads the default currency, and (for scheduler) ensures the singleton WeeklySummary + BudgetAlertSweep jobs exist. `secrets::retrieve` calls inside `build_llm_provider` run on the actor thread; that's safe because the secrets module has its own file mutex independent of the actor's channel.

### Internal

- Both integration test fixtures (`tests/integration_telegram.rs::make_deps`, `tests/integration_scheduler.rs::fresh_deps`) drop the `conn:` field from their `RouterDeps` construction. They still hold a Mutex-wrapped `Connection` separately so test code can seed fixtures via direct SQL — WAL keeps that in sync with the actor's separate Connection on the same on-disk file.
- After this batch the codebase has a single DB-access API. Anyone wanting to read or write SQLite goes through `state.db_actor.run(...)` (async) or `state.db_actor.blocking_run(...)` (sync). The audit's CC-1, CC-4, and CC-6 findings are all closed.

## [0.3.22] - 2026-05-14

Phase 3b of the audit CC-1/Pf-4 fix: the Telegram poller and the router's LLM agentic loop are migrated off `Arc<Mutex<Connection>>`. Every async-hot-path DB callsite in the app now flows through the `DbHandle` actor; phase 4 will retire the legacy Mutex field entirely.

### Changed

- **`telegram/poller.rs`**: `read_offset`, `already_processed`, `persist_offset_and_mark`, and `gc_processed_updates` are now async and route through the actor. The `persist_offset_and_mark` transaction (idempotency row INSERT + `last_update_id` UPDATE in one atomic commit) runs entirely on the actor thread — Tokio workers no longer block on the poller's DB writes.
- **`telegram/router.rs::handle_update`**: the auth gate (`auth::is_authorized`), `/cancel`'s pending-clear, `handle_start`'s pairing-code redemption, `handle_undo`'s find + delete, and the LLM-side callsites in `handle_free_text` (system-prompt build, `llm_usage::log`, dispatcher tool execution) all migrated. 14 lock sites total.
- **`daily_cap_refusal_text`** became `async`. Both the cap setting read and the `llm_usage::today_cost_micros` rollup happen in one actor round-trip; the function returns `None` (let the call through) on any DB error, preserving the "best-effort" gate from v0.3.15.
- **`try_resolve_pending_confirmation`** now serializes its DB work as 3 actor calls: load pending, load rule, then either the insert-expense-and-clear closure (yes path) or the clear-pending closure (no/skip path). User-visible reply text is composed in Rust after the actor returns to keep strings out of the SQL hot path.
- **Tool dispatcher integration**: `dispatcher::execute(conn, ctx, ...)` is now invoked inside `deps.db_actor.run(move |conn| Ok(dispatcher::execute(conn, ...)))`. The dispatcher's sync signature is preserved — only the surrounding wrapper changed.

### Internal

- `tests/integration_telegram.rs::make_deps` rewritten to the shared-tempfile pattern (matching `integration_scheduler.rs` from v0.3.20). Both the Mutex Connection and the DbHandle open against the same on-disk SQLite file under a `TempDir`; SQLite WAL handles the two simultaneous writer-eligible connections. The old `fresh()` helper that returned a private in-memory Connection is gone; all 17 test sites updated to `let (deps, conn_arc, _tmp) = make_deps(llm, tg);` with the `_tmp` TempDir held on the test's stack to outlive the actor.
- After this batch, the `db: Arc<Mutex<Connection>>` field on `AppState` + `RouterDeps` is still alive but only used by: `app_state::ensure_poller_running` / `ensure_scheduler_running` (startup-only sync code), `lib::run`'s setup hook (one-time at boot), the `ExitRequested` handler in the run callback, and various test fixtures that still hand a Connection around. Phase 4 removes the field once those last few startup paths migrate to the actor or accept a path argument.
- One subtle `clippy::clone_on_copy` lint surfaced in the router's usage-logging closure (`Usage` derives `Copy`); fixed with a plain move.

## [0.3.21] - 2026-05-14

Phase 3a of the audit CC-1/Pf-4 fix: every Tauri command in `commands.rs` now routes its DB work through the `DbHandle` actor instead of locking the legacy `Arc<Mutex<Connection>>`. Combined with phase 2's scheduler migration, the only async-hot-path code still on the Mutex is the telegram poller + the router's LLM agentic loop (queued for phase 3b).

### Changed

- **All 48 lock sites in `commands.rs` migrated.** Roughly 40 Tauri commands now look like `state.db_actor.run(move |conn| { /* sync body */ }).await.map_err(err)` instead of `let conn = state.db.lock().unwrap(); /* sync body */`. The migration is type-level only — no semantic change for callers.
- **Multi-query handlers fold their reads into one closure.** Pre-migration, handlers like `get_setup_state`, `finalize_setup`, `csv_import_preview`, and `csv_import_ai_suggest` issued multiple sequential SQL statements while holding the lock. Each now becomes a single actor round-trip whose closure runs the queries against `&mut Connection`. Same total work, one trip across the Tokio↔actor boundary instead of N.
- **`csv_import_commit`** keeps its transaction wrapper, now inside the actor closure: `let tx = conn.unchecked_transaction()?; … tx.commit()?`. The transactional INSERT loop for bulk imports happens entirely on the actor thread without blocking any Tokio worker.

### Internal

- The actor handles every command-side query type: simple settings reads/writes, FK-cascading deletes, multi-row aggregations (`list_investment_categories`, `run_scenario`), the bulk CSV import transaction, and complex closures like `csv_import_ai_suggest` that read 5 settings + a category list before any LLM round-trip happens.
- Handler return-type quirks worth noting: `set_category_target`, `set_category_active`, `set_starting_balance`, `delete_csv_import_profile`, and `delete_merchant_rule` all wrap repository functions that return `Result<bool>` but the commands surface `Result<()>` — those callsites now do `repo_fn(conn, …)?; Ok(())` inside the closure to discard the bool.
- `secrets::*` calls remain outside the actor (they go through a separate concurrency model — disk-encrypted file with its own internal mutex), so handlers that mix secrets + settings (e.g., `get_setup_state`, `finalize_setup`) do their secret checks before/after the actor round-trip.
- Phase 3b (telegram poller + router LLM loop) is queued as a separate ship to keep the diffs reviewable.

## [0.3.20] - 2026-05-14

Phase 2 of the audit CC-1/Pf-4 fix: the entire scheduler subsystem is migrated off `Arc<Mutex<Connection>>` and onto the v0.3.19 `DbHandle` actor. Tokio worker threads no longer block on sync rusqlite for any scheduler-side work — the actor thread serializes SQL while workers await a `oneshot`. 16 lock-and-query sites identified by the audit are gone.

### Changed

- **`RouterDeps`** now carries `db_actor: DbHandle` alongside the legacy `conn: Arc<Mutex<Connection>>`. The two fields share the same `AppState` actor (one clone forwarded), so the single-writer invariant holds across the scheduler and (still-Mutex-path) router. The Mutex field is retained for phase 3 callsites; phase 4 will remove it.
- **`scheduler/mod.rs::tick`** — 3 lock sites migrated. The pre-handler `list_due`, the stale-job bump, and the post-handler reschedule/disable each become a `db_actor.run(move |conn| …)` await. No semantic change in retry / `JobOutcome` handling.
- **`scheduler/recurring.rs::handle`** — 5 lock sites migrated. The rule fetch, the expense insert in auto mode, the owner lookup in confirm mode, the chat-has-pending check, and the pending-row insert all route through the actor. Removed the dead `let _ = params![]` no-op (and the now-unused `rusqlite::params` import).
- **`scheduler/budget_alerts.rs::handle`** — 7 lock sites migrated and consolidated. The toggle + owner lookup are folded into one round-trip; the categories + currency fetch into another; the per-category SUM + "already-alerted threshold list" pulls into one round-trip per category (down from two locks per loop iteration). The fire-and-record loop's INSERT happens via the actor.
- **`scheduler/weekly_summary.rs::handle`** — 6 lock sites collapsed into 2 actor round-trips. The toggle + owner check fold together at the top; the 7-day rollup (total, count, top-3 categories, currency) fold into a single closure that runs all four queries on the actor thread before returning.

### Internal

- **Test bench**: `tests/integration_scheduler.rs::fresh_deps` now creates a `tempfile::TempDir` and opens both the legacy `Connection` and the `DbHandle` against the same on-disk SQLite file. In-memory DBs (`open_in_memory`) are per-Connection-private, so the actor wouldn't see seed rows the test inserts via the Mutex path. All 12 call sites updated to the 4-tuple return (the `TempDir` is held on the test's stack to keep the file alive). `tests/integration_telegram.rs::make_deps` adds a separate in-memory `DbHandle` placeholder since the router still routes through the Mutex.
- The scheduler's overall round-trip count across `tick` + each handler dropped from 16 lock acquisitions to roughly 10–12 actor calls, mostly because the budget-alert and weekly-summary handlers now batch related reads into single closures. Each actor call has higher per-call overhead than a Mutex lock but the Tokio runtime stays responsive throughout, which is the audit's actual concern.

## [0.3.19] - 2026-05-14

Phase 1 of the audit CC-1/Pf-4 fix: a single-writer SQLite actor lands alongside the existing `Arc<Mutex<Connection>>` API. No callsite migration this release beyond one smoke-test handler — the actor is exercised in production to validate the abstraction before later phases move scheduler + commands.rs over to it.

### Added

- **`src-tauri/src/db/actor.rs`**: `DbHandle::spawn(conn)` and `spawn_from_path(path)` give callers a cheap-to-clone async handle. `DbHandle::run(|conn| -> Result<T>)` submits a closure to the actor's dedicated OS thread; the closure runs synchronously with exclusive `&mut Connection`, the result returns via a `oneshot::channel` the caller awaits. Bounded `mpsc::channel(64)` provides natural backpressure. Closures that panic are caught (`catch_unwind` + `AssertUnwindSafe`) and surface as `Err("db closure panicked: …")` rather than killing the actor thread. Dropping the last `DbHandle` clone closes the channel; the actor exits cleanly. 7 unit tests cover the round-trip, mutate-then-read across calls, concurrent submissions, panic recovery, error propagation, drop-shutdown, and dropped-caller paths.
- **`AppState.db_actor: DbHandle`** ships alongside `AppState.db: Arc<Mutex<Connection>>`. New handlers should reach for `db_actor`; legacy handlers continue using `db` until phases 2-3 migrate them. The actor opens its own `Connection` to the same SQLite file — WAL mode handles two simultaneous connections cleanly. Connection count goes up by 1; resident-memory cost is ~50 KB.
- **`get_check_updates_on_launch` and `set_check_updates_on_launch`** migrated to the actor as a smoke test. Read + write coverage on a low-blast-radius pair (read once on Settings load, write on toggle). Both still work identically from the UI — the migration is type-level only.

### Internal

- `AppState::new()` signature changed from `new(db: Connection)` to `new(db: Connection, db_actor: DbHandle)`. Only one caller (`lib.rs`), updated.
- Migration runs twice at startup (once via the `Mutex<Connection>` path, once via `DbHandle::spawn_from_path`). Migrations are idempotent so the second call is a no-op against the already-migrated DB. Slight redundancy in service of the phased rollout.
- Audit cross-references for this work: CC-1 (sync rusqlite on Tokio workers), CC-4 (Mutex-across-await fragility), CC-6 (scheduler unwraps on a poisoned lock). The actor pattern is the recommended fix for all three; phases 2-4 migrate the affected sites.

## [0.3.18] - 2026-05-14

Frontend bundle is now code-split (audit Pf-3) and the release workflow is race-safe across multi-tag pushes. Pairs naturally with v0.3.17's close-window-on-tray work — every tray-click reload now parses a 65 KB initial shell instead of the previous ~225 KB monolith, with view chunks fetched on demand.

### Changed

- **Routed views are lazy-loaded** via `React.lazy` + `Suspense` (audit Pf-3). `App.tsx` defers Insights, Forecast, Ledger, Categories, Household, and Settings to dynamic imports; only the layout shell (`MainApp` + Wizard chrome) is in the initial chunk. The bundler emits a shared 386 KB Recharts chunk (`LineChart`) that loads only when a chart-using view (Insights or Forecast) is first rendered.
- **Suspense fallback** is scoped to the view region inside `MainApp.tsx`'s `<Outlet>`, so the sidebar and header stay mounted while a route chunk fetches — only the main view area shows the brief "Loading…" placeholder. From-disk chunk loads in Tauri are typically <100 ms so the placeholder usually flashes imperceptibly.

### Internal

- `release.yml` promote step is now **race-safe across multi-tag pushes**. When `git push` ships multiple release tags in one go (as we did with v0.3.13–v0.3.15 and again with v0.3.16/v0.3.17), each tag spawns its own workflow run and the matrix builds finish in arbitrary order. The old unconditional `gh release edit … --latest` meant whichever promote ran *last* won the latest flag, even if it was the older version — which is exactly what bit us on v0.3.17 (built first, but v0.3.16 finished its matrix later and stomped the latest flag). Promote now computes the highest non-prerelease tag across all releases (including draft siblings still being built) and only flags `--latest` when it matches ours. Both race orders converge to "highest tag = latest" without manual intervention.
- Bundle sizes (gzipped) after the split: 65 KB initial shell + 11–18 KB per content view + 106 KB shared Recharts chunk. Total transferred per cold load with default route (`/insights`): ~190 KB gzipped vs ~225 KB before; total transferred for non-chart routes (Settings, Ledger, etc.): ~75 KB gzipped.
- Dependabot bumps merged this batch: `tokio 1.52.1 → 1.52.3`, `postcss 8.5.12 → 8.5.14`, `hkdf 0.12 → 0.13`, `sha2 0.10 → 0.11`, `machine-uid 0.5 → 0.6` (perf-only: macOS now uses `gethostuuid(3)` instead of shelling out to `ioreg`; UID value unchanged so existing users' KDF-derived secrets still decrypt), `actions/checkout 4 → 6`, `actions/setup-node 4 → 6`.

## [0.3.17] - 2026-05-11

Background-mode RAM optimization. Measured a v0.3.x install sitting in the tray and found ~560 MB resident, of which ~545 MB was the WebKit process and its IPC shared memory mapped into the parent. Killing the webview on window close drops tray-mode resident to roughly the Rust binary + plugins, in the ~30–50 MB range. First open from the tray now reloads the bundle (perceived latency: ~0.5–1 s on a warm machine); all subsequent UI is the same as before.

### Changed

- **The window is destroyed on close, not hidden.** When `RUN_IN_BACKGROUND=1` (default), the tray icon + scheduler + poller keep running so notifications and recurring rules still fire; the WebKit process exits, releasing ~440 MB resident on Linux. When the user clicks the tray icon (left-click or "Open Mr. Moneypenny") the window is rebuilt — bundle reloads, fresh React tree.
- **Autostart (`--silent`) no longer creates a window at all.** Previously, autostart launched the app, built the window, then immediately hid it — leaving the WebKit process loaded for the lifetime of the session. Now silent launches go straight to tray-only mode; the window is only built when the user actually asks for it.
- **`RUN_IN_BACKGROUND=0`** still exits the app cleanly on window close (no semantic change). The setting now means "should I keep the tray running after you close the window?"

### Internal

- `tauri.conf.json` no longer declares a startup window (`app.windows: []`). The window is constructed programmatically by `show_or_create_main_window()` with the same size/theme/decorations the JSON used to specify.
- New `UserQuit(Arc<AtomicBool>)` managed state lets the Quit menu item flip a flag before calling `app.exit(0)`, so the `RunEvent::ExitRequested` guard can tell "user clicked Quit" apart from "user closed the window."
- Switched from `Builder::run()` to `Builder::build().run(callback)` to get the event-loop callback. The callback `prevent_exit()`s while `RUN_IN_BACKGROUND=1` and the user hasn't quit; otherwise it lets the process exit normally.
- The old `on_window_event` close-prevention handler is gone — close requests now actually close the window.

## [0.3.16] - 2026-05-11

Two small v0.4.0-roadmap items lifted forward: composite expense index (D-4) and SPDX file headers across the source tree (Co-3). No user-visible behavior changes.

### Added

- **Migration 0016** creates `idx_expenses_category_occurred ON expenses(category_id, occurred_at)`. Targets `expenses::list_in_range_by_category` and `monthly_totals_for_category` (Category Analyzer hot path), which previously picked `idx_expenses_category_id` and post-filtered by date. Pre-existing single-column indexes are left in place so the planner can still pick them for non-category-scoped queries.
- **SPDX license headers** on every `.rs`, `.ts`, and `.tsx` source file under `src/`, `src-tauri/src/`, and `src-tauri/tests/` — 93 files. Two-line block: `SPDX-License-Identifier: AGPL-3.0-or-later` + `Copyright (C) 2026 Wyatt Smith and contributors`. Helps downstream forks and matters when files are vendored without the repo LICENSE.
- **`scripts/add-spdx-headers.sh`** runs the header insertion idempotently — skips files that already have `SPDX-License-Identifier`. Safe to re-run on every new source file going forward; could be wired into CI later if drift becomes a concern.

### Internal

- `db::tests::category_occurred_index_chosen_by_planner` pins the planner choice via `EXPLAIN QUERY PLAN`. If a future migration accidentally drops the composite index, the test fails and surfaces the regression before users feel it.
- `integration_db::migrations_are_idempotent` + `db::tests::upgrade_migration_preserves_engaged_categories` now expect `user_version = 16`.

## [0.3.15] - 2026-05-11

Closes the last two open audit items from the v0.3.9 defense-in-depth batch: prompt-injection mitigations on the LLM tool-result echo-back path (LLM-1) and cost-guardrails on the agentic loop (Pf-6). With this release the entire v0.3.9 audit batch from `docs/audit-v0.3.7.md` is shipped.

### Added

- **Daily LLM cost ceiling** (audit Pf-6). New `llm_daily_cost_cap_micros` setting, default $1.00/day. Once today's `llm_usage.cost_micros` sum hits the cap the Telegram bot politely refuses further free-text turns ("You've hit today's LLM budget — bump it in Settings → API usage.") and never invokes the provider. Ollama rows cost zero, so the cap never trips on local-only setups. Tauri commands: `get_llm_daily_cost_cap_micros`, `set_llm_daily_cost_cap_micros`.
- **Settings → API usage** has a "Daily LLM budget" input (dollars/day; 0 disables).
- **Per-message 2 KB cap on incoming Telegram free text** (audit Pf-6). Messages over the cap get a friendly refusal and never reach the LLM. Slash commands are exempt.
- **`<user_data>...</user_data>` wrapping** on every user-supplied string echoed back to the LLM via `tool_result` (audit LLM-1). Wraps expense descriptions (`query_expenses`), category names (`list_categories`), recurring-rule labels (`list_recurring_rules`), and member display names (`list_household_members`). Each wrapped string is truncated to 256 chars; nested `<user_data>` tags are stripped before wrapping so an attacker can't fake a close.
- **System-prompt clause** instructing the model that anything inside `<user_data>` tags is data, never instructions — with worked examples showing how to refuse a description that says "ignore prior instructions" and a category name that asks for extra tool calls.
- **Tool-result byte cap** (audit P-9 / LLM-1). Tool responses serialized to >8 KB are replaced with a short `{"ok":false,"error":"result_too_large"}` stub that nudges the model to narrow its query, so a single oversized `query_expenses` can't dominate the next-turn context.
- **`llm_usage::today_cost_micros(conn, now)`** helper backing the cost-cap check.

### Changed

- **`MAX_AGENT_ITERATIONS` bumped from 5 to 8** (audit Pf-6). Legitimate multi-tool flows ("how am I doing this month and oh also log $5 coffee") were tripping the old ceiling more than the threat model called for. The cost cap is now the load-bearing ceiling, not the iteration count.

### Internal

- 4 new dispatcher unit tests pin the wrap/truncate behavior + a nested-tag stripping case.
- 2 new integration_telegram tests pin the oversize-message refusal and the daily-cap refusal paths (LLM is asserted never-called in both).
- 3 existing integration_dispatcher tests updated to expect `<user_data>` wrapping on descriptions, category names, and rule labels.

## [0.3.14] - 2026-05-11

Security defense-in-depth batch (audit items S-1, S-3, S-4, S-5 from `docs/audit-v0.3.7.md`). Closes the last audit-flagged High severity (pairing-code brute force), removes the expired-vs-invalid oracle, gates the Ollama endpoint behind URL validation + a remote-host opt-in, and tightens the Telegram poller against crash-window duplicate inserts. Includes migration 0015 that adds three new tables/columns.

### Added

- **Migration 0015 (`0015_telegram_hardening.sql`)** introduces:
  - `telegram_redemption_attempts` — per-chat rate-limit state with an escalating cooldown ladder.
  - `telegram_redemption_global` — single-row counter for the per-minute global ceiling.
  - `kind` column on `telegram_pending_pairings` — `'owner'` or `'member'`; the redeemer's role now comes from the code, not from "first redeemer wins."
  - `processed_telegram_updates` — idempotency table for the poller (S-5).
- **`PairingKind` enum** (`telegram/auth.rs`) with `Owner` / `Member` variants. `generate_pairing_code()` takes a `PairingKind`; `redeem_pairing_code()` uses the code's stored kind to decide the role.
- **Tauri command surface**: `generate_pairing_code` now takes `is_owner_invite: bool`; new `get_ollama_allow_remote` + `set_ollama_allow_remote` commands.
- **Settings → Local Ollama** section with a "Allow remote Ollama endpoint" toggle. Default off; toggle on if you run Ollama on a LAN/public host you trust.
- **`llm::ollama::validate_endpoint()`** — used by `save_ollama_config` and `list_ollama_models` to gate the URL before reqwest sees it. Enforces `http`/`https` scheme, ≤2048 chars, and loopback/RFC1918/ULA host unless the Settings opt-in is enabled. 9 new unit tests cover the rules.
- **Poller idempotency** (`telegram/poller.rs`): pre-check against `processed_telegram_updates` skips re-processing on restart; the post-handle bump of `last_update_id` and the idempotency-row insert share one transaction so a crash between them cannot reopen the v0.3.7 duplicate window.

### Changed

- **Pairing codes are now 8 digits** (was 6). Code space goes from 10⁶ to 10⁸; pairs with the new rate-limit layers to make brute-force enumeration impractical even at Telegram's flood-rate ceiling. Still leading-zero-padded decimal — copy-paste friendly on phones.
- **Per-chat rate limit on `redeem_pairing_code`**: 5 wrong codes in 60s triggers an exponential cooldown ladder — 30s → 1m → 5m → 30m → 24h. Each successive trip moves up one step; a successful redemption deletes the row and resets the level.
- **Global ceiling**: across all chats, >10 invalid attempts in any rolling minute pauses every redemption for 30s. Defends against Sybil attackers spinning up many chat_ids that the per-chat counter would not catch.
- **Expired-vs-invalid oracle collapsed** (audit S-3): both paths return the single user-facing string `"invalid or expired pairing code"`. Internal distinction stays in `tracing::debug` for support. The router's `pairing_failed_text` wraps both in the same "Couldn't pair this chat" reply.
- **Owner role can no longer be claimed by being first to redeem on a fresh install** (audit S-1 ownership hardening). The wizard's first-time pairing passes `is_owner_invite: true`; Settings rotation passes `true` only when the authorized-chat list is empty (post-factory-reset); the Household invite flow always passes `false`. A brute-forced member code cannot escalate to Owner.

### Fixed

- **Telegram poller no longer double-inserts expenses on a crash between `handle_update` and `persist_offset`** (audit S-5). The narrow residual window — between the expense INSERT inside `handle_update` and the post-handle transaction — is microseconds and will only fully close with a v0.4.0 transaction-handle refactor (documented inline in `telegram/poller.rs`).

### Internal

- New helpers in `telegram/poller.rs`: `already_processed()` (idempotency check), `persist_offset_and_mark()` (atomic offset+idempotency commit), `gc_processed_updates()` (drops rows older than 30 days, runs from the poll loop's existing housekeeping branch). 3 unit tests pin the SQL.
- 4 new tests in `telegram/auth.rs`: per-chat cooldown trip, per-chat cooldown escalation across multiple windows, global pause across multiple chats, successful-redeem clears the attempts row.
- `clear_all()` now also wipes the rate-limit state so a factory reset really starts clean.
- `url = "2"` added as a Rust dependency for endpoint validation. (~30 KB to the bundle.)
- `OnDiskFile` / `SecretsFile` and other v0.3.13 internals untouched.

## [0.3.13] - 2026-05-11

Audit-roadmap cleanup batch (items 18–22 from `docs/audit-v0.3.7.md`): drops the OS-keyring crate, adds a third-party-notices bundle, sets up Dependabot, hardens secret handling in the wizard + Settings token-rotation flows, and adds `aria-pressed` to every pill-toggle group in the UI. No user-visible behavior changes beyond the new notices link in Settings → About.

### Added

- **Third-party notices** under `src-tauri/notices/` (`THIRD_PARTY_RUST.md` for Rust crates, `THIRD_PARTY_NPM.txt` for npm packages, `README.md` explaining the layout). Both files are committed to the repo, shipped inside the release bundle via `tauri.conf.json` → `bundle.resources`, and regenerated by `scripts/generate-notices.sh` (cargo-about + license-checker-rseidelsohn). Surfaced in Settings → About as a link to the directory on GitHub.
- `src-tauri/about.toml` + `about.hbs` — cargo-about config + Handlebars template driving the Rust notices generator. License whitelist gates on copyleft drift.
- `.github/dependabot.yml` — weekly Dependabot updates for cargo, npm, and github-actions. Cargo crates grouped (`tauri*`, `tokio*`, `tracing*`, crypto deps); npm grouped (`react*`, `typescript*`, `@tauri-apps/*`). Caps at 5 open PRs per ecosystem.
- `src/lib/redact.ts` — `redactSecrets(s)` utility that masks Telegram bot tokens (`\d{6,12}:[A-Za-z0-9_-]{30,}`), Anthropic API keys (`sk-ant-…`), and generic `sk-…` keys before strings hit UI error banners.
- ARIA group + `aria-pressed` on every pill-toggle button in `Forecast.tsx` (Simulator mode, confidence preset, return preset, target-dollar interpretation, Debt-Manager mode, payoff strategy) and on `ThemeToggle`. Screen readers now announce which option is selected.

### Changed

- **Wizard + Settings token entry forms now clear secret React state on success, error, cancel, and unmount.** Previously, a failed `saveAnthropicKey` or `saveTelegramToken` left the raw key/token sitting in component state with no scrub path until the component itself unmounted. Pairs with the new `redactSecrets` filter — any token-shaped substring in an error message gets masked before reaching the toast. Affects `wizard/steps/AnthropicConfig.tsx`, `wizard/steps/Telegram.tsx`, and the `RotateAnthropicKey` + `RotateTelegramToken` panels in `views/Settings.tsx`.

### Removed

- **`keyring` Rust crate** (`keyring = "3"` from `src-tauri/Cargo.toml`) and `src-tauri/src/secrets/migration.rs`. The v0.2.6 → disk-store one-shot drain shipped in v0.3.8 and ran through v0.3.12; anyone still on v0.2.6 must update via an interim release or re-enter their secrets in Settings.
- `OnDiskFile.migrated_keyring_keys` field + `SecretsFile::mark_migrated` / `migrated_keyring_keys` methods + their two unit tests. serde's default permissive ignore-unknown-fields means v0.3.8–v0.3.12 secret files load cleanly (new regression test pins this).

### Internal

- `secrets/mod.rs::handle()` no longer triggers a keyring probe on first acquisition — the function drops to a single `OnceLock::get_or_init`. `SecretsFile::contains` is now `#[cfg(test)]`-gated since the migration drain was its only non-test caller.
- `src-tauri/notices/` shipped as `bundle.resources`; users see the third-party notices file alongside `LICENSE` in installed builds without needing to rebuild from source.

## [0.3.12] - 2026-05-07

Reframes the spot SWR shown on the projection chart from a PMT-drain calculation to a **perpetual sustainable withdrawal** — the rate at which withdrawals exactly offset real growth, so the portfolio's purchasing power stays flat forever. Fixes the v0.3.10/v0.3.11 bug where SWR values shown in the tooltip exceeded 100% of the portfolio near the horizon end.

### Fixed

- **SWR no longer reports values larger than the portfolio.** The previous PMT-drain formula returned the *annualized* constant withdrawal that would drain the balance in `remaining_months` from the hover point to horizon end. As `remaining_months` shrank toward zero (mousing rightward across the chart), the annualized rate exploded past 100% and the $/yr line in the tooltip exceeded the portfolio itself — at year 29 of a 30y horizon at 4.5% real return the formula returned ~102%, at year 29.9 it returned ~1,203%. The new framing is conceptually independent of remaining horizon: at any chart point, the SWR equals the real return (annual return − inflation), clamped at zero. Stable at ~4.5% across the chart for the default 7% return / 2.5% inflation assumptions; the displayed $/yr varies because the balance varies. Matches the "at this portfolio size if I stopped investing I could withdraw X/yr and maintain the portfolio" framing the chart is supposed to communicate.

### Changed

- Tooltip header on the projection chart: "Safe withdrawal at this point" → "If you stopped investing here, you could pull". Row label: "Deterministic (PMT)" → "Sustainable (real value held)".
- Sidebar section header: "Spot SWR" → "Spot sustainable withdrawal". Blurb rewritten in plain language ("equals real return — annual return minus inflation — applied to that month's balance").

### Internal

- `monte_carlo::swr_deterministic_pct(annual_return, annual_inflation, remaining_months)` → `monte_carlo::swr_perpetual_pct(annual_return, annual_inflation)`. Body collapses to `(annual_return − annual_inflation).max(0.0)` — no horizon parameter at all.
- Frontend `swrDeterministicPct` → `swrPerpetualPct` mirrors the new Rust math.
- `ProjectionDatum.swrDetPct/swrDetAnnual` renamed to `swrPerpPct/swrPerpAnnual`. The percentage is computed once via `useMemo` (constant across the chart) and the $/yr derives per-datum from `nominal_cents`.
- 5 PMT-formula unit tests retired; 4 perpetual-SWR tests added (equals real return; zero when return matches inflation; clamps at zero for negative real return; documents horizon-independence).

## [0.3.11] - 2026-05-07

Trims the spot SWR feature shipped in v0.3.10 down to the deterministic layer only — the closed-form, instant-feedback PMT formula proved to be the right call all along, and the heavier probabilistic pass added confusion without enough additional signal.

### Changed

- **Spot SWR is now deterministic-only.** Mouse over the projection chart to see the safe withdrawal rate at any point along the horizon, computed live via the PMT formula on real return. The "Compute probabilistic SWR" button + the second tooltip row that v0.3.10 introduced are gone — the chart's bands already communicate the volatility story, and the deterministic answer is the one users actually act on.

### Removed

- `compute_probabilistic_swr` function in `src-tauri/src/insights/simulator.rs` (and its `ProbabilisticSwrInput` / `ProbabilisticSwrResult` / `SwrPoint` types).
- `simulator_probabilistic_swr` Tauri command + invoke_handler entry.
- Frontend `simulatorProbabilisticSwr` binding + cached-result state + cache-invalidation effect + the per-datum probabilistic SWR fields + the tooltip's probabilistic Row + the Compute/Recompute button.
- Two simulator unit tests that exercised the removed function.

### Internal

- The `swr_deterministic_pct` helper in `monte_carlo.rs` stays put. The frontend mirrors its math in JS for the live tooltip; the Rust helper keeps its 5 unit tests as the spec the JS implementation tracks.
- Net change: 343 deletions, 4 insertions.

## [0.3.10] - 2026-05-07

Recurring withdrawals + spot safe-withdrawal-rate (SWR) on the Investment Simulator. Mouse over the projection chart to see the safe withdrawal rate at any point along the horizon, and type a withdrawal rate of your own to see how it reshapes the trajectory.

### Added

- **Recurring withdrawal box.** Type an annual percentage; the Simulator subtracts that fraction of the *current* balance every month, spread evenly across the year. Constant-percent semantics — withdrawals fluctuate with the market (smaller in down years, larger in up years), and a moderate rate against a positive real return asymptotes rather than ever fully depleting. The "≈ $X/yr at today's balance" hint makes the implied dollar amount visible the moment you type. Withdrawals integrate with both Simulator modes and with the lump-sum scaffolding from v0.3.9.
- **Spot SWR on the projection tooltip.** Two layers, both keyed to the moused-over month:
  - **Deterministic (PMT).** Closed-form PMT formula on the real return: `swr_pct = 12 × r_real / (1 − (1 + r_real)^−n_remaining)`. Updates instantly with the user's return, inflation, and horizon inputs — frontend mirrors the Rust helper for parity.
  - **Probabilistic.** Click "Compute probabilistic SWR" to run a backend bisection at 10 evenly-spaced months: at each point, find the largest withdrawal rate where ≥ confidence% of paths still have a positive balance at horizon end. Confidence tracks the simulator's existing slider — match the rest of the panel. Result caches in component state and invalidates whenever any sim input changes, so the tooltip never shows a stale answer.

### Internal

- New `monte_carlo::swr_deterministic_pct(annual_return, annual_inflation, n_remaining_months)` closed-form helper. Handles the zero-real-return linear-drawdown special case and the at-horizon-end zero case.
- New `simulator::compute_probabilistic_swr` + `ProbabilisticSwrInput` / `ProbabilisticSwrResult` / `SwrPoint` types. Reuses an unconditional `simulate` run for balance seeding, then bisects rate ∈ [0, 30] at each point with 200 paths × 10 iterations.
- New Tauri command `simulator_probabilistic_swr` and TS binding.
- `PathInput` and `CommonInputs` gain `#[serde(default)] withdrawal_rate_pct: f64`. Threaded through `simulate`, `goal_probability`, `mc_input`, `build_trajectory`. Per path: `value -= value × rate / 100 / 12` after growth + monthly contribution + this month's lump, with the existing zero-clamp inherited.
- Frontend `ProjectionDatum` extended with `swrDetPct`, `swrDetAnnual`, `swrProbPct`, `swrProbAnnual`. `chartData` populates them per point — deterministic via JS-side closed-form, probabilistic via nearest-month lookup (±6mo tolerance) into the cached result.
- Eleven new unit tests across `monte_carlo` (withdrawal monotonicity, broke-clamp, asymptote behavior at modest rates, PMT formula correctness for known horizons, real-return convention, zero-return linear special case, monotonicity in remaining horizon) and `simulator` (probabilistic SWR returns points within bisection bounds; lower confidence ≥ higher confidence at the same point).

## [0.3.9] - 2026-05-07

Lump sums in the Investment Simulator — schedule one-time deposits (bonus, tax refund, sale of an asset) or planned withdrawals (wedding, down-payment, tuition) at any month within the horizon, and watch the math reflow in both modes.

### Added

- **Lump sums in the Simulator panel.** Mirrors the Debt Manager's affordance: "+ Add lump sum" button creates rows of `month-offset + amount`, with a remove button per row. The amount field accepts negative values to model planned withdrawals — a `-20000` entry at month 18 drags the trajectory down by exactly that much (compounded by the lost growth on the withdrawn capital). Both calculator modes integrate:
  - **Find required:** the bisection over monthly contribution treats the lump schedule as fixed scheduled cash flows. A $50k bonus at year 5 visibly reduces the monthly required to hit the same target.
  - **Show probability:** the lumps deterministically apply at their `month_offset` on every Monte Carlo path. The chart's Nominal trace and the Contributions trace both reflect the signed lumps.
- **Broke-clamp on the trajectory.** A withdrawal that exceeds the path's balance leaves it at zero rather than going negative — the chart shows the path bottoming out at $0, which matches what a real broke account would do.

### Internal

- New `LumpSum { month_offset: u32, amount_cents: i64 }` in `src-tauri/src/insights/monte_carlo.rs`. Threaded through `PathInput`, `simulate()`, `goal_probability()`, and `simulator::CommonInputs`. Marked `#[serde(default)]` everywhere so callers that omit the field still deserialize.
- `bucket_lumps()` helper coalesces same-month lumps and silently drops out-of-range entries. Same-month lumps sum together so users can paste multiple flows on the same date without accidentally cancelling them.
- `build_trajectory` switches from closed-form FV (`p × (1+r)^m + c × ((1+r)^m − 1) / r`) to a month-by-month step. Closed-form can't accommodate scheduled cash flows; stepping costs one tight loop of `n_months` iterations per simulator call (negligible at desktop scale) and keeps the deterministic Nominal trace consistent with the Monte Carlo paths.
- Frontend reuses the existing `LumpSum` TypeScript interface (wire format identical to Debt Manager's). The Simulator does not carry lumps over the Debt Manager hand-off — payoff-side lumps belong to a debt mental model, investment-side lumps belong to the Simulator.
- Eight new unit tests: zero-vol with positive lump matches closed-form-with-FV-of-injection; negative lump drags result by FV-of-withdrawal; broke-trajectory clamps at zero; out-of-range `month_offset` silently ignored; duplicate-month lumps sum; `goal_probability` monotonic in lump value; `trajectory.contributions` includes signed lumps; goal-seek `required_monthly_cents` strictly decreases when a lump is added.

## [0.3.8] - 2026-05-04

Security + correctness patch driven by the v0.3.7 codebase audit (`docs/audit-v0.3.7.md`). All eleven items the audit flagged for v0.3.8 — five Highs and six Mediums covering Telegram pairing hygiene, migration durability, AGPL §6 compliance, accessibility, and crypto defense-in-depth — landed in this release.

### Security

- **[S-2 / High] Bot token no longer leaks into logs.** `reqwest::Error::Display` includes the URL on transport failures (TLS, DNS, timeout, redirect), and the Telegram Bot API embeds the token directly in URL paths (`/bot<token>/<method>`). Stderr / journalctl could capture the token on any connect failure. Fix at the source: in `TelegramClient::invoke`, catch `reqwest::Error`, strip the URL with `.without_url()`, and run a regex-free `redact_path()` pass that replaces any `/bot<...>/` segment with `/bot<REDACTED>/`. Four new unit tests guard the scrub.

### Correctness

- **[D-1 / High] Migrations now run inside transactions.** Migrations 0004 / 0006 / 0011 / 0014 (the table-recreate dance and the two-`ALTER` 0011) previously ran via `execute_batch` with autocommit per statement. A mid-batch failure (disk full, OOM, panic) left a half-applied schema with `user_version` unchanged — and on the next launch the migration retried and immediately crashed on "table categories_new already exists" or "duplicate column". App bricked, every startup. Each migration now runs inside `unchecked_transaction()`; for the four `recreate: true` migrations the runner toggles `PRAGMA foreign_keys` outside the wrapping tx (SQLite forbids the pragma inside one). Two new tests confirm partial-failure rollback for both recreate and non-recreate paths.
- **[D-2 / High] CSV bulk import is now atomic.** `csv_import_commit` looped `expenses::insert` per row with autocommit; row 387 of a 500-row batch failing left 386 rows on disk with no clean retry. Wrapped in `conn.unchecked_transaction()`.
- **[M-1 / Medium] CSV amount parser handles European locale.** `parse_amount("1.234,56")` previously stripped commas and parsed `1.23456`, returning 12,346 cents — off by 100×. Anyone with a German / French / Italian bank silently logged every transaction at 1% of its real value. New locale heuristic: if a comma appears AFTER the last period, treat the comma as decimal (EU); ambiguous single-comma inputs use trailing-fragment length (1–2 digits = decimal, 3 = thousands). Also rejects `inf` / `nan` / empty (was returning `i64::MAX` / `0` / silent surprise).
- **[CC-3 / Medium] Recurring catch-up stamps `occurred_at = job.next_due_at`** instead of `now`. After a multi-day offline period, every catch-up insert previously collapsed onto the same wall-clock timestamp, polluting the spend-by-day chart. Each fire now gets its historically-correct timestamp.

### Crypto / privacy

- **[C-1 / Medium] Master ChaCha20-Poly1305 key is zeroized on drop.** The 32-byte key lived in the global `OnceLock` for the process lifetime with no Drop scrub — recoverable from swap or core dumps. Added `zeroize = "1"` and wrapped `SecretsFile.master_key` + the intermediate KDF buffer in `Zeroizing<[u8; 32]>`. Rust deref coercion keeps cipher.rs call sites unchanged.
- **[C-2 / Medium] Secrets file save is durable across crashes.** `save_atomic()` previously called `f.sync_all().ok()` (errors silently swallowed) and never `fsync`ed the parent directory after `rename`. Disk-full / read-only / hardware errors during the temp write could place a corrupted file; a power loss between rename and journal commit could leave the old file alongside a stray `.tmp`. Now propagates `sync_all` errors and fsyncs the parent dir on Unix (no-op on Windows where NTFS metadata journals differently).
- **[R-1 / Medium] Keyring migration is now eager-on-open with a sentinel.** The on-demand `try_copy_from_keyring` path probed the OS keyring on every `retrieve()` for a missing key, forever — a dbus call per probe on Linux, blocking on any slow keyring agent. Replaced with a one-shot eager drain inside `secrets::handle()`: walk the known legacy keys, copy any present values to disk, mark each with a `migrated_keyring_keys` sentinel persisted in the secrets file. Subsequent launches skip the probe entirely. New on-disk field is `#[serde(default)]` so v0.3.7-and-earlier files deserialize without a schema bump. Sets up clean removal of the `keyring` crate in v0.3.9.

### Accessibility

- **[F-1 / High] Sliders announce labels and values.** Every `<input type="range">` in the Forecast / Simulator / Debt / Scenario views — eight via `NumberSlider`, plus the bespoke Confidence slider and the per-category ScenarioTool sliders — was previously announced as "slider, 30" with no unit context, making the entire view unusable with a screen reader. Added `aria-label` and `aria-valuetext` on all of them.

### Compliance

- **[Co-1 / High] AGPL §6 source-offer in shipped binaries.** Bundles previously identified themselves as AGPL but didn't tell the user where to obtain the source — the §6 obligation. Three layers of fix: `tauri.conf.json` `bundle.copyright` appends the source URL; `bundle.resources` ships `LICENSE` inside every AppImage / `.app` / NSIS / MSI / DMG; the GitHub release body template appends a short "this is AGPL-3.0, source at https://github.com/wyattts/Mr.-Moneypenny" paragraph. New Settings → About section in the app shows version + license (linked) + source URL + the standard AGPL §6 user-facing paragraph, sourced via a new `get_app_version` Tauri command.

### Build / release

- **[B-1 / Medium] Releases auto-promote to `--latest`.** The Release workflow created a draft release and required a manual `gh release edit "$TAG" --draft=false --latest` afterwards — forgetting it silently broke the auto-updater (which fetches `releases/latest/download/latest.json` gated on the `latest` flag). v0.3.4 had to be retracted for exactly this reason. New `promote` job depends on the matrix `release` job (preserves the all-platforms-built gate) and runs the `gh release edit` step automatically.

### Internal

- New `Migration` struct in `src-tauri/src/db/mod.rs` with a `recreate: bool` flag. The runner manages `PRAGMA foreign_keys` toggling outside the wrapping tx; embedded `PRAGMA foreign_keys = OFF/ON` statements inside the four recreate-flagged `.sql` files become inert no-ops within the tx (harmless, self-documenting in the SQL).
- `chacha20poly1305 0.10.1` doesn't expose a `zeroize` feature flag — its zeroize behavior is internal to the crate at this version. Our `Zeroizing` wrapper of `master_key` is the user-visible defense.
- New unit tests: 4 in `telegram::client` (token scrub), 2 in `db::tests` (partial-migration rollback), 7 in `csv_import::parser` (EU locale + non-finite rejection), 3 in `secrets::store` (sentinel persistence + serde-default forward-compat). All 281 prior tests still pass.

### Audit

- New `docs/audit-v0.3.7.md` — full read-only audit of the v0.3.7 codebase: 130 findings across 12 categories (0 Critical, 6 High, 33 Medium, 47 Low, 44 Info). Recommended remediation roadmap for v0.3.8 (this release), v0.3.9, and v0.4.0.

## [0.3.7] - 2026-05-03

Debt Manager — pure-deterministic debt amortization tool inside the Forecast view, with goal seek, lump sums, portfolio mode (snowball vs. avalanche), inflation-adjusted today's-dollars cost, and a one-click hand-off to the investment Simulator for "after payoff, where do these dollars go?" planning.

### Added

- **Debt Manager** is a new section in the Forecast view. Two modes for a single debt:
  - **Forward calc** — given a balance, APR, compounding, monthly payment, and any lump sums, returns payoff month + year, total interest, total paid (nominal and today's $), and a payoff trajectory chart.
  - **Goal seek** — given a target payoff in months (configurable as years + months), bisects the smallest monthly payment that hits the target. Reports payoff year *and* month-within-year (debt amortization is deterministic, so the precision is meaningful — unlike the Simulator's probabilistic horizon).
- **Compounding frequency** picker: monthly, daily, yearly, or continuous. APR is converted to an effective monthly periodic rate using textbook conversions before iterating the schedule.
- **Lump sums** — add any number of one-time payments (tax refund, bonus, etc.) at specific months. Lump sums apply on top of the monthly payment.
- **Inflation slider** drives a parallel today's-dollars total — every month's payment is discounted to present value and summed alongside the nominal total.
- **Portfolio mode** (toggle, off by default) handles multiple debts at once with a fixed monthly budget. Strategy selector chooses **avalanche** (extra goes to highest APR) or **snowball** (extra goes to smallest balance); the *other* strategy runs in parallel and the result card shows the side-by-side interest delta so the trade-off is visible without toggling. Per-debt fields: balance, APR, compounding, minimum payment.
- **Breakeven warning** — if your monthly payment is at or below the initial interest charge with no lump sums, the result card shows a warning and surfaces the breakeven payment so you know what it takes to make progress.
- **Equivalent guaranteed return callout** next to every result: paying off this debt is equivalent to a guaranteed return at the debt's APR. Compared inline against the Simulator's current nominal-return assumption, with a one-line verdict in either direction.
- **"After payoff: invest [payment]/mo → Simulator"** button hands off the freed-up monthly payment + a 30-year horizon into the Simulator above. The Simulator switches to "Show probability" mode and prefills contribution + horizon so the user sees the after-payoff investing trajectory.

### Internal

- New `src-tauri/src/insights/debt.rs` (~700 LOC). Exposes `simulate_schedule`, `goal_seek` (bisection), and `simulate_portfolio` (snowball / avalanche). Pure functions; no DB reads. 10 unit tests cover compounding-rate conversions, zero-APR linear payoff, the textbook 5%/60-month amortization, the below-breakeven warning, lump-sum acceleration, goal-seek convergence + year/month decomposition, inflation discount, snowball-vs-avalanche on inverted balance/APR, and the below-minimums warning.
- New Tauri commands: `debt_simulate_schedule`, `debt_goal_seek`, `debt_simulate_portfolio`. All three are pure (no `State<AppState>`).
- TS bindings + 11 new types in `src/lib/tauri.ts`.
- Forecast.tsx gains `DebtManager`, `SingleResultCard`, `PortfolioResultCard`, `DebtChart`, `PortfolioChart`, `SelectField`. Simulator gains an optional `prefill?: SimulatorPrefill | null` prop and a `onReturnPctChange` callback so the Debt Manager can react to the user's chosen return assumption. The `Forecast` parent owns both pieces of shared state.

## [0.3.6] - 2026-05-02

Simulator chart polish — band tooltip values, distinct band color, and Y-axis labels that no longer bleed off the chart.

### Fixed

- **Band edge values now appear in the projection-chart tooltip.** The v0.3.5 chart relied on transparent Recharts `Line`s named `Lower` / `Upper` to register tooltip entries for the band edges; with `stroke="transparent" + activeDot={false}`, Recharts dropped them from the active hover payload, so the band's P_lo / P_hi numbers never showed up. Replaced with a custom Tooltip content function that reads the band edges directly from the chart datum — values appear reliably, and the labels now include the actual percentile (e.g., "Lower (P10)" / "Upper (P90)") so users see what band width the current confidence resolves to.
- **Y-axis labels no longer bleed off the chart's left edge** for large projections. Tick formatter now scales by magnitude (`$1.5M` for ≥$1M, `$200k` for ≥$1k, otherwise `$X`); YAxis width bumped to 78px so seven-figure labels render in their gutter instead of overflowing into the plot area. Same fix applied to the Category Analyzer chart.

### Changed

- **Probability band color** switches from forest-400/30 to blue-400/30 (`#60a5fa` at 18% opacity) so the band is visually distinct from the forest-green Nominal line. The band reads as "uncertainty around the deterministic curve" instead of competing with it.
- Chart cursor on hover is now a faint vertical guideline (graphite-700) so users can see exactly which year the tooltip is reading.

### Internal

- Drop the redundant `band_lo` / `Lower` / `Upper` fields from the chart datum; band stacking now uses `band_offset` (transparent base) + `band_span` (filled) and the tooltip reads `pLo` / `pHi` straight from the datum.
- New `formatYAxisDollars(v)` helper used by both Simulator and Category Analyzer charts.
- New `ProjectionTooltipContent` + `Row` components in `Forecast.tsx` for the custom tooltip.

## [0.3.5] - 2026-05-02

Forecast view simplification — Investment Calculator and Goal-seek removed (their probabilistic-but-deterministic nature was less accurate than what the Simulator already does), the Simulator absorbs their best affordances, and the projection chart now lives inside the Simulator with bands that scale with confidence.

### Changed

- **Forecast: Goal-seek section removed.** Its closed-form algebraic answer ignores volatility — the Simulator's "Find required contribution" mode replaces it with a Monte-Carlo-aware solver that returns the smallest contribution hitting the target with the user's chosen confidence (50–95%).
- **Forecast: Investment Calculator section removed.** The projection chart relocates inside the Simulator panel and renders from the same Monte Carlo run that produced the headline number — no more separate IPC call.
- **Probability bands now scale with confidence.** In "Find required" mode, the band width matches the confidence slider (80% confidence → P10–P90 band; 90% → P5–P95; 70% → P15–P85). In "Show probability" mode, the band matches the *resulting* probability — so "the central X% of where you'd actually end up" lines up with "your X% chance of hitting target." The chart legend always shows the band's current width.
- **Simulator inherits Investment Calculator's "Pre-fill from account" affordance.** A small dropdown in the Simulator header offers all your investing-kind categories (Savings, 401k, Roth IRA, etc.) plus an "All investing accounts (sum)" option; picking one auto-fills starting balance + monthly contribution from your saved data.
- **Simulator gains return-rate preset chips** (Conservative / Balanced / Stock-heavy) lifted from the old Investment Calculator.
- **Heatmap text color is now legible on every tile.** Charcoal (`#1f2937`) on amber-700 and lime-700 cells; light text on red-900 and green-800 cells. Earlier, the medium-luminance tiles disappeared into uniform graphite-400 text.

### Internal

- `monte_carlo::simulate` gains `band_pct` input; `MonthBand` simplifies to `month + p_lo + p50 + p_hi` (custom percentiles instead of a fixed P5/P10/.../P95 set). Same paths under the hood — just the percentile extraction is parameterized now.
- `simulator::solve_required_contribution` and `compute_probability` results gain a `trajectory: Vec<TrajectoryPoint>` field carrying nominal + real + contributions + band edges per month, so the chart renders from a single payload.
- `insights::forecast::solve_goal_seek` + `GoalSeekInput`/`GoalSeekResult` removed. `project_investment` stays internal but no longer exposed via IPC. 3 IPC commands removed: `solve_goal_seek`, `project_investment`, `monte_carlo_investment`.
- Frontend: the InvestmentCalculator and GoalSeekTool components removed entirely. Forecast view now mounts only Simulator + CategoryAnalyzer + ScenarioTool. 280 tests passing.

## [0.3.4] - 2026-05-02

**Same code as the (briefly-published, now-deleted) v0.3.3 redux release.** The version number was bumped to 0.3.4 because the scrapped first-attempt v0.3.3 had already been auto-updated to some installs (Wyatt's), and Tauri's updater does strict semver `>` comparison — meaning a re-released v0.3.3 with different code couldn't reach those installs. v0.3.4 is the same redux content shipped under a fresh version number so the auto-updater promotes it correctly. No v0.3.3 release exists in the public history.

Forecast wave 2 (redux) — bidirectional Monte Carlo Simulator + Category Analyzer + 80% probability bands on the Investment Calculator chart. The first attempt at v0.3.3 shipped Monte Carlo as two passive numbers bolted onto existing tools and was scrapped after Wyatt installed and reviewed it; this version gives probability questions their own surface with proper levers.

### Added

- **Forecast → Simulator** (new section). Bidirectional Monte Carlo: pick "Find required contribution" mode and the tool bisects to the smallest monthly contribution that hits your target with your chosen confidence (50–95%, 70/80/90 chip presets). Or pick "Show probability" mode and pin a contribution to see the resulting probability live. Inputs include target $, horizon, return rate, inflation, starting balance, and a "Target is in: today's $ vs nominal future $" toggle. Advanced disclosure exposes a σ override slider (default tied to return preset: 5/10/15% σ for Conservative/Balanced/Stock-heavy).
- **Probability heatmap** under the Simulator: 12×12 grid of (monthly contribution × horizon years) → probability of hitting target, color-coded red/amber/lime/green at 50/70/90% thresholds. Hover any cell for exact values. Anchored on whichever value the active solver just produced so users see the trade-space *around* their answer.
- **Investment Calculator: 80% probability band overlay (reinstated).** Default off; checkbox below the chart toggles a forest-green ribbon between P10 and P90 from a 1,000-path Monte Carlo simulation. Chart tooltip on hover shows the band's actual P10 (lower) and P90 (upper) dollar values at every year — not just the deterministic Nominal line.
- **Forecast → Category Analyzer** (renamed and expanded from Trend Analyzer). Pick a category + window from {2 weeks, month, quarter, half year, year}; granularity auto-derives to ~12 buckets per window. Side-by-side stats panels: per-transaction (n purchases, mean / median / σ / min / max — refunds excluded; surfaced as a separate net-spent line) and per-bucket (totals at the auto-derived granularity). Linear-regression chart with slope normalized to `$/mo per year` regardless of window size, plus a plain-English headline ("Spending is rising at $42/mo per year — strong trend (R²=0.71)").

### Internal

- New `src-tauri/src/insights/monte_carlo.rs`: 1,000-path simulator with Box-Muller Normal sampling, optional fixed RNG seed for reproducibility, percentile extraction. Final P5/P10/P50/P90/P95 fields on the result for convenient UI consumption.
- New `src-tauri/src/insights/simulator.rs`: bidirectional solver. `solve_required_contribution` bisects over contribution (up to 14 iterations), `compute_probability` does a single run, `heatmap` produces a 12×12 grid (200 paths per cell to keep total under ~30k paths). `effective_target` helper handles today's-$ → nominal inflation. 7 unit tests including bidirectional consistency (round-trip from solver to probability).
- New `src-tauri/src/insights/category_analyzer.rs`: replaces the scrapped `trend.rs` with auto-bucketing per window, dual stats (per-transaction and per-bucket), refund summary, slope normalization. 6 unit tests.
- `repository::expenses` gains `list_in_range_by_category` so the analyzer can query individual rows for per-transaction stats without re-querying.
- 5 new IPC commands: `monte_carlo_investment`, `simulator_solve_required_contribution`, `simulator_compute_probability`, `simulator_heatmap`, `analyze_category`. 21 new tests; 278 total passing.
- No new dependencies (Monte Carlo uses existing `rand`).

### What was scrapped from the original v0.3.3

- The Survivability tool (out of scope per Wyatt — a different category of question than what Mr. Moneypenny is for right now).
- The probability badge on Goal-seek (the old design implied users could see a probability without offering any way to act on it; the Simulator is the canonical place for probability questions).

### Privacy / honesty

Every probabilistic output makes its assumptions visible: the Simulator headline always quotes the volatility figure used and the inflation interpretation. The "Target is in: today's $" toggle inflates the target before checking simulated paths so a user typing "$1M" actually means "$1M today's purchasing power" by default, not literal nominal $1M at the horizon date.

## [0.3.2] - 2026-05-01

CSV importer. Bulk-load bank and credit-card export CSVs into the local expense ledger without paying API tokens for every row. Built around a `merchant_rules` table that the import wizard populates one click at a time on its review screen — first import of a new bank takes a few minutes, every subsequent import from the same bank is instant and free.

### Added

- **Settings → CSV import panel.** Launches a 5-step wizard (pick file → pick or create bank profile → map columns → categorize unmatched merchants → review probable duplicates → commit). The panel also lists saved bank profiles and merchant rules with delete affordances for each.
- **Auto-detected bank profiles.** Each CSV's column-header row is hashed; if a saved profile's signature matches, the wizard auto-suggests it and skips the mapping screen entirely on re-imports.
- **Three-layer merchant categorization** applied in order:
  1. Saved `merchant_rules` (`STARBUCKS*` → Coffee).
  2. Fuzzy match against existing `expenses.description` history (Levenshtein < 3 from a recent expense with a category).
  3. Manual review screen for anything left. Each pick auto-saves a rule for next time.
- **Optional ✨ AI-suggest** button on the review screen. Sends the unmatched merchant list + your category list to the configured LLM in **one batched call** and returns JSON. Cost ~$0.001-$0.01 per import regardless of row count. Off by default; only sends merchant strings (no amounts, dates, or descriptions).
- **Probable-duplicate detection** at import time. Within-CSV dedupe (exact match on date + amount + merchant), against-DB dedupe (same date ±2d, exact amount, Levenshtein-fuzzy merchant). Surfaced in a review screen with checkboxes default-checked-to-skip.
- **Negative amounts auto-marked as refunds.** Bank statements universally use negatives for credits/refunds/returns; the importer flips sign and sets `is_refund=1` to feed v0.2.6's signed-sum aggregation correctly. Configurable per profile.
- **Date format flexibility.** `MM/DD/YYYY`, `DD/MM/YYYY`, `YYYY-MM-DD`, `MM-DD-YYYY`, `DD-MM-YYYY`, `M/D/YYYY` — picked once per profile.
- **Amount parsing handles bank quirks**: leading `$`, comma thousands separators, parens-as-negation (`(1,234.56)`), explicit minus signs.

### Internal

- New `csv_import/` Rust module: `parser.rs` (csv-crate wrapper + column projection + amount/date parsers), `dedupe.rs` (within-CSV + against-DB Levenshtein), `categorize.rs` (3-layer match + auto-pattern suggestion), `ai_suggest.rs` (batched LLM call + JSON parse).
- New `repository::csv_import_profiles` and `repository::merchant_rules` modules. Glob matching for merchant patterns is implemented in Rust (small recursive matcher with `*` and `?` semantics) so we control case-folding precisely.
- 4 new migrations: 0012 (`csv_import_profiles`), 0013 (`merchant_rules`), 0014 (`source` CHECK gains `'csv'` so imported expenses are traceable). 0014 repeats v0.2.6's table-recreate dance because SQLite can't ALTER a CHECK in place.
- `ExpenseSource` enum gains a `Csv` variant.
- 10 new IPC commands: `csv_import_preview`, `csv_import_save_profile`, `csv_import_parse`, `csv_import_categorize_and_dedupe`, `csv_import_ai_suggest`, `csv_import_commit`, `list_csv_import_profiles`, `delete_csv_import_profile`, `list_merchant_rules`, `delete_merchant_rule`.
- 48 new tests across `repository::csv_import_profiles` (6), `repository::merchant_rules` (7), `csv_import::parser` (12), `csv_import::dedupe` (5), `csv_import::categorize` (5), `csv_import::ai_suggest` (5). 257 tests passing total.
- New deps: `csv = "1"` (~600 lines, MIT, no `unsafe`), `strsim = "0.11"` for Levenshtein.

### Privacy

CSV content stays on your machine. The only optional outbound traffic is the AI-suggest batched call, which sends only merchant strings + your category names — no amounts, dates, descriptions, or row counts beyond the merchant set. Off by default; opt in per-import.

## [0.3.1] - 2026-05-01

Small follow-up patch on top of v0.3.0's Forecast view. No schema changes, no new IPC commands — just polish.

### Added

- **Investment calculator: editable starting balance.** Previously the starting balance was sourced read-only from the saved investing-account record. Now there's a "$ Starting balance" input in the calculator itself; the input prefills from the saved balance when you switch accounts and is freely editable, so you can play out hypotheticals like "what if I rolled an extra $10k in?"
- **Investment calculator: cumulative-contributions overlay.** New "Show contributions" checkbox below the chart toggles a third dashed line representing starting balance + cumulative deposits over time. Default off so the chart stays uncluttered; flip it on to see compounded growth visually separate from money you put in.

### Changed

- **Two decimal places everywhere.** All monetary amounts and percent displays now render to two decimals across the app — including investment-calculator hints, settings investment-account summaries, KPI "% of budget spent", and MoM-comparison delta (`+12.30%`, not `+12.3%`). Uniformity for tabular alignment. Chart-axis tick labels still use compact form (`$200k`) to avoid overflow.
- **Forecast scenario tool** renamed and reframed: "Scenario: what if I cut..." → "Scenario: what if I changed..." since the sliders also accept positive (raise the cap) values. The result panel now shows "Saves per year" or "Costs per year" depending on the sign, with green/yellow color cues respectively.

## [0.3.0] - 2026-05-01

The power-user update — wave 1. New "Forecast" view in the sidebar with three tools (investment calculator, goal-seek, scenario sliders), plus per-category descriptive stats and a histogram on every Categories row. All deterministic for now; Monte Carlo / bootstrap variants land in v0.3.1.

### Added

- **New Forecast view** (sidebar nav) bundles three look-forward tools.
- **Investment calculator**: pick an investing-kind account (or aggregate across all), enter monthly contribution + annual return + horizon + inflation, see a trajectory chart with both nominal and real (inflation-adjusted) curves, plus final value, contributions vs growth breakdown. Auto-prefills monthly contribution from the user's actual 12-month contribution average to that account. Three preset return rates (Conservative 4% / Balanced 7% / Stock-heavy 10%). Standard "not financial advice" disclaimer.
- **Goal-seek**: enter target $ + horizon + return rate + starting balance → returns the required monthly contribution. Detects "already on track" when the starting balance compounds past the target on its own.
- **Scenario sliders**: each active variable category with a monthly target gets a -100% to +50% slider. Live-recomputes the adjusted variable budget and annualized savings as you drag.
- **Per-category descriptive stats**: every Categories row gets a "▾ stats" toggle. Expanded view shows N, mean, median, P10/P90, std-dev, min, max — plus a 12-month equal-width histogram bar chart visualizing the spending distribution. Refuses to compute below N=3 with a clear "not enough history yet" message.
- **Settings → Investment balances**: dedicated panel for entering current balance + as-of date for each investing-kind category. Without these, the investment calculator can't accurately project for accounts opened before installing the app — so this is where the "current $20k Roth IRA" gets entered.

### Internal

- New `src-tauri/src/insights/stats.rs`: descriptive stats (mean / median / percentile / stddev / min / max) with `MIN_N=3` guard, plus equal-width histogram bucketing. 9 unit tests.
- New `src-tauri/src/insights/forecast.rs`: closed-form future-value formula (`FV = P(1+r)^n + C·((1+r)^n − 1)/r`) with end-of-month deposits, real-vs-nominal inflation deflation, algebraic goal-seek inverse, and scenario-delta arithmetic. 9 unit tests including an Excel-FV cross-check ($500/mo @ 7% × 30y matches $609,985.71 within $5).
- `src-tauri/src/repository/expenses.rs` gets `monthly_totals_for_category(category_id, now, months_back)` so the stats module has clean signed-sum monthly inputs.
- Migration 0011 adds `starting_balance_cents` + `balance_as_of` columns to `categories` (both nullable, meaningful only for `kind = 'investing'`).
- 6 new IPC commands: `get_category_stats`, `project_investment`, `solve_goal_seek`, `run_scenario`, `set_starting_balance`, `list_investment_categories`. 218 total tests passing.

### Sequencing note

Original roadmap had v0.3.0 also including CSV import + tax report generator. The keyring rework (v0.2.7), copy fix (v0.2.8), and cost tracker / friction fixes (v0.2.9) consumed the slots originally meant for those, so this v0.3.0 ships the forecast tools standalone. CSV import + tax report move to v0.3.1 / v0.3.2 alongside the planned Monte Carlo variants.

## [0.2.9] - 2026-05-01

API cost tracker plus two friction kills on the bot side.

### Added

- **Settings → API usage** panel surfaces what you've spent on Anthropic API calls. Three big numbers (today / this month / lifetime) plus a per-model breakdown with call count, input/output tokens, and cost. Ollama models show with a "local" tag and zero cost.
- New `llm_usage` table logs one row per successful chat() response. Cost is computed at insert time from a hardcoded price table (`src-tauri/src/llm/pricing.rs`) — historical totals stay frozen even if Anthropic adjusts pricing later.
- Pricing table covers Claude Haiku / Sonnet / Opus 4.5+ snapshots; cache-read and cache-creation tokens use the standard 0.1× and 1.25× input-rate multipliers. Unknown models log a row with `cost_micros = 0` so call counts still work.

### Changed

- **Bot no longer asks "are you sure?" before deletes.** When you say "delete that last one" or "remove the rent recurring", it just does it. The undo cost is one message; the confirmation cost was a turn round-trip plus extra tokens — net friction win.
- **Bot picks a borderline category instead of asking.** "$20 pan" lands in Household automatically rather than prompting "household or misc?" Only asks if a message is genuinely uninterpretable as an expense or no category fits even loosely. Specific categories preferred over Misc; Misc is now treated as a last resort.

### Internal

- `LLMProvider` trait grew `provider_name()` and `model()` accessors so the router can attribute usage rows correctly.
- Migration 0010 adds the `llm_usage` table + `idx_llm_usage_occurred` index.
- 13 new unit tests across `llm/pricing` and `repository/llm_usage` cover model lookup, cost computation including cache token components, format precision buckets, today/month/lifetime windowing, and per-model aggregation. 196 total tests passing.

## [0.2.8] - 2026-05-01

Copy fix. v0.2.7 replaced the OS keyring with an encrypted-on-disk store, but four UI strings still claimed "stored in your OS keychain." Updated to reflect reality: secrets are encrypted on disk under a machine-bound key.

### Changed

- Settings → Anthropic / Telegram panels: "Key/Token is saved (in OS keychain)" → "(encrypted on disk)".
- Setup wizard subtitles for Anthropic key entry and Telegram token entry: explain machine-bound encryption rather than the old keyring story.

## [0.2.7] - 2026-05-01

Reliability hotfix. The OS keyring backend that v0.2.6 and earlier relied on for the Anthropic API key and Telegram bot token has too many silent-failure modes on Linux — most notably, GNOME Keyring storing secrets in a session-only collection that gets wiped on reboot. v0.2.7 replaces the keyring entirely with an encrypted-on-disk store that just works across reboots, package switches, and desktop environments.

### Changed

- **Secrets now live in `~/.local/share/moneypenny/secrets.bin`** (and platform equivalents on macOS / Windows), encrypted with ChaCha20-Poly1305 under a key derived from a stable per-machine identifier. No daemon, no dbus, no PAM dependency. The file is `chmod 600` on Unix; same threat model as the OS keyring on a single-user machine.
- **Master key derivation**: HKDF-SHA256 over `machine-uid || data_dir_path || per-installation salt`. Matches the keyring's per-machine + per-user binding — secrets don't decrypt if the file is moved to a different machine or user.
- **Migration is transparent**: on first launch after upgrade, the new code opportunistically reads any existing keyring entries and copies them into the disk store. Users with intact keyrings notice nothing. Users whose keyrings had broken (the bug this release fixes) re-enter their credentials once via Settings — and they persist correctly from then on.

### Sequencing note

v0.2.7 was originally slotted as "API cost tracker" in the roadmap. That work shifts to v0.2.8; the local-whisper.cpp voice work moves to v0.2.9. Sequence is otherwise unchanged.

### Internal

- New `src-tauri/src/secrets/` module: `mod.rs` (public API matching the v0.2.6 surface), `kdf.rs` (HKDF-SHA256 over machine-uid + data dir), `cipher.rs` (ChaCha20-Poly1305 wrap/unwrap), `store.rs` (atomic save with `fsync` + rename), `migration.rs` (one-shot keyring → disk copy). 16 new unit tests covering round-trip, tamper detection, wrong-key failure, persistence across reopens, and chmod-600 enforcement on Unix.
- New crate deps: `chacha20poly1305`, `hkdf`, `sha2`, `machine-uid`, `base64`. The `keyring` crate stays in the dep tree for v0.2.7 only — read-only, used by the migration shim — and will be dropped in v0.2.8.

## [0.2.6] - 2026-04-30

First v0.2.6-track patch on the road to v1.0.0 — *bot reliability + recurring infrastructure*. Three new bot capabilities are wired through one shared scheduler primitive, and refunds finally have first-class support throughout the app.

### Added

- **Refund support, modeled as first-class rows.** New LLM tool `add_refund` lets the bot log refunds — money returned (Amazon return, cancelled subscription, chargeback). On disk the row sits in the same `expenses` table with `is_refund = 1` and an optional `refund_for_expense_id` FK. Aggregations subtract refunds via `SUM(CASE WHEN is_refund THEN -amount ELSE amount END)`. Net spend, dashboard category totals, KPI cards, MoM math, over-budget detection, member spend, daily trend, and the LLM `query_expenses` total all become refund-aware. Top-expenses panel filters refunds out (a refund isn't a top *spend*).
- **Recurring expense rules.** Tell the bot "add Netflix $15.49 monthly on the 7th" and a `recurring_rules` row is created. New LLM tools: `add_recurring_rule`, `list_recurring_rules`, `delete_recurring_rule`, `pause_recurring_rule`. Frequency = monthly / weekly / yearly; anchor_day clamps gracefully (anchor=31 → Feb 28/29, anchor=Mon → next Monday). Modes: `confirm` (default — bot DMs "yes/no/skip" before logging) and `auto` (silent insert, for true auto-pay items the user has validated).
- **Bot-confirmed recurrence.** When a `confirm`-mode rule fires, the bot DMs the household owner: *"Recurring: Netflix $15.49 today — reply yes/no/skip"*. The router intercepts the user's next reply *before* the LLM ever sees it (the LLM should never silently log money on the user's behalf), parses yes/no/skip aliases, and either inserts the expense or skips. Pending confirmations time out after 36 hours; second rules for the same chat wait their turn rather than stacking.
- **Weekly summary push (default ON).** Once a week the bot DMs the owner a 7-day recap: total spend, expense count, top 3 categories. New `Settings → Bot notifications` toggle.
- **Budget threshold alerts (default ON).** Hourly sweep evaluates active variable categories against their monthly target. Bot DMs at 80% and 100% — once per threshold per calendar month, tracked in `budget_alert_state` so a single big expense doesn't re-fire the same alert next hour. Investing categories are excluded (savings goals, not spending caps). Toggle in `Settings → Bot notifications`.

### Internal

- **New `scheduler` module + tokio task.** Wakes every 60s, dispatches due jobs from `scheduled_jobs` by kind, advances `next_due_at`. Stale-job protection: jobs more than 7 days overdue (e.g., the user's machine was off for two weeks) are skipped, not silently fired. Handlers return `Reschedule` / `Done` / `Retry` outcomes; the scheduler interprets each. Three handlers shipped: `recurring_expense`, `weekly_summary`, `budget_alert_sweep`. The same primitive will carry sync heartbeats and other v0.3+ background work.
- **Singleton job pattern.** `weekly_summary` and `budget_alert_sweep` each ensure exactly one row exists at startup via `scheduler::ensure_singleton`. Idempotent across relaunches; re-enables disabled rows.
- **Migration 0006**: `expenses` table recreated to lift the `amount_cents >= 0` CHECK (now `> 0`), add `is_refund` flag and `refund_for_expense_id` FK with `ON DELETE SET NULL`. Forward-only.
- **Migrations 0007–0009**: `scheduled_jobs`, `recurring_rules` + `pending_recurring_confirmations`, and `budget_alert_state` tables.
- **`RouterDeps` is now `Clone`** so the scheduler task can share the same Telegram client + LLM provider + DB handle the poller already uses.
- **MutexGuard discipline tightened in the router's confirmation flow** — the spawned async task requires no SQLite lock guard to be live across an `.await`, which would otherwise break Send-safety on the spawned future.

### Tests

- Refund signed-sum across all 5 aggregation sites; refund migration round-trip; FK cascade behavior on parent delete; CHECK rejects zero/negative.
- Scheduler queue helpers (enqueue / list_due / disable / singleton / stale detection); tick semantics for stale-skip and Retry; orphan-job disable.
- Recurring rule LLM tool round-trips (add / list / delete / pause); auto-mode inserts + reschedules; confirm-mode DMs + records pending + defers second rule; paused rule advances without DMing or inserting; missing-rule disables orphan job; clamp behavior at month edges and leap years.
- Bot-confirm flow: yes inserts + clears pending, no/skip clears without inserting, unknown reply re-prompts without dropping pending, expired pending falls through to the LLM, `/cancel` clears pending without going through the confirmation parser.
- Weekly summary: no-owner just slips schedule, with-owner DMs a recap.
- Budget alerts: 80% fires once and stays silent for the rest of the month, disabled setting short-circuits without DM.

## [0.2.5] - 2026-04-30

### Fixed

- **Bar charts no longer flash a giant white highlight on hover.** Recharts paints a translucent white "cursor" rectangle behind the hovered bar by default, plus restyles the bar itself via its `activeBar` overlay — both visually loud on the dark theme. Disabled both on the per-category and household-member bar charts (`cursor={false}` on Tooltip + `activeBar={false}` on Bar).

## [0.2.4] - 2026-04-30

### Fixed

- **Insights dashboard was broken in v0.2.3**: every load failed with `invalid args 'range' for command 'get_dashboard': unknown variant 'month', expected one of 'this_week', 'this_month', 'this_quarter', 'this_year', 'ytd', 'custom'`. v0.2.3 added a `Month { year, month }` variant to the internal `DateRange` enum but missed the *IPC-boundary* `RangeArg` enum that deserializes the frontend payload. Serde rejected `kind: "month"` before it ever reached the converted `DateRange`. Adds the matching variant to `RangeArg` and the From impl.

## [0.2.3] - 2026-04-30

### Added

- **Variable spending trajectory chart** on the Insights dashboard. Plots cumulative variable spend day-by-day, plus a least-squares line of best fit extrapolated to month-end, plus the variable budget as a flat reference line. Subtitle reads off whether the trend is projecting over or under budget.
- **Sum-total cards** at the top of the Categories tab: grand total plus per-kind subtotals (Fixed, Variable, Saving / Investing). Sums only include active categories with a saved monthly target — what's actually contributing to the live monthly plan.

### Changed

- **Insights time-range dropdown is now a calendar-month picker.** The app's budget model is monthly; the prior week / quarter / year / YTD ranges aggregated across multiple months in ways the totals/pacing math couldn't honor. The dropdown now lists the last 12 calendar months (current first); each selection scopes the dashboard to that month. Past-month views show static totals + over-budget detection but skip pacing/MoM (those only make sense for the current month).
- **KPI text wraps inside its box.** "Daily allowance" with longer numbers was clipping. Cards now use `text-xl` + `break-words` so primary and secondary lines wrap cleanly.
- **Per-category bar chart bars are a uniform thickness** (~18px) regardless of how many categories have spend in the period — small bar counts no longer stretch each bar to fill the chart. The chart panel grows or shrinks; the bars don't.
- **Bar chart title** dropped its "over budget = orange, savings goal met = deep green" explainer subtitle. The coloring is intuitive enough on its own.

### Internal

- New `DateRange::Month { year, month }` variant + `is_monthly` / `is_current_month` helpers. `insights/mod.rs` now gates each panel on the right helper: pacing snapshot + MoM + upcoming-fixed only render for the current month, but over-budget detection works for any monthly view.
- `KpiCard` gains `variable_budget_cents` and `fixed_budget_cents` so the new trajectory chart can draw the variable-budget cap line for any monthly range.

## [0.2.2] - 2026-04-30

### Added

- **Total budget** and **Total remaining** KPI cards on the Insights dashboard. Previously the strip only surfaced variable-spend pacing (because that's the actionable daily-allowance signal); now the headline numbers — fixed + variable budgeted, fixed + variable remaining — are visible at a glance too. The "Total remaining" card colors itself: green when >10% of budget left, yellow when <10%, red when over budget.
- "Total remaining" secondary line now shows the `% of budget spent` so you can read pace without doing math.

### Internal

- `KpiCard` gains `total_budget_cents` and `total_remaining_cents`. Both are populated only for the `ThisMonth` range (the budget model is monthly); other ranges render "—" for these cards. Investing-kind targets are intentionally excluded from the total — they're savings goals, not a spending allowance, and they already have their own visual on the per-category bar chart.
- KPI strip re-laid-out from a 4-card grid to a 6-card grid (`grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6`) so the new cards fit cleanly across breakpoints.

## [0.2.1] - 2026-04-30

### Added

- **Electric** and **Water** as fixed-cost seed categories (inactive by default — tick on under Categories → Fixed if either applies). Migration `0005_seed_electric_water.sql` adds them to existing installs via INSERT OR IGNORE.

### Why this release exists

First end-to-end dogfood of the v0.2.0 in-app updater. AppImage / DMG / MSI / EXE users on v0.2.0 should see the update banner on next launch.

## [0.2.0] - 2026-04-30

### Added

- **Single-instance enforcement on Linux / Windows** via [`tauri-plugin-single-instance`](https://v2.tauri.app/plugin/single-instance/). Previously every desktop-icon click spawned a full second process (own tray entry, own bot poller, own DB lock contention) — easy to rack up memory without realizing it. The new behavior: a second launch hands its argv to the already-running app and exits, and the running window comes to the foreground. macOS already does this natively through the Dock, so the plugin is functionally a no-op there.
- **In-app auto-update** for AppImage / DMG / MSI / EXE installs via [`tauri-plugin-updater`](https://v2.tauri.app/plugin/updater/) against GitHub Releases.
  - On launch (toggleable in Settings → "App updates" → "Check for updates on launch", default ON) the app pings GitHub Releases for the manifest. If a newer version exists, a sticky banner offers **Install** or **Skip** at the top of the main window.
  - Settings → "App updates" → **Check now** triggers a manual check.
  - Update payloads are signed with a project-specific ed25519 key (separate from the GPG key that signs the AppImage download). The pubkey is embedded in the binary; tampered updates fail verification and the install is refused.
  - One outbound request to `api.github.com` per launch when the toggle is ON. No analytics, no telemetry, nothing else changes about the project's privacy posture.
- **RPM and DEB packages do not auto-update** — system package managers own their install path. Those users keep upgrading via `sudo dnf upgrade ./Mr.Moneypenny.rpm` or `sudo apt upgrade ./Mr.Moneypenny.deb`. A real Fedora COPR / Debian PPA is a separate, larger project; it's on the long-term roadmap but not in this release.

### Internal

- New `tauri-plugin-updater` dependency, gated on the existing `desktop` feature so headless tests still run with `cargo test --no-default-features`.
- New Tauri commands: `check_for_update`, `install_update`, `get_check_updates_on_launch`, `set_check_updates_on_launch`. Settings key `check_updates_on_launch` mirrors the existing `run_in_background` / `autostart` toggle pattern.
- CSP `connect-src` now includes `https://api.github.com`, `https://github.com`, and `https://objects.githubusercontent.com` — the only outbound destinations the updater touches.
- `tauri.conf.json` gains a `bundle.createUpdaterArtifacts: true` flag and a `plugins.updater` stanza with the GitHub-Releases manifest endpoint and the embedded ed25519 pubkey.
- `release.yml` passes `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` into `tauri-action`, which now produces signed updater bundles + per-platform `latest.json` patches alongside the regular installers.

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
