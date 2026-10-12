use providers_openai::provider::parse_openai_rate_limit_headers as parse;
use reqwest::header::HeaderMap;
use shared_kernel::ProviderId;
fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(k, v)| (k.parse().unwrap(), v.parse().unwrap()))
        .collect()
}

#[test]
fn parses_tokens_and_project_scope_and_handles_missing_malformed_and_reset() {
    let mut h = headers(&[
        ("x-ratelimit-limit-tokens", "10000"),
        ("x-ratelimit-remaining-tokens", "9000"),
        ("x-ratelimit-reset-tokens", "1m2.345s"),
        ("x-ratelimit-limit-project-tokens", "8000"),
        ("x-ratelimit-remaining-project-tokens", "7000"),
    ]);
    let id = ProviderId::new("openai-1");
    let got = parse(&h, id.clone(), chrono::Utc::now());
    assert_eq!(got[0].reset_after_milliseconds, Some(62_345));
    assert_eq!(got[1].scope.as_deref(), Some("project"));
    assert_eq!(got[1].limit, Some(8_000.0));
    assert_eq!(got[1].remaining, Some(7_000.0));
    h.insert("x-ratelimit-reset-tokens", "bad".parse().unwrap());
    h.remove("x-ratelimit-remaining-project-tokens");
    let got = parse(&h, id.clone(), chrono::Utc::now());
    assert_eq!(got[0].reset_after_milliseconds, None);
}
