# C2 — Container Diagram

**Level:** Container (L1 — Major building blocks and how they communicate)

**Purpose:** Show the internal crates (containers) of Rook and how data flows between them.

---

## Crate Overview

```mermaid
graph TD
    subgraph Apps["apps/rook (Binary)"]
        main[main.rs]
        config[config.rs]
        di[di.rs]
        server[server.rs]
    end

    subgraph Transport["transport-axum (HTTP Layer)"]
        routes[routes.rs]
        openai[openai_adapter.rs]
        anthropic[anthropic_adapter.rs]
        provider_routes[provider_routes.rs]
        provider_dto[provider_dto.rs]
    end

    subgraph Usecases["rook-usecases (Application)"]
        route_req[RouteRequest]
        fallback[FallbackRouter]
        health[HealthCheck]
        manage_conn[ManageConnections]
        manage_prov[ManageProviders]
    end

    subgraph Core["rook-core (Domain)"]
        model[model.rs]
        ports[ports.rs]
    end

    subgraph Kernel["shared-kernel (Zero-deps types)"]
        ids[id.rs]
        errors[error.rs]
        time[time_.rs]
    end

    subgraph Providers["Provider Implementations"]
        openai_p[providers-openai]
        anthropic_p[providers-anthropic]
        ollama_p[providers-ollama]
        groq_p[providers-groq]
        gemini_p[providers-gemini]
    end

    subgraph Infrastructure["Infrastructure"]
        cache[cache-memory]
        audit[audit-sqlite]
        encrypt[encryption-inmemory]
        provider_repo[provider-sqlite]
    end

    %% Data flows
    Transport --> Usecases
    Usecases --> Core
    core_model[model] -.->|ports| Providers
    Transport --> |CompletionRequest| Usecases
    Core --> |Domain types| Usecases
    cache --> |CachePort| Usecases
    audit --> |AuditPort| Usecases
    encrypt --> |KeyManager| provider_repo
    provider_repo --> |ProviderRepositoryPort| Usecases
    Usecases --> Providers
    main --> config
    config --> di
    di --> |assembled container| server
```

---

## Crate Responsibilities

| Crate | Responsibility | Public API |
|-------|----------------|------------|
| `apps/rook` | Binary — main entry, config loading, DI assembly | None (binary) |
| `transport-axum` | HTTP server, wire format ↔ domain translation | `router()`, adapter funcs |
| `rook-usecases` | Request orchestration, routing, health checks | `RouteRequest`, `FallbackRouter`, traits |
| `rook-core` | Domain model + port traits (interfaces) | `CompletionRequest`, `ProviderPort`, traits |
| `shared-kernel` | Zero-deps ID types, error types | `ProviderId`, `ModelId`, `CortexError` |
| `providers-*` | Per-provider LLM API implementation | `ProviderPort` impl |
| `cache-memory` | In-memory TTL cache | `CachePort` impl |
| `audit-sqlite` | SQLite audit log | `AuditPort` impl |
| `encryption-inmemory` | AES-256-GCM encryption | `KeyManager` impl |
| `provider-sqlite` | SQLite provider repository | `ProviderRepositoryPort` impl |

---

## Inter-Crate Communication

```mermaid
graph LR
    subgraph Client
        HTTP[HTTP Request]
    end

    subgraph Inbound
        transport[transport-axum]
    end

    subgraph Application
        usecases[rook-usecases]
    end

    subgraph Domain
        core[rook-core]
    end

    subgraph Kernel
        kernel[shared-kernel]
    end

    HTTP -->|OpenAI/Anthropic| transport
    transport -->|CompletionRequest| usecases
    usecases -->|domain model| core
    core -->|port traits| kernel

    %% Style
    classDef external fill:#f9f,stroke:#333,stroke-width:2px
    classDef internal fill:#bbf,stroke:#333,stroke-width:2px
    class HTTP external
    class transport,usecases,core,kernel internal
```

## Key Ports (Interfaces)

```
┌──────────────────────────────────┐
│      rook-usecases              │
│  (Depend-on-abstractions)       │
└──────┬───────────────────────────┘
       │ implements
┌──────▼───────────────────────────┐
│      rook-core ports.rs          │
│  ProviderPort, RouterPort,       │
│  CachePort, AuditPort,          │
│  ProviderRepositoryPort,         │
│  KeyManager                      │
└──────────────────────────────────┘
       │ used by
┌──────▼───────────────────────────┐
│      Infrastructure impls        │
│  providers-*, cache-memory,      │
│  audit-sqlite, encryption-*,     │
│  provider-sqlite                │
└──────────────────────────────────┘
```

---

## Data Flow (C2 Level)

```mermaid
sequenceDiagram
    participant Client
    participant Transport as transport-axum
    participant Usecases as rook-usecases
    participant Core as rook-core
    participant Provider as providers-*

    Client->>Transport: HTTP Request (wire format)
    Transport->>Transport: wire → domain translation
    Transport->>Usecases: CompletionRequest (domain)
    Usecases->>Usecases: cache lookup
    Usecases->>Core: ask for provider
    Core-->>Usecases: selected provider
    Usecases->>Provider: complete(req)
    Provider-->>Usecases: CompletionResponse (domain)
    Usecases->>Usecases: cache set
    Usecases->>Usecases: audit record
    Usecases-->>Transport: CompletionResponse
    Transport->>Transport: domain → wire translation
    Transport-->>Client: HTTP Response (wire format)
```

---

## Boundary Notes

**Out of scope for C2:**
- Internal struct details (RouteRequest fields, FallbackRouter state)
- Database schema details (SQL in audit-sqlite, provider-sqlite)
- Encryption algorithm specifics
- Dashboard Vue.js app (apps/rook/dashboard/)

**Known limitations:**
- Provider CRUD does NOT hot-register providers into the router (see architecture.md)
- Ollama provider uses Ollama native format, not OpenAI compatibility layer
- Streaming implemented only for OpenAI provider (Phase 2 for others)

---

## Evolution Notes

This diagram will be updated as:
- New provider crates are added
- Infrastructure crates change (e.g., replace SQLite with Postgres)
- New usecases are added (e.g., batch processing, prompt templates)
- Portraits change (e.g., new `MetricsPort` for observability)
