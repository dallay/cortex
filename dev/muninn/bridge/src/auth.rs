//! Identidad del harness, nunca derivada del cuerpo de una petición.

use subtle::ConstantTimeEq;

pub fn authorized(header: &[u8], token: &str) -> bool {
    (32..=128).contains(&token.len())
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && bool::from(header.ct_eq(format!("Bearer {token}").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::authorized;

    #[test]
    fn requires_the_exact_host_bearer_without_accepting_claims() {
        let token = "a".repeat(48);
        assert!(authorized(format!("Bearer {token}").as_bytes(), &token));
        assert!(!authorized(b"", &token));
        assert!(!authorized(b"Bearer other", &token));
        assert!(!authorized(b"Bearer ", ""));
        assert!(!authorized(b"Bearer weak", "weak"));
    }
}
