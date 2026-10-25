use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use rook_core::EndpointPolicy;

/// DNS resolver that validates every answer immediately before reqwest opens
/// an upstream connection. IP literals are separately pinned with reqwest's
/// per-host override because hyper skips custom resolution for IP literals.
pub struct GuardedResolver {
    policy: EndpointPolicy,
    scheme: String,
    configured_host: String,
    port: u16,
}

impl GuardedResolver {
    pub fn new(
        policy: EndpointPolicy,
        scheme: impl Into<String>,
        configured_host: impl Into<String>,
        port: u16,
    ) -> Self {
        Self {
            policy,
            scheme: scheme.into(),
            configured_host: configured_host.into(),
            port,
        }
    }
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let policy = self.policy.clone();
        let scheme = self.scheme.clone();
        let configured_host = self.configured_host.clone();
        let port = self.port;
        let queried_host = name.as_str().to_string();
        Box::pin(async move {
            if !queried_host.eq_ignore_ascii_case(&configured_host) {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "upstream resolver queried an unexpected host",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let resolved = tokio::net::lookup_host((configured_host.as_str(), port)).await?;
            let mut safe = Vec::new();
            for address in resolved {
                policy
                    .validate_ip(&configured_host, address.ip(), port, &scheme)
                    .map_err(|error| {
                        std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!("upstream endpoint blocked by server policy: {error:?}"),
                        )
                    })?;
                safe.push(SocketAddr::new(address.ip(), port));
            }
            if safe.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "upstream host resolved to no addresses",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(safe.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

pub(crate) struct ResolvedEndpointPolicy {
    pub resolver: Arc<dyn reqwest::dns::Resolve>,
    pub literal: Option<(String, Vec<SocketAddr>)>,
}

pub(crate) fn resolver_for_url(
    policy: EndpointPolicy,
    base_url: &str,
) -> anyhow::Result<ResolvedEndpointPolicy> {
    let url = reqwest::Url::parse(base_url)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("upstream endpoint has no host"))?;
    policy.validate_url(&url).map_err(|error| {
        anyhow::anyhow!("upstream endpoint rejected by server policy: {error:?}")
    })?;
    let literal_ip = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .ok();
    let port = url.port_or_known_default().unwrap_or(443);
    let literal = literal_ip
        .map(|ip| {
            policy
                .validate_ip(host, ip, port, url.scheme())
                .map_err(|error| {
                    anyhow::anyhow!("upstream endpoint rejected by server policy: {error:?}")
                })?;
            Ok::<_, anyhow::Error>((host.to_string(), vec![SocketAddr::new(ip, port)]))
        })
        .transpose()?;

    Ok(ResolvedEndpointPolicy {
        resolver: Arc::new(GuardedResolver::new(
            policy,
            url.scheme(),
            host,
            url.port_or_known_default().unwrap_or(443),
        )),
        literal,
    })
}
