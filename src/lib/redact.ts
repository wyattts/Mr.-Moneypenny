// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
// Strip likely-secret tokens from strings before showing them in UI
// banners or copying them to clipboard. Conservative — favors
// over-redaction (false positives that look like "[redacted-x]") over
// leaking a real secret into a toast.
//
// Why this exists: error strings from `saveTelegramToken`,
// `saveAnthropicKey`, `testAnthropic`, and friends bubble up through
// anyhow chains that occasionally include the request body or URL in
// `Display`. Even after the v0.3.8 `reqwest::Error::without_url()`
// scrub on the Rust side, defense-in-depth on the UI side is cheap.
//
// Patterns recognized:
//   - Telegram bot token: <numeric_id>:<35+ alnum/`_-` chars>
//   - Anthropic API key:  sk-ant-<16+ chars>
//   - Generic sk-* key:   sk-<20+ chars>

export function redactSecrets(s: string): string {
  if (!s) return s;
  return s
    .replace(
      /\b\d{6,12}:[A-Za-z0-9_-]{30,}\b/g,
      "[redacted-telegram-token]",
    )
    .replace(
      /\bsk-ant-[A-Za-z0-9_-]{16,}\b/g,
      "[redacted-anthropic-key]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{20,}\b/g, "[redacted-api-key]");
}
