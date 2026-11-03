//! Probes for encrypted retention and nonce exhaustion.
use super::{Clock, ErrorDeduplicator};
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use std::sync::Arc;

pub fn with_key(clock: Arc<dyn Clock>, key: [u8; 32]) -> ErrorDeduplicator {
    let mut dedup = ErrorDeduplicator::new(clock);
    dedup.cipher = Aes256Gcm::new_from_slice(&key).expect("256-bit key");
    dedup
}

pub fn last_fingerprint(dedup: &ErrorDeduplicator) -> &[u8] {
    &dedup.entries.back().expect("cached message").fingerprint
}

pub fn decrypts_to(dedup: &ErrorDeduplicator, fingerprint: &[u8], plaintext: &[u8]) -> bool {
    if fingerprint.len() < 12 {
        return false;
    }
    let Ok(nonce) = Nonce::try_from(&fingerprint[..12]) else {
        return false;
    };
    dedup
        .cipher
        .decrypt(&nonce, &fingerprint[12..])
        .is_ok_and(|decoded| decoded == plaintext)
}

pub const fn exhaust_nonce_after_next(dedup: &mut ErrorDeduplicator) {
    dedup.nonce_counter = u64::MAX - 1;
}

pub fn retained_debug(dedup: &ErrorDeduplicator) -> String {
    format!("{:?}", dedup.entries)
}
