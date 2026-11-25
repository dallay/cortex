<!-- coderabbit-ai-generated:v1 -->

# Cortex patch for dbus-secret-service

Based on the published [4.1.0 crate](https://crates.io/crates/dbus-secret-service/4.1.0)
(upstream commit `d739c219802a213746dfcd6358c46723c3fc42e3`). The original MIT and Apache
licenses are included.

Both upstream crypto features share the same non-cryptographic RNG for the Diffie–Hellman
private key and AES-CBC initialization vector. The patch replaces both uses with
`getrandom::fill`, which obtains CSPRNG bytes from the operating system. If entropy is
unavailable, session negotiation or encryption returns a crypto error before sending a
public key or a secret. Huginn still uses the Rust crypto feature and encrypted sessions.

Remove the workspace patch and refresh the Cargo and Bazel lockfiles once a compatible
upstream release uses a CSPRNG for both values.
