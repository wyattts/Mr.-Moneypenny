//! Authenticated encryption: ChaCha20-Poly1305.
//!
//! Each stored secret has its own random 12-byte nonce. The Poly1305 tag
//! detects tampering — a flipped bit anywhere in the ciphertext or
//! associated data yields a decrypt failure rather than silently garbled
//! plaintext.

use anyhow::{anyhow, Result};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;

pub const NONCE_LEN: usize = 12;

/// Encrypt `plaintext` under `key`. Returns `(nonce, ciphertext_with_tag)`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow!("chacha20poly1305 encrypt: {e}"))?;
    Ok((nonce_bytes.to_vec(), ct))
}

/// Decrypt `ciphertext` (which includes the 16-byte Poly1305 tag).
/// Returns an error if the tag doesn't verify (wrong key, tampered ct,
/// truncated ct).
pub fn decrypt(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if nonce.len() != NONCE_LEN {
        return Err(anyhow!(
            "nonce length is {}, expected {NONCE_LEN}",
            nonce.len()
        ));
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| anyhow!("chacha20poly1305 decrypt: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [42u8; 32];
        let plaintext = b"sk-ant-api03-XXXX";
        let (nonce, ct) = encrypt(&key, plaintext).unwrap();
        assert_eq!(nonce.len(), NONCE_LEN);
        let pt = decrypt(&key, &nonce, &ct).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn tampered_ciphertext_fails_decrypt() {
        let key = [42u8; 32];
        let (nonce, mut ct) = encrypt(&key, b"important").unwrap();
        ct[0] ^= 0x01; // flip a bit
        assert!(decrypt(&key, &nonce, &ct).is_err());
    }

    #[test]
    fn wrong_key_fails_decrypt() {
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        let (nonce, ct) = encrypt(&k1, b"important").unwrap();
        assert!(decrypt(&k2, &nonce, &ct).is_err());
    }

    #[test]
    fn nonces_are_unique_across_calls() {
        // Probabilistic: two consecutive 96-bit random nonces colliding
        // is ~2^-96. If this test ever flakes you have bigger problems.
        let key = [0u8; 32];
        let (n1, _) = encrypt(&key, b"a").unwrap();
        let (n2, _) = encrypt(&key, b"a").unwrap();
        assert_ne!(n1, n2);
    }
}
