use rook_core::{CompletionRequest, RequiredCapabilities};
use shared_kernel::RequestId;

fn request(tools: Option<serde_json::Value>) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: "test-model".into(),
        messages: Vec::new(),
        stream: false,
        max_tokens: None,
        temperature: None,
        tools,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: Default::default(),
    }
}

#[test]
fn tools_requirement_preserves_missing_non_array_and_empty_array_semantics() {
    assert!(!RequiredCapabilities::from_request(&request(None)).tools);
    assert!(!RequiredCapabilities::from_request(&request(Some(serde_json::Value::Null))).tools);
    assert!(!RequiredCapabilities::from_request(&request(Some(serde_json::json!([])))).tools);
    assert!(
        RequiredCapabilities::from_request(&request(Some(serde_json::json!([{
            "type": "function"
        }]))))
        .tools
    );
    assert!(
        RequiredCapabilities::from_request(&request(Some(serde_json::json!({
            "type": "function"
        }))))
        .tools
    );
}
