//! On-disk secrets file: load, save, atomic replace.
//!
//! File format (JSON, base64-encoded bytes):
//! ```json
//! {
//!   "version": 1,
//!   "salt": "base64...",
//!   "entries": {
//!     "anthropic_api_key": { "nonce": "base64...", "ct": "base64..." },
//!     ...
//!   }
//! }
//! ```
//!
//! - `salt` is created on first save and never changes for the life of
//!   the file. Rotating the salt would orphan all stored secrets.
//! - Atomic save: write to `secrets.bin.tmp`, fsync, rename onto
//!   `secrets.bin`. POSIX rename is atomic, so a crash mid-save can't
//!   produce a half-written file.
//! - File mode: 0o600 on Unix (only the owner can read). Best-effort on
//!   Windows (NTFS permissions inherit from the parent dir, which is the
//!   user's AppData folder — already user-scoped).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use super::cipher;
use super::kdf::{self, KEY_LEN, SALT_LEN};

/// Filename under the platform data directory.
pub const SECRETS_FILENAME: &str = "secrets.bin";

/// Latest schema version. Bump when the on-disk layout changes.
pub const CURRENT_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OnDiskFile {
    pub version: u8,
    /// Base64-encoded 16-byte salt (created on first save).
    pub salt: String,
    pub entries: HashMap<String, OnDiskEntry>,
    /// Keys that have already been considered for one-shot migration
    /// from the v0.2.6 OS keyring. The presence of a key here means
    /// "we asked the keyring once for this key; don't ask again."
    /// `#[serde(default)]` keeps v0.3.7-and-earlier files (which
    /// lacked this field) deserializable without a schema bump.
    #[serde(default)]
    pub migrated_keyring_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OnDiskEntry {
    /// Base64-encoded 12-byte nonce.
    pub nonce: String,
    /// Base64-encoded ChaCha20-Poly1305 ciphertext (includes 16-byte tag).
    pub ct: String,
}

/// In-memory representation of the secrets file with the master key
/// already derived. Operations modify this and `save_atomic` writes it.
pub(super) struct SecretsFile {
    path: PathBuf,
    master_key: [u8; KEY_LEN],
    salt: [u8; SALT_LEN],
    entries: HashMap<String, OnDiskEntry>,
    migrated_keyring_keys: Vec<String>,
}

impl SecretsFile {
    /// Open the secrets file at `path`. If it doesn't exist, create an
    /// empty in-memory file with a fresh random salt (saved on first
    /// `save_atomic`).
    ///
    /// `data_dir` is included in the master-key derivation so two users
    /// on the same machine get different keys.
    pub fn open(path: PathBuf, data_dir: &str) -> Result<Self> {
        let (salt, entries, migrated_keyring_keys) = if path.exists() {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let file: OnDiskFile = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", path.display()))?;
            if file.version != CURRENT_VERSION {
                anyhow::bail!(
                    "{}: unsupported secrets file version {} (expected {})",
                    path.display(),
                    file.version,
                    CURRENT_VERSION
                );
            }
            let salt_bytes = B64
                .decode(&file.salt)
                .with_context(|| format!("decoding salt in {}", path.display()))?;
            if salt_bytes.len() != SALT_LEN {
                anyhow::bail!(
                    "{}: salt has wrong length {} (expected {SALT_LEN})",
                    path.display(),
                    salt_bytes.len()
                );
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&salt_bytes);
            (salt, file.entries, file.migrated_keyring_keys)
        } else {
            // New file — generate a fresh salt. It only persists once we
            // actually write something via `save_atomic`.
            let mut salt = [0u8; SALT_LEN];
            rand::thread_rng().fill_bytes(&mut salt);
            (salt, HashMap::new(), Vec::new())
        };

        let master_key = kdf::derive_master_key(&salt, data_dir)?;
        Ok(Self {
            path,
            master_key,
            salt,
            entries,
            migrated_keyring_keys,
        })
    }

    /// Snapshot of the keys this file has already considered for the
    /// one-shot v0.2.6 keyring migration.
    pub fn migrated_keyring_keys(&self) -> Vec<String> {
        self.migrated_keyring_keys.clone()
    }

    /// Record that `key` has been considered for keyring migration so
    /// future opens skip the (slow, dbus-blocking) keyring probe.
    /// Persists to disk.
    pub fn mark_migrated(&mut self, key: &str) -> Result<()> {
        if self.migrated_keyring_keys.iter().any(|k| k == key) {
            return Ok(());
        }
        self.migrated_keyring_keys.push(key.to_string());
        self.save_atomic()
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let Some(entry) = self.entries.get(key) else {
            return Ok(None);
        };
        let nonce = B64
            .decode(&entry.nonce)
            .with_context(|| format!("decoding nonce for {key}"))?;
        let ct = B64
            .decode(&entry.ct)
            .with_context(|| format!("decoding ct for {key}"))?;
        let pt = cipher::decrypt(&self.master_key, &nonce, &ct)
            .with_context(|| format!("decrypting {key} (machine identity changed?)"))?;
        let s = String::from_utf8(pt).with_context(|| format!("non-utf8 plaintext for {key}"))?;
        Ok(Some(s))
    }

    pub fn put(&mut self, key: &str, value: &str) -> Result<()> {
        let (nonce, ct) = cipher::encrypt(&self.master_key, value.as_bytes())?;
        self.entries.insert(
            key.to_string(),
            OnDiskEntry {
                nonce: B64.encode(nonce),
                ct: B64.encode(ct),
            },
        );
        self.save_atomic()
    }

    pub fn remove(&mut self, key: &str) -> Result<bool> {
        let removed = self.entries.remove(key).is_some();
        if removed {
            self.save_atomic()?;
        }
        Ok(removed)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    fn save_atomic(&self) -> Result<()> {
        let on_disk = OnDiskFile {
            version: CURRENT_VERSION,
            salt: B64.encode(self.salt),
            entries: self.entries.clone(),
            migrated_keyring_keys: self.migrated_keyring_keys.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&on_disk).context("serializing secrets file")?;

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }

        let tmp = self.path.with_extension("bin.tmp");
        {
            let mut f =
                fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
            f.write_all(&bytes)
                .with_context(|| format!("writing {}", tmp.display()))?;
            f.sync_all().ok();
        }
        set_owner_only(&tmp)?;
        fs::rename(&tmp, &self.path)
            .with_context(|| format!("renaming {} → {}", tmp.display(), self.path.display()))?;
        // Re-apply mode on the final path defensively, in case the rename
        // didn't preserve it on some filesystems.
        set_owner_only(&self.path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms).with_context(|| format!("chmod 600 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<()> {
    // On Windows, files inherit ACLs from the user's AppData directory,
    // which is already scoped to the user. Anything stricter requires
    // SetSecurityInfo / ACL manipulation we don't need here.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_file(tmp: &TempDir) -> SecretsFile {
        let path = tmp.path().join(SECRETS_FILENAME);
        SecretsFile::open(path, tmp.path().to_str().unwrap()).unwrap()
    }

    #[test]
    fn put_get_round_trips() {
        let tmp = TempDir::new().unwrap();
        let mut f = fresh_file(&tmp);
        f.put("anthropic_api_key", "sk-ant-api03-XXXX").unwrap();
        assert_eq!(
            f.get("anthropic_api_key").unwrap().as_deref(),
            Some("sk-ant-api03-XXXX")
        );
    }

    #[test]
    fn data_persists_across_reopens() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        {
            let mut f = SecretsFile::open(path.clone(), dd).unwrap();
            f.put("token", "abc123").unwrap();
        }
        let f2 = SecretsFile::open(path, dd).unwrap();
        assert_eq!(f2.get("token").unwrap().as_deref(), Some("abc123"));
    }

    #[test]
    fn salt_persists_across_reopens() {
        // The whole point of persisting the salt is that the master key
        // derives the same way on a second open. If the salt got rotated
        // we'd orphan the stored secrets.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        let mut f1 = SecretsFile::open(path.clone(), dd).unwrap();
        f1.put("k", "v").unwrap();
        let salt1 = f1.salt;
        let f2 = SecretsFile::open(path, dd).unwrap();
        assert_eq!(f2.salt, salt1);
        assert_eq!(f2.get("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn remove_drops_entry_and_persists() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        let mut f = SecretsFile::open(path.clone(), dd).unwrap();
        f.put("k", "v").unwrap();
        assert!(f.remove("k").unwrap());
        assert!(!f.contains("k"));
        let f2 = SecretsFile::open(path, dd).unwrap();
        assert!(!f2.contains("k"));
    }

    #[test]
    fn remove_missing_returns_false() {
        let tmp = TempDir::new().unwrap();
        let mut f = fresh_file(&tmp);
        assert!(!f.remove("nope").unwrap());
    }

    #[test]
    fn ciphertext_is_actually_encrypted_on_disk() {
        // Sanity: the secret string must NOT appear in the file as
        // plaintext. (Belt-and-suspenders check; if this fails we've
        // managed to write the secret in cleartext.)
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        let mut f = SecretsFile::open(path.clone(), dd).unwrap();
        f.put("k", "PLAINTEXT-MARKER-123").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("PLAINTEXT-MARKER-123"),
            "secret leaked into on-disk file: {raw}"
        );
    }

    #[test]
    fn tampered_file_returns_clear_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        let mut f = SecretsFile::open(path.clone(), dd).unwrap();
        f.put("k", "v").unwrap();

        // Corrupt the file by deterministically replacing the ct
        // base64 value with one of equal length but a single bit
        // different. Earlier this used a "find the ct field, flip a
        // byte" heuristic that silently no-op'd ~1 run in 30 because
        // the random byte at the chosen offset happened to be
        // non-alphanumeric. Now we parse the JSON, mutate, re-write —
        // which always actually tampers.
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let entries = json
            .get_mut("entries")
            .and_then(|e| e.as_object_mut())
            .expect("entries object present");
        let entry = entries
            .values_mut()
            .next()
            .expect("at least one entry")
            .as_object_mut()
            .unwrap();
        let ct = entry
            .get_mut("ct")
            .and_then(|v| v.as_str())
            .expect("ct field present")
            .to_string();
        // Flip the first character to a different valid base64 char.
        // 'A' ↔ 'B' covers both cases without having to handle padding.
        let mut chars: Vec<char> = ct.chars().collect();
        chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        let tampered_ct: String = chars.into_iter().collect();
        entry.insert("ct".into(), serde_json::Value::String(tampered_ct));

        std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        let f2 = SecretsFile::open(path, dd).unwrap();
        assert!(f2.get("k").is_err(), "tampered ct must produce an error");
    }

    #[test]
    fn migrated_keyring_keys_persist_across_reopens() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        {
            let mut f = SecretsFile::open(path.clone(), dd).unwrap();
            f.mark_migrated("anthropic_api_key").unwrap();
            f.mark_migrated("telegram_bot_token").unwrap();
        }
        let f2 = SecretsFile::open(path, dd).unwrap();
        let migrated = f2.migrated_keyring_keys();
        assert!(migrated.iter().any(|k| k == "anthropic_api_key"));
        assert!(migrated.iter().any(|k| k == "telegram_bot_token"));
    }

    #[test]
    fn mark_migrated_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        let mut f = SecretsFile::open(path, dd).unwrap();
        f.mark_migrated("x").unwrap();
        f.mark_migrated("x").unwrap();
        f.mark_migrated("x").unwrap();
        let migrated = f.migrated_keyring_keys();
        assert_eq!(migrated.iter().filter(|k| *k == "x").count(), 1);
    }

    #[test]
    fn old_v1_files_without_migrated_field_open_cleanly() {
        // Forward-compat: v0.3.7-and-earlier files lack the
        // `migrated_keyring_keys` field. Ensure we deserialize them as
        // an empty Vec via `#[serde(default)]`.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        // Write a v0.3.7-shaped JSON manually (no migrated_keyring_keys).
        let raw = r#"{"version":1,"salt":"AAAAAAAAAAAAAAAAAAAAAA==","entries":{}}"#;
        std::fs::write(&path, raw).unwrap();
        let f = SecretsFile::open(path, dd).unwrap();
        assert!(f.migrated_keyring_keys().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn file_is_chmod_600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(SECRETS_FILENAME);
        let dd = tmp.path().to_str().unwrap();
        let mut f = SecretsFile::open(path.clone(), dd).unwrap();
        f.put("k", "v").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "owner-only mode required");
    }
}
