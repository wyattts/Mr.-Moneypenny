//! Key derivation: machine-bound, deterministic.
//!
//! The master key is derived from a stable per-installation identifier so
//! the secrets file decrypts on the same machine across reboots / package
//! switches / desktop environments — but does NOT decrypt if the file is
//! lifted to a different machine. Same property as the OS keyring (which
//! is also per-machine + per-user).
//!
//! Derivation:
//!   ikm  = "moneypenny-secrets-v1" || machine_uid || data_dir_path
//!   key  = HKDF-SHA256(salt, ikm, info = "master-key", L = 32)
//!
//! - `machine_uid` comes from the platform's stable machine ID:
//!   Linux: /etc/machine-id (or /var/lib/dbus/machine-id);
//!   macOS: IOPlatformUUID (IOKit);
//!   Windows: HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid.
//! - `data_dir_path` includes the user's home directory, so two users on
//!   the same machine get different keys (and thus can't read each
//!   other's secrets even if their files are world-readable).
//! - `salt` is a 16-byte random value generated on first save and stored
//!   in the secrets file header. Different installs of the same app on
//!   the same machine (uncommon, but possible via portable/AppImage runs
//!   from different paths) get different keys.

use anyhow::{Context, Result};
use hkdf::Hkdf;
use sha2::Sha256;

/// Length of the symmetric key passed to ChaCha20-Poly1305.
pub const KEY_LEN: usize = 32;

/// Length of the random salt persisted in the secrets file header.
pub const SALT_LEN: usize = 16;

const IKM_PREFIX: &str = "moneypenny-secrets-v1";
const HKDF_INFO: &[u8] = b"master-key";

/// Derive the 32-byte master key from the salt + the machine identifier.
///
/// `data_dir` is included in the input keying material so that
/// per-user installations on the same machine get distinct keys.
pub fn derive_master_key(salt: &[u8; SALT_LEN], data_dir: &str) -> Result<[u8; KEY_LEN]> {
    let machine = read_machine_uid()?;
    let mut ikm = String::with_capacity(IKM_PREFIX.len() + machine.len() + data_dir.len() + 4);
    ikm.push_str(IKM_PREFIX);
    ikm.push(':');
    ikm.push_str(&machine);
    ikm.push(':');
    ikm.push_str(data_dir);

    let hk = Hkdf::<Sha256>::new(Some(salt), ikm.as_bytes());
    let mut out = [0u8; KEY_LEN];
    hk.expand(HKDF_INFO, &mut out)
        .map_err(|e| anyhow::anyhow!("hkdf expand: {e}"))?;
    Ok(out)
}

/// Read the platform's stable machine UID. Wraps the `machine-uid` crate
/// with a clearer error message — if this fails on Linux, /etc/machine-id
/// or /var/lib/dbus/machine-id is missing, which is unusual.
fn read_machine_uid() -> Result<String> {
    machine_uid::get()
        .map_err(|e| anyhow::anyhow!("reading machine UID: {e}"))
        .map(|s| s.trim().to_string())
        .context("could not read a stable machine identifier")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_uid_is_readable_on_this_host() {
        // Smoke test: machine-uid should always succeed on a normally
        // configured Linux/macOS/Windows machine.
        let s = read_machine_uid().expect("machine UID readable");
        assert!(!s.is_empty(), "machine UID should be non-empty");
    }

    #[test]
    fn same_inputs_yield_same_key() {
        let salt = [7u8; SALT_LEN];
        let k1 = derive_master_key(&salt, "/home/alice/data").unwrap();
        let k2 = derive_master_key(&salt, "/home/alice/data").unwrap();
        assert_eq!(k1, k2, "deterministic derivation");
    }

    #[test]
    fn different_data_dir_yields_different_key() {
        let salt = [7u8; SALT_LEN];
        let k1 = derive_master_key(&salt, "/home/alice/data").unwrap();
        let k2 = derive_master_key(&salt, "/home/bob/data").unwrap();
        assert_ne!(k1, k2, "different user paths must derive different keys");
    }

    #[test]
    fn different_salt_yields_different_key() {
        let s1 = [1u8; SALT_LEN];
        let s2 = [2u8; SALT_LEN];
        let k1 = derive_master_key(&s1, "/x").unwrap();
        let k2 = derive_master_key(&s2, "/x").unwrap();
        assert_ne!(k1, k2, "salt must affect the derived key");
    }
}
