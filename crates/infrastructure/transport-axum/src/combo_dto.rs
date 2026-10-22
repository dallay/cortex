use rook_core::{Combo, ComboStep, ComboStrategy};
use serde::{Deserialize, Serialize};
use shared_kernel::{ComboId, ConnectionId, ModelId, ProviderId};

/// Request body for creating a new combo
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateComboRequest {
    pub name: String,
    pub strategy: String,
    pub steps: Vec<CreateComboStepRequest>,
}

/// Request body for updating an existing combo
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateComboRequest {
    pub name: String,
    pub strategy: String,
    pub steps: Vec<CreateComboStepRequest>,
}

/// Step within a combo request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateComboStepRequest {
    pub provider_id: String,
    pub model: String,
    pub connection_id: Option<String>,
    pub priority: u8,
}

/// Response body for a single combo
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComboResponse {
    pub id: String,
    pub name: String,
    pub strategy: String,
    pub steps: Vec<ComboStepResponse>,
    pub created_at: String,
    pub updated_at: String,
}

/// Response body for listing combos
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComboListResponse {
    pub combos: Vec<ComboResponse>,
}

/// Step within a combo response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComboStepResponse {
    pub provider_id: String,
    pub model: String,
    pub connection_id: Option<String>,
    pub priority: u8,
}

impl CreateComboRequest {
    /// Convert to domain Combo (validation happens in Combo::new + validate)
    pub fn to_domain(&self) -> Result<rook_core::Combo, String> {
        let strategy = parse_combo_strategy(&self.strategy)?;
        let steps = self
            .steps
            .iter()
            .map(|s| {
                let connection_id = s
                    .connection_id
                    .as_deref()
                    .map(ConnectionId::parse_str)
                    .transpose()
                    .map_err(|error| format!("invalid connectionId: {error}"))?;
                Ok(ComboStep {
                    provider_id: ProviderId::new(&s.provider_id),
                    model: ModelId::new(&s.model),
                    connection_id,
                    priority: s.priority,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let combo = rook_core::Combo::new(self.name.clone(), strategy, steps);
        combo
            .validate()
            .map_err(|e| validation_error_to_string(&e))?;
        Ok(combo)
    }
}

impl UpdateComboRequest {
    /// Convert to domain Combo with given ID (validation happens in Combo::new + validate)
    pub fn to_domain(&self, combo_id: ComboId) -> Result<rook_core::Combo, String> {
        let strategy = parse_combo_strategy(&self.strategy)?;
        let steps = self
            .steps
            .iter()
            .map(|s| {
                let connection_id = s
                    .connection_id
                    .as_deref()
                    .map(ConnectionId::parse_str)
                    .transpose()
                    .map_err(|error| format!("invalid connectionId: {error}"))?;
                Ok(ComboStep {
                    provider_id: ProviderId::new(&s.provider_id),
                    model: ModelId::new(&s.model),
                    connection_id,
                    priority: s.priority,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        let mut combo = rook_core::Combo::new(self.name.clone(), strategy, steps);
        combo.id = combo_id;
        combo
            .validate()
            .map_err(|e| validation_error_to_string(&e))?;
        Ok(combo)
    }
}

impl From<&rook_core::Combo> for ComboResponse {
    fn from(combo: &Combo) -> Self {
        Self {
            id: combo.id.to_string(),
            name: combo.name.clone(),
            strategy: strategy_to_string(combo.strategy),
            steps: combo.steps.iter().map(ComboStepResponse::from).collect(),
            created_at: combo.created_at.to_rfc3339(),
            updated_at: combo.updated_at.to_rfc3339(),
        }
    }
}

impl From<&ComboStep> for ComboStepResponse {
    fn from(step: &ComboStep) -> Self {
        Self {
            provider_id: step.provider_id.to_string(),
            model: step.model.to_string(),
            connection_id: step.connection_id.as_ref().map(ToString::to_string),
            priority: step.priority,
        }
    }
}

fn parse_combo_strategy(value: &str) -> Result<ComboStrategy, String> {
    let lower = value.to_lowercase();
    match lower.as_str() {
        "priority" => Ok(ComboStrategy::Priority),
        _ => Err(format!(
            "invalid strategy: {value}. Only 'priority' is supported."
        )),
    }
}

fn strategy_to_string(strategy: ComboStrategy) -> String {
    match strategy {
        ComboStrategy::Priority => "priority".to_string(),
    }
}

fn validation_error_to_string(error: &rook_core::ComboValidationError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_combo_request_valid() {
        let req = CreateComboRequest {
            name: "Test Combo".to_string(),
            strategy: "priority".to_string(),
            steps: vec![
                CreateComboStepRequest {
                    provider_id: "openai".to_string(),
                    model: "gpt-4o".to_string(),
                    connection_id: None,
                    priority: 1,
                },
                CreateComboStepRequest {
                    provider_id: "anthropic".to_string(),
                    model: "claude-opus-4".to_string(),
                    connection_id: None,
                    priority: 2,
                },
            ],
        };

        let result = req.to_domain();
        assert!(result.is_ok());
        let combo = result.unwrap();
        assert_eq!(combo.name, "Test Combo");
        assert_eq!(combo.steps.len(), 2);
    }

    #[test]
    fn create_combo_request_keeps_connection_id() {
        let connection_id = shared_kernel::ConnectionId::new();
        let body = serde_json::json!({
            "name": "Bound Combo",
            "strategy": "priority",
            "steps": [{
                "providerId": "openai-primary",
                "model": "gpt-4o",
                "connectionId": connection_id.to_string(),
                "priority": 1
            }]
        });

        let req: CreateComboRequest = serde_json::from_value(body).expect("deserialize request");
        let combo = req.to_domain().expect("convert to domain");

        assert_eq!(combo.steps[0].connection_id, Some(connection_id));
        assert_eq!(
            combo.steps[0].provider_id,
            ProviderId::new("openai-primary"),
            "the step keeps the runtime identity"
        );
    }

    #[test]
    fn create_combo_request_without_connection_id_stays_legacy() {
        let body = serde_json::json!({
            "name": "Legacy Combo",
            "strategy": "priority",
            "steps": [{
                "providerId": "openai",
                "model": "gpt-4o",
                "priority": 1
            }]
        });

        let req: CreateComboRequest = serde_json::from_value(body).expect("deserialize request");
        let combo = req.to_domain().expect("convert to domain");

        assert_eq!(combo.steps[0].connection_id, None);
    }

    #[test]
    fn create_combo_request_rejects_invalid_connection_id() {
        let body = serde_json::json!({
            "name": "Bound Combo",
            "strategy": "priority",
            "steps": [{
                "providerId": "openai-primary",
                "model": "gpt-4o",
                "connectionId": "not-a-uuid",
                "priority": 1
            }]
        });

        let req: CreateComboRequest = serde_json::from_value(body).expect("deserialize request");
        let error = req
            .to_domain()
            .expect_err("invalid connection id must fail");

        assert!(error.contains("invalid connectionId"), "got {error}");
    }

    #[test]
    fn update_combo_request_keeps_connection_id() {
        let connection_id = shared_kernel::ConnectionId::new();
        let body = serde_json::json!({
            "name": "Bound Combo",
            "strategy": "priority",
            "steps": [{
                "providerId": "openai-primary",
                "model": "gpt-4o",
                "connectionId": connection_id.to_string(),
                "priority": 1
            }]
        });

        let req: UpdateComboRequest = serde_json::from_value(body).expect("deserialize request");
        let combo = req
            .to_domain(shared_kernel::ComboId::new())
            .expect("convert to domain");

        assert_eq!(combo.steps[0].connection_id, Some(connection_id));
    }

    #[test]
    fn combo_response_exposes_connection_id_and_keeps_null_for_legacy_steps() {
        let connection_id = shared_kernel::ConnectionId::new();
        let combo = rook_core::Combo::new(
            "Mixed".to_string(),
            rook_core::ComboStrategy::Priority,
            vec![
                ComboStep {
                    provider_id: ProviderId::new("openai-primary"),
                    model: ModelId::new("gpt-4o"),
                    connection_id: Some(connection_id),
                    priority: 1,
                },
                ComboStep {
                    provider_id: ProviderId::new("anthropic"),
                    model: ModelId::new("claude-opus-4"),
                    connection_id: None,
                    priority: 2,
                },
            ],
        );

        let response = ComboResponse::from(&combo);
        assert_eq!(
            response.steps[0].connection_id,
            Some(connection_id.to_string())
        );
        assert_eq!(response.steps[1].connection_id, None);

        let json = serde_json::to_value(&response).expect("serialize response");
        assert_eq!(
            json["steps"][0]["connectionId"].as_str(),
            Some(connection_id.to_string().as_str())
        );
        assert!(json["steps"][1]["connectionId"].is_null());
    }

    #[test]
    fn create_combo_request_empty_name() {
        let req = CreateComboRequest {
            name: "".to_string(),
            strategy: "priority".to_string(),
            steps: vec![CreateComboStepRequest {
                provider_id: "openai".to_string(),
                model: "gpt-4o".to_string(),
                connection_id: None,
                priority: 1,
            }],
        };

        let result = req.to_domain();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot be empty"));
    }

    #[test]
    fn create_combo_request_invalid_strategy() {
        let req = CreateComboRequest {
            name: "Test".to_string(),
            strategy: "round-robin".to_string(),
            steps: vec![CreateComboStepRequest {
                provider_id: "openai".to_string(),
                model: "gpt-4o".to_string(),
                connection_id: None,
                priority: 1,
            }],
        };

        let result = req.to_domain();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid strategy"));
    }

    #[test]
    fn create_combo_request_duplicate_priority() {
        let req = CreateComboRequest {
            name: "Test".to_string(),
            strategy: "priority".to_string(),
            steps: vec![
                CreateComboStepRequest {
                    provider_id: "openai".to_string(),
                    model: "gpt-4o".to_string(),
                    connection_id: None,
                    priority: 1,
                },
                CreateComboStepRequest {
                    provider_id: "anthropic".to_string(),
                    model: "claude-opus-4".to_string(),
                    connection_id: None,
                    priority: 1,
                },
            ],
        };

        let result = req.to_domain();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("duplicate priority"));
    }

    #[test]
    fn combo_response_serializes_correctly() {
        let combo = Combo::new(
            "Test Combo".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        let response = ComboResponse::from(&combo);
        assert_eq!(response.name, "Test Combo");
        assert_eq!(response.strategy, "priority");
        assert_eq!(response.steps.len(), 1);
        assert_eq!(response.steps[0].provider_id, "openai");
        assert_eq!(response.steps[0].model, "gpt-4o");
        assert_eq!(response.steps[0].priority, 1);
    }
}
