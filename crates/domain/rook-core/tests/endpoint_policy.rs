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
fn reserved_documentation_ranges_are_rejected_even_when_the_origin_is_granted() {
    let granted = policy("https://service.example");
    for address in [
        "192.0.2.1",
        "198.51.100.1",
        "203.0.113.1",
        "2001:db8::1",
        "::ffff:192.0.2.1",
        "::ffff:198.51.100.1",
        "::ffff:203.0.113.1",
        "2001:2::1",
        "2001:10::1",
        "::8.8.8.8",
        "3fff::1",
    ] {
        let ip = address.parse::<IpAddr>().expect("valid test address");
        assert_eq!(
            granted.validate_ip("service.example", ip, 443, "https"),
            Err(EndpointPolicyError::NonGlobalAddress),
            "reserved address must be rejected: {address}"
        );
    }
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

fn grant_config(origins: &[&str]) -> EndpointPolicyConfig {
    EndpointPolicyConfig {
        grants: vec![EndpointGrant {
            connection_id: "c56a4180-65aa-42ec-a945-5fd21dec0538".into(),
            allowed_origins: origins.iter().map(|origin| (*origin).into()).collect(),
        }],
    }
}

#[test]
fn invalid_origin_invalidates_the_entire_grant() {
    for origin in [
        "not a URL",
        "ftp://upstream.example",
        "file:///etc/passwd",
        "https://user@upstream.example",
        "https://:password@upstream.example",
        "https://upstream.example/v1",
        "https://upstream.example?key=value",
        "https://upstream.example#fragment",
    ] {
        let config = grant_config(&["https://safe.example", origin]);
        assert_eq!(
            config.validate(),
            Err(EndpointPolicyConfigError {
                grant_index: 0,
                reason: EndpointPolicyConfigErrorReason::InvalidOrigin,
            }),
            "origin: {origin}"
        );
        let effective = config.for_connection(&config.grants[0].connection_id);
        assert_eq!(
            effective.validate_url(&url::Url::parse("https://safe.example/v1").unwrap()),
            Err(EndpointPolicyError::OriginNotGranted),
            "a valid sibling origin must not rescue {origin}"
        );
    }
}

#[test]
fn empty_origins_fail_validation_and_grant_no_access() {
    let config = grant_config(&[]);
    assert_eq!(
        config.validate().unwrap_err().reason,
        EndpointPolicyConfigErrorReason::EmptyOrigins
    );
    assert_eq!(
        config.for_connection(&config.grants[0].connection_id),
        EndpointPolicy::default()
    );
    assert_eq!(EndpointPolicyConfig::default().validate(), Ok(()));
}

#[test]
fn equivalent_uuid_spellings_select_the_same_grant_and_detect_duplicates() {
    let mut config = grant_config(&["https://upstream.example"]);
    let uppercase = config.grants[0].connection_id.to_uppercase();
    assert_eq!(
        config.for_connection(&uppercase),
        policy("https://upstream.example")
    );
    config.grants.push(EndpointGrant {
        connection_id: uppercase.clone(),
        allowed_origins: vec!["https://other.example".into()],
    });
    assert_eq!(
        config.validate(),
        Err(EndpointPolicyConfigError {
            grant_index: 1,
            reason: EndpointPolicyConfigErrorReason::DuplicateConnectionId,
        })
    );
    assert_eq!(config.for_connection(&uppercase), EndpointPolicy::default());
    assert_eq!(
        config.for_connection("not-a-uuid"),
        EndpointPolicy::default()
    );
}

#[test]
fn exact_origins_allow_base_paths_but_reject_host_suffix_confusion() {
    let config = grant_config(&["https://UPSTREAM.example:443/", "http://localhost:8080"]);
    assert_eq!(config.validate(), Ok(()));
    let effective = config.for_connection(&config.grants[0].connection_id);
    for endpoint in [
        "https://upstream.example/v1",
        "https://upstream.example/gateway/openai/v1",
        "http://localhost:8080/v1",
    ] {
        assert_eq!(
            effective.validate_url(&url::Url::parse(endpoint).unwrap()),
            Ok(())
        );
    }
    for endpoint in [
        "https://upstream.example.attacker.example/v1",
        "https://sub.upstream.example/v1",
        "https://upstream.example:444/v1",
        "http://upstream.example/v1",
    ] {
        assert_eq!(
            effective.validate_url(&url::Url::parse(endpoint).unwrap()),
            Err(EndpointPolicyError::OriginNotGranted),
            "endpoint: {endpoint}"
        );
    }
}

#[test]
fn metadata_host_aliases_are_denied_even_with_an_exact_grant() {
    for host in [
        "metadata",
        "metadata.google",
        "metadata.google.internal",
        "METADATA.GOOGLE.INTERNAL.",
        "instance-data.ec2.internal",
        "metadata.azure.com",
        "metadata.tencentyun.com",
        "metadata.oraclecloud.com",
    ] {
        let origin = format!("https://{host}");
        assert_eq!(
            policy(&origin).validate_url(&url::Url::parse(&origin).unwrap()),
            Err(EndpointPolicyError::MetadataNotAllowed),
            "host: {host}"
        );
    }
}

#[test]
fn unsafe_dns_answers_are_denied_even_for_granted_hosts() {
    use EndpointPolicyError::{LinkLocalNotAllowed, MetadataNotAllowed, NonGlobalAddress};
    let granted = policy("https://upstream.example");
    for (address, expected) in [
        ("169.254.169.254", MetadataNotAllowed),
        ("100.100.100.200", MetadataNotAllowed),
        ("168.63.129.16", MetadataNotAllowed),
        ("fd00:ec2::254", MetadataNotAllowed),
        ("::ffff:168.63.129.16", MetadataNotAllowed),
        ("169.254.0.1", LinkLocalNotAllowed),
        ("169.254.255.255", LinkLocalNotAllowed),
        ("fe80::1", LinkLocalNotAllowed),
        ("febf:ffff::1", LinkLocalNotAllowed),
        ("::ffff:169.254.1.1", LinkLocalNotAllowed),
        ("0.0.0.0", NonGlobalAddress),
        ("0.1.2.3", NonGlobalAddress),
        ("100.64.0.1", NonGlobalAddress),
        ("100.127.255.254", NonGlobalAddress),
        ("198.18.0.1", NonGlobalAddress),
        ("198.19.255.254", NonGlobalAddress),
        ("224.0.0.1", NonGlobalAddress),
        ("240.0.0.1", NonGlobalAddress),
        ("255.255.255.255", NonGlobalAddress),
        ("::", NonGlobalAddress),
        ("ff02::1", NonGlobalAddress),
        ("::ffff:100.64.0.1", NonGlobalAddress),
        ("::8.8.8.8", NonGlobalAddress),
        ("3fff::1", NonGlobalAddress),
    ] {
        assert_eq!(
            granted.validate_ip("upstream.example", address.parse().unwrap(), 443, "https"),
            Err(expected),
            "address: {address}"
        );
    }
}

#[test]
fn private_address_ranges_and_mapped_ipv4_require_the_same_exact_origin() {
    let granted = policy("http://inference.internal:8080");
    for address in [
        "10.0.0.1",
        "10.255.255.254",
        "172.16.0.1",
        "172.31.255.254",
        "192.168.0.1",
        "192.168.255.254",
        "fc00::1",
        "fdff::1",
        "::ffff:10.0.0.1",
    ] {
        let ip = address.parse().unwrap();
        assert_eq!(
            granted.validate_ip("inference.internal", ip, 8080, "http"),
            Ok(()),
            "{address}"
        );
        for (host, port, scheme) in [
            ("other.internal", 8080, "http"),
            ("inference.internal", 8081, "http"),
            ("inference.internal", 8080, "https"),
        ] {
            assert_eq!(
                granted.validate_ip(host, ip, port, scheme),
                Err(EndpointPolicyError::PrivateHostNotAllowlisted),
                "{address}, {scheme}://{host}:{port}"
            );
        }
    }
}

#[test]
fn public_addresses_adjacent_to_private_ranges_still_require_https() {
    for address in [
        "9.255.255.254",
        "11.0.0.1",
        "172.15.255.254",
        "172.32.0.1",
        "192.167.255.254",
        "192.169.0.1",
        "93.184.216.34",
        "2606:4700:4700::1111",
        "::ffff:93.184.216.34",
    ] {
        let ip = address.parse().unwrap();
        assert_eq!(
            policy("https://upstream.example").validate_ip("upstream.example", ip, 443, "https"),
            Ok(()),
            "{address}"
        );
        assert_eq!(
            policy("http://upstream.example").validate_ip("upstream.example", ip, 80, "http"),
            Err(EndpointPolicyError::InsecureScheme),
            "{address}"
        );
    }
}

#[test]
fn dns_loopback_answers_require_a_loopback_hostname_as_well_as_a_grant() {
    for host in ["localhost", "llm.localhost"] {
        let granted = policy(&format!("http://{host}:8080"));
        for address in ["127.0.0.2", "::1", "::ffff:127.0.0.1"] {
            assert_eq!(
                granted.validate_ip(host, address.parse().unwrap(), 8080, "http"),
                Ok(()),
                "{host}: {address}"
            );
        }
    }
    for host in [
        "localhost.attacker.example",
        "notlocalhost",
        "upstream.example",
    ] {
        assert_eq!(
            policy(&format!("http://{host}:8080")).validate_ip(
                host,
                "::ffff:127.0.0.1".parse().unwrap(),
                8080,
                "http"
            ),
            Err(EndpointPolicyError::LoopbackNotAllowed),
            "{host}"
        );
    }
}

// ---------------------------------------------------------------------------
// Contract coverage that used to live in an inline `#[cfg(test)]` module.
// The private classifiers are exercised through `EndpointPolicyConfig` and
// `EndpointPolicy`, which are the only public surface of this module.
// ---------------------------------------------------------------------------

#[test]
fn endpoint_policy_config_deserializes_snake_case_grants() {
    let config: EndpointPolicyConfig = serde_json::from_str(
        r#"{"grants":[{"connection_id":"550e8400-e29b-41d4-a716-446655440000","allowed_origins":["https://api.example.com"]}]}"#,
    )
    .expect("deserialize endpoint policy");
    assert_eq!(config.grants.len(), 1);
    assert_eq!(
        config.grants[0].connection_id,
        "550e8400-e29b-41d4-a716-446655440000"
    );
    assert_eq!(
        config.grants[0].allowed_origins,
        vec!["https://api.example.com"]
    );
    assert_eq!(config.validate(), Ok(()));
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
fn non_http_schemes_are_rejected_with_the_insecure_scheme_reason() {
    let granted = policy("https://api.example.com");
    for url in ["ftp://api.example.com/", "ws://api.example.com/"] {
        assert_eq!(
            granted.validate_url(&url::Url::parse(url).expect("valid url")),
            Err(EndpointPolicyError::InsecureScheme),
            "url: {url}"
        );
    }
}

#[test]
fn userinfo_in_any_form_is_rejected_with_the_invalid_host_reason() {
    let granted = policy("https://api.example.com");
    for url in [
        "https://user:secret@api.example.com/",
        "https://user@api.example.com/",
    ] {
        assert_eq!(
            granted.validate_url(&url::Url::parse(url).expect("valid url")),
            Err(EndpointPolicyError::InvalidHost),
            "url: {url}"
        );
    }
}

#[test]
fn loopback_hosts_are_matched_case_insensitively_and_by_ip_literal() {
    let ipv4: IpAddr = "127.0.0.1".parse().expect("valid address");
    let ipv6: IpAddr = "::1".parse().expect("valid address");
    for (origin, host, address) in [
        ("http://localhost:8080", "localhost", ipv4),
        ("http://LOCALHOST:8080", "LOCALHOST", ipv4),
        ("http://api.localhost:8080", "api.localhost", ipv4),
        ("http://127.0.0.1:8080", "127.0.0.1", ipv4),
        ("http://[::1]:8080", "[::1]", ipv6),
    ] {
        assert_eq!(
            policy(origin).validate_ip(host, address, 8080, "http"),
            Ok(()),
            "origin: {origin}"
        );
    }
    // A host that merely contains "localhost" is not a loopback host.
    assert_eq!(
        policy("http://notlocalhost:8080").validate_ip("notlocalhost", ipv4, 8080, "http"),
        Err(EndpointPolicyError::LoopbackNotAllowed)
    );
}

#[test]
fn private_addresses_need_an_exact_host_port_and_scheme_grant() {
    let ip: IpAddr = "10.0.0.5".parse().expect("valid address");
    assert_eq!(
        policy("http://10.0.0.5").validate_ip("10.0.0.5", ip, 80, "http"),
        Ok(())
    );
    // A different port does not match the grant.
    assert_eq!(
        policy("http://10.0.0.5").validate_ip("10.0.0.5", ip, 443, "https"),
        Err(EndpointPolicyError::PrivateHostNotAllowlisted)
    );
    // A different host does not match the grant.
    assert_eq!(
        policy("http://10.0.0.5").validate_ip("other.internal", ip, 80, "http"),
        Err(EndpointPolicyError::PrivateHostNotAllowlisted)
    );
}

#[test]
fn granted_public_ip_literals_are_accepted() {
    assert_eq!(
        policy("https://93.184.216.34")
            .validate_url(&url::Url::parse("https://93.184.216.34/").expect("valid url")),
        Ok(())
    );
    assert_eq!(
        policy("https://[2001:4860:4860::8888]")
            .validate_url(&url::Url::parse("https://[2001:4860:4860::8888]/").expect("valid url")),
        Ok(())
    );
}

#[test]
fn ipv6_address_families_are_classified_by_the_public_denial_reason() {
    // Without an origin grant every private, loopback, link-local, metadata
    // and reserved address is denied, each with its own reason.
    let ungranted = EndpointPolicy::default();
    for (address, expected) in [
        ("::1", EndpointPolicyError::LoopbackNotAllowed),
        ("::ffff:127.0.0.1", EndpointPolicyError::LoopbackNotAllowed),
        ("fe80::1", EndpointPolicyError::LinkLocalNotAllowed),
        (
            "::ffff:169.254.10.1",
            EndpointPolicyError::LinkLocalNotAllowed,
        ),
        (
            "::ffff:169.254.169.254",
            EndpointPolicyError::MetadataNotAllowed,
        ),
        // Unique local addresses are private before they are non-global.
        ("fc00::1", EndpointPolicyError::PrivateHostNotAllowlisted),
        ("fd00::1", EndpointPolicyError::PrivateHostNotAllowlisted),
        ("::", EndpointPolicyError::NonGlobalAddress),
        ("ff02::1", EndpointPolicyError::NonGlobalAddress),
        ("3fff::1", EndpointPolicyError::NonGlobalAddress),
        ("2001:db8::1", EndpointPolicyError::NonGlobalAddress),
        ("::ffff:100.64.0.1", EndpointPolicyError::NonGlobalAddress),
    ] {
        let ip: IpAddr = address.parse().expect("valid address");
        assert_eq!(
            ungranted.validate_ip("upstream.example", ip, 443, "https"),
            Err(expected),
            "address: {address}"
        );
    }
    // A globally routable IPv6 address stays reachable over HTTPS.
    let global: IpAddr = "2001:4860:4860::8888".parse().expect("valid address");
    assert_eq!(
        policy("https://upstream.example").validate_ip("upstream.example", global, 443, "https"),
        Ok(())
    );
}

#[test]
fn granted_origins_allow_listed_hosts_and_reject_unlisted_credentialed_or_local_targets() {
    let policy = EndpointPolicy {
        allowed_origins: vec![
            "https://api.example.com".into(),
            "https://[2606:4700:4700::1111]".into(),
        ],
    };
    for endpoint in [
        "https://localhost:11434/v1",
        "https://user:password@10.0.0.1/v1",
        "https://[::1]/v1",
        "https://[fc00::1]/v1",
        "https://[fdff::1]/v1",
    ] {
        let url = url::Url::parse(endpoint).unwrap();
        assert!(policy.validate_url(&url).is_err(), "{endpoint}");
    }
    for endpoint in [
        "https://api.example.com/v1",
        "https://[2606:4700:4700::1111]/v1",
    ] {
        assert!(
            policy
                .validate_url(&url::Url::parse(endpoint).unwrap())
                .is_ok(),
            "{endpoint}"
        );
    }
}
