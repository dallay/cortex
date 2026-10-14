use huginn_runtime::model::OpenAiProvider;

#[test]
fn default_constructor_rejects_all_plaintext_endpoints() {
    for endpoint in [
        "http://example.com/v1",
        "http://10.0.0.1/v1",
        "http://127.0.0.1/v1",
    ] {
        assert!(
            OpenAiProvider::new(endpoint, "m".into(), None, 1).is_err(),
            "{endpoint}"
        );
    }
}

#[test]
fn remote_finish_reason_is_not_echoed() {
    let mut parser = huginn_runtime::model::SseParser::default();
    let error = parser
        .push(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"sentinel-secret\"}]}\n\n")
        .unwrap_err();
    assert!(!error.to_string().contains("sentinel-secret"));
}

#[test]
fn explicit_loopback_exception_is_narrow() {
    for endpoint in [
        "http://10.0.0.1/v1",
        "http://192.168.1.1/v1",
        "http://example.com/v1",
        "http://localhost/v1",
        "http://[::ffff:192.168.0.1]/v1",
        "https://user:secret@example.com/v1",
        "https://example.com/v1?key=secret",
        "https://example.com/v1#secret",
    ] {
        assert!(
            OpenAiProvider::with_transport(endpoint, "m".into(), None, 1, true).is_err(),
            "{endpoint}"
        );
    }
    for endpoint in [
        "http://127.0.0.1/v1",
        "http://[::1]/v1",
        "https://example.com/v1",
    ] {
        assert!(OpenAiProvider::with_transport(endpoint, "m".into(), None, 1, true).is_ok());
    }
}
