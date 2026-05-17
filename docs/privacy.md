# Privacy

Mr. Moneypenny is designed so that no party other than you can read your expense data. This document describes what that means in practice and the limits of that promise.

## What we (the project) collect

**Nothing.**

The Mr. Moneypenny project operates no servers and runs no telemetry. We do not have a copy of:

- Your expense data
- Your Telegram bot token
- Your Anthropic API key
- Your IP address
- Anything you type into the app

If your machine is destroyed and you have no backup, the project cannot help you recover your data — because we never had it.

## Where your data lives

- **Expense database:** a SQLite file on your computer.
  - Linux: `~/.local/share/moneypenny/db.sqlite`
  - macOS: `~/Library/Application Support/moneypenny/db.sqlite`
  - Windows: `%APPDATA%\moneypenny\db.sqlite`
- **Secrets** (Telegram bot token, Anthropic API key, optional database passphrase): your operating system's keychain.
  - macOS: Keychain Access
  - Windows: Credential Manager
  - Linux: Secret Service (libsecret), with `keyutils` fallback for headless setups.
- **Settings, category preferences, budgets:** in the same SQLite file as your expenses.

## Exporting your data (the open hub)

Your data is yours, and that ownership is meant to be *exercisable*, not just asserted. Ledger → **Export your data** writes a complete, faithful copy to a file you choose.

Deliberate properties of this feature:

- **User-initiated and desktop-only.** Nothing exports on its own, on a schedule, or in the background. The Telegram bot has no part in it — a full ledger is a large, sensitive payload and is never sent through Telegram. You run it, on your machine, when you want it.
- **A snapshot, not a live feed.** Each export is a point-in-time photocopy taken when you ask. There is no server, no port, and nothing listening.
- **One-way.** Export is a portability guarantee — you can always leave with a perfect copy. It is not a re-import pipeline; nothing reads an edited file back in.
- **No network, no third party.** The export is read locally and written to a local path. It involves no outbound call and does not touch the CSP allowlist.
- **Reads a stable, published contract.** All formats read the `v_ledger_v1` SQL view (DB migration 0017), never the raw tables, so the internal schema can evolve without breaking your exports or silently changing their shape.

Formats: an ordinary **spreadsheet (CSV)**, a **programmer file (JSON Lines)**, and **plain-text accounting (Beancount)** for the Fava/Beancount ecosystem. Timeframes: **All time** (the full portability copy, and the default) or this **year / quarter / month** (for taxes or an accountant). Caveat: a non–"All time" Beancount slice is valid but less self-contained than a full history, since it omits earlier context.

## What goes over the network

The desktop app makes outbound HTTPS calls to exactly three classes of endpoint:

1. **Telegram Bot API** at `api.telegram.org`, using your personal bot's token. Telegram receives your messages to the bot, your bot's replies, and metadata about your chat. This is governed by [Telegram's privacy policy](https://telegram.org/privacy). The Mr. Moneypenny project does not have access to any of this traffic.

2. **Your chosen LLM provider:**
   - If you choose **Anthropic**, your expense descriptions and a structured budget context are sent to `api.anthropic.com` using *your* API key. This traffic is governed by [Anthropic's privacy and data-usage policies](https://www.anthropic.com/legal/privacy). Anthropic's enterprise privacy commitments apply to API usage. Note that prompt caching sends category names (not expense content) to Anthropic for cache reuse.
   - If you choose **Ollama**, all LLM traffic stays on your machine. Nothing leaves localhost.

3. **Release-update check** (optional, off in privacy mode). When enabled, the app makes a single HTTPS GET against the project's GitHub Releases endpoint to check for new versions. It transmits no data about you beyond what's contained in a normal HTTPS request.

There are **no other outbound calls**. No analytics, no crash uploaders, no font CDNs, no third-party scripts.

## What we cannot promise

Privacy is a property of the software *and* the third parties you choose to involve. We cannot make promises on their behalf:

- **Telegram** can read messages between you and your bot (bot chats are not end-to-end encrypted; this is a Telegram Bot API limitation that applies to every Telegram bot, not just ours).
- **Anthropic** receives the prompts you send to Claude, governed by their policies.
- **Your operating system** controls how secrets are stored in the keychain. We rely on its security model.
- **Your hardware** is in your hands. If your machine is compromised, an attacker with user-session access can read the database file.

Choose Ollama if you want to eliminate the third-party LLM data flow. The Telegram dependency is fundamental to the bot model.

## What about my household members?

Mr. Moneypenny supports shared-household use: multiple Telegram users can chat with the same bot and contribute to one shared database. Every expense is tagged with which household member logged it (`logged_by_chat_id`). This is local attribution only — the project still has no access to any of it.

## Crash reports

Crash dumps are written locally to `<data dir>/crashes/`. They are **never automatically uploaded**. If you want to send one to the maintainer, open the file in a text editor first, redact any sensitive content (raw expense messages, etc.), and email it manually.

## Backups

You are responsible for backing up your SQLite database. The app provides:

- **Manual export** to CSV, JSON Lines, or Beancount (Ledger → Export your data — see [the open hub](#exporting-your-data-the-open-hub) above). A faithful one-way copy; not encrypted, so store the file somewhere you trust.
- **Optional auto-backup** to a folder of your choice. You can point this at iCloud Drive, Dropbox, OneDrive, Syncthing, or any other folder. This is your choice; the project does not provide its own sync.

## Changes to this policy

Changes to the privacy posture (e.g., adding any new outbound network call, changing what's stored where) require a major version bump and a changelog entry that calls out the change explicitly.
