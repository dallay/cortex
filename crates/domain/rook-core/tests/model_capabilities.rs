//! Contract coverage for the public capability model: evidence defaults,
//! override application, adapter ceilings and request requirement detection.

use rook_core::{
    CapabilityEvidence, CapabilityOverrides, CapabilityState, CompletionRequest, ContextEvidence,
    Message, MessageContent, MessageToolCall, ModelCapabilities, ModelDeclaration, RequestMetadata,
    RequiredCapabilities, Role,
};
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
        metadata: RequestMetadata {
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

fn request_with(
    tools: Option<serde_json::Value>,
    tool_choice: Option<serde_json::Value>,
    messages: Vec<Message>,
    stream: bool,
) -> CompletionRequest {
    CompletionRequest {
        messages,
        stream,
        tools,
        tool_choice,
        ..request(None)
    }
}

fn text_message(text: &str) -> Message {
    Message {
        role: Role::User,
        content: MessageContent::Text(text.to_string()),
        tool_calls: Vec::new(),
    }
}

fn evidence(state: CapabilityState, source: &str) -> CapabilityEvidence {
    CapabilityEvidence {
        state,
        source: source.to_string(),
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

#[test]
fn capability_state_defaults_to_unknown_and_serializes_in_snake_case() {
    assert_eq!(CapabilityState::default(), CapabilityState::Unknown);
    assert_eq!(
        serde_json::to_string(&CapabilityState::Supported).expect("serialize supported"),
        "\"supported\""
    );
    assert_eq!(
        serde_json::to_string(&CapabilityState::Unsupported).expect("serialize unsupported"),
        "\"unsupported\""
    );
    assert_eq!(
        serde_json::to_string(&CapabilityState::Unknown).expect("serialize unknown"),
        "\"unknown\""
    );
    assert_eq!(
        serde_json::from_str::<CapabilityState>("\"unsupported\"").expect("deserialize state"),
        CapabilityState::Unsupported
    );
}

#[test]
fn capability_evidence_defaults_to_unknown_with_unknown_source() {
    let default = CapabilityEvidence::default();
    assert_eq!(default.state, CapabilityState::Unknown);
    assert_eq!(default.source, "unknown");
}

#[test]
fn model_capabilities_default_leaves_every_capability_unknown() {
    let capabilities = ModelCapabilities::default();
    assert_eq!(capabilities.tools.state, CapabilityState::Unknown);
    assert_eq!(capabilities.vision.state, CapabilityState::Unknown);
    assert_eq!(capabilities.reasoning.state, CapabilityState::Unknown);
    assert_eq!(capabilities.streaming.state, CapabilityState::Unknown);
    assert!(capabilities.context_tokens.is_none());
}

#[test]
fn model_capabilities_deserialize_with_defaults_and_reject_unknown_fields() {
    let capabilities: ModelCapabilities = serde_json::from_str("{}").expect("deserialize defaults");
    assert_eq!(capabilities, ModelCapabilities::default());

    let capabilities: ModelCapabilities = serde_json::from_str(
        r#"{"tools":{"state":"supported","source":"adapter"},"context_tokens":{"value":128000,"source":"adapter"}}"#,
    )
    .expect("deserialize capabilities");
    assert_eq!(capabilities.tools.state, CapabilityState::Supported);
    assert_eq!(capabilities.tools.source, "adapter");
    assert_eq!(
        capabilities.context_tokens.as_ref().map(|c| c.value),
        Some(128_000)
    );

    let error = serde_json::from_str::<ModelCapabilities>(r#"{"nope":true}"#)
        .expect_err("unknown field must be rejected");
    assert!(
        error.to_string().contains("unknown field"),
        "unexpected error: {error}"
    );
}

#[test]
fn capability_overrides_deserialize_inheriting_missing_fields() {
    let overrides: CapabilityOverrides =
        serde_json::from_str(r#"{"vision":"unsupported"}"#).expect("deserialize overrides");
    assert_eq!(overrides.tools, None);
    assert_eq!(overrides.vision, Some(CapabilityState::Unsupported));
    assert_eq!(overrides.reasoning, None);
    assert_eq!(overrides.streaming, None);
    assert_eq!(overrides.context_tokens, None);

    let overrides: CapabilityOverrides =
        serde_json::from_str(r#"{"context_tokens":4096}"#).expect("deserialize overrides");
    assert_eq!(overrides.context_tokens, Some(4096));

    let error = serde_json::from_str::<CapabilityOverrides>(r#"{"nope":true}"#)
        .expect_err("unknown field must be rejected");
    assert!(
        error.to_string().contains("unknown field"),
        "unexpected error: {error}"
    );
}

#[test]
fn model_declaration_accepts_the_legacy_provider_kind_alias() {
    let declaration: ModelDeclaration =
        serde_json::from_str(r#"{"provider_id":"openai","model_id":"gpt-4o"}"#)
            .expect("deserialize declaration");
    assert_eq!(declaration.provider_id, "openai");
    assert_eq!(declaration.model_id, "gpt-4o");
    assert!(declaration.connection_id.is_none());
    assert!(declaration.capabilities.tools.is_none());
    assert!(declaration.capabilities.vision.is_none());
    assert!(declaration.capabilities.reasoning.is_none());
    assert!(declaration.capabilities.streaming.is_none());
    assert!(declaration.capabilities.context_tokens.is_none());

    let legacy: ModelDeclaration = serde_json::from_str(
        r#"{"provider_kind":"openai","model_id":"gpt-4o","capabilities":{"tools":"supported"}}"#,
    )
    .expect("deserialize legacy declaration");
    assert_eq!(legacy.provider_id, "openai");
    assert_eq!(legacy.capabilities.tools, Some(CapabilityState::Supported));
}

#[test]
fn apply_sets_only_the_capabilities_present_in_the_patch() {
    // Vision and reasoning are seeded with non-default evidence. A patch that
    // omits them must leave that state and that source untouched.
    let mut capabilities = ModelCapabilities {
        vision: evidence(CapabilityState::Unsupported, "declaration"),
        reasoning: evidence(CapabilityState::Supported, "declaration"),
        ..ModelCapabilities::default()
    };
    capabilities.apply(
        &CapabilityOverrides {
            tools: Some(CapabilityState::Supported),
            context_tokens: Some(8_000),
            ..CapabilityOverrides::default()
        },
        "operator",
    );

    assert_eq!(capabilities.tools.state, CapabilityState::Supported);
    assert_eq!(capabilities.tools.source, "operator");
    assert_eq!(
        capabilities.context_tokens.as_ref().map(|c| c.value),
        Some(8_000)
    );
    assert_eq!(
        capabilities
            .context_tokens
            .as_ref()
            .map(|c| c.source.as_str()),
        Some("operator")
    );
    // Capabilities omitted from the patch keep their seeded state and source.
    assert_eq!(capabilities.vision.state, CapabilityState::Unsupported);
    assert_eq!(capabilities.vision.source, "declaration");
    assert_eq!(capabilities.reasoning.state, CapabilityState::Supported);
    assert_eq!(capabilities.reasoning.source, "declaration");
    // A capability that was never seeded stays at the unknown default.
    assert_eq!(capabilities.streaming.state, CapabilityState::Unknown);
    assert_eq!(capabilities.streaming.source, "unknown");
}

#[test]
fn apply_leaves_context_tokens_untouched_when_the_patch_omits_it() {
    let mut capabilities = ModelCapabilities {
        context_tokens: Some(ContextEvidence {
            value: 4_000,
            source: "declaration".to_string(),
        }),
        ..ModelCapabilities::default()
    };
    capabilities.apply(
        &CapabilityOverrides {
            vision: Some(CapabilityState::Supported),
            ..CapabilityOverrides::default()
        },
        "operator",
    );

    assert_eq!(
        capabilities.context_tokens.as_ref().map(|c| c.value),
        Some(4_000)
    );
    assert_eq!(
        capabilities
            .context_tokens
            .as_ref()
            .map(|c| c.source.as_str()),
        Some("declaration")
    );
    assert_eq!(capabilities.vision.state, CapabilityState::Supported);
    assert_eq!(capabilities.vision.source, "operator");
}

#[test]
fn apply_records_the_source_of_every_override() {
    let mut capabilities = ModelCapabilities::default();
    capabilities.apply(
        &CapabilityOverrides {
            tools: Some(CapabilityState::Unsupported),
            vision: Some(CapabilityState::Supported),
            reasoning: Some(CapabilityState::Supported),
            streaming: Some(CapabilityState::Supported),
            context_tokens: Some(1_000),
        },
        "adapter-probe",
    );

    assert_eq!(capabilities.tools.source, "adapter-probe");
    assert_eq!(capabilities.vision.source, "adapter-probe");
    assert_eq!(capabilities.reasoning.source, "adapter-probe");
    assert_eq!(capabilities.streaming.source, "adapter-probe");
    assert_eq!(
        capabilities
            .context_tokens
            .as_ref()
            .map(|c| c.source.as_str()),
        Some("adapter-probe")
    );
}

#[test]
fn constrain_downgrades_supported_capabilities_to_the_adapter_ceiling() {
    let mut capabilities = ModelCapabilities {
        tools: evidence(CapabilityState::Supported, "declaration"),
        vision: evidence(CapabilityState::Supported, "declaration"),
        reasoning: evidence(CapabilityState::Supported, "declaration"),
        streaming: evidence(CapabilityState::Supported, "declaration"),
        context_tokens: None,
    };
    let ceiling = ModelCapabilities {
        tools: evidence(CapabilityState::Unsupported, "adapter"),
        vision: evidence(CapabilityState::Supported, "adapter"),
        reasoning: evidence(CapabilityState::Unknown, "adapter"),
        streaming: evidence(CapabilityState::Unsupported, "adapter"),
        context_tokens: None,
    };

    capabilities.constrain(&ceiling);

    assert_eq!(capabilities.tools.state, CapabilityState::Unsupported);
    assert_eq!(capabilities.tools.source, "adapter");
    assert_eq!(capabilities.vision.state, CapabilityState::Supported);
    assert_eq!(capabilities.vision.source, "declaration");
    assert_eq!(capabilities.reasoning.state, CapabilityState::Unknown);
    assert_eq!(capabilities.reasoning.source, "adapter");
    assert_eq!(capabilities.streaming.state, CapabilityState::Unsupported);
    assert_eq!(capabilities.streaming.source, "adapter");
}

#[test]
fn constrain_leaves_unsupported_and_unknown_capabilities_untouched() {
    let mut capabilities = ModelCapabilities {
        tools: evidence(CapabilityState::Unsupported, "declaration"),
        vision: evidence(CapabilityState::Unknown, "declaration"),
        ..ModelCapabilities::default()
    };
    let ceiling = ModelCapabilities {
        tools: evidence(CapabilityState::Supported, "adapter"),
        vision: evidence(CapabilityState::Supported, "adapter"),
        ..ModelCapabilities::default()
    };

    capabilities.constrain(&ceiling);

    assert_eq!(capabilities.tools.state, CapabilityState::Unsupported);
    assert_eq!(capabilities.tools.source, "declaration");
    assert_eq!(capabilities.vision.state, CapabilityState::Unknown);
    assert_eq!(capabilities.vision.source, "declaration");
}

#[test]
fn required_capabilities_from_request_detects_tools_and_streaming() {
    let required = RequiredCapabilities::from_request(&request_with(
        None,
        None,
        vec![text_message("hello")],
        true,
    ));
    assert!(!required.tools);
    assert!(required.streaming);
    assert!(!required.vision);
    assert!(!required.reasoning);

    let required = RequiredCapabilities::from_request(&request_with(
        Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}])),
        None,
        vec![text_message("hello")],
        false,
    ));
    assert!(required.tools);
    assert!(!required.streaming);
}

#[test]
fn null_and_empty_tool_payloads_do_not_require_tools() {
    let required = RequiredCapabilities::from_request(&request_with(
        Some(serde_json::Value::Null),
        Some(serde_json::Value::Null),
        vec![text_message("hello")],
        false,
    ));
    assert!(!required.tools);

    let required = RequiredCapabilities::from_request(&request_with(
        Some(serde_json::json!([])),
        None,
        vec![text_message("hello")],
        false,
    ));
    assert!(!required.tools);
}

#[test]
fn required_capabilities_respect_tool_choice_none() {
    let required = RequiredCapabilities::from_request(&request_with(
        None,
        Some(serde_json::json!("none")),
        vec![text_message("hello")],
        false,
    ));
    assert!(!required.tools);

    let required = RequiredCapabilities::from_request(&request_with(
        None,
        Some(serde_json::json!({"type":"none"})),
        vec![text_message("hello")],
        false,
    ));
    assert!(!required.tools);

    let required = RequiredCapabilities::from_request(&request_with(
        None,
        Some(serde_json::json!("auto")),
        vec![text_message("hello")],
        false,
    ));
    assert!(required.tools);

    let required = RequiredCapabilities::from_request(&request_with(
        None,
        Some(serde_json::json!({"type":"function"})),
        vec![text_message("hello")],
        false,
    ));
    assert!(required.tools);
}

#[test]
fn required_capabilities_detect_tool_calls_and_tool_content_in_messages() {
    let required = RequiredCapabilities::from_request(&request_with(
        None,
        None,
        vec![Message {
            role: Role::Assistant,
            content: MessageContent::Text("calling".to_string()),
            tool_calls: vec![MessageToolCall {
                id: Some("call_1".to_string()),
                name: "lookup".to_string(),
                arguments: serde_json::json!({}),
            }],
        }],
        false,
    ));
    assert!(required.tools);

    let required = RequiredCapabilities::from_request(&request_with(
        None,
        None,
        vec![Message {
            role: Role::User,
            content: MessageContent::ToolUse {
                id: "tool_1".to_string(),
                name: "lookup".to_string(),
                input: serde_json::json!({}),
            },
            tool_calls: Vec::new(),
        }],
        false,
    ));
    assert!(required.tools);
}

#[test]
fn check_distinguishes_missing_evidence_from_incompatibility() {
    let unknown = ModelCapabilities {
        tools: evidence(CapabilityState::Unknown, "adapter"),
        ..ModelCapabilities::default()
    };
    let unsupported = ModelCapabilities {
        tools: evidence(CapabilityState::Unsupported, "adapter"),
        ..ModelCapabilities::default()
    };
    let supported = ModelCapabilities {
        tools: evidence(CapabilityState::Supported, "adapter"),
        ..ModelCapabilities::default()
    };

    let required_tools = RequiredCapabilities {
        tools: true,
        ..RequiredCapabilities::default()
    };
    assert_eq!(
        required_tools.check(&unknown),
        Err("required_capability_unknown")
    );
    assert_eq!(
        required_tools.check(&unsupported),
        Err("required_capability_unsupported")
    );
    assert_eq!(required_tools.check(&supported), Ok(()));
    // A capability that is not required never fails the check.
    assert_eq!(RequiredCapabilities::default().check(&unknown), Ok(()));
}

#[test]
fn check_reports_the_first_unsatisfied_capability() {
    let capabilities = ModelCapabilities {
        tools: evidence(CapabilityState::Supported, "adapter"),
        vision: evidence(CapabilityState::Unsupported, "adapter"),
        reasoning: evidence(CapabilityState::Unknown, "adapter"),
        streaming: evidence(CapabilityState::Supported, "adapter"),
        context_tokens: None,
    };
    let required = RequiredCapabilities {
        tools: true,
        vision: true,
        reasoning: true,
        streaming: true,
    };
    // Vision is evaluated before reasoning, so its unsupported verdict wins.
    assert_eq!(
        required.check(&capabilities),
        Err("required_capability_unsupported")
    );

    let required = RequiredCapabilities {
        tools: true,
        vision: false,
        reasoning: true,
        streaming: false,
    };
    assert_eq!(
        required.check(&capabilities),
        Err("required_capability_unknown")
    );
}

#[test]
fn check_requires_streaming_only_when_the_request_streams() {
    let capabilities = ModelCapabilities {
        streaming: evidence(CapabilityState::Unsupported, "adapter"),
        ..ModelCapabilities::default()
    };
    let required = RequiredCapabilities {
        streaming: true,
        ..RequiredCapabilities::default()
    };
    assert_eq!(
        required.check(&capabilities),
        Err("required_capability_unsupported")
    );
    assert_eq!(RequiredCapabilities::default().check(&capabilities), Ok(()));
}
