use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Effective, server-derived trust settings for one compatible connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointPolicy {
    /// Exact origins (`scheme://host:port`) granted to this connection by the operator.
    pub allowed_origins: Vec<String>,
}

/// Operator-owned outbound grants. No client CRUD field can modify this policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct EndpointPolicyConfig {
    pub grants: Vec<EndpointGrant>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct EndpointGrant {
    pub connection_id: String,
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointPolicyConfigError {
    pub grant_index: usize,
    pub reason: EndpointPolicyConfigErrorReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointPolicyConfigErrorReason {
    InvalidConnectionId,
    DuplicateConnectionId,
    EmptyOrigins,
    InvalidOrigin,
}

impl std::fmt::Display for EndpointPolicyConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "grant {}: {:?}", self.grant_index, self.reason)
    }
}

impl std::error::Error for EndpointPolicyConfigError {}

impl EndpointPolicyConfig {
    pub fn validate(&self) -> Result<(), EndpointPolicyConfigError> {
        let mut ids = std::collections::HashSet::new();
        for (index, grant) in self.grants.iter().enumerate() {
            let id = uuid::Uuid::parse_str(&grant.connection_id).map_err(|_| {
                EndpointPolicyConfigError {
                    grant_index: index,
                    reason: EndpointPolicyConfigErrorReason::InvalidConnectionId,
                }
            })?;
            if !ids.insert(id) {
                return Err(EndpointPolicyConfigError {
                    grant_index: index,
                    reason: EndpointPolicyConfigErrorReason::DuplicateConnectionId,
                });
            }
            if grant.allowed_origins.is_empty() {
                return Err(EndpointPolicyConfigError {
                    grant_index: index,
                    reason: EndpointPolicyConfigErrorReason::EmptyOrigins,
                });
            }
            if grant
                .allowed_origins
                .iter()
                .any(|origin| !is_valid_grant_origin(origin))
            {
                return Err(EndpointPolicyConfigError {
                    grant_index: index,
                    reason: EndpointPolicyConfigErrorReason::InvalidOrigin,
                });
            }
        }
        Ok(())
    }

    pub fn for_connection(&self, connection_id: &str) -> EndpointPolicy {
        let Ok(requested_id) = uuid::Uuid::parse_str(connection_id) else {
            return EndpointPolicy::default();
        };
        let mut matching = self.grants.iter().filter(|grant| {
            uuid::Uuid::parse_str(&grant.connection_id).is_ok_and(|id| id == requested_id)
        });
        let Some(grant) = matching.next() else {
            return EndpointPolicy::default();
        };
        // Duplicate entries are ambiguous. Fail closed instead of choosing one
        // and accidentally ignoring an operator's conflicting configuration.
        if matching.next().is_some() || !is_valid_connection_id(&grant.connection_id) {
            return EndpointPolicy::default();
        }
        if grant
            .allowed_origins
            .iter()
            .any(|origin| !is_valid_grant_origin(origin))
        {
            return EndpointPolicy::default();
        }
        EndpointPolicy {
            allowed_origins: grant.allowed_origins.clone(),
        }
    }
}

fn is_valid_connection_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

fn is_valid_grant_origin(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointPolicyError {
    InsecureScheme,
    InvalidHost,
    OriginNotGranted,
    LoopbackNotAllowed,
    PrivateHostNotAllowlisted,
    LinkLocalNotAllowed,
    MetadataNotAllowed,
    NonGlobalAddress,
}

impl EndpointPolicy {
    /// Require an exact operator grant for the URL origin. Userinfo is never accepted.
    pub fn validate_url(&self, url: &url::Url) -> Result<(), EndpointPolicyError> {
        let host = url.host_str().ok_or(EndpointPolicyError::InvalidHost)?;
        if !url.username().is_empty() || url.password().is_some() || is_metadata_host(host) {
            return Err(if is_metadata_host(host) {
                EndpointPolicyError::MetadataNotAllowed
            } else {
                EndpointPolicyError::InvalidHost
            });
        }
        if url.scheme() != "https" && url.scheme() != "http" {
            return Err(EndpointPolicyError::InsecureScheme);
        }
        let port = url
            .port_or_known_default()
            .ok_or(EndpointPolicyError::InvalidHost)?;
        if !self
            .allowed_origins
            .iter()
            .any(|origin| origin_matches(origin, url))
        {
            return Err(EndpointPolicyError::OriginNotGranted);
        }
        match url.host().ok_or(EndpointPolicyError::InvalidHost)? {
            url::Host::Ipv4(ip) => {
                self.validate_ip(host, IpAddr::V4(ip), port, url.scheme())?;
            }
            url::Host::Ipv6(ip) => {
                self.validate_ip(host, IpAddr::V6(ip), port, url.scheme())?;
            }
            url::Host::Domain(_) => {}
        }
        // Hostnames are classified by the guarded resolver at connection time.
        // This permits an explicitly granted private HTTP hostname while still
        // rejecting it if DNS resolves to a public or otherwise unsafe address.
        Ok(())
    }

    /// Validate every DNS answer at connection time. Reject the whole answer set on any violation.
    pub fn validate_ip(
        &self,
        host: &str,
        ip: IpAddr,
        port: u16,
        scheme: &str,
    ) -> Result<(), EndpointPolicyError> {
        if is_metadata_ip(ip) {
            return Err(EndpointPolicyError::MetadataNotAllowed);
        }
        if is_link_local(ip) {
            return Err(EndpointPolicyError::LinkLocalNotAllowed);
        }
        if is_loopback_ip(ip) {
            if !host_is_loopback(host) || !self.origin_host_port_granted(scheme, host, port) {
                return Err(EndpointPolicyError::LoopbackNotAllowed);
            }
            return Ok(());
        }
        if is_private_ip(ip) {
            if !self.origin_host_port_granted(scheme, host, port) {
                return Err(EndpointPolicyError::PrivateHostNotAllowlisted);
            }
            return Ok(());
        }
        if is_non_global_ip(ip) {
            return Err(EndpointPolicyError::NonGlobalAddress);
        }
        if scheme != "https" {
            return Err(EndpointPolicyError::InsecureScheme);
        }
        Ok(())
    }

    fn origin_host_port_granted(&self, scheme: &str, host: &str, port: u16) -> bool {
        self.allowed_origins.iter().any(|origin| {
            url::Url::parse(origin).is_ok_and(|url| {
                url.scheme().eq_ignore_ascii_case(scheme)
                    && url.host_str().is_some_and(|h| h.eq_ignore_ascii_case(host))
                    && url.port_or_known_default() == Some(port)
            })
        })
    }
}

fn host_is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok_and(is_loopback_ip)
}

fn origin_matches(grant: &str, url: &url::Url) -> bool {
    let Ok(grant) = url::Url::parse(grant) else {
        return false;
    };
    grant.scheme().eq_ignore_ascii_case(url.scheme())
        && grant
            .host_str()
            .zip(url.host_str())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && grant.port_or_known_default() == url.port_or_known_default()
        && grant.path() == "/"
        && grant.query().is_none()
        && grant.fragment().is_none()
}

fn is_metadata_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().trim_end_matches('.'),
        "metadata.google.internal"
            | "metadata.google"
            | "metadata"
            | "instance-data.ec2.internal"
            | "metadata.azure.com"
            | "metadata.tencentyun.com"
            | "metadata.oraclecloud.com"
    )
}

fn is_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_metadata_ip_v4(ip),
        IpAddr::V6(ip) => {
            ip == "fd00:ec2::254"
                .parse::<Ipv6Addr>()
                .expect("valid metadata address")
                || ip.to_ipv4().is_some_and(is_metadata_ip_v4)
        }
    }
}

fn is_metadata_ip_v4(ip: Ipv4Addr) -> bool {
    ip == Ipv4Addr::new(169, 254, 169, 254)
        || ip == Ipv4Addr::new(100, 100, 100, 200)
        || ip == Ipv4Addr::new(168, 63, 129, 16)
}

fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.to_ipv4().is_some_and(|v4| v4.is_loopback()),
    }
}

fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.octets()[0] == 169 && ip.octets()[1] == 254,
        IpAddr::V6(ip) => {
            (ip.segments()[0] & 0xffc0) == 0xfe80
                || ip
                    .to_ipv4()
                    .is_some_and(|v4| v4.octets()[0] == 169 && v4.octets()[1] == 254)
        }
    }
}

fn is_non_global_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_non_global_ip(IpAddr::V4(v4));
            }
            let segments = ip.segments();
            ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xfff0) == 0x3ff0
                || (segments[0] & 0xe000) != 0x2000
                || (segments[0] == 0x2001 && ((segments[1] & 0xfe00) == 0 || segments[1] == 0x0db8))
        }
    }
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
        }
        IpAddr::V6(ip) => {
            (ip.segments()[0] & 0xfe00) == 0xfc00
                || ip.to_ipv4().is_some_and(|v4| {
                    let [a, b, _, _] = v4.octets();
                    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
                })
        }
    }
}
