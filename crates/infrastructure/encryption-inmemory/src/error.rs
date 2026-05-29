#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("plaintext must not be empty")]
    EmptyPlaintext,
    #[error("cipher operation failed")]
    CipherError,
    #[error("encrypted value has invalid format")]
    DecryptFormat,
    #[error("encrypted value is too short")]
    DecryptLength,
    #[error("encrypted value could not be decrypted")]
    DecryptKey,
}
