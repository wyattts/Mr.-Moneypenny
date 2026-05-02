# Changelog

All notable changes to Mr. Moneypenny are documented here. The format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.3] - 2026-05-01

Forecast wave 2 (redux) — bidirectional Monte Carlo Simulator + Category Analyzer + 80% probability bands on the Investment Calculator chart. The first attempt at v0.3.3 shipped Monte Carlo as two passive numbers bolted onto existing tools and was scrapped before any users saw it; this version gives probability questions their own surface with proper levers.

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
