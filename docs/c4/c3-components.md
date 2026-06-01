# C3 — Component Diagram

**Level:** Component (L2 — What lives inside each major crate)

**Purpose:** Show the key components inside the most important crates and how they relate at a finer grain than C2.

---

## `transport-axum` Components

```mermaid
graph TD
    subgraph TransportAXUM["transport-axum"]
        routes["routes.rs
        ──────────────
        POST /v1/chat/completions
        POST /v1/messages
        GET /health
        GET /api/providers
        (if enabled)"]

        openai["openai_adapter.rs
        ──────────────
        wire → domain
        domain → wire
        Streaming SSE support"]

        anthropic["anthropic_adapter.rs
        ──────────────
        wire → domain
        domain → wire"]

        provider_routes["provider_routes.rs
        ──────────────
        Provider CRUD handlers
        Requires provider_crud.enabled"]

        provider_dto["provider_dto.rs
        ──────────────
        DTOs for API
        Always strips credentials"]
    end

    routes --> openai
    routes --> anthropic
    routes --> provider_routes
    provider_routes --> provider_dto
```

| Component              | Responsibility                                          |
|------------------------|---------------------------------------------------------|
| `routes.rs`            | Axum router setup, endpoint mounting, request dispatch  |
| `openai_adapter.rs`    | OpenAI `/v1/chat/completions` wire ↔ domain translation |
| `anthropic_adapter.rs` | Anthropic `/v1/messages` wire ↔ domain translation      |
| `provider_routes.rs`   | CRUD endpoints for runtime provider management          |
| `provider_dto.rs`      | Data transfer objects (request/response serialization)  |

---

## `rook-usecases` Components

```mermaid
graph TD
    subgraph RookUsecases["rook-usecases"]
        route_req["RouteRequest
        ──────────────
        Orchestrates full request:
        cache → select provider
        → execute → cache
        → audit → response
        Entry point for routing"]

        fallback["FallbackRouter
        ──────────────
        Implements RouterPort
        3 strategies:
        • Priority
        • RoundRobin
        • ModelBased
        Circuit breaker:
        3 failures → 30s cooldown"]

        health["HealthCheck
        ──────────────
        Aggregates health
        across all providers
        Returns HealthStatus"]

        manage_prov["ManageProviders
        ──────────────
        Enable/disable providers
        (interface only)"]

        manage_conn["ManageConnections
        ──────────────
        Runtime provider
        CRUD + test workflow"]
    end

    route_req --> fallback
    route_req --> health
    route_req --> manage_prov
    route_req --> manage_conn
```

| Component           | Responsibility                                                                                     |
|---------------------|----------------------------------------------------------------------------------------------------|
| `RouteRequest`      | Main orchestrator: hit cache → select provider → execute → cache response → audit → handle failure |
| `FallbackRouter`    | Implements `RouterPort` with Priority/RoundRobin/ModelBased strategies + circuit breaker           |
| `HealthCheck`       | Aggregated health status from all registered providers                                             |
| `ManageProviders`   | Enable/disable providers (interface only for now)                                                  |
| `ManageConnections` | Runtime provider connection CRUD and test workflow                                                 |

---

## `rook-core` Components

```mermaid
graph TD
    subgraph RookCore["rook-core"]
        model["model.rs
        ──────────────
        CompletionRequest
        CompletionResponse
        Message / Role / MessageContent
        TokenUsage
        StreamChunk
        HealthStatus
        AuditEntry"]

        ports["ports.rs
        ──────────────
        ProviderPort
        RouterPort
        CachePort
        AuditPort
        ProviderRepositoryPort
        ProviderRegistryPort
        KeyManager"]
    end

    model -.->|defines| ports
```

| Component  | Responsibility                                                                                          |
|------------|---------------------------------------------------------------------------------------------------------|
| `model.rs` | Domain types — completely provider-agnostic. No wire format knowledge.                                  |
| `ports.rs` | Trait definitions for infrastructure dependencies. "What" the domain needs, not "how" it's implemented. |

### Domain Model Detail

```mermaid
classDiagram
    class CompletionRequest {
        +RequestId id
        +ModelId model
        +Vec~Message~ messages
        +bool stream
        +Option~u32~ max_tokens
        +Option~f32~ temperature
        +RequestMetadata metadata
    }

    class Message {
        +Role role
        +MessageContent content
    }

    class Role {
        <<enum>>
        System
        Developer
        User
        Assistant
    }

    class MessageContent {
        <<enum>>
        Text(String)
        -- Phase 2 --
        ToolUse
        ToolResult
    }

    class CompletionResponse {
        +String id
        +ModelId model
        +Vec~Message~ message
        +String finish_reason
        +TokenUsage usage
    }

    class TokenUsage {
        +u32 prompt_tokens
        +u32 completion_tokens
        +u32 total_tokens
    }

    CompletionRequest --> Message
    CompletionRequest --> Role
    Message --> MessageContent
    CompletionResponse --> Message
    CompletionResponse --> TokenUsage
```

---

## `providers-*` Components (Common Pattern)

All provider crates follow the same structure:

```mermaid
graph TD
    subgraph Provider["providers-* (e.g., providers-openai)"]
        config["Config
        ──────────────
        id: ProviderId
        api_key: SecretString
        base_url: Url
        models: Vec~ModelId~
        timeout_secs: u64"]

        impl["ProviderImpl
        ──────────────
        Implements ProviderPort
        • complete()
        • stream() -- stub
        • health_check()
        • is_available()"]

        client["reqwest::Client
        ──────────────
        TLS via rustls
        Shared across reqs"]
    end

    impl --> config
    impl --> client
```

| Method           | OpenAI | Anthropic | Ollama | Groq | Gemini |
|------------------|--------|-----------|--------|------|--------|
| `complete()`     | ✅      | ✅         | ✅      | ✅    | ✅      |
| `stream()`       | stub   | stub      | stub   | stub | stub   |
| `health_check()` | ✅      | ✅         | ✅      | ✅    | ✅      |
| `is_available()` | ✅      | ✅         | ✅      | ✅    | ✅      |

---

## `apps/rook` Components

```mermaid
graph TD
    subgraph AppsRook["apps/rook"]
        main["main.rs
        ──────────────
        1. Init tracing
        2. Load config
        3. Build container
        4. Start server
        5. Graceful shutdown"]

        config["config.rs
        ──────────────
        toml::from_str
        Expand ~ in paths
        Expand ${ENV_VAR}
        in api_key values"]

        di["di.rs
        ──────────────
        RookContainer::build()
        Assembles all crates
        Returns assembled
        RookUsecases"]

        server["server.rs
        ──────────────
        axum::Server::bind
        Router mount
        Graceful shutdown"]
    end

    main --> config
    main --> di
    main --> server
    di --> server
    config --> di
```

---

## Key Trait Definitions

```mermaid
classDiagram
    class ProviderPort {
        <<trait>>
        +fn id() -&PluginId
        +fn supported_models() -&)~ModelId~
        +fn is_available() bool
        +async fn health_check() HealthStatus
        +async fn complete(&CompletionRequest) Result~CompletionResponse~
        +async fn stream(&CompletionRequest) Result~BoxStream~()
    }

    class RouterPort {
        <<trait>>
        +async fn select(&CompletionRequest) Result~Arc~dyn ProviderPort~~
        +async fn on_failure(&PluginId, &CortexError)
        +fn providers() Vec~PluginId~
    }

    class CachePort {
        <<trait>>
        +async fn get(&CacheKey) Result~Option~CompletionResponse~~
        +async fn set(&CacheKey, &CompletionResponse, Duration) Result~()~
        +async fn delete(&CacheKey) Result~()~
        +async fn clear() Result~()~
    }

    class AuditPort {
        <<trait>>
        +async fn record(AuditEntry) Result~()~
    }
```

---

## Out of Scope for C3

- **Database internals**: specific SQL schemas for audit-sqlite and provider-sqlite
- **Encryption details**: AES-256-GCM parameters, Argon2id rounds
- **Dashboard Vue.js app**: lives in `apps/rook/dashboard/`, separate frontend concern
- **`tmp/OmniRoute`**: experimental/transient code, not production

---

## Evolution Notes

This diagram will be updated as:

- Phase 2 components are added (`SseBuffer`, tool call support)
- `streaming()` methods are implemented across providers
- New ports/traits are introduced (e.g., `MetricsPort`, `RateLimitPort`)
- `ManageConnections` gains full CRUD implementation
