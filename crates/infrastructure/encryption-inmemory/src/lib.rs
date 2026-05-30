pub use error::EncryptionError;
pub use key_manager::AesGcmKeyManager;
pub use password::Argon2idHasher;
pub use rook_core::ports::{KeyManager, PasswordHashError, PasswordHasher};

pub type Result<T> = std::result::Result<T, EncryptionError>;

mod error;
mod key_manager;
mod password;
