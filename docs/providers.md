# Providers

Each provider implements `ProviderPort` for a specific LLM API. All providers share the same config structure but differ in defaults and capability implementation.

## Common Config Fields

Every provider accepts these fields when created via the CRUD API:

| Field           | Type   | Required | Default           | Description                               |
|-----------------|--------|----------|-------------------|-------------------------------------------|
| `name`          | string | Yes      | —                 | Human-readable connection name            |
| `provider_kind` | string | Yes      | —                 | Provider type (see below)                 |
| `auth_type`     | string | Yes      | —                 | `api_key` or `oauth`                      |
| `credentials`   | object | Yes      | —                 | Key-value pairs (see per provider)        |
| `base_url`      | string | No       | Provider-specific | API base URL                              |
| `is_active`     | bool   | No       | `false`           | Whether connection is in rotation         |
| `priority`      | u8     | No       | 0                 | Priority for routing (higher = preferred) |
| `default_model` | string | No       | —                 | Default model ID if provider has multiple |

## OpenAI

**Kind:** `openai`

**Auth:** API key (Bearer token) or OAuth access token

**Default base URL:** `https://api.openai.com`

**Default timeout:** 60s

**API example:**

```json
{
  "name": "openai-primary",
  "provider_kind": "openai",
  "auth_type": "api_key",
  "credentials": { "api_key": "${OPENAI_API_KEY}" },
  "is_active": true,
  "priority": 1
}
```

**Health check behavior:**

- Makes a `GET /models` request to `base_url`
- Returns `HealthStatus::Healthy` if response is 2xx
- Returns `HealthStatus::Unhealthy` with error message on non-2xx or connection failure

**Health status enum:**
Providers return one of: `Healthy { provider, latency_ms }`, `Unhealthy { provider, latency_ms, error }`, or `Unknown { provider, reason }`. The `/health` endpoint renders backwards-compatible JSON with `healthy` (bool), `latency_ms`, and `last_error` derived from the enum.

**Implementation status:**

- `complete()` — ✅ Implemented
- `stream()` — ❌ Not yet implemented

---

## Anthropic

**Kind:** `anthropic`

**Auth:** API key (x-api-key header) or OAuth access token

**Default base URL:** `https://api.anthropic.com`

**Default timeout:** 60s

**API example:**

```json
{
  "name": "anthropic-primary",
  "provider_kind": "anthropic",
  "auth_type": "api_key",
  "credentials": { "api_key": "${ANTHROPIC_API_KEY}" },
  "is_active": true,
  "priority": 1
}
```

**Health check behavior:**

- Placeholder: always returns `HealthStatus::Unknown { reason: "health_check_not_supported" }`
- TODO: real health check against Anthropic API

**Implementation status:**

- `complete()` — ❌ Not yet implemented
- `stream()` — ❌ Not yet implemented

---

## Ollama

**Kind:** `ollama`

**Auth:** None (local deployment)

**Default base URL:** `http://localhost:11434`

**Default timeout:** 300s (higher than cloud providers — local models are slower)

**API example:**

```json
{
  "name": "ollama-local",
  "provider_kind": "ollama",
  "base_url": "http://localhost:11434",
  "is_active": true,
  "priority": 1
}
```

> **Note:** `base_url` is required for Ollama since there is no sensible default for non-local deployments.

**Health check behavior:**

- Two-step probe:
  1. `GET /api/tags` — verify host reachability
  2. `POST /api/chat` (if API key configured) — verify credentials
- Returns `HealthStatus::Warning` if no API key configured (for Ollama Cloud use cases)

**Implementation status:**

- `complete()` — ✅ Implemented
- `stream()` — ✅ Implemented

**Note:** Ollama uses its native API format (`/api/chat`), NOT the OpenAI-compatible endpoint.

---

## Ollama Cloud

**Kind:** `ollama-cloud`

**Auth:** API key (Bearer token)

**Default base URL:** `https://ollama.com`

**Default timeout:** 300s (cloud models may take longer than standard API providers)

**API example:**

```json
{
  "name": "ollama-cloud-primary",
  "provider_kind": "ollama-cloud",
  "auth_type": "api_key",
  "credentials": { "api_key": "${OLLAMA_CLOUD_API_KEY}" },
  "is_active": true,
  "priority": 1
}
```

**Getting an API key:**

1. Go to [ollama.com/settings/keys](https://ollama.com/settings/keys) — Sign in required; you'll be redirected to [signin.ollama.com](https://signin.ollama.com) if not authenticated
2. Generate a new API key
3. Store it securely (env var, secrets manager, etc.)

**Health check behavior:**

- Two-step probe:
  1. `GET /api/tags` — verify `ollama.com` is reachable (public endpoint)
  2. `POST /api/chat` with Bearer auth — verify API key is valid
- Returns `HealthStatus::Warning` if API key is missing (provider still works but credentials aren't validated)
- Returns `HealthStatus::Unhealthy` with error if auth is rejected (401/403)

**Implementation status:**

- `complete()` — ✅ Implemented
- `stream()` — ✅ Implemented

**Note:** Both local Ollama and Ollama Cloud use the same `providers_ollama::OllamaProvider` adapter, driven by `OllamaProviderConfig`:
- **Ollama Cloud** (`ProviderKind::OllamaCloud`): `base_url` defaults to `https://ollama.com`, `api_key` set to Bearer token enabling `Authorization: Bearer ...` headers
- **Local Ollama** (`ProviderKind::Ollama`): `base_url` defaults to `http://localhost:11434`, `api_key` unset (no auth)

---

## Gemini

**Kind:** `gemini`

**Auth:** API key (query param `?key=...`) or OAuth access token

**Default timeout:** 60s

**API example:**

```json
{
  "name": "gemini-primary",
  "provider_kind": "gemini",
  "auth_type": "api_key",
  "credentials": { "api_key": "${GEMINI_API_KEY}" },
  "is_active": true,
  "priority": 1
}
```

**Health check behavior:**

- Placeholder: always returns `HealthStatus::Unknown { reason: "health_check_not_supported" }`
- Per `HealthStatus::is_healthy()`, this yields `is_healthy() === false` and the `/health` endpoint renders `healthy: false`
- TODO: real health check

**Implementation status:**

- `complete()` — ❌ Not yet implemented
- `stream()` — ❌ Not yet implemented

---

## Groq

**Kind:** `groq`

**Auth:** API key (Bearer token) or OAuth access token

**Default timeout:** 60s

**API example:**

```json
{
  "name": "groq-fast",
  "provider_kind": "groq",
  "auth_type": "api_key",
  "credentials": { "api_key": "${GROQ_API_KEY}" },
  "is_active": true,
  "priority": 1
}
```

**Health check behavior:**

- Placeholder: always returns `HealthStatus::Unknown { reason: "health_check_not_supported" }`
- Per `HealthStatus::is_healthy()`, this yields `is_healthy() === false` and the `/health` endpoint renders `healthy: false`
- TODO: real health check

**Implementation status:**

- `complete()` — ❌ Not yet implemented
- `stream()` — ❌ Not yet implemented

---

## Provider Capability Matrix

| Provider      | complete() | stream() | Real health check | Default timeout |
|---------------|------------|----------|-------------------|-----------------|
| OpenAI        | ✅          | ✅        | ✅                 | 60s             |
| Anthropic     | ❌          | ❌        | ❌ (placeholder)   | 60s             |
| Ollama (1)    | ✅          | ✅        | ✅ (2-step)        | 300s            |
| Ollama Cloud (1) | ✅       | ✅        | ✅ (2-step)        | 300s            |

(1) Ollama and Ollama Cloud share the same `OllamaProvider` implementation. Local Ollama uses `http://localhost:11434` with no API key; Ollama Cloud uses `https://ollama.com` with Bearer auth configured via `api_key`.
| Gemini        | ❌          | ❌        | ❌ (placeholder)   | 60s             |
| Groq          | ❌          | ❌        | ❌ (placeholder)   | 60s             |

## Circuit Breaker

All providers share the same circuit breaker behavior in `FallbackRouter`:

- **Threshold:** 3 consecutive failures
- **Cooldown:** 30 seconds before retry
- **Behavior:** Provider is skipped in selection until cooldown expires

When a provider fails, call `RouterPort::on_failure(provider_id, error)` to record the failure in the circuit breaker.

## HealthStatus Contract

Providers now return an enum instead of a bool-shaped status:

```rust
pub enum HealthStatus {
    Healthy { provider: ProviderId, latency_ms: u64 },
    Unhealthy { provider: ProviderId, latency_ms: Option<u64>, error: String },
    Unknown { provider: ProviderId, reason: String },
}
```

OpenAI performs a real `/models` probe. Provider adapters without a real probe (Anthropic, Ollama, Gemini, Groq) return `Unknown { reason: "health_check_not_supported" }`. For these providers, `HealthStatus::is_healthy()` returns `false`, and the public `/health` endpoint renders `healthy: false`, `latency_ms: null`, and `last_error` containing the reason string.

## Dynamic Provider Registry

When `[provider_crud].enabled = true`, Rook exposes `/api/providers` CRUD endpoints backed by SQLite. Credentials are encrypted at rest using `enc:v1:{nonce}:{ciphertext_and_tag}` and API responses always return `credentials: {}`.

The provider registry is populated from the database on startup via `refresh_registry()`, which lists all active connections, decrypts credentials, and builds providers. The registry is also refreshed after each create, update, or delete operation. If `provider_crud.enabled = false`, no providers are routed (all requests return 503).

## Adding a New Provider

1. Create `crates/infrastructure/providers-{name}/src/lib.rs`
2. Define a provider config struct and constructor
3. Implement `ProviderPort` with `#[async_trait]`
4. Add to `Cargo.toml` workspace members
5. Add match arm in `apps/rook/src/di.rs` → `build_provider_from_connection()`
6. Add tests
