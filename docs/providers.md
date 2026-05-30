# Providers

Each provider implements `ProviderPort` for a specific LLM API. All providers share the same config structure but differ in defaults and capability implementation.

## Common Config Fields

Every provider accepts:

```toml
[[providers]]
id = "provider-id"           # unique name (e.g., "openai-primary")
kind = "openai"             # provider type
api_key = "${API_KEY}"      # provider-specific
base_url = "https://..."    # provider-specific
models = ["model-a", "model-b"]  # supported model IDs
timeout_secs = 60           # request timeout (provider-specific default)
```

## OpenAI

**Kind:** `openai`

**Auth:** API key (Bearer token)

**Default base URL:** `https://api.openai.com`

**Default timeout:** 60s

**Config example:**
```toml
[[providers]]
id = "openai-primary"
kind = "openai"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com"  # optional, this is the default
models = ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-3.5-turbo"]
timeout_secs = 60
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

**Auth:** API key (x-api-key header)

**Default base URL:** `https://api.anthropic.com`

**Default timeout:** 60s

**Config example:**
```toml
[[providers]]
id = "anthropic-primary"
kind = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
base_url = "https://api.anthropic.com"  # optional, this is the default
models = ["claude-opus-4-5", "claude-sonnet-4-5", "claude-3-5-haiku", "claude-3-haiku"]
timeout_secs = 60
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

**Config example:**
```toml
[[providers]]
id = "ollama-local"
kind = "ollama"
base_url = "http://localhost:11434"  # required (no default for non-local)
models = ["llama3.2", "llama3.2:1b", "codellama", "mistral", "phi3"]
timeout_secs = 300
```

**Health check behavior:**
- Always returns `is_healthy: true` (no health check implementation)
- TODO: real health check (e.g., `GET /api/tags`)

**Implementation status:**
- `complete()` — ❌ Not yet implemented
- `stream()` — ❌ Not yet implemented

**Note:** Ollama uses an OpenAI-compatible API format (`/v1/chat/completions`), so the OpenAI adapter in `transport-axum` can translate requests to Ollama's format.

---

## Gemini

**Kind:** `gemini`

**Auth:** API key (query param `?key=...`)

**Default timeout:** 60s

**Config example:**
```toml
[[providers]]
id = "gemini-primary"
kind = "gemini"
api_key = "${GEMINI_API_KEY}"
models = ["gemini-1.5-flash", "gemini-1.5-pro", "gemini-2.0-flash-exp"]
timeout_secs = 60
```

**Health check behavior:**
- Placeholder: always returns `is_healthy: true`
- TODO: real health check

**Implementation status:**
- `complete()` — ❌ Not yet implemented
- `stream()` — ❌ Not yet implemented

---

## Groq

**Kind:** `groq`

**Auth:** API key (Bearer token)

**Default timeout:** 60s

**Config example:**
```toml
[[providers]]
id = "groq-fast"
kind = "groq"
api_key = "${GROQ_API_KEY}"
models = ["llama-3.1-8b-instant", "mixtral-8x7b-32768", "llama-3.2-1b-preview"]
timeout_secs = 60
```

**Health check behavior:**
- Placeholder: always returns `is_healthy: true`
- TODO: real health check

**Implementation status:**
- `complete()` — ❌ Not yet implemented
- `stream()` — ❌ Not yet implemented

---

## Provider Capability Matrix

| Provider   | complete() | stream() | Real health check | Default timeout |
|------------|------------|----------|-------------------|-----------------|
| OpenAI     | ✅         | ❌       | ✅                | 60s             |
| Anthropic  | ❌         | ❌       | ❌ (placeholder)  | 60s             |
| Ollama     | ❌         | ❌       | ❌ (placeholder)  | 300s            |
| Gemini     | ❌         | ❌       | ❌ (placeholder)  | 60s             |
| Groq       | ❌         | ❌       | ❌ (placeholder)  | 60s             |

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

## Runtime-Managed Provider Connections

When `[provider_crud].enabled = true`, Rook exposes `/api/providers` CRUD endpoints backed by SQLite. Credentials are encrypted at rest using `enc:v1:{nonce}:{ciphertext_and_tag}` and API responses always return `credentials: {}`.

These stored provider connections are not automatically added to request routing in v1. Existing TOML-configured providers remain the routing source; runtime-managed connections support metadata management and health probes only.

## Adding a New Provider

1. Create `crates/infrastructure/providers-{name}/src/lib.rs`
2. Define `ProviderConfig` struct (id, api_key, base_url, models, timeout_secs)
3. Implement `ProviderPort` with `#[async_trait]`
4. Add to `Cargo.toml` workspace members
5. Add match arm in `apps/rook/src/di.rs` → `build_provider()`
6. Add tests
