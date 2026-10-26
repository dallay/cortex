use serde::{Deserialize, Serialize};

use crate::{CompletionRequest, ConnectionId, MessageContent, ProviderKind};

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
    pub provider_kind: ProviderKind,
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
                    && match v.as_array() {
                        None => true,
                        Some(values) => !values.is_empty(),
                    }
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
            metadata: crate::RequestMetadata {
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
            RequiredCapabilities::from_request(&request(Some(
                serde_json::json!([{"type":"function"}])
            )))
            .tools
        );
        assert!(
            RequiredCapabilities::from_request(&request(Some(
                serde_json::json!({"type":"function"})
            )))
            .tools
        );
    }
}
