# Mr. Moneypenny

> A FOSS personal-budgeting app you talk to in plain English. Your data lives only on your computer.

Mr. Moneypenny is a desktop app that pairs with a personal Telegram bot. You log expenses by chatting with the bot ("$5 coffee," "paid rent $1500"), and the bot — powered by your choice of Anthropic's Claude API or a local Ollama LLM — parses, stores, and queries them in a local SQLite database on your own machine. The project operates **zero servers** and keeps **zero copies** of your data.

## Status

🚧 **Pre-alpha.** Active development. Not yet usable.

The implementation plan lives at [`docs/architecture.md`](docs/architecture.md). The current focus is bootstrapping the Tauri app, the SQLite schema, and the Telegram polling loop.

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

## License

[GNU Affero General Public License v3.0](LICENSE). The AGPL is chosen specifically so that any forked or hosted version must also publish source — protecting the privacy thesis from being eroded by a closed-source SaaS clone.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). All contributions are welcome — bug reports, code, docs, design, translations.

## Security

See [SECURITY.md](SECURITY.md) for the responsible-disclosure process.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
