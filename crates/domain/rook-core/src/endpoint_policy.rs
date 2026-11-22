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
    /// Built-in Ollama destinations plus operator-approved connection origins.
    /// Custom local ports and hosts require an explicit connection grant.
    pub fn for_ollama(&self) -> Self {
        let mut policy = self.clone();
        for origin in [
            "http://localhost:11434",
            "http://127.0.0.1:11434",
            "http://[::1]:11434",
            "https://localhost:11434",
            "https://127.0.0.1:11434",
            "https://[::1]:11434",
            "https://ollama.com",
        ] {
            policy.allowed_origins.push(origin.to_string());
        }
        policy
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    const CONNECTION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const OTHER_CONNECTION_ID: &str = "6ba7b810-9dad-41d1-80b4-00c04fd430c8";

    fn grant(connection_id: &str, origins: &[&str]) -> EndpointGrant {
        EndpointGrant {
            connection_id: connection_id.to_string(),
            allowed_origins: origins.iter().map(|origin| origin.to_string()).collect(),
        }
    }

    fn config(grants: Vec<EndpointGrant>) -> EndpointPolicyConfig {
        EndpointPolicyConfig { grants }
    }

    fn parse(url: &str) -> url::Url {
        url::Url::parse(url).expect("valid url")
    }

    fn granted_policy(origins: &[&str]) -> EndpointPolicy {
        EndpointPolicy {
            allowed_origins: origins.iter().map(|origin| origin.to_string()).collect(),
        }
    }

    #[test]
    fn endpoint_policy_config_defaults_to_no_grants() {
        let config = EndpointPolicyConfig::default();
        assert!(config.grants.is_empty());
        assert_eq!(config.validate(), Ok(()));
        assert_eq!(
            config.for_connection(CONNECTION_ID),
            EndpointPolicy::default()
        );
        assert!(EndpointPolicy::default().allowed_origins.is_empty());
    }

    #[test]
    fn endpoint_policy_config_deserializes_snake_case_grants() {
        let config: EndpointPolicyConfig = serde_json::from_str(
            r#"{"grants":[{"connection_id":"550e8400-e29b-41d4-a716-446655440000","allowed_origins":["https://api.example.com"]}]}"#,
        )
        .expect("deserialize endpoint policy");
        assert_eq!(config.grants.len(), 1);
        assert_eq!(config.grants[0].connection_id, CONNECTION_ID);
        assert_eq!(
            config.grants[0].allowed_origins,
            vec!["https://api.example.com"]
        );
    }

    #[test]
    fn endpoint_policy_config_accepts_well_formed_grants() {
        let config = config(vec![grant(
            CONNECTION_ID,
            &["https://api.example.com", "http://localhost:8080"],
        )]);
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn endpoint_policy_config_rejects_an_invalid_connection_id() {
        let config = config(vec![grant("not-a-uuid", &["https://api.example.com"])]);
        let error = config
            .validate()
            .expect_err("invalid connection id must fail");
        assert_eq!(error.grant_index, 0);
        assert_eq!(
            error.reason,
            EndpointPolicyConfigErrorReason::InvalidConnectionId
        );
    }

    #[test]
    fn endpoint_policy_config_rejects_a_duplicate_connection_id() {
        let config = config(vec![
            grant(CONNECTION_ID, &["https://api.example.com"]),
            grant(CONNECTION_ID, &["https://other.example.com"]),
        ]);
        let error = config
            .validate()
            .expect_err("duplicate connection id must fail");
        assert_eq!(error.grant_index, 1);
        assert_eq!(
            error.reason,
            EndpointPolicyConfigErrorReason::DuplicateConnectionId
        );
    }

    #[test]
    fn endpoint_policy_config_rejects_empty_origins() {
        let config = config(vec![grant(CONNECTION_ID, &[])]);
        let error = config.validate().expect_err("empty origins must fail");
        assert_eq!(error.grant_index, 0);
        assert_eq!(error.reason, EndpointPolicyConfigErrorReason::EmptyOrigins);
    }

    #[test]
    fn endpoint_policy_config_rejects_invalid_origins() {
        let invalid_origins = [
            "ftp://api.example.com",
            "https://user:secret@api.example.com",
            "https://api.example.com/v1",
            "https://api.example.com?query=1",
            "https://api.example.com#fragment",
            "not a url",
        ];
        for origin in invalid_origins {
            let config = config(vec![grant(CONNECTION_ID, &[origin])]);
            let error = config.validate().expect_err("invalid origin must fail");
            assert_eq!(error.grant_index, 0);
            assert_eq!(error.reason, EndpointPolicyConfigErrorReason::InvalidOrigin);
        }
    }

    #[test]
    fn endpoint_policy_config_error_display_names_the_grant_and_reason() {
        let error = EndpointPolicyConfigError {
            grant_index: 2,
            reason: EndpointPolicyConfigErrorReason::EmptyOrigins,
        };
        assert_eq!(error.to_string(), "grant 2: EmptyOrigins");
    }

    #[test]
    fn for_connection_returns_the_granted_origins() {
        let config = config(vec![grant(
            CONNECTION_ID,
            &["https://api.example.com", "http://localhost:8080"],
        )]);
        let policy = config.for_connection(CONNECTION_ID);
        assert_eq!(
            policy.allowed_origins,
            vec!["https://api.example.com", "http://localhost:8080"]
        );
    }

    #[test]
    fn for_connection_returns_the_default_policy_for_unmatched_requests() {
        let config = config(vec![grant(CONNECTION_ID, &["https://api.example.com"])]);

        // An unknown but valid connection id has no grant.
        assert_eq!(
            config.for_connection(OTHER_CONNECTION_ID),
            EndpointPolicy::default()
        );
        // A malformed connection id never resolves.
        assert_eq!(
            config.for_connection("not-a-uuid"),
            EndpointPolicy::default()
        );
    }

    #[test]
    fn for_connection_returns_the_default_policy_for_duplicate_grants() {
        let config = config(vec![
            grant(CONNECTION_ID, &["https://api.example.com"]),
            grant(CONNECTION_ID, &["https://other.example.com"]),
        ]);
        assert_eq!(
            config.for_connection(CONNECTION_ID),
            EndpointPolicy::default()
        );
    }

    #[test]
    fn for_connection_returns_the_default_policy_for_invalid_grant_origins() {
        let config = config(vec![grant(CONNECTION_ID, &["https://api.example.com/v1"])]);
        assert_eq!(
            config.for_connection(CONNECTION_ID),
            EndpointPolicy::default()
        );
    }

    #[test]
    fn validate_url_rejects_userinfo() {
        let policy = granted_policy(&["https://api.example.com"]);
        let error = policy
            .validate_url(&parse("https://user:secret@api.example.com/"))
            .expect_err("userinfo must be rejected");
        assert_eq!(error, EndpointPolicyError::InvalidHost);

        let error = policy
            .validate_url(&parse("https://user@api.example.com/"))
            .expect_err("userinfo must be rejected");
        assert_eq!(error, EndpointPolicyError::InvalidHost);
    }

    #[test]
    fn validate_url_rejects_metadata_hosts() {
        let policy = granted_policy(&["https://metadata.google.internal"]);
        let error = policy
            .validate_url(&parse("http://metadata.google.internal/"))
            .expect_err("metadata host must be rejected");
        assert_eq!(error, EndpointPolicyError::MetadataNotAllowed);

        let policy = granted_policy(&["https://metadata.google.internal"]);
        let error = policy
            .validate_url(&parse("http://metadata.google.internal./"))
            .expect_err("metadata host must be rejected");
        assert_eq!(error, EndpointPolicyError::MetadataNotAllowed);
    }

    #[test]
    fn validate_url_rejects_insecure_schemes() {
        let policy = granted_policy(&["ftp://api.example.com"]);
        let error = policy
            .validate_url(&parse("ftp://api.example.com/"))
            .expect_err("insecure scheme must be rejected");
        assert_eq!(error, EndpointPolicyError::InsecureScheme);
    }

    #[test]
    fn validate_url_rejects_origins_without_a_grant() {
        let policy = granted_policy(&["https://api.example.com"]);
        let error = policy
            .validate_url(&parse("https://other.example.com/"))
            .expect_err("ungranted origin must be rejected");
        assert_eq!(error, EndpointPolicyError::OriginNotGranted);
    }

    #[test]
    fn validate_url_matches_the_granted_port() {
        let policy = granted_policy(&["https://api.example.com:8443"]);
        assert_eq!(
            policy.validate_url(&parse("https://api.example.com:8443/")),
            Ok(())
        );
        let error = policy
            .validate_url(&parse("https://api.example.com/"))
            .expect_err("a different port must not match the grant");
        assert_eq!(error, EndpointPolicyError::OriginNotGranted);
    }

    #[test]
    fn validate_url_accepts_a_granted_https_origin() {
        let policy = granted_policy(&["https://api.example.com"]);
        assert_eq!(
            policy.validate_url(&parse("https://api.example.com/")),
            Ok(())
        );
        assert_eq!(
            policy.validate_url(&parse("https://api.example.com")),
            Ok(())
        );
    }

    #[test]
    fn validate_ip_rejects_metadata_addresses() {
        let policy = granted_policy(&[]);
        for address in ["169.254.169.254", "100.100.100.200", "168.63.129.16"] {
            let ip: IpAddr = address.parse().expect("valid address");
            let error = policy
                .validate_ip(address, ip, 443, "https")
                .expect_err("metadata address must be rejected");
            assert_eq!(error, EndpointPolicyError::MetadataNotAllowed);
        }

        let ip: IpAddr = "fd00:ec2::254".parse().expect("valid address");
        let error = policy
            .validate_ip("fd00:ec2::254", ip, 443, "https")
            .expect_err("metadata address must be rejected");
        assert_eq!(error, EndpointPolicyError::MetadataNotAllowed);
    }

    #[test]
    fn validate_ip_rejects_link_local_addresses() {
        let policy = granted_policy(&[]);
        let ip: IpAddr = "169.254.10.1".parse().expect("valid address");
        let error = policy
            .validate_ip("169.254.10.1", ip, 443, "https")
            .expect_err("link-local address must be rejected");
        assert_eq!(error, EndpointPolicyError::LinkLocalNotAllowed);

        let ip: IpAddr = "fe80::1".parse().expect("valid address");
        let error = policy
            .validate_ip("fe80::1", ip, 443, "https")
            .expect_err("link-local address must be rejected");
        assert_eq!(error, EndpointPolicyError::LinkLocalNotAllowed);
    }

    #[test]
    fn validate_ip_rejects_loopback_without_an_exact_grant() {
        let policy = granted_policy(&["http://localhost:8080"]);
        let ip: IpAddr = "127.0.0.1".parse().expect("valid address");
        let error = policy
            .validate_ip("127.0.0.1", ip, 80, "http")
            .expect_err("loopback without a grant must be rejected");
        assert_eq!(error, EndpointPolicyError::LoopbackNotAllowed);
    }

    #[test]
    fn validate_ip_accepts_loopback_with_an_exact_grant() {
        let policy = granted_policy(&["http://localhost:8080"]);
        let ip: IpAddr = "127.0.0.1".parse().expect("valid address");
        assert_eq!(policy.validate_ip("localhost", ip, 8080, "http"), Ok(()));

        let policy = granted_policy(&["http://127.0.0.1:8080"]);
        assert_eq!(policy.validate_ip("127.0.0.1", ip, 8080, "http"), Ok(()));
    }

    #[test]
    fn validate_ip_rejects_private_addresses_without_a_grant() {
        let policy = granted_policy(&[]);
        let ip: IpAddr = "10.0.0.5".parse().expect("valid address");
        let error = policy
            .validate_ip("10.0.0.5", ip, 443, "https")
            .expect_err("private address must be rejected");
        assert_eq!(error, EndpointPolicyError::PrivateHostNotAllowlisted);
    }

    #[test]
    fn validate_ip_accepts_private_addresses_with_a_grant() {
        let policy = granted_policy(&["http://10.0.0.5"]);
        let ip: IpAddr = "10.0.0.5".parse().expect("valid address");
        assert_eq!(policy.validate_ip("10.0.0.5", ip, 80, "http"), Ok(()));
    }

    #[test]
    fn validate_ip_rejects_non_global_addresses() {
        let policy = granted_policy(&["https://api.example.com"]);
        for address in ["0.0.0.0", "100.64.0.1", "240.0.0.1", "255.255.255.255"] {
            let ip: IpAddr = address.parse().expect("valid address");
            let error = policy
                .validate_ip(address, ip, 443, "https")
                .expect_err("non-global address must be rejected");
            assert_eq!(error, EndpointPolicyError::NonGlobalAddress);
        }
    }

    #[test]
    fn validate_ip_requires_https_for_public_addresses() {
        let policy = granted_policy(&["http://93.184.216.34"]);
        let ip: IpAddr = "93.184.216.34".parse().expect("valid address");
        let error = policy
            .validate_ip("93.184.216.34", ip, 80, "http")
            .expect_err("plain http must be rejected for a public address");
        assert_eq!(error, EndpointPolicyError::InsecureScheme);

        assert_eq!(
            policy.validate_ip("93.184.216.34", ip, 443, "https"),
            Ok(())
        );
    }

    #[test]
    fn validate_ip_accepts_public_addresses_over_https() {
        let policy = granted_policy(&["https://api.example.com"]);
        let ip: IpAddr = "93.184.216.34".parse().expect("valid address");
        assert_eq!(
            policy.validate_ip("api.example.com", ip, 443, "https"),
            Ok(())
        );
    }

    #[test]
    fn validate_url_accepts_a_granted_public_origin() {
        let policy = granted_policy(&["https://api.example.com"]);
        assert_eq!(
            policy.validate_url(&parse("https://api.example.com/")),
            Ok(())
        );
    }

    #[test]
    fn validate_url_rejects_a_granted_loopback_origin_from_a_foreign_host() {
        // The origin grant is checked against the URL host, so a private address
        // can only be reached through the exact granted origin.
        let policy = granted_policy(&["http://localhost:8080"]);
        let error = policy
            .validate_url(&parse("http://127.0.0.1:8080/"))
            .expect_err("a different host must not match the grant");
        assert_eq!(error, EndpointPolicyError::OriginNotGranted);
    }

    #[test]
    fn validate_url_accepts_a_granted_ipv6_origin() {
        let policy = granted_policy(&["https://[2001:4860:4860::8888]"]);
        assert_eq!(
            policy.validate_url(&parse("https://[2001:4860:4860::8888]/")),
            Ok(())
        );
    }

    #[test]
    fn validate_url_accepts_a_granted_ipv4_origin() {
        let policy = granted_policy(&["https://93.184.216.34"]);
        assert_eq!(
            policy.validate_url(&parse("https://93.184.216.34/")),
            Ok(())
        );
    }

    #[test]
    fn validate_ip_accepts_a_granted_ipv6_loopback() {
        let policy = granted_policy(&["http://[::1]:8080"]);
        let ip: IpAddr = "::1".parse().expect("valid address");
        assert_eq!(policy.validate_ip("[::1]", ip, 8080, "http"), Ok(()));
    }

    #[test]
    fn validate_ip_rejects_an_ipv4_mapped_loopback_without_a_grant() {
        let policy = granted_policy(&[]);
        let ip: IpAddr = "::ffff:127.0.0.1".parse().expect("valid address");
        let error = policy
            .validate_ip("127.0.0.1", ip, 443, "https")
            .expect_err("loopback must be rejected");
        assert_eq!(error, EndpointPolicyError::LoopbackNotAllowed);
    }

    #[test]
    fn validate_ip_rejects_an_ipv4_mapped_link_local_address() {
        let policy = granted_policy(&[]);
        let ip: IpAddr = "::ffff:169.254.10.1".parse().expect("valid address");
        let error = policy
            .validate_ip("169.254.10.1", ip, 443, "https")
            .expect_err("link-local must be rejected");
        assert_eq!(error, EndpointPolicyError::LinkLocalNotAllowed);
    }

    #[test]
    fn validate_ip_rejects_an_ipv4_mapped_non_global_address() {
        let policy = granted_policy(&[]);
        let ip: IpAddr = "::ffff:100.64.0.1".parse().expect("valid address");
        let error = policy
            .validate_ip("100.64.0.1", ip, 443, "https")
            .expect_err("non-global must be rejected");
        assert_eq!(error, EndpointPolicyError::NonGlobalAddress);
    }

    #[test]
    fn address_helpers_classify_ipv6_families() {
        let global: IpAddr = "2001:4860:4860::8888".parse().expect("valid address");

        assert!(is_loopback_ip("::1".parse().expect("valid address")));
        assert!(is_loopback_ip(
            "::ffff:127.0.0.1".parse().expect("valid address")
        ));
        assert!(!is_loopback_ip(global));

        assert!(is_link_local("fe80::1".parse().expect("valid address")));
        assert!(is_link_local(
            "::ffff:169.254.10.1".parse().expect("valid address")
        ));
        assert!(!is_link_local(global));

        assert!(is_metadata_ip(
            "fd00:ec2::254".parse().expect("valid address")
        ));
        assert!(is_metadata_ip(
            "::ffff:169.254.169.254".parse().expect("valid address")
        ));
        assert!(!is_metadata_ip(global));

        assert!(is_private_ip("fc00::1".parse().expect("valid address")));
        assert!(is_private_ip(
            "::ffff:10.0.0.5".parse().expect("valid address")
        ));
        assert!(!is_private_ip(global));

        assert!(is_non_global_ip("::".parse().expect("valid address")));
        assert!(is_non_global_ip("ff02::1".parse().expect("valid address")));
        assert!(is_non_global_ip("fc00::1".parse().expect("valid address")));
        assert!(is_non_global_ip("3fff::1".parse().expect("valid address")));
        assert!(is_non_global_ip(
            "2001:db8::1".parse().expect("valid address")
        ));
        assert!(is_non_global_ip(
            "::ffff:100.64.0.1".parse().expect("valid address")
        ));
        assert!(!is_non_global_ip(global));
    }

    #[test]
    fn host_helpers_classify_loopback_and_metadata_names() {
        assert!(host_is_loopback("localhost"));
        assert!(host_is_loopback("LOCALHOST"));
        assert!(host_is_loopback("api.localhost"));
        assert!(host_is_loopback("[::1]"));
        assert!(host_is_loopback("127.0.0.1"));
        assert!(!host_is_loopback("api.example.com"));

        assert!(is_metadata_host("metadata.google.internal"));
        assert!(is_metadata_host("METADATA.GOOGLE.INTERNAL"));
        assert!(is_metadata_host("metadata.google.internal."));
        assert!(is_metadata_host("metadata"));
        assert!(is_metadata_host("instance-data.ec2.internal"));
        assert!(is_metadata_host("metadata.azure.com"));
        assert!(is_metadata_host("metadata.tencentyun.com"));
        assert!(is_metadata_host("metadata.oraclecloud.com"));
        assert!(!is_metadata_host("api.example.com"));
    }

    #[test]
    fn grant_helpers_validate_identifiers_and_origins() {
        assert!(is_valid_connection_id(CONNECTION_ID));
        assert!(!is_valid_connection_id("not-a-uuid"));

        assert!(is_valid_grant_origin("https://api.example.com"));
        assert!(is_valid_grant_origin("http://localhost:8080"));
        assert!(!is_valid_grant_origin("https://api.example.com/v1"));
        assert!(!is_valid_grant_origin("ftp://api.example.com"));
        assert!(!is_valid_grant_origin("not a url"));
    }

    #[test]
    fn origin_matches_requires_the_same_scheme_host_and_port() {
        let url = parse("https://api.example.com/");
        assert!(origin_matches("https://api.example.com", &url));
        assert!(origin_matches("https://api.example.com/", &url));
        assert!(!origin_matches("http://api.example.com", &url));
        assert!(!origin_matches("https://other.example.com", &url));
        assert!(!origin_matches("https://api.example.com:8443", &url));
        assert!(!origin_matches("not a url", &url));
    }
}
