use huginn_core::{
    providers::{supports_openai_reasoning_effort, supports_openai_vision},
    siwc::{SiwcCredentials, SIWC_REQUIRED_SCOPES},
    ImageAttachment, ImageFormat, Message, Session, MAX_IMAGE_BYTES,
};

fn minimal_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes
}

fn minimal_jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&[3, 0x01, 0x11, 0, 0x02, 0x11, 0]);
    bytes
}

fn minimal_webp(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0; 30];
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[8..12].copy_from_slice(b"WEBP");
    bytes[12..16].copy_from_slice(b"VP8X");
    bytes[24..27].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
    bytes[27..30].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
    bytes
}

fn valid_creds() -> SiwcCredentials {
    SiwcCredentials {
        host_id: "host-1".into(),
        client_id: "client-issued-1".into(),
        access_token: "access-secret".into(),
        refresh_token: "refresh-secret".into(),
        expires_at_unix: 1_900_000_000,
        granted_scopes: SIWC_REQUIRED_SCOPES
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        account_sub: Some("user-sub-1".into()),
        account_email: Some("dev@example.com".into()),
    }
}

#[test]
fn validates_magic_bytes_dimensions_and_redacts_debug() {
    let img = ImageAttachment::from_bytes(minimal_png(640, 480), None).unwrap();
    assert_eq!(img.format, ImageFormat::Png);
    assert_eq!(img.dimensions, Some((640, 480)));
    assert_eq!(img.chip_label(1), "[Image 1: PNG 640x480, 29 B]");
    let debug = format!("{img:?}");
    assert!(debug.contains("byte_len: 29"));
    assert!(!debug.contains("137, 80, 78"));
}

#[test]
fn parses_jpeg_and_webp_dimensions_and_formats_attachment_sizes() {
    let jpeg = ImageAttachment::from_bytes(minimal_jpeg(640, 480), None).unwrap();
    assert_eq!(jpeg.format, ImageFormat::Jpeg);
    assert_eq!(jpeg.dimensions, Some((640, 480)));

    let webp = ImageAttachment::from_bytes(minimal_webp(320, 240), None).unwrap();
    assert_eq!(webp.format, ImageFormat::WebP);
    assert_eq!(webp.dimensions, Some((320, 240)));

    let mut kilobyte_png = minimal_png(1, 1);
    kilobyte_png.resize(1024, 0);
    let kilobyte = ImageAttachment::from_bytes(kilobyte_png, None).unwrap();
    assert_eq!(kilobyte.chip_label(2), "[Image 2: PNG 1x1, 1 KB]");

    let mut megabyte_png = minimal_png(1, 1);
    megabyte_png.resize(1024 * 1024, 0);
    let megabyte = ImageAttachment::from_bytes(megabyte_png, None).unwrap();
    assert_eq!(megabyte.chip_label(3), "[Image 3: PNG 1x1, 1.0 MB]");
}

#[test]
fn malformed_jpeg_and_truncated_webp_keep_dimensions_unknown() {
    let zero_width_jpeg = ImageAttachment::from_bytes(minimal_jpeg(0, 480), None).unwrap();
    assert_eq!(zero_width_jpeg.dimensions, None);

    let mut invalid_segment_jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01];
    invalid_segment_jpeg.resize(16, 0);
    let invalid_segment = ImageAttachment::from_bytes(invalid_segment_jpeg, None).unwrap();
    assert_eq!(invalid_segment.dimensions, None);

    let mut truncated_webp = minimal_webp(320, 240);
    truncated_webp.truncate(29);
    let truncated = ImageAttachment::from_bytes(truncated_webp, None).unwrap();
    assert_eq!(truncated.format, ImageFormat::WebP);
    assert_eq!(truncated.dimensions, None);
}

#[test]
fn rejects_unsupported_and_oversized_payloads() {
    assert!(ImageAttachment::from_bytes(b"not an image".to_vec(), None).is_err());
    let mut huge = minimal_png(1, 1);
    huge.resize(MAX_IMAGE_BYTES + 1, 0);
    assert!(ImageAttachment::from_bytes(huge, None).is_err());
}

#[test]
fn session_for_persistence_strips_image_bytes_and_preserves_marker() {
    let mut session = Session::new(std::path::PathBuf::from("/tmp"));
    let img = ImageAttachment::from_bytes(minimal_png(16, 16), None).unwrap();
    session
        .messages
        .push(Message::user_with_images("describe this", vec![img]));
    assert_eq!(session.in_memory_image_bytes(), 29);

    let persisted = session.for_persistence();
    assert_eq!(persisted.in_memory_image_bytes(), 0);
    assert!(persisted.messages[0].images.is_empty());
    assert!(persisted.messages[0]
        .content
        .contains("[Image attachment omitted from persisted history]"));

    let json = serde_json::to_string(&persisted).unwrap();
    assert!(!json.contains("images"));
}

#[test]
fn vision_and_reasoning_capability_gates_enforce_exact_verified_allowlists() {
    assert!(supports_openai_vision(
        "openai",
        Some("https://api.openai.com/v1"),
        "gpt-4o"
    ));
    assert!(supports_openai_vision(
        "openai",
        Some("https://api.openai.com/v1"),
        "gpt-5"
    ));
    assert!(supports_openai_vision(
        "openai",
        Some("https://api.openai.com/v1"),
        "o3"
    ));
    assert!(supports_openai_vision(
        "openai",
        Some("https://api.openai.com/v1"),
        "o4-mini"
    ));
    // Responses-only, audio, text-only, and non-/v1 endpoints must be rejected.
    for rejected in [
        "o3-pro",
        "o3-mini",
        "o3-deep-research",
        "o4-mini-deep-research",
        "gpt-5-pro",
        "gpt-5-codex",
        "gpt-4o-audio-preview",
        "o3-2025-99-99",
    ] {
        assert!(
            !supports_openai_vision("openai", Some("https://api.openai.com/v1"), rejected),
            "expected {rejected} to be excluded from Chat Completions vision allowlist"
        );
    }
    assert!(!supports_openai_vision(
        "openai",
        Some("https://api.openai.com"),
        "gpt-4o"
    ));
    assert!(!supports_openai_vision(
        "openai",
        Some("https://custom.example.com/v1"),
        "gpt-4o"
    ));
    assert!(!supports_openai_vision(
        "ollama",
        Some("http://127.0.0.1:11434"),
        "llava"
    ));

    // Reasoning allowlist requires /v1 and exact verified snapshots.
    assert!(supports_openai_reasoning_effort(
        "openai",
        Some("https://api.openai.com/v1"),
        "gpt-5-mini-2025-08-07"
    ));
    assert!(!supports_openai_reasoning_effort(
        "openai",
        Some("https://api.openai.com"),
        "o3"
    ));
    assert!(!supports_openai_reasoning_effort(
        "openai",
        Some("https://api.openai.com/v1"),
        "o3-2025-99-99"
    ));
}

#[test]
fn siwc_validates_required_scopes_and_redacts_tokens_in_debug() {
    let creds = valid_creds();
    assert!(creds.validate_grants().is_ok());
    let debug = format!("{creds:?}");
    assert!(!debug.contains("access-secret"));
    assert!(!debug.contains("refresh-secret"));
    assert!(debug.contains("[REDACTED]"));

    let mut missing_scope = creds;
    missing_scope.granted_scopes.pop();
    assert!(missing_scope.validate_grants().is_err());
}

#[test]
fn siwc_secret_json_round_trips_and_rejects_malformed_or_under_scoped_credentials() {
    let creds = valid_creds();
    let secret_json = creds.to_secret_json().unwrap();
    assert_eq!(
        SiwcCredentials::from_secret_json(&secret_json).unwrap(),
        creds
    );
    assert!(SiwcCredentials::from_secret_json("not JSON").is_err());

    let mut missing_scope = valid_creds();
    missing_scope.granted_scopes.pop();
    let secret_json = serde_json::to_string(&missing_scope).unwrap();
    let error = SiwcCredentials::from_secret_json(&secret_json).unwrap_err();
    assert!(error.to_string().contains("missing required scope"));
}

#[test]
fn siwc_refresh_threshold_triggers_within_sixty_seconds() {
    let creds = valid_creds();
    assert!(!creds.needs_refresh(1_899_999_900));
    assert!(creds.needs_refresh(1_899_999_940));
    assert!(creds.needs_refresh(1_900_000_000));
}
