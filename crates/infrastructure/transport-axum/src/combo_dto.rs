use rook_core::{Combo, ComboStep, ComboStrategy};
use serde::{Deserialize, Serialize};
use shared_kernel::{ComboId, ModelId, ProviderId};

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
    pub priority: u8,
}

impl CreateComboRequest {
    /// Convert to domain Combo (validation happens in Combo::new + validate)
    pub fn to_domain(&self) -> Result<rook_core::Combo, String> {
        let strategy = parse_combo_strategy(&self.strategy)?;
        let steps = self
            .steps
            .iter()
            .map(|s| ComboStep {
                provider_id: ProviderId::new(&s.provider_id),
                model: ModelId::new(&s.model),
                connection_id: None,
                priority: s.priority,
            })
            .collect();

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
            .map(|s| ComboStep {
                provider_id: ProviderId::new(&s.provider_id),
                model: ModelId::new(&s.model),
                connection_id: None,
                priority: s.priority,
            })
            .collect();

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
                    priority: 1,
                },
                CreateComboStepRequest {
                    provider_id: "anthropic".to_string(),
                    model: "claude-opus-4".to_string(),
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
    fn create_combo_request_empty_name() {
        let req = CreateComboRequest {
            name: "".to_string(),
            strategy: "priority".to_string(),
            steps: vec![CreateComboStepRequest {
                provider_id: "openai".to_string(),
                model: "gpt-4o".to_string(),
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
                    priority: 1,
                },
                CreateComboStepRequest {
                    provider_id: "anthropic".to_string(),
                    model: "claude-opus-4".to_string(),
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
