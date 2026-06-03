// models_dto — DTOs for the model catalog endpoint

use serde::Serialize;

/// One group of models for a single active provider connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_kind: String,
    pub models: Vec<String>,
}

/// Response body for `GET /api/models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListModelsResponse {
    pub models: Vec<ProviderModelsGroup>,
}
