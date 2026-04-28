# Security Policy

## Reporting a vulnerability

If you discover a security vulnerability in Mr. Moneypenny, please report it privately rather than opening a public issue.

**Email:** `wyatts+moneypenny@proton.me`

If you'd like to encrypt your report, request the project's PGP key in your initial email and one will be provided. (Once published, the public key fingerprint will be listed here.)

Please include:

- A description of the vulnerability and its impact.
- Steps to reproduce, if possible.
- Affected version(s).
- Any suggested mitigation.

## Response expectations

This is a single-maintainer project. I will acknowledge receipt within 7 days and aim to provide an initial assessment within 14 days. Realistic timelines for fixes will be communicated as soon as the issue is triaged.

## Scope

In scope:

- The Mr. Moneypenny desktop application (Tauri shell, Rust backend, TypeScript frontend).
- The build and release tooling under `.github/workflows/`.
- Documentation that could mislead users about privacy or security guarantees.

Out of scope:

- Vulnerabilities in third-party dependencies (please report those upstream — but feel free to flag them here too).
- Vulnerabilities in Telegram, Anthropic, or Ollama themselves.
- Issues that require physical access to a user's machine. (The data is stored locally on user-session-protected files; an attacker with admin access to the device can read everything by design.)

## Coordinated disclosure

Once a fix is available, I will publish a security advisory via GitHub's advisory system, credit the reporter (with permission), and tag a patched release. If the issue is critical, I will request a CVE ID.

## Hall of fame

Security researchers who responsibly disclose verified vulnerabilities will be credited in the project's security advisories and (with permission) in this file.
