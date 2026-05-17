# Mr. Moneypenny

> A FOSS personal-budgeting app you talk to in plain English. Your data stays on your computer.

Mr. Moneypenny is a desktop app that pairs with a Telegram bot you create yourself. You log expenses by messaging the bot ("$5 coffee", "paid rent $1500"); the bot parses, stores, and queries them in a local SQLite database on your machine. Parsing runs through either Anthropic's Claude API (your key) or a local Ollama model. No servers, no copies of your data.

## Status

**0.5.1 — alpha.** End-to-end works on Linux (tested on Fedora + Wayland). macOS and Windows builds are produced by CI but unsigned until [sponsorship](https://github.com/sponsors/wyattts) covers code-signing certificates.

## Install

Pre-built artifacts are on the [Releases](https://github.com/wyattts/Mr.-Moneypenny/releases) page. See [`docs/distribution.md`](docs/distribution.md) for per-platform install steps and signature verification.

Build from source:

```bash
git clone https://github.com/wyattts/Mr.-Moneypenny.git
cd Mr.-Moneypenny
npm install
NO_STRIP=true npm run tauri:build       # Linux release artifacts
# OR
npm run tauri:dev                       # development with hot-reload
```

Full prerequisites in [`BUILDING.md`](BUILDING.md).

## How it works

```
You ──►  Telegram (your bot)  ──►  Mr. Moneypenny on your desktop  ──►  SQLite (your machine)
                                            │
                                            ▼
                            Anthropic API  or  local Ollama
```

The desktop app holds a long-poll connection to Telegram. When you message the bot from any device, the desktop receives the message, asks the LLM to parse it into a structured operation (never raw SQL), applies it to your local database, and replies through Telegram. Money is formatted server-side, so the bot never does its own arithmetic on amounts.

Closing the window minimizes to the system tray so the bot stays online. Auto-start on login is opt-in.

## Features

- **Plain-English logging** via your own Telegram bot (created with @BotFather). The bot is yours; the project never sees your messages.
- **Insights dashboard** — budget pacing, category breakdown, daily trends (fixed vs. variable), per-household-member attribution, top expenses, over-budget and upcoming-fixed warnings, month-over-month delta.
- **Ledger** — searchable, filterable list of every expense with inline delete.
- **Categories** — edit defaults, add categories, toggle activation, set monthly budgets.
- **Household** — share one bot and database with a partner via a 6-digit pairing code; spend is attributed per member.
- **CSV import** — parse a bank export, dedupe, and categorize (optionally with AI suggestions).
- **Forecast / Simulator** — Monte Carlo investment projection, with an optional deterministic single-path mode.
- **AI Report Wizard** — a written analysis of a chosen period. All figures are computed deterministically in Rust from local data; the model only narrates over them. Includes PDF export.
- **Data export** — a complete one-way copy of your ledger to CSV, JSON Lines, or Beancount (all-time or a chosen year/quarter/month). A local snapshot read from a stable view; no server, no third party.
- **Settings** — rotate keys, switch LLM provider, toggle background mode and auto-start.

## Privacy

No relay, no telemetry, no analytics. Outbound traffic is limited to: (1) Telegram's API with your bot token, (2) your chosen LLM provider (Anthropic with your key, or local Ollama), and (3) an optional release-update check (off in privacy mode). The list is enforced by a CSP allowlist; adding an endpoint requires a documented change.

## License

[GNU Affero General Public License v3.0](LICENSE). AGPL is used so that any forked or hosted version must also publish its source.

## More

- [CONTRIBUTING.md](CONTRIBUTING.md) — bug reports, code, docs, design, translations all welcome.
- [SECURITY.md](SECURITY.md) — responsible-disclosure process.
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — Contributor Covenant.
</content>
</invoke>
