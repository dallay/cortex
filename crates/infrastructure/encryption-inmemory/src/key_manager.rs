use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rook_core::ports::{CredentialEncryptionError, KeyManager};

use super::{EncryptionError, Result};

const VERSION_PREFIX: &str = "enc:v1:";
const NONCE_BYTES: usize = 12;

#[derive(Clone)]
pub struct AesGcmKeyManager {
    key: [u8; 32],
}

impl AesGcmKeyManager {
    pub fn from_passphrase_and_salt(passphrase: &str, salt_base64url_no_pad: &str) -> Result<Self> {
        let salt_bytes = URL_SAFE_NO_PAD
            .decode(salt_base64url_no_pad)
            .map_err(|_| EncryptionError::KeyDerivation)?;

        if salt_bytes.len() != 16 {
            return Err(EncryptionError::KeyDerivation);
        }

        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(65_536, 3, 4, Some(32))
                .map_err(|_| EncryptionError::KeyDerivation)?,
        );
        let salt =
            SaltString::encode_b64(&salt_bytes).map_err(|_| EncryptionError::KeyDerivation)?;
        let hash = argon2
            .hash_password(passphrase.as_bytes(), &salt)
            .map_err(|_| EncryptionError::KeyDerivation)?;
        let hash_output = hash.hash.ok_or(EncryptionError::KeyDerivation)?;

        let key: [u8; 32] = hash_output
            .as_bytes()
            .try_into()
            .map_err(|_| EncryptionError::KeyDerivation)?;
        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        if plaintext.is_empty() {
            return Err(EncryptionError::EmptyPlaintext);
        }

        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| EncryptionError::CipherError)?;
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| EncryptionError::CipherError)?;

        Ok(format!(
            "{}{}:{}",
            VERSION_PREFIX,
            URL_SAFE_NO_PAD.encode(nonce_bytes),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ))
    }

    pub fn decrypt(&self, ciphertext: &str) -> Result<String> {
        let rest = ciphertext
            .strip_prefix(VERSION_PREFIX)
            .ok_or(EncryptionError::DecryptFormat)?;
        let (nonce_b64, data_b64) = rest.split_once(':').ok_or(EncryptionError::DecryptFormat)?;

        let nonce_bytes = URL_SAFE_NO_PAD
            .decode(nonce_b64)
            .map_err(|_| EncryptionError::DecryptFormat)?;
        if nonce_bytes.len() != NONCE_BYTES {
            return Err(EncryptionError::DecryptFormat);
        }

        let ciphertext_with_tag = URL_SAFE_NO_PAD
            .decode(data_b64)
            .map_err(|_| EncryptionError::DecryptFormat)?;
        if ciphertext_with_tag.len() < 16 {
            return Err(EncryptionError::DecryptLength);
        }

        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| EncryptionError::CipherError)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext_with_tag.as_ref())
            .map_err(|_| EncryptionError::DecryptKey)?;

        String::from_utf8(plaintext).map_err(|_| EncryptionError::DecryptFormat)
    }
}

impl KeyManager for AesGcmKeyManager {
    fn encrypt(&self, plaintext: &str) -> std::result::Result<String, CredentialEncryptionError> {
        self.encrypt(plaintext)
            .map_err(|e| CredentialEncryptionError::Encrypt(e.to_string()))
    }

    fn decrypt(&self, ciphertext: &str) -> std::result::Result<String, CredentialEncryptionError> {
        self.decrypt(ciphertext)
            .map_err(|e| CredentialEncryptionError::Decrypt(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> AesGcmKeyManager {
        let salt = URL_SAFE_NO_PAD.encode([7_u8; 16]);
        AesGcmKeyManager::from_passphrase_and_salt("passphrase", &salt).expect("manager")
    }

    #[test]
    fn round_trip() {
        let manager = manager();
        let encrypted = manager.encrypt("secret").expect("encrypt");
        assert!(encrypted.starts_with("enc:v1:"));
        assert_eq!(manager.decrypt(&encrypted).expect("decrypt"), "secret");
    }

    #[test]
    fn same_plaintext_uses_different_nonces() {
        let manager = manager();
        assert_ne!(
            manager.encrypt("secret").expect("first"),
            manager.encrypt("secret").expect("second")
        );
    }

    #[test]
    fn malformed_values_are_rejected() {
        let manager = manager();
        assert!(manager.decrypt("enc:v2:abc:def").is_err());
        assert!(manager.decrypt("enc:v1:not-base64:def").is_err());
        assert!(manager.decrypt("enc:v1:abc").is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let encrypted = manager().encrypt("secret").expect("encrypt");
        let salt = URL_SAFE_NO_PAD.encode([8_u8; 16]);
        let other =
            AesGcmKeyManager::from_passphrase_and_salt("other-passphrase", &salt).expect("other");
        assert!(other.decrypt(&encrypted).is_err());
    }

    #[test]
    fn salt_must_decode_to_sixteen_bytes() {
        assert!(AesGcmKeyManager::from_passphrase_and_salt("passphrase", "bad").is_err());
        let short = URL_SAFE_NO_PAD.encode([0_u8; 8]);
        assert!(AesGcmKeyManager::from_passphrase_and_salt("passphrase", &short).is_err());
    }

    #[test]
    fn empty_plaintext_is_rejected() {
        assert!(manager().encrypt("").is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_nonce_length() {
        let mgr = manager();
        // Nonce must be exactly 12 bytes; use 8 bytes instead
        let bad_nonce = URL_SAFE_NO_PAD.encode([0_u8; 8]);
        let data = URL_SAFE_NO_PAD.encode([0_u8; 32]);
        let malformed = format!("enc:v1:{}:{}", bad_nonce, data);
        assert!(mgr.decrypt(&malformed).is_err());
    }

    #[test]
    fn decrypt_rejects_short_ciphertext() {
        let mgr = manager();
        // Get a valid nonce from a real encryption, then replace data with <16 bytes
        let valid = mgr.encrypt("secret").expect("encrypt");
        let parts: Vec<&str> = valid.split(':').collect();
        assert_eq!(parts.len(), 4);
        // 16 bytes is the minimum for AES-GCM auth tag; use 8 bytes instead
        let short_data = URL_SAFE_NO_PAD.encode([0_u8; 8]);
        let malformed = format!("{}:{}:{}:{}", parts[0], parts[1], parts[2], short_data);
        assert!(mgr.decrypt(&malformed).is_err());
    }
}
