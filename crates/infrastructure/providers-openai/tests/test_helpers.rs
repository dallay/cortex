// Shared test helpers for provider tests to reduce duplication

use rook_core::{CompletionRequest, Message, MessageContent, RequestMetadata, Role};
use shared_kernel::RequestId;

/// Creates a default CompletionRequest for testing
pub fn create_test_request(model: &str, stream: bool) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: shared_kernel::ModelId::new(model),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("Hi".to_string()),
        }],
        stream,
        max_tokens: Some(100),
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    }
}
