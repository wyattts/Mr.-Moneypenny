# Threat Model

This is the threat model Mr. Moneypenny is designed against. The intent is to be honest about what we defend, what we don't, and why.

## Assets

In rough order of sensitivity:

1. **Expense data** — what the user spent money on, when, with whom. Free-text descriptions can contain sensitive financial, medical, legal, or personal information.
2. **Telegram bot token** — gives full control of the user's bot. Compromise = attacker can impersonate the bot to the user and read messages sent to it.
3. **Anthropic API key** — billing exposure if leaked.
4. **Authorized chat IDs** — the whitelist of which Telegram chats can talk to the bot.
5. **App settings, category list, budgets** — less sensitive but still personal.

## Adversaries

- **A1: Network attacker** — observes traffic between user's machine and the internet.
- **A2: Malicious or compromised third-party service** — Telegram, Anthropic, Ollama, GitHub, npm, crates.io.
- **A3: Compromised dependency** — a transitive npm or Cargo crate ships malicious code.
- **A4: Local malware** — code running with the user's local user privileges.
- **A5: Physical thief** — has the user's unlocked machine.
- **A6: Other Telegram users** — discover the bot's username and try to interact with it.
- **A7: Project maintainer** (this is us) — what damage could a malicious upstream do?

## Defenses by adversary

### A1: Network attacker

- All outbound traffic is HTTPS with certificate validation (default `reqwest` + `rustls` behavior).
- No plaintext protocols anywhere.
- Limited surface area: only three classes of endpoint (Telegram, LLM, optional update check).

**Residual:** TLS interception by a CA the user's machine trusts (corporate MITM proxy). We do not pin certificates because users in such environments often need to use the app.

### A2: Compromised third-party

- **If Anthropic is compromised:** prompts and category names exposed. Mitigation: use Ollama instead.
- **If Telegram is compromised:** all bot messages exposed. This is a fundamental property of the Telegram Bot API; it has no E2E. Mitigation: don't put highly sensitive descriptions in Telegram chat. Document this in onboarding.
- **If GitHub or npm or crates.io is compromised:** supply-chain attack. Mitigation: dependency audits in CI, locked versions, reproducible-build effort, signed releases.

### A3: Compromised dependency

- `cargo audit` and `npm audit` run in CI.
- `Cargo.lock` and `package-lock.json` are committed and required for reproducible builds.
- Small dependency tree by policy.
- New dependencies require justification in PR description.

**Residual:** zero-day in a transitive dep before audit catches it. Standard industry risk.

### A4: Local malware (with user-session privileges)

This is the hardest adversary because the SQLite database lives in user-readable files and the keychain is unlocked while the user is logged in.

- Secrets are in the OS keychain (better than plaintext on disk, but still readable by the user-session).
- Optional SQLite encryption with a passphrase the user types each launch (gated behind an "advanced" toggle; not default because losing the passphrase = losing all data).
- Tauri CSP locked, IPC commands explicitly allowlisted, no `unsafe_code` in our Rust crate.

**Residual:** if you have malware running as you, you have bigger problems than a budgeting app. We do not claim defense in depth against this.

### A5: Physical thief

- OS-level disk encryption is the user's responsibility (FileVault / BitLocker / LUKS).
- Optional SQLite passphrase as a second factor.

**Residual:** an unencrypted disk gives full access to the database.

### A6: Other Telegram users

This is where the threat model bites the most for a v1 chatbot.

- Bots are **publicly addressable** by their username. Anyone who guesses or learns it can send messages to the bot.
- **Pairing-code authorization:** during setup, the app generates a 6-digit code; the first chat to send `/start <code>` becomes the authorized owner. Subsequent invites use fresh codes. Unauthorized chats are silently ignored (or sent a "this bot is private" reply).
- **Per-chat rate limiting** prevents a malicious paired chat from flooding the database.
- **Owner-only sensitive commands** (invite, remove, change settings) — non-owner authorized chats cannot escalate.

**Residual:** a leaked pairing code (e.g., shouted across an open office) lets a stranger pair their account. Mitigation: short TTL on pairing codes (10 minutes); easy to revoke an authorized chat from the GUI.

### A7: Malicious project maintainer / upstream

What if the maintainer (or an attacker who compromises the maintainer's account) ships a build that exfiltrates user data?

- **AGPL-3.0** ensures any hosted fork must publish source — but this is post-hoc, not preventive.
- **Reproducible builds** (effort, not yet a guarantee) so a determined user can verify the published binary corresponds to the published source.
- **Signed releases** with a key that is *not* the same as the GitHub auth token, so a single GitHub account compromise doesn't immediately yield signed binaries.
- **No automatic updates without user opt-in** — by default, the user pulls updates manually. Auto-update can be enabled in settings.

**Residual:** ultimately, you have to trust the project to some degree. The structural mitigations above raise the bar against a single-point compromise.

## Out of scope

- Defending against state-level adversaries with zero-days in the operating system or browser engine.
- Defending against the user voluntarily pasting their bot token into a phishing site.
- Privacy of metadata visible to network observers about the *fact* that you use Mr. Moneypenny (no anti-fingerprinting measures).
- Multi-host database synchronization (v1 is single-host).

## Open questions / future work

- [ ] Decide on certificate-pinning policy for the LLM and Telegram endpoints.
- [ ] Investigate hardware-token (YubiKey) integration for the SQLite passphrase.
- [ ] Document a "panic delete" workflow for users in coercive situations.
- [ ] Quarterly review of this document as the codebase grows.
