# Delta for Provider Model List Construction

## MODIFIED Requirements

### Requirement: OllamaCloud Provider Supports Requested Model

When `FallbackRouter::available_providers()` filters providers for a requested model (e.g., `ollamacloud/qwen3-coder-next`), an `OllamaCloud` provider MUST be included in the result if that model is present in the `StaticModelCatalog` for `ProviderKind::OllamaCloud`.

The system SHALL construct `OllamaProvider` instances with `supported_models` populated from the model catalog, rather than an empty list.

(Previously: `supported_models` was always `Vec::new()`, causing `supports_model()` to return `false` for all models, making all providers appear exhausted on first request.)

#### Scenario: OllamaCloud provider available for cataloged model

- GIVEN an active `OllamaCloud` connection exists in the provider registry
- AND the `StaticModelCatalog` contains entry `ollamacloud/qwen3-coder-next` with `ProviderKind::OllamaCloud`
- WHEN `FallbackRouter::available_providers()` is called with model `ollamacloud/qwen3-coder-next`
- THEN the `OllamaCloud` provider SHALL be included in the result
- AND the provider's `supports_model("ollamacloud/qwen3-coder-next")` SHALL return `true`

#### Scenario: OllamaCloud provider NOT available for non-cataloged model

- GIVEN an active `OllamaCloud` connection exists in the provider registry
- AND the `StaticModelCatalog` does NOT contain `ollamacloud/nonexistent-model`
- WHEN `FallbackRouter::available_providers()` is called with model `ollamacloud/nonexistent-model`
- THEN the `OllamaCloud` provider SHALL NOT be included in the result
- AND `supports_model("ollamacloud/nonexistent-model")` SHALL return `false`

#### Scenario: All providers return "all providers exhausted" when no providers support requested model

- GIVEN two active providers exist (OllamaCloud priority 50, OpenAI priority 50)
- AND neither provider's catalog contains `ollamacloud/nonexistent-model`
- WHEN `FallbackRouter::route()` is called with model `ollamacloud/nonexistent-model`
- THEN the response SHALL be `CortexError::AllProvidersExhausted`
- AND the error message SHALL indicate no providers support the requested model

#### Scenario: Health check unaffected by model list change

- GIVEN an active `OllamaCloud` connection with valid credentials
- WHEN the health check endpoint is called
- THEN the response SHALL show `status: "healthy"` with a measured latency
- AND the circuit breaker SHALL be `closed`
- AND the failure count SHALL be `0`

### Requirement: ProviderBuilderPort Implementation Receives Model Catalog

The `DynamicProviderBuilder` implementation of `ProviderBuilderPort` SHALL have access to the `ModelCatalogPort` at construction time, and SHALL query the catalog when building each provider to populate the `supported_models` list.

#### Scenario: OllamaCloud provider built with catalog models

- GIVEN `DynamicProviderBuilder` was constructed with a reference to `StaticModelCatalog`
- AND a connection with `ProviderKind::OllamaCloud` and `connection_id: conn-123` is being activated
- WHEN `ProviderBuilderPort::build()` is called with `ProviderBuildInput { provider_kind: OllamaCloud, connection_id: conn-123, ... }`
- THEN the builder SHALL call `model_catalog.list()` and filter entries where `provider_kind == ProviderKind::OllamaCloud`
- AND the resulting `Vec<ModelId>` SHALL be passed to `build_provider_from_connection` as the `models` argument
- AND the returned `OllamaProvider` SHALL have `supported_models` containing `ollamacloud/qwen3-coder-next` and all other OllamaCloud catalog entries