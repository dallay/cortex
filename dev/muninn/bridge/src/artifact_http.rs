//! Bounded artifact acquisition, not a preparation service or model execution.

use super::{ArtifactRoute, RouteError};
use crate::model_manifest::ManifestError;
use std::time::Duration;
#[path = "artifact_http_tls.rs"]
mod tls;

#[derive(Clone, Copy)]
struct Limits {
    total: Duration,
    idle: Duration,
    connect: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(120),
            idle: Duration::from_secs(15),
            connect: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TransferError {
    Configuration,
    Route(RouteError),
    Manifest(ManifestError),
    Network,
    Timeout,
    Status(u16),
    Headers,
    Size,
}

/// No Debug/Display: HTTP diagnostics must not expose signed request URLs.
pub struct ArtifactHttp {
    client: reqwest::Client,
    limits: Limits,
}

impl ArtifactHttp {
    pub fn new() -> Result<Self, TransferError> {
        let certificates =
            rustls_native_certs::load_native_certs().map_err(|_| TransferError::Configuration)?;
        let mut roots = rustls::RootCertStore::empty();
        roots.add_parsable_certificates(certificates);
        if roots.is_empty() {
            return Err(TransferError::Configuration);
        }
        let limits = Limits::default();
        let client = tls::builder(roots, limits)?
            .build()
            .map_err(|_| TransferError::Configuration)?;
        Ok(Self { client, limits })
    }

    pub async fn fetch(
        &self,
        manifest: &crate::model_manifest::PinnedMiniLmManifest,
        path: &str,
    ) -> Result<crate::artifacts::VerifiedArtifact, TransferError> {
        let deadline = tokio::time::Instant::now() + self.limits.total;
        tokio::time::timeout_at(deadline, self.fetch_inner(manifest, path, deadline))
            .await
            .map_err(|_| TransferError::Timeout)?
    }

    async fn fetch_inner(
        &self,
        manifest: &crate::model_manifest::PinnedMiniLmManifest,
        path: &str,
        deadline: tokio::time::Instant,
    ) -> Result<crate::artifacts::VerifiedArtifact, TransferError> {
        let mut route = ArtifactRoute::new(manifest, path).map_err(TransferError::Route)?;
        let artifact = manifest
            .files()
            .iter()
            .find(|artifact| artifact.path() == path)
            .ok_or(TransferError::Route(RouteError::UnknownArtifact))?;
        let mut response = loop {
            let response = self
                .client
                .get(route.current.clone())
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(request_error)?;
            validate_header_budget(response.headers())?;
            match response.status().as_u16() {
                301 | 302 | 303 | 307 | 308 => {
                    let location = single_header(response.headers(), reqwest::header::LOCATION)?
                        .ok_or(TransferError::Headers)?;
                    route.follow(location).map_err(TransferError::Route)?;
                }
                _ => break response,
            }
        };
        if response.status() != reqwest::StatusCode::OK {
            return Err(TransferError::Status(response.status().as_u16()));
        }
        validate_terminal(response.headers(), artifact.size())?;
        let mut bytes = Vec::new();
        let maximum = usize::try_from(artifact.size()).map_err(|_| TransferError::Size)?;
        bytes
            .try_reserve_exact(maximum)
            .map_err(|_| TransferError::Size)?;
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if chunk.len() > maximum.saturating_sub(bytes.len()) {
                return Err(TransferError::Size);
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != maximum {
            return Err(TransferError::Size);
        }
        let checked = manifest
            .verify_artifact(path, std::io::Cursor::new(bytes))
            .map_err(TransferError::Manifest)?;
        if tokio::time::Instant::now() >= deadline {
            return Err(TransferError::Timeout);
        }
        Ok(checked)
    }
}

fn request_error(error: reqwest::Error) -> TransferError {
    if error.is_timeout() {
        TransferError::Timeout
    } else {
        TransferError::Network
    }
}

fn single_header(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Result<Option<&str>, TransferError> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|value| value.to_str())
        .transpose()
        .map_err(|_| TransferError::Headers)?;
    if values.next().is_some() {
        return Err(TransferError::Headers);
    }
    Ok(value)
}

fn validate_terminal(
    headers: &reqwest::header::HeaderMap,
    expected: u64,
) -> Result<(), TransferError> {
    if single_header(headers, reqwest::header::CONTENT_ENCODING)?
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(TransferError::Headers);
    }
    if headers.contains_key(reqwest::header::CONTENT_RANGE) {
        return Err(TransferError::Headers);
    }
    let length = single_header(headers, reqwest::header::CONTENT_LENGTH)?;
    let transfer = single_header(headers, reqwest::header::TRANSFER_ENCODING)?;
    if transfer.is_some_and(|value| !value.eq_ignore_ascii_case("chunked"))
        || (transfer.is_some() && length.is_some())
    {
        return Err(TransferError::Headers);
    }
    if let Some(length) = length {
        if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(TransferError::Headers);
        }
        if length.parse::<u64>().map_err(|_| TransferError::Headers)? != expected {
            return Err(TransferError::Size);
        }
    }
    Ok(())
}

fn validate_header_budget(headers: &reqwest::header::HeaderMap) -> Result<(), TransferError> {
    if headers.len() > 64
        || headers
            .iter()
            .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
            .sum::<usize>()
            > 32 * 1024
    {
        return Err(TransferError::Headers);
    }
    Ok(())
}

#[cfg(test)]
#[path = "artifact_http_tests.rs"]
mod tests;
