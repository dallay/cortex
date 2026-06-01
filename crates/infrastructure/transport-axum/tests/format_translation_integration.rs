// format_translation_integration.rs
//
// Integration tests for the provider format translation layer.
//
// These tests verify the full adapter chain:
//   JSON body → adapter struct (serde) → CompletionRequest (domain) →
//   CompletionResponse (mock) → response struct → JSON body
//
// ACs covered:
//   SC-04 + SC-09: OpenAI round-trip (content preserved, object == "chat.completion")
//   SC-05 + SC-10: Anthropic round-trip (content[0].type == "text", stop_reason == "end_turn")
//   SC-01 + SC-02: No parse error on requests that include `tools` or `stream_options` fields

use rook_core::{CompletionResponse, MessageContent, ModelId, Role, TokenUsage};
use shared_kernel::{ProviderId, RequestId};
use transport_axum::{
    anthropic_adapter::{AnthropicMessagesRequest, AnthropicMessagesResponse},
    openai_adapter::{OpenAIChatRequest, OpenAIChatResponse},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mock_completion_response(content: &str) -> CompletionResponse {
    CompletionResponse {
        id: RequestId::new(),
        provider: ProviderId::new("test-provider"),
        model: ModelId::new("test-model"),
        content: content.to_string(),
        usage: TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            estimated_cost_usd: None,
        },
        latency_ms: 42,
    }
}

// ---------------------------------------------------------------------------
// OpenAI format round-trip
// ---------------------------------------------------------------------------

#[test]
fn openai_minimal_request_round_trip() {
    // Deserialize a minimal OpenAI request
    let json = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}]}"#;
    let req: OpenAIChatRequest = serde_json::from_str(json).expect("should parse minimal request");

    // Convert to domain
    let domain_req = rook_core::CompletionRequest::from(req);
    assert_eq!(domain_req.model.as_str(), "gpt-4o");
    assert_eq!(domain_req.messages.len(), 1);
    assert_eq!(domain_req.messages[0].role, Role::User);
    assert_eq!(
        domain_req.messages[0].content,
        MessageContent::Text("Hello".to_string())
    );

    // Build a mock response and convert back to OpenAI format
    let mock_resp = mock_completion_response("Hello back!");
    let openai_resp = OpenAIChatResponse::from(&mock_resp);
    let resp_json = serde_json::to_value(&openai_resp).expect("should serialize");

    // SC-09: object == "chat.completion"
    assert_eq!(resp_json["object"], "chat.completion");
    // SC-04: choices[0].message.content preserved
    assert_eq!(resp_json["choices"][0]["message"]["content"], "Hello back!");
}

#[test]
fn openai_request_with_tools_and_stream_options_does_not_error() {
    // SC-01: tools / stream_options must not cause a parse error (no 422)
    let json = r#"{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hi"}],
        "tools": [{"type": "function", "function": {"name": "get_weather"}}],
        "tool_choice": "auto",
        "stream_options": {"include_usage": true},
        "response_format": {"type": "text"}
    }"#;

    let req: OpenAIChatRequest =
        serde_json::from_str(json).expect("should parse request with tools and stream_options");

    assert_eq!(req.model, "gpt-4o");
    assert!(req.tools.is_some(), "tools field should be present");
    assert!(
        req.stream_options.is_some(),
        "stream_options should be present"
    );
    assert!(
        req.response_format.is_some(),
        "response_format should be present"
    );
}

#[test]
fn openai_response_has_correct_structure() {
    let mock_resp = mock_completion_response("The answer is 42.");
    let openai_resp = OpenAIChatResponse::from(&mock_resp);
    let json = serde_json::to_value(&openai_resp).expect("should serialize");

    assert_eq!(json["object"], "chat.completion");
    assert!(json["id"].is_string());
    assert!(json["created"].is_number());
    assert_eq!(json["choices"][0]["message"]["role"], "assistant");
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "The answer is 42."
    );
    assert_eq!(json["choices"][0]["finish_reason"], "stop");
    assert_eq!(json["usage"]["prompt_tokens"], 10);
    assert_eq!(json["usage"]["completion_tokens"], 5);
}

// ---------------------------------------------------------------------------
// Anthropic format round-trip
// ---------------------------------------------------------------------------

#[test]
fn anthropic_minimal_request_round_trip() {
    // Deserialize a minimal Anthropic request
    let json = r#"{"model":"claude-opus-4-5","messages":[{"role":"user","content":"Hello"}],"max_tokens":1024}"#;
    let req: AnthropicMessagesRequest =
        serde_json::from_str(json).expect("should parse minimal Anthropic request");

    // Convert to domain
    let domain_req = rook_core::CompletionRequest::from(req);
    assert_eq!(domain_req.model.as_str(), "claude-opus-4-5");
    assert_eq!(domain_req.messages[0].role, Role::User);

    // Build a mock response and convert back to Anthropic format
    let mock_resp = mock_completion_response("Bonjour!");
    let anthropic_resp = AnthropicMessagesResponse::from(&mock_resp);
    let resp_json = serde_json::to_value(&anthropic_resp).expect("should serialize");

    // SC-10: content[0].type == "text"
    assert_eq!(resp_json["content"][0]["type"], "text");
    // SC-05: content preserved
    assert_eq!(resp_json["content"][0]["text"], "Bonjour!");
    // stop_reason == "end_turn"
    assert_eq!(resp_json["stop_reason"], "end_turn");
}

#[test]
fn anthropic_request_with_tools_does_not_error() {
    // SC-02: tools / tool_choice must not cause a parse error (no 422)
    let json = r#"{
        "model": "claude-opus-4-5",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What is the weather?"}],
        "tools": [{"name": "get_weather", "description": "Get weather", "input_schema": {}}],
        "tool_choice": {"type": "auto"}
    }"#;

    let req: AnthropicMessagesRequest =
        serde_json::from_str(json).expect("should parse request with tools");

    assert_eq!(req.model, "claude-opus-4-5");
    assert!(req.tools.is_some(), "tools field should be present");
    assert!(req.tool_choice.is_some(), "tool_choice should be present");
}

#[test]
fn anthropic_system_field_prepends_system_message() {
    // SC-16: system field at the top level is prepended as a Role::System message
    let json = r#"{
        "model": "claude-opus-4-5",
        "max_tokens": 512,
        "system": "You are a helpful assistant.",
        "messages": [{"role": "user", "content": "Hi"}]
    }"#;

    let req: AnthropicMessagesRequest =
        serde_json::from_str(json).expect("should parse request with system field");
    let domain_req = rook_core::CompletionRequest::from(req);

    // Should have 2 messages: system first, then user
    assert_eq!(domain_req.messages.len(), 2);
    assert_eq!(domain_req.messages[0].role, Role::System);
    assert_eq!(
        domain_req.messages[0].content,
        MessageContent::Text("You are a helpful assistant.".to_string())
    );
    assert_eq!(domain_req.messages[1].role, Role::User);
}

#[test]
fn anthropic_response_has_correct_structure() {
    let mock_resp = mock_completion_response("42 is the answer.");
    let anthropic_resp = AnthropicMessagesResponse::from(&mock_resp);
    let json = serde_json::to_value(&anthropic_resp).expect("should serialize");

    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "assistant");
    assert_eq!(json["stop_reason"], "end_turn");
    assert_eq!(json["content"][0]["type"], "text");
    assert_eq!(json["content"][0]["text"], "42 is the answer.");
    assert_eq!(json["usage"]["input_tokens"], 10);
    assert_eq!(json["usage"]["output_tokens"], 5);
}
