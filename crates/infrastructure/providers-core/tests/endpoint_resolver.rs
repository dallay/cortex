use providers_core::endpoint_resolver::{resolver_for_url, GuardedResolver};
use reqwest::dns::Resolve;
use rook_core::EndpointPolicy;

#[tokio::test]
async fn dns_answers_require_a_local_grant_at_connection_time() {
    let resolver = GuardedResolver::new(EndpointPolicy::default(), "http", "localhost", 11434);
    assert!(resolver
        .resolve("localhost".parse().unwrap())
        .await
        .is_err());
    let resolver = GuardedResolver::new(
        EndpointPolicy::default().for_ollama(),
        "http",
        "localhost",
        11434,
    );
    assert!(resolver.resolve("localhost".parse().unwrap()).await.is_ok());
}

#[test]
fn metadata_is_blocked_even_when_explicitly_granted() {
    for endpoint in [
        "http://169.254.169.254",
        "http://[fd00:ec2::254]",
        "http://metadata.google.internal",
    ] {
        let policy = EndpointPolicy {
            allowed_origins: vec![endpoint.into()],
        };
        assert!(resolver_for_url(policy, endpoint).is_err());
    }
}
