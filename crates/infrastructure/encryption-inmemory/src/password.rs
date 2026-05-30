use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordVerifier},
    Argon2, PasswordHasher as _,
};
use rook_core::model::PasswordHash as CorePasswordHash;
use rook_core::ports::{PasswordHashError, PasswordHasher};

/// Argon2id password hasher using the argon2 crate's password_hash API.
///
/// Uses OWASP-recommended default params (64 MiB, 3 iterations, 1 parallelism).
/// This is SEPARATE from the AesGcmKeyManager's key derivation which uses
/// different params (65_536 memory, 3 iterations, 4 parallelism) for
/// encryption key derivation.
#[derive(Clone)]
pub struct Argon2idHasher {
    argon2: Argon2<'static>,
}

impl Argon2idHasher {
    pub fn new() -> Self {
        Self {
            argon2: Argon2::default(),
        }
    }
}

impl Default for Argon2idHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl PasswordHasher for Argon2idHasher {
    fn hash_password(&self, password: &str) -> Result<CorePasswordHash, PasswordHashError> {
        // Generate a random salt using OS RNG (16 bytes is the default)
        let salt = argon2::password_hash::SaltString::generate(&mut OsRng);

        // Hash the password with Argon2id using default (OWASP) params
        // Argon2::default() uses: m=65536 (64MB), t=3, p=1
        let hash = self
            .argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| PasswordHashError::HashGeneration)?;

        Ok(CorePasswordHash::from(hash.to_string()))
    }

    fn verify_password(
        &self,
        password: &str,
        hash: &CorePasswordHash,
    ) -> Result<bool, PasswordHashError> {
        // Parse the hash string - if parsing fails, return Ok(false) not an error
        // (malformed hashes simply don't match any password)
        let parsed_hash = match PasswordHash::new(hash.as_str()) {
            Ok(h) => h,
            Err(_) => return Ok(false),
        };

        // Verify the password against the parsed hash
        match self.argon2.verify_password(password.as_bytes(), &parsed_hash) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hasher() -> Argon2idHasher {
        Argon2idHasher::new()
    }

    #[test]
    fn hash_produces_argon2id_format() {
        let h = hasher();
        let hash = h.hash_password("password123").expect("hash should succeed");
        assert!(
            hash.as_str().starts_with("$argon2id$"),
            "hash should start with $argon2id$, got: {}",
            hash.as_str()
        );
    }

    #[test]
    fn hash_does_not_contain_plaintext() {
        let h = hasher();
        let hash = h
            .hash_password("super_secret_password_123")
            .expect("hash should succeed");
        let hash_str = hash.as_str();
        assert!(
            !hash_str.contains("super_secret"),
            "hash should not contain plaintext password"
        );
        assert!(
            !hash_str.contains("password"),
            "hash should not contain password substring"
        );
    }

    #[test]
    fn verify_succeeds_for_correct_password() {
        let h = hasher();
        let hash = h
            .hash_password("correct_password")
            .expect("hash should succeed");
        let result = h
            .verify_password("correct_password", &hash)
            .expect("verify should return Ok");
        assert!(result, "verify should return true for correct password");
    }

    #[test]
    fn verify_fails_for_wrong_password() {
        let h = hasher();
        let hash = h
            .hash_password("correct_password")
            .expect("hash should succeed");
        let result = h
            .verify_password("wrong_password", &hash)
            .expect("verify should return Ok");
        assert!(!result, "verify should return false for wrong password");
    }

    #[test]
    fn different_salts_produce_different_hashes() {
        let h = hasher();
        let hash1 = h
            .hash_password("same_password")
            .expect("hash should succeed");
        let hash2 = h
            .hash_password("same_password")
            .expect("hash should succeed");
        assert_ne!(
            hash1.as_str(),
            hash2.as_str(),
            "same password with different salts should produce different hashes"
        );
    }

    #[test]
    fn verify_fails_for_malformed_hash() {
        let h = hasher();
        let malformed = CorePasswordHash::from("not_a_valid_argon2_hash".to_string());
        let result = h
            .verify_password("any_password", &malformed)
            .expect("verify should return Ok even for malformed hash");
        assert!(
            !result,
            "verify should return false for malformed hash, not panic"
        );
    }

    #[test]
    fn verify_fails_for_empty_password() {
        let h = hasher();
        let hash = h.hash_password("real_password").expect("hash should succeed");
        let result = h
            .verify_password("", &hash)
            .expect("verify should return Ok");
        assert!(!result, "verify should return false for wrong (empty) password");
    }
}