//! Buffers de artefactos: no cargar rutas ni modelos desde este módulo.

use sha2::{Digest, Sha256};
use std::io::Read;

/// Límite del incremento MiniLM; no habilita pesos mayores de otros modelos.
pub const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

pub(crate) fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn validate_spec(size: u64, sha256: &str) -> Result<(), ArtifactError> {
    if size == 0 || size > MAX_ARTIFACT_BYTES || !is_lower_hex(sha256, 64) {
        return Err(ArtifactError::InvalidSpecification);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArtifactError {
    InvalidSpecification,
    ReadFailed,
    SizeMismatch,
    DigestMismatch,
}

/// Conserva los bytes comprobados sin exponer mutación ni Debug del contenido.
pub struct VerifiedArtifact {
    bytes: Vec<u8>,
}

impl VerifiedArtifact {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Verifica tamaño y SHA-256 de un stream ya abierto y devuelve sus mismos bytes.
///
/// Los valores esperados deben proceder del manifiesto confiable. Esta función
/// no autentica ese manifiesto, no abre rutas y no implementa deadlines: el
/// adaptador de I/O debe limitar tiempo y accesos. Aquí se acotan los bytes.
pub fn load_verified<R: Read>(
    reader: R,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<VerifiedArtifact, ArtifactError> {
    validate_spec(expected_size, expected_sha256)?;
    let mut bytes = Vec::new();
    reader
        .take(expected_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ArtifactError::ReadFailed)?;
    if u64::try_from(bytes.len()).ok() != Some(expected_size) {
        return Err(ArtifactError::SizeMismatch);
    }
    if format!("{:x}", Sha256::digest(&bytes)) != expected_sha256 {
        return Err(ArtifactError::DigestMismatch);
    }
    Ok(VerifiedArtifact { bytes })
}

#[cfg(test)]
mod tests {
    use super::{load_verified, ArtifactError};
    use std::io::Cursor;

    const ABC_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn rejects_modified_bytes_even_when_the_size_matches() {
        let result = load_verified(Cursor::new(b"abd"), 3, ABC_HASH);
        assert!(matches!(result, Err(ArtifactError::DigestMismatch)));
    }

    #[test]
    fn rejects_incorrect_declared_size_even_with_a_matching_hash() {
        let result = load_verified(Cursor::new(b"abc"), 4, ABC_HASH);
        assert!(matches!(result, Err(ArtifactError::SizeMismatch)));
    }

    #[test]
    fn reads_at_most_expected_size_plus_one_byte() {
        let mut reader = Cursor::new(vec![b'x'; 4096]);
        let result = load_verified(&mut reader, 3, ABC_HASH);
        assert!(matches!(result, Err(ArtifactError::SizeMismatch)));
        assert_eq!(reader.position(), 4);
    }

    struct Unreadable;

    impl std::io::Read for Unreadable {
        fn read(&mut self, _output: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic unreadable source"))
        }
    }

    #[test]
    fn rejects_invalid_specification_before_io() {
        for size in [0, super::MAX_ARTIFACT_BYTES + 1, u64::MAX] {
            assert!(matches!(
                load_verified(Unreadable, size, ABC_HASH),
                Err(ArtifactError::InvalidSpecification)
            ));
        }
        for digest in ["".to_owned(), "z".repeat(64), ABC_HASH.to_uppercase()] {
            assert!(matches!(
                load_verified(Unreadable, 3, &digest),
                Err(ArtifactError::InvalidSpecification)
            ));
        }
    }

    #[test]
    fn returns_the_verified_buffer_without_reloading_or_copying_it() {
        let mut reader = Cursor::new(b"abc".to_vec());
        let verified = load_verified(&mut reader, 3, ABC_HASH).expect("valid fixture");
        reader.get_mut().copy_from_slice(b"xyz");
        assert_eq!(verified.as_bytes(), b"abc");
        let pointer = verified.as_bytes().as_ptr();
        let owned = verified.into_bytes();
        assert_eq!(owned.as_ptr(), pointer);
        assert_eq!(owned, b"abc");
    }

    #[test]
    fn reports_io_failure_without_returning_unverified_bytes() {
        assert!(matches!(
            load_verified(Unreadable, 3, ABC_HASH),
            Err(ArtifactError::ReadFailed)
        ));
    }
}
