//! One-shot transparent migration from the v0.2.6 OS-keyring backend
//! into the new disk-encrypted store.
//!
//! Strategy: when the secrets handle is first acquired in a process,
//! eagerly probe the OS keyring once for each known secret key. If the
//! keyring has the value, copy it to the disk store. Either way, mark
//! the key as "migrated" inside the disk file so subsequent process
//! launches don't probe the keyring again — that probe is a dbus call
//! on Linux and a slow, occasionally-blocking system call elsewhere.
//!
//! This eager-on-open approach replaces the v0.3.7-and-earlier behavior
//! that probed the keyring on *every* `retrieve` call for a missing
//! key, forever (the doc-comment promised "first call after upgrade"
//! but the implementation didn't enforce that).
//!
//! v0.3.9 will drop the `keyring` crate entirely. By then every active
//! user has either been migrated (sentinel set) or re-entered their
//! secrets through Settings (which writes through the disk path).

use std::sync::Mutex;

use anyhow::Result;

use super::keys;
use super::store::SecretsFile;

const KEYRING_SERVICE: &str = "moneypenny";

/// All secret keys that v0.2.6 might have stored in the OS keyring.
const LEGACY_KEYS: &[&str] = &[keys::ANTHROPIC_API_KEY, keys::TELEGRAM_BOT_TOKEN];

/// Eagerly drain any v0.2.6 keyring entries into the disk store. Runs
/// once per process, on the first call to `secrets::handle()`. After
/// this, the disk file's `migrated_keyring_keys` sentinel records each
/// key we've considered, so future launches skip the probe entirely.
pub(super) fn eager_drain_keyring(handle: &Mutex<SecretsFile>) -> Result<()> {
    let already: Vec<String> = {
        let f = handle.lock().unwrap();
        f.migrated_keyring_keys()
    };
    for key in LEGACY_KEYS {
        if already.iter().any(|m| m == key) {
            continue;
        }
        // Only attempt the keyring read if we don't already have the
        // value on disk — avoids a dbus call when the user already
        // entered the secret through Settings on this device.
        let need_probe = {
            let f = handle.lock().unwrap();
            !f.contains(key)
        };
        if need_probe {
            if let Some(value) = read_keyring(key) {
                let mut f = handle.lock().unwrap();
                f.put(key, &value)?;
                drop(f);
                tracing::info!(
                    target: "secrets::migration",
                    key,
                    "drained secret from OS keyring → disk store"
                );
                let _ = delete_keyring(key);
            }
        }
        // Mark as migrated regardless of whether the keyring had a
        // value — the contract is "we asked once."
        let mut f = handle.lock().unwrap();
        f.mark_migrated(key)?;
    }
    Ok(())
}

fn read_keyring(key: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key).ok()?;
    entry.get_password().ok()
}

fn delete_keyring(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key)?;
    let _ = entry.delete_credential();
    Ok(())
}
