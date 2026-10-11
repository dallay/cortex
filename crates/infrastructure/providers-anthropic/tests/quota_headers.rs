use providers_anthropic::parse_anthropic_rate_limit_headers as parse;
use reqwest::header::HeaderMap;
use shared_kernel::ProviderId;
fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(k, v)| (k.parse().unwrap(), v.parse().unwrap()))
        .collect()
}
#[test]
fn preserves_request_and_input_metrics_and_handles_missing_malformed_and_reset() {
    let mut h = headers(&[
        ("anthropic-ratelimit-requests-limit", "100"),
        ("anthropic-ratelimit-requests-remaining", "90"),
        (
            "anthropic-ratelimit-requests-reset",
            "2026-01-01T00:00:01.123Z",
        ),
        ("anthropic-ratelimit-input-tokens-limit", "6000"),
        ("anthropic-ratelimit-input-tokens-remaining", "5000"),
    ]);
    let id = ProviderId::new("anthropic-1");
    let got = parse(&h, id.clone(), chrono::Utc::now());
    assert_eq!(got[0].reset_at.unwrap().timestamp_subsec_millis(), 123);
    assert_eq!(got[1].metric, "input_tokens");
    h.insert("anthropic-ratelimit-requests-reset", "bad".parse().unwrap());
    h.remove("anthropic-ratelimit-input-tokens-remaining");
    parse(&h, id, chrono::Utc::now());
}
