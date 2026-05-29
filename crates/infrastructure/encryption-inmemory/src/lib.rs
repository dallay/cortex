pub use error::EncryptionError;
pub use key_manager::AesGcmKeyManager;
pub use rook_core::ports::KeyManager;

pub type Result<T> = std::result::Result<T, EncryptionError>;

mod error;
mod key_manager;
