//! One-shot transparent migration from the v0.2.6 OS-keyring backend
//! into the new disk-encrypted store.
//!
//! Strategy: on the first `retrieve` call for a given key, if the disk
//! store has no entry, ask the OS keyring once. If the keyring has it,
//! copy to disk. From then on the disk store answers; the keyring is
//! never consulted again for that key.
//!
//! This is best-effort: any keyring error (no entry, locked collection,
//! Secret Service unavailable, dbus down) is silently treated as "not
//! found" — the user can re-enter via Settings, which writes to the
//! disk store.
//!
//! v0.2.7 will drop the `keyring` crate dependency entirely. By then
//! every active user will have either had their secrets migrated or
//! re-entered them through the disk path.

use std::sync::Mutex;

use anyhow::Result;

use super::store::SecretsFile;

const KEYRING_SERVICE: &str = "moneypenny";

/// Attempt to copy `key`'s value from the legacy OS keyring into the
/// disk store. Returns `Ok(true)` if migration succeeded, `Ok(false)`
/// otherwise (no entry, or any keyring failure). Never returns `Err`
/// for keyring-side failures — those are the very thing we're routing
/// around.
pub(super) fn try_copy_from_keyring(handle: &Mutex<SecretsFile>, key: &str) -> Result<bool> {
    let value = match read_keyring(key) {
        Some(v) => v,
        None => return Ok(false),
    };
    {
        let mut f = handle.lock().unwrap();
        // Guard: another thread might have raced and already migrated.
        if f.contains(key) {
            return Ok(true);
        }
        f.put(key, &value)?;
    }
    tracing::info!(
        target: "secrets::migration",
        key,
        "migrated secret from OS keyring → disk store"
    );
    // Try to delete from the old keyring to be tidy, but don't sweat
    // failures — orphaned keyring entries are harmless.
    let _ = delete_keyring(key);
    Ok(true)
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
