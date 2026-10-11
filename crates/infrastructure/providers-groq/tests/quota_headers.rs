use providers_groq::parse_groq_rate_limit_headers as parse;
use reqwest::header::HeaderMap;
use shared_kernel::ProviderId;
fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(k, v)| (k.parse().unwrap(), v.parse().unwrap()))
        .collect()
}
#[test]
fn distinguishes_rpd_tpm_and_handles_missing_malformed_and_reset() {
    let mut h = headers(&[
        ("x-ratelimit-limit-requests", "14400"),
        ("x-ratelimit-remaining-requests", "14370"),
        ("x-ratelimit-reset-requests", "2m59.56s"),
        ("x-ratelimit-limit-tokens", "18000"),
        ("x-ratelimit-remaining-tokens", "17997"),
    ]);
    let id = ProviderId::new("groq-1");
    let got = parse(&h, id.clone(), chrono::Utc::now());
    assert_eq!(got[0].window.as_deref(), Some("day"));
    assert_eq!(got[0].reset_after_milliseconds, Some(179_560));
    assert_eq!(got[1].window.as_deref(), Some("minute"));
    h.insert("x-ratelimit-reset-requests", "bad".parse().unwrap());
    h.insert("x-ratelimit-limit-tokens", "NaN".parse().unwrap());
    h.remove("x-ratelimit-remaining-tokens");
    let got = parse(&h, id.clone(), chrono::Utc::now());
    assert_eq!(got[0].reset_after_milliseconds, None);
}
