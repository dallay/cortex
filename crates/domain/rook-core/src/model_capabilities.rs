use serde::{Deserialize, Serialize};

use crate::{CompletionRequest, ConnectionId, MessageContent};

/// Absence of evidence never means support.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    pub state: CapabilityState,
    pub source: String,
}

impl Default for CapabilityEvidence {
    fn default() -> Self {
        Self {
            state: CapabilityState::Unknown,
            source: "unknown".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEvidence {
    pub value: u64,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelCapabilities {
    pub tools: CapabilityEvidence,
    pub vision: CapabilityEvidence,
    pub reasoning: CapabilityEvidence,
    pub streaming: CapabilityEvidence,
    pub context_tokens: Option<ContextEvidence>,
}

/// Optional fields inherit; an explicit `unknown` suppresses lower-priority evidence.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityOverrides {
    pub tools: Option<CapabilityState>,
    pub vision: Option<CapabilityState>,
    pub reasoning: Option<CapabilityState>,
    pub streaming: Option<CapabilityState>,
    pub context_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDeclaration {
    /// Canonical provider descriptor id (legacy `provider_kind` spellings are
    /// accepted on read through the table's alias map).
    #[serde(alias = "provider_kind")]
    pub provider_id: String,
    pub model_id: String,
    #[serde(default)]
    pub connection_id: Option<ConnectionId>,
    #[serde(default)]
    pub capabilities: CapabilityOverrides,
}

impl ModelCapabilities {
    pub fn apply(&mut self, patch: &CapabilityOverrides, source: &str) {
        for (target, state) in [
            (&mut self.tools, patch.tools),
            (&mut self.vision, patch.vision),
            (&mut self.reasoning, patch.reasoning),
            (&mut self.streaming, patch.streaming),
        ] {
            if let Some(state) = state {
                *target = CapabilityEvidence {
                    state,
                    source: source.into(),
                };
            }
        }
        if let Some(value) = patch.context_tokens {
            self.context_tokens = Some(ContextEvidence {
                value,
                source: source.into(),
            });
        }
    }

    /// A declaration cannot enable a capability absent from the adapter.
    pub fn constrain(&mut self, ceiling: &Self) {
        for (target, adapter) in [
            (&mut self.tools, &ceiling.tools),
            (&mut self.vision, &ceiling.vision),
            (&mut self.reasoning, &ceiling.reasoning),
            (&mut self.streaming, &ceiling.streaming),
        ] {
            if target.state == CapabilityState::Supported
                && adapter.state != CapabilityState::Supported
            {
                *target = adapter.clone();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RequiredCapabilities {
    pub tools: bool,
    pub vision: bool,
    pub reasoning: bool,
    pub streaming: bool,
}

impl RequiredCapabilities {
    pub fn from_request(req: &CompletionRequest) -> Self {
        Self {
            tools: req.tools.as_ref().is_some_and(|v| {
                !v.is_null()
                    && (v.as_array().is_none()
                        || v.as_array().is_none_or(|values| !values.is_empty()))
            }) || req.tool_choice.as_ref().is_some_and(|v| {
                !v.is_null()
                    && v.as_str() != Some("none")
                    && v.get("type").and_then(|t| t.as_str()) != Some("none")
            }) || req
                .messages
                .iter()
                .any(|m| !m.tool_calls.is_empty() || !matches!(m.content, MessageContent::Text(_))),
            streaming: req.stream,
            ..Self::default()
        }
    }

    /// Return a stable reason, distinguishing missing evidence from incompatibility.
    pub fn check(&self, capabilities: &ModelCapabilities) -> Result<(), &'static str> {
        for (required, evidence) in [
            (self.tools, &capabilities.tools),
            (self.vision, &capabilities.vision),
            (self.reasoning, &capabilities.reasoning),
            (self.streaming, &capabilities.streaming),
        ] {
            if required {
                match evidence.state {
                    CapabilityState::Supported => {}
                    CapabilityState::Unknown => return Err("required_capability_unknown"),
                    CapabilityState::Unsupported => return Err("required_capability_unsupported"),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, MessageContent, RequestMetadata, Role};
    use shared_kernel::ModelId;

    fn evidence(state: CapabilityState, source: &str) -> CapabilityEvidence {
        CapabilityEvidence {
            state,
            source: source.to_string(),
        }
    }

    fn request(
        tools: Option<serde_json::Value>,
        tool_choice: Option<serde_json::Value>,
        messages: Vec<Message>,
        stream: bool,
    ) -> CompletionRequest {
        CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: ModelId::new("gpt-4o"),
            messages,
            stream,
            max_tokens: None,
            temperature: None,
            tools,
            tool_choice,
            metadata: RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 100,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: crate::ApiKeyRestrictions::default(),
        }
    }

    fn text_message(text: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
            tool_calls: Vec::new(),
        }
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
        let capabilities: ModelCapabilities =
            serde_json::from_str("{}").expect("deserialize defaults");
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
        let mut capabilities = ModelCapabilities::default();
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
        // Untouched capabilities keep their previous state and source.
        assert_eq!(capabilities.vision.state, CapabilityState::Unknown);
        assert_eq!(capabilities.vision.source, "unknown");
        assert_eq!(capabilities.reasoning.state, CapabilityState::Unknown);
        assert_eq!(capabilities.streaming.state, CapabilityState::Unknown);
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
        let required = RequiredCapabilities::from_request(&request(
            None,
            None,
            vec![text_message("hello")],
            true,
        ));
        assert!(!required.tools);
        assert!(required.streaming);
        assert!(!required.vision);
        assert!(!required.reasoning);

        let required = RequiredCapabilities::from_request(&request(
            Some(serde_json::json!([{"type":"function","function":{"name":"lookup"}}])),
            None,
            vec![text_message("hello")],
            false,
        ));
        assert!(required.tools);
        assert!(!required.streaming);
    }

    #[test]
    fn required_capabilities_ignore_null_and_empty_tool_payloads() {
        let required = RequiredCapabilities::from_request(&request(
            Some(serde_json::Value::Null),
            Some(serde_json::Value::Null),
            vec![text_message("hello")],
            false,
        ));
        assert!(!required.tools);

        let required = RequiredCapabilities::from_request(&request(
            Some(serde_json::json!([])),
            None,
            vec![text_message("hello")],
            false,
        ));
        assert!(!required.tools);
    }

    #[test]
    fn required_capabilities_respect_tool_choice_none() {
        let required = RequiredCapabilities::from_request(&request(
            None,
            Some(serde_json::json!("none")),
            vec![text_message("hello")],
            false,
        ));
        assert!(!required.tools);

        let required = RequiredCapabilities::from_request(&request(
            None,
            Some(serde_json::json!({"type":"none"})),
            vec![text_message("hello")],
            false,
        ));
        assert!(!required.tools);

        let required = RequiredCapabilities::from_request(&request(
            None,
            Some(serde_json::json!("auto")),
            vec![text_message("hello")],
            false,
        ));
        assert!(required.tools);

        let required = RequiredCapabilities::from_request(&request(
            None,
            Some(serde_json::json!({"type":"function"})),
            vec![text_message("hello")],
            false,
        ));
        assert!(required.tools);
    }

    #[test]
    fn required_capabilities_detect_tool_calls_and_tool_content_in_messages() {
        let required = RequiredCapabilities::from_request(&request(
            None,
            None,
            vec![Message {
                role: Role::Assistant,
                content: MessageContent::Text("calling".to_string()),
                tool_calls: vec![crate::MessageToolCall {
                    id: Some("call_1".to_string()),
                    name: "lookup".to_string(),
                    arguments: serde_json::json!({}),
                }],
            }],
            false,
        ));
        assert!(required.tools);

        let required = RequiredCapabilities::from_request(&request(
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
}
