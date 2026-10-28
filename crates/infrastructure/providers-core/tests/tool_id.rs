use providers_core::tool_id::ToolId;
use serde::Deserialize;

#[derive(Deserialize)]
struct Wrapper {
    #[serde(default)]
    id: ToolId,
}

#[test]
fn test_tool_id_variants_methods() {
    let missing = ToolId::Missing;
    assert!(!missing.is_invalid_type());
    assert_eq!(missing.as_str(), None);
    assert_eq!(missing.canonical(), None);
    assert_eq!(missing.delta(), None);

    let null = ToolId::Null;
    assert!(!null.is_invalid_type());
    assert_eq!(null.as_str(), None);
    assert_eq!(null.canonical(), Some("".to_string()));
    assert_eq!(null.delta(), None);

    let valid_str = ToolId::String("call_123".to_string());
    assert!(!valid_str.is_invalid_type());
    assert_eq!(valid_str.as_str(), Some("call_123"));
    assert_eq!(valid_str.canonical(), Some("call_123".to_string()));
    assert_eq!(valid_str.delta(), Some("call_123".to_string()));

    let invalid = ToolId::Invalid;
    assert!(invalid.is_invalid_type());
    assert_eq!(invalid.as_str(), None);
    assert_eq!(invalid.canonical(), Some("".to_string()));
    assert_eq!(invalid.delta(), Some("".to_string()));
}

#[test]
fn test_tool_id_deserialization() {
    let w: Wrapper = serde_json::from_str("{}").unwrap();
    assert!(matches!(w.id, ToolId::Missing));

    let w: Wrapper = serde_json::from_str(r#"{"id": null}"#).unwrap();
    assert!(matches!(w.id, ToolId::Null));

    let w: Wrapper = serde_json::from_str(r#"{"id": "call_abc"}"#).unwrap();
    assert!(matches!(w.id, ToolId::String(ref s) if s == "call_abc"));

    for json in [
        r#"{"id": true}"#,
        r#"{"id": 123}"#,
        r#"{"id": -456}"#,
        r#"{"id": 3.14}"#,
        r#"{"id": [1, 2]}"#,
        r#"{"id": {"a": 1}}"#,
    ] {
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert!(matches!(w.id, ToolId::Invalid), "failed for {json}");
    }
}
