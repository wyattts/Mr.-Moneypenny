# Architecture

This document gives a high-level overview of how Mr. Moneypenny is built. For the full implementation plan with file-level detail and phasing, see the design document in the project history.

## Components

```
┌─────────── User's Machine ───────────┐                      ┌──────────────┐
│  ┌─────────── Tauri App ───────────┐ │   long poll          │  Telegram    │
│  │  React UI                       │ │   (getUpdates)       │  Bot API     │
│  │       ↕ IPC                     │◄┼──────────────────────►              │
│  │  Rust backend                   │ │                      └──────────────┘
│  │   ├ telegram::poller            │ │                              ▲
│  │   ├ llm::{anthropic,ollama}     │ │                              │
│  │   ├ db (rusqlite, bundled)      │ │                              │
│  │   └ keyring (OS keychain)       │ │   HTTPS                      │ user
│  └─────────────────────────────────┘ │ ─────────────────►           │ types
│  SQLite DB + OS keychain (secrets)   │                              │ from
└──────────────────────────────────────┘                      ┌──────────────┐
                                                              │  Anthropic   │
                                                              │  / Ollama    │
                                                              └──────────────┘
```

## Data flow: logging an expense

1. User sends "$5 coffee" to their personal bot from any device (phone, web, tablet).
2. Telegram queues the message server-side under the bot's update queue.
3. Mr. Moneypenny on the user's desktop holds an open `getUpdates` long-poll connection. Telegram immediately delivers the message.
4. The Rust backend hands the message to the LLM provider (Anthropic or Ollama) with a system prompt that includes the user's category list and a structured budget context.
5. The LLM returns a *tool call* (e.g., `add_expense(amount=500, currency="USD", category="Coffee", description="$5 coffee", occurred_at=...)`) — never raw SQL.
6. The dispatcher validates the tool call against its JSON schema and, if valid, executes it through the parameterized SQLite repository.
7. The backend emits a Tauri event `expense:added`; the dashboard re-fetches if mounted.
8. The bot calls `sendMessage` with a confirmation. Telegram delivers the reply to whichever device the user is on.

## Key design choices

- **No relay.** The local app polls Telegram directly. The project owner operates zero servers.
- **Tool-use, not raw SQL.** The LLM emits validated structured operations; the database boundary is parameterized at all times.
- **One source of truth for budget math.** `src-tauri/src/domain/period.rs` is used by both the LLM `summarize_period` tool and the GUI dashboard; they cannot disagree.
- **OS keychain for secrets.** Bot token and Anthropic API key are never stored in plaintext on disk.
- **Multi-user via authorized chat whitelist.** A single shared database can be addressed by multiple Telegram users (household partners), with attribution preserved per expense.
- **Insights dashboard is the default landing view.** Pulls 100% from local SQLite. No LLM calls — the dashboard is fast, deterministic, and works offline.

## Module layout (Rust)

```
src-tauri/src/
├── db/                  — connection, migrations
├── domain/              — Expense, Category, Budget, period math
├── repository/          — parameterized CRUD against SQLite
├── insights/            — dashboard aggregation queries
├── llm/                 — provider trait, Anthropic + Ollama adapters, tool dispatcher
├── telegram/            — typed client, long-poll loop, router, auth (pairing codes)
├── commands/            — Tauri IPC commands exposed to the React frontend
└── state.rs             — app state, event emission
```

## Module layout (TypeScript)

```
src/
├── App.tsx              — router
├── wizard/              — 8-step setup wizard
├── views/               — Insights (default), Ledger, Categories, Budgets, Settings, Household
├── components/insights/ — KPI cards, charts, tables
├── lib/                 — Tauri command wrappers, dashboard subscription
└── styles/              — theme tokens (forest green / dark grey)
```

## Outbound network calls (allowlist)

The Tauri configuration restricts outbound HTTP to:

1. `https://api.telegram.org/*` — using the user's bot token.
2. `https://api.anthropic.com/*` *or* the user-configured Ollama endpoint (default `http://localhost:11434`).
3. The release-update endpoint (off in privacy mode).

No third-party fonts, CDNs, analytics, or telemetry. All assets bundled.
