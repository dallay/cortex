use std::net::IpAddr;

use rook_core::*;

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
    assert_eq!(duplicate.validate().unwrap_err().grant_index, 1);
    let malformed = EndpointPolicyConfig {
        grants: vec![EndpointGrant {
            connection_id: id.into(),
            allowed_origins: vec!["https://upstream.example/v1".into()],
        }],
    };
    assert!(malformed.for_connection(id).allowed_origins.is_empty());
    assert_eq!(malformed.validate().unwrap_err().grant_index, 0);
    assert_eq!(
        EndpointPolicyConfig {
            grants: vec![EndpointGrant {
                connection_id: "not-a-uuid".into(),
                allowed_origins: vec!["https://upstream.example".into()]
            }],
        }
        .validate()
        .unwrap_err()
        .reason,
        EndpointPolicyConfigErrorReason::InvalidConnectionId
    );
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
        granted.validate_url(&url::Url::parse("https://user:pass@upstream.example/v1").unwrap()),
        Err(EndpointPolicyError::InvalidHost)
    );
}

#[test]
fn ipv6_loopback_url_literal_is_checked_against_the_granted_origin() {
    let url = url::Url::parse("http://[::1]:11434/v1").unwrap();
    let unrelated_grant = policy("http://[::2]:11434");

    assert_eq!(
        unrelated_grant.validate_url(&url),
        Err(EndpointPolicyError::OriginNotGranted)
    );
    assert_eq!(
        unrelated_grant.validate_ip("[::1]", IpAddr::V6("::1".parse().unwrap()), 11434, "http"),
        Err(EndpointPolicyError::LoopbackNotAllowed)
    );

    let exact_grant = policy("http://[::1]:11434");
    assert_eq!(exact_grant.validate_url(&url), Ok(()));
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
        remote_loopback.validate_ip("public.example", "127.0.0.1".parse().unwrap(), 8080, "http"),
        Err(EndpointPolicyError::LoopbackNotAllowed)
    );
    let private_http = policy("http://inference.internal:8080");
    assert_eq!(
        private_http.validate_url(&url::Url::parse("http://inference.internal:8080/v1").unwrap()),
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
