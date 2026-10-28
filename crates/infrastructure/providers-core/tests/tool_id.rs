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

#[test]
fn nested_invalid_ids_are_consumed_without_corrupting_the_next_field_or_call() {
    #[derive(Deserialize)]
    struct Call {
        id: ToolId,
        name: String,
    }
    for invalid in [
        r#"[]"#,
        r#"{}"#,
        r#"[{"nested":[null,true,{"id":"not-an-identity"}]}]"#,
        r#"{"nested":[{"deep":[1,2,3]}],"other":false}"#,
    ] {
        let body =
            format!(r#"[{{"id":{invalid},"name":"first"}},{{"id":"call_valid","name":"second"}}]"#);
        let calls: Vec<Call> = serde_json::from_str(&body).unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].id.is_invalid_type(), "{invalid}");
        assert_eq!(calls[0].id.canonical().as_deref(), Some(""));
        assert_eq!(calls[0].name, "first");
        assert_eq!(calls[1].id.as_str(), Some("call_valid"));
        assert_eq!(calls[1].name, "second");
    }
}

#[test]
fn explicit_empty_and_control_character_ids_remain_explicit_for_later_validation() {
    for value in ["", "call\ninvalid", "\u{0}", "call_é"] {
        let parsed: Wrapper = serde_json::from_value(serde_json::json!({"id":value})).unwrap();
        assert!(!parsed.id.is_invalid_type());
        assert_eq!(parsed.id.as_str(), Some(value));
        assert_eq!(parsed.id.canonical().as_deref(), Some(value));
        assert_eq!(parsed.id.delta().as_deref(), Some(value));
    }
}

#[test]
fn malformed_nested_ids_remain_deserialization_errors() {
    for body in [r#"{"id":[1,]}"#, r#"{"id":{"nested":}}"#] {
        assert!(serde_json::from_str::<Wrapper>(body).is_err(), "{body}");
    }
}
