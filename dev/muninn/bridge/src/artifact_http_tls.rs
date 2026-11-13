//! Matching Rustls/reqwest types and explicit per-client crypto; no global install.

use super::{Limits, TransferError};

pub(super) fn builder(
    roots: rustls::RootCertStore,
    limits: Limits,
) -> Result<reqwest::ClientBuilder, TransferError> {
    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .map_err(|_| TransferError::Configuration)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .connection_verbose(false)
        .referer(false)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .http1_only()
        .pool_max_idle_per_host(0)
        .connect_timeout(limits.connect)
        .read_timeout(limits.idle)
        .timeout(limits.total))
}
