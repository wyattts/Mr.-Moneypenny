//! Disk-backed encrypted secret storage.
//!
//! Replaces the OS-keyring-backed implementation that shipped through
//! v0.2.6. The OS keyring on Linux has too many silent-failure modes
//! (session-only collections, autologin without PAM, AppImage outside
//! dbus, headless sessions, KDE/sway without auto-unlock) — every one
//! of which can cause secrets to vanish on reboot with no signal to
//! the user.
//!
//! This module stores secrets in a single file under the platform data
//! directory, encrypted under a key derived from a stable per-machine
//! identifier + the user's data path. Same threat model as the OS
//! keyring on a single-user laptop — but reliable across all the
//! environments the keyring fails in.
//!
//! See `kdf.rs`, `cipher.rs`, `store.rs` for the building blocks and
//! `migration.rs` for the one-shot transparent copy from the old
//! keyring backend on first launch after upgrade.

mod cipher;
mod kdf;
mod migration;
mod store;

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};

use store::{SecretsFile, SECRETS_FILENAME};

pub mod keys {
    pub const ANTHROPIC_API_KEY: &str = "anthropic_api_key";
    pub const TELEGRAM_BOT_TOKEN: &str = "telegram_bot_token";
}

/// Default location of the secrets file under the platform data dir
/// (the same dir that holds the SQLite database).
fn default_secrets_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "moneypenny", "moneypenny")
        .context("could not resolve platform data directory")?;
    Ok(dirs.data_dir().join(SECRETS_FILENAME))
}

fn default_data_dir() -> Result<String> {
    let dirs = directories::ProjectDirs::from("dev", "moneypenny", "moneypenny")
        .context("could not resolve platform data directory")?;
    Ok(dirs.data_dir().to_string_lossy().into_owned())
}

/// Process-wide handle to the secrets file. Opening is cheap (a single
/// read + KDF call) but we cache it so callers don't repeatedly do the
/// HKDF derivation.
fn handle() -> Result<&'static Mutex<SecretsFile>> {
    use std::sync::OnceLock;
    static H: OnceLock<Mutex<SecretsFile>> = OnceLock::new();
    if let Some(m) = H.get() {
        return Ok(m);
    }
    let path = default_secrets_path()?;
    let data_dir = default_data_dir()?;
    let f = SecretsFile::open(path, &data_dir)?;
    Ok(H.get_or_init(|| Mutex::new(f)))
}

/// Store a secret. Overwrites any existing entry under the same key.
pub fn store(key: &str, value: &str) -> Result<()> {
    let h = handle()?;
    let mut f = h.lock().unwrap();
    f.put(key, value)
}

/// Retrieve a secret. Returns `Ok(None)` if no entry exists.
///
/// On first call after upgrading from v0.2.6, this transparently copies
/// any entries the user previously stored in the OS keyring into the new
/// disk store, then returns the value from disk. See `migration::run`.
pub fn retrieve(key: &str) -> Result<Option<String>> {
    let h = handle()?;
    {
        let f = h.lock().unwrap();
        if let Some(v) = f.get(key)? {
            return Ok(Some(v));
        }
    }
    // Disk had no entry — try the migration path. If it succeeds the
    // entry will exist on disk for the next call.
    if migration::try_copy_from_keyring(h, key)? {
        let f = h.lock().unwrap();
        return f.get(key);
    }
    Ok(None)
}

/// Delete a secret. Returns true if a row existed.
pub fn delete(key: &str) -> Result<bool> {
    let h = handle()?;
    let mut f = h.lock().unwrap();
    f.remove(key)
}

/// True if a secret exists under the given key.
pub fn exists(key: &str) -> Result<bool> {
    Ok(retrieve(key)?.is_some())
}
