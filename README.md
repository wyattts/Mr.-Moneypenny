# Mr. Moneypenny

> A FOSS personal-budgeting app you talk to in plain English. Your data lives only on your computer.

Mr. Moneypenny is a desktop app that pairs with a personal Telegram bot. You log expenses by chatting with the bot ("$5 coffee," "paid rent $1500"), and the bot — powered by your choice of Anthropic's Claude API or a local Ollama LLM — parses, stores, and queries them in a local SQLite database on your own machine. The project operates **zero servers** and keeps **zero copies** of your data.

## Status

🚧 **0.1.0 — early alpha.** End-to-end works on Linux (smoke-tested on Fedora 43 + Mutter/Wayland). macOS and Windows builds are produced by CI but unsigned until [project sponsorship](https://github.com/sponsors/wyattts) covers code-signing certificates. The branding pass is still pending.

## Install

Pre-built artifacts are at [Releases](https://github.com/wyattts/Mr.-Moneypenny/releases). See [`docs/distribution.md`](docs/distribution.md) for per-platform install steps + signature verification.

If you'd rather build from source:

```bash
git clone https://github.com/wyattts/Mr.-Moneypenny.git
cd Mr.-Moneypenny
npm install
NO_STRIP=true npm run tauri:build       # Linux release artifacts
# OR
npm run tauri:dev                       # development with hot-reload
```

Full prerequisites in [`BUILDING.md`](BUILDING.md).

## Goals

- **Plain-English expense logging** via a personal Telegram bot you create yourself with @BotFather. The bot is yours; we never see your messages.
- **Insights dashboard** built into the desktop app — KPIs, category breakdowns, daily trends, fixed-vs-variable pacing, per-household-member attribution. Pulls 100% from your local SQLite database.
- **Shared-household support** — partners can both chat with the same bot and contribute to the same database, with attribution preserved per expense.
- **Privacy by architecture** — no relay, no telemetry, no analytics. Outbound network traffic is limited to (1) Telegram's API using your bot token, (2) your chosen LLM provider (Anthropic with your API key, or your own local Ollama), and (3) optionally a release-update check (off in privacy mode).
- **Plug-and-play install** for non-technical users on Linux, macOS, and Windows. The Anthropic path requires no terminal, no shell commands, and no prerequisite installs.

## How it works

```
You ────────►  Telegram (your bot)  ────────►  Mr. Moneypenny on your desktop  ────────►  SQLite (your machine)
                                                       │
                                                       ▼
                                       Anthropic API  or  local Ollama
```

Mr. Moneypenny on your desktop holds an open long-poll connection to Telegram. When you message the bot from any device (phone, laptop, web), the desktop receives the message, asks the LLM to parse it into a structured operation, applies the operation to your local database, and sends a response back through Telegram.

Closing the window minimizes to the system tray so the bot stays online without a visible window. Auto-start on login is opt-in.

## What you get in the desktop app

- **Insights dashboard** — KPI strip with budget pacing, category donut, daily trend (variable + fixed), per-household-member attribution, top expenses, over-budget warnings, upcoming-fixed list, month-over-month delta.
- **Ledger** — searchable / filterable list of every expense with inline delete.
- **Categories / Budgets** — edit defaults, add new, set monthly targets.
- **Household** — invite a partner via a 6-digit pairing code; per-member spend shown on the dashboard.
- **Settings** — rotate keys, switch LLM provider, toggle background mode and auto-start.

## License

[GNU Affero General Public License v3.0](LICENSE). The AGPL is chosen specifically so that any forked or hosted version must also publish source — protecting the privacy thesis from being eroded by a closed-source SaaS clone.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). All contributions are welcome — bug reports, code, docs, design, translations.

## Security

See [SECURITY.md](SECURITY.md) for the responsible-disclosure process.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
