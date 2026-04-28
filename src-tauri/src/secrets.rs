//! OS-keychain-backed secret storage.
//!
//! Secrets (Anthropic API key, Telegram bot token) NEVER touch the
//! SQLite database or any plaintext file. They go straight to the
//! platform keychain:
//!
//! - macOS:   Keychain Access
//! - Windows: Credential Manager
//! - Linux:   Secret Service (libsecret); falls back to keyutils.

use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE: &str = "moneypenny";

pub mod keys {
    pub const ANTHROPIC_API_KEY: &str = "anthropic_api_key";
    pub const TELEGRAM_BOT_TOKEN: &str = "telegram_bot_token";
}

/// Store a secret. Overwrites any existing entry under the same key.
pub fn store(key: &str, value: &str) -> Result<()> {
    let entry = Entry::new(SERVICE, key).with_context(|| format!("opening keychain for {key}"))?;
    entry
        .set_password(value)
        .with_context(|| format!("writing keychain entry {key}"))?;
    Ok(())
}

/// Retrieve a secret. Returns `Ok(None)` if no entry exists.
pub fn retrieve(key: &str) -> Result<Option<String>> {
    let entry = Entry::new(SERVICE, key).with_context(|| format!("opening keychain for {key}"))?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading keychain entry {key}")),
    }
}

/// Delete a secret. Returns true if a row existed.
pub fn delete(key: &str) -> Result<bool> {
    let entry = Entry::new(SERVICE, key).with_context(|| format!("opening keychain for {key}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e).with_context(|| format!("deleting keychain entry {key}")),
    }
}

/// True if a secret exists under the given key.
pub fn exists(key: &str) -> Result<bool> {
    Ok(retrieve(key)?.is_some())
}
