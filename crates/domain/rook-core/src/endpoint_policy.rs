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

impl EndpointPolicyConfig {
    pub fn validate(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        self.grants.iter().all(|grant| {
            let Ok(id) = uuid::Uuid::parse_str(&grant.connection_id) else {
                return false;
            };
            ids.insert(id)
                && !grant.allowed_origins.is_empty()
                && grant
                    .allowed_origins
                    .iter()
                    .all(|origin| is_valid_grant_origin(origin))
        })
    }

    pub fn for_connection(&self, connection_id: &str) -> EndpointPolicy {
        let mut matching = self
            .grants
            .iter()
            .filter(|grant| grant.connection_id.eq_ignore_ascii_case(connection_id));
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
        if let Ok(ip) = host.parse::<IpAddr>() {
            self.validate_ip(host, ip, port, url.scheme())?;
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
    host.parse::<IpAddr>().is_ok_and(is_loopback_ip)
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
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(ip) => {
            ip.is_unspecified()
                || ip.is_multicast()
                || ip
                    .to_ipv4()
                    .is_some_and(|v4| is_non_global_ip(IpAddr::V4(v4)))
                || (ip.segments()[0] & 0xffc0) == 0xfe80
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

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(origin: &str) -> EndpointPolicy {
        EndpointPolicy {
            allowed_origins: vec![origin.to_string()],
        }
    }

    #[test]
    fn grants_are_selected_by_immutable_connection_id_and_fail_closed_when_ambiguous() {
        let id = "c56a4180-65aa-42ec-a945-5fd21dec0538";
        let config = EndpointPolicyConfig {
            grants: vec![EndpointGrant {
                connection_id: id.into(),
                allowed_origins: vec!["https://upstream.example".into()],
            }],
        };
        assert_eq!(config.for_connection(id).allowed_origins.len(), 1);
        assert!(config
            .for_connection("another-id")
            .allowed_origins
            .is_empty());
        let duplicate = EndpointPolicyConfig {
            grants: vec![
                EndpointGrant {
                    connection_id: id.into(),
                    allowed_origins: vec!["https://upstream.example".into()],
                },
                EndpointGrant {
                    connection_id: id.into(),
                    allowed_origins: vec!["https://other.example".into()],
                },
            ],
        };
        assert!(duplicate.for_connection(id).allowed_origins.is_empty());
        assert!(!duplicate.validate());
        let malformed = EndpointPolicyConfig {
            grants: vec![EndpointGrant {
                connection_id: id.into(),
                allowed_origins: vec!["https://upstream.example/v1".into()],
            }],
        };
        assert!(malformed.for_connection(id).allowed_origins.is_empty());
        assert!(!malformed.validate());
        assert!(!EndpointPolicyConfig {
            grants: vec![EndpointGrant {
                connection_id: "not-a-uuid".into(),
                allowed_origins: vec!["https://upstream.example".into()]
            }],
        }
        .validate());
    }

    #[test]
    fn origin_grant_matches_scheme_host_and_effective_port_only() {
        let granted = policy("https://upstream.example:443");
        assert!(granted
            .validate_url(&url::Url::parse("https://upstream.example/v1").unwrap())
            .is_ok());
        assert_eq!(
            granted.validate_url(&url::Url::parse("http://upstream.example/v1").unwrap()),
            Err(EndpointPolicyError::OriginNotGranted)
        );
        assert_eq!(
            granted.validate_url(&url::Url::parse("https://upstream.example:8443/v1").unwrap()),
            Err(EndpointPolicyError::OriginNotGranted)
        );
        assert_eq!(
            granted.validate_url(&url::Url::parse("https://other.example/v1").unwrap()),
            Err(EndpointPolicyError::OriginNotGranted)
        );
        assert_eq!(
            granted
                .validate_url(&url::Url::parse("https://user:pass@upstream.example/v1").unwrap()),
            Err(EndpointPolicyError::InvalidHost)
        );
    }

    #[test]
    fn private_and_loopback_addresses_require_exact_origin_grant() {
        let local = policy("http://127.0.0.1:11434");
        assert!(local
            .validate_url(&url::Url::parse("http://127.0.0.1:11434/v1").unwrap())
            .is_ok());
        assert_eq!(
            local.validate_ip("127.0.0.1", "127.0.0.1".parse().unwrap(), 11435, "http"),
            Err(EndpointPolicyError::LoopbackNotAllowed)
        );
        let remote_loopback = policy("http://public.example:8080");
        assert_eq!(
            remote_loopback.validate_ip(
                "public.example",
                "127.0.0.1".parse().unwrap(),
                8080,
                "http"
            ),
            Err(EndpointPolicyError::LoopbackNotAllowed)
        );
        let private_http = policy("http://inference.internal:8080");
        assert_eq!(
            private_http
                .validate_url(&url::Url::parse("http://inference.internal:8080/v1").unwrap()),
            Ok(())
        );
        assert_eq!(
            private_http.validate_ip(
                "inference.internal",
                "10.0.0.8".parse().unwrap(),
                8080,
                "http"
            ),
            Ok(())
        );
        assert_eq!(
            private_http.validate_ip(
                "inference.internal",
                "93.184.216.34".parse().unwrap(),
                8080,
                "http"
            ),
            Err(EndpointPolicyError::InsecureScheme)
        );
        let private = policy("https://inference.internal:8443");
        assert!(private
            .validate_ip(
                "inference.internal",
                "10.0.0.8".parse().unwrap(),
                8443,
                "https"
            )
            .is_ok());
        assert_eq!(
            private.validate_ip(
                "inference.internal",
                "10.0.0.8".parse().unwrap(),
                8444,
                "https"
            ),
            Err(EndpointPolicyError::PrivateHostNotAllowlisted)
        );
    }

    #[test]
    fn metadata_and_link_local_are_rejected_even_when_granted() {
        let metadata = policy("https://metadata.google.internal");
        assert_eq!(
            metadata.validate_url(&url::Url::parse("https://metadata.google.internal").unwrap()),
            Err(EndpointPolicyError::MetadataNotAllowed)
        );
        let link_local = policy("https://service.example");
        assert_eq!(
            link_local.validate_ip(
                "service.example",
                "169.254.1.1".parse().unwrap(),
                443,
                "https"
            ),
            Err(EndpointPolicyError::LinkLocalNotAllowed)
        );
        assert_eq!(
            link_local.validate_ip(
                "service.example",
                "::ffff:169.254.169.254".parse().unwrap(),
                443,
                "https"
            ),
            Err(EndpointPolicyError::MetadataNotAllowed)
        );
    }

    #[test]
    fn credentials_in_url_and_non_http_schemes_are_rejected() {
        let granted = policy("https://upstream.example");
        assert_eq!(
            granted.validate_url(&url::Url::parse("https://user:pass@upstream.example").unwrap()),
            Err(EndpointPolicyError::InvalidHost)
        );
        assert!(url::Url::parse("file:///etc/passwd")
            .map(|url| granted.validate_url(&url))
            .unwrap()
            .is_err());
    }
}
