# Configuration Guide

## Config File Location

Rook looks for config in this order:

1. `ROOK_CONFIG` environment variable (path to TOML file)
2. `$XDG_CONFIG_HOME/cortex/rook.toml`
3. `~/.config/cortex/rook.toml`

## Full Schema

```toml
[server]
host = "127.0.0.1"   # default: "127.0.0.1"
port = 8080          # default: 8080

[routing]
strategy = "priority"  # "priority" | "round-robin" | "model-based"

[cache]
enabled = true
ttl_secs = 300         # 5 minutes

[database]
db_path = "~/.local/share/cortex/rook/rook.db"

[audit]

[auth.api_keys]
enabled = false
allow_env_fallback = true

[provider_crud]
enabled = false

[rate_limiting]
enabled = false
default_tier = "free"

[rate_limiting.tiers.free]
requests_per_minute = 100
requests_per_day = 1000
tokens_per_minute = 10000

[rate_limiting.tiers.pro]
requests_per_minute = 1000
requests_per_day = 100000
tokens_per_minute = 100000

[rate_limiting.tiers.enterprise]
requests_per_minute = 10000
requests_per_day = 10000000
tokens_per_minute = 1000000

[rate_limiting.ip_limits]
requests_per_minute = 30
```

> **Note:** Provider configuration is no longer in TOML. Providers are managed dynamically
> via the Provider CRUD API (`/api/providers`). See [Provider CRUD API](#provider-crud-api) below.

## Field Reference

### `[server]`

| Field                        | Type  | Default | Description                                 |
|------------------------------|-------|---------|---------------------------------------------|
| `host`                       | string | `"127.0.0.1"` | Bind address                            |
| `port`                       | u16    | `8080`        | Listen port                             |
| `health_check_interval_secs` | u64    | `30`          | Background health check interval in seconds |

### `[routing]`

| Field           | Type   | Default      | Description                                    |
|-----------------|--------|--------------|------------------------------------------------|
| `strategy`      | enum   | `"priority"` | Routing strategy. See Routing Strategies       |
| `default_combo` | string | `null`       | Optional default combo ID (UUID) to use when no `X-Rook-Combo` header is present |

Supported values for `strategy`: `priority`, `round-robin`, `model-based`

### `[[combos]]` — Multi-step Fallback Chains

Combos define multi-step fallback chains for automatic provider failover. Each combo is an ordered list of provider/model pairs that are tried in priority order until one succeeds.

```toml
[routing]
strategy = "priority"
default_combo = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"  # Optional default combo

[[combos]]
id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
name = "OpenAI → Anthropic → Ollama"
strategy = "priority"  # Currently only "priority" is supported

  [[combos.steps]]
  provider_id = "openai-primary"
  model = "gpt-4o"
  priority = 1

  [[combos.steps]]
  provider_id = "anthropic-primary"
  model = "claude-opus-4"
  priority = 2

  [[combos.steps]]
  provider_id = "ollama-local"
  model = "llama3"
  priority = 3
```

**Combo Fields:**

| Field      | Type   | Required | Description                                               |
|------------|--------|----------|-----------------------------------------------------------|
| `id`       | string | Yes      | Unique combo UUID (used for `X-Rook-Combo` header)       |
| `name`     | string | Yes      | Human-readable name (1-100 chars, unique)                 |
| `strategy` | string | No       | Execution strategy (default: `"priority"`)                |
| `steps`    | array  | Yes      | Ordered steps to try (1-10 steps)                         |

**Step Fields:**

| Field         | Type   | Required | Description                                        |
|---------------|--------|----------|----------------------------------------------------|
| `provider_id` | string | Yes      | Provider ID to use for this step                   |
| `model`       | string | Yes      | Model to request from the provider                 |
| `priority`    | u8     | Yes      | Priority order (1-255, lower = attempted first)    |

**Validation Rules:**

- Combo name must be 1-100 characters, unique
- Must have 1-10 steps
- Priorities must be unique within a combo, range 1-255
- Warnings are logged at startup for:
  - Duplicate combo names or IDs
  - Invalid strategy
  - Duplicate priorities
  - Provider IDs not found in registry

**Execution Behavior:**

Steps are tried in priority order (lower priority = attempted first):

1. **Success**: Return immediately with the response
2. **4xx error (except 429)**: Stop immediately, return error (no fallback)
3. **429 / 5xx / network error**: Continue to next step
4. **Circuit breaker open**: Skip step, continue to next
5. **Provider not in registry**: Skip step, continue to next
6. **Per-step timeout (10s)**: Continue to next step
7. **Overall timeout (60s)**: Stop combo execution

If all steps fail, return `AllProvidersExhausted` error.

**Usage:**

1. **Via header**: Set `X-Rook-Combo: <combo-id>` on any chat completion request
2. **Default combo**: Set `routing.default_combo = "<combo-id>"` to use when no header is present
3. **Manage via API**: Use `/api/combos` CRUD endpoints to create/update/delete combos

**Streaming Limitation:**

⚠️ **Combos only apply before streaming starts.** Once the first chunk is sent to the client, no fallback occurs. For maximum reliability, use combos with non-streaming requests or ensure the first provider in the chain is highly available.

**Example Use Cases:**

- **Cost optimization**: Try cheaper model first, fall back to premium if it fails
- **Availability**: Primary provider → backup provider → local fallback
- **Geography**: Try regional provider, fall back to global endpoint

### `[cache]`

| Field      | Type | Default | Description                     |
|------------|------|---------|---------------------------------|
| `enabled`  | bool | `true`  | Enable/disable response caching |
| `ttl_secs` | u64  | `300`   | Cache TTL in seconds            |

### `[database]`

| Field     | Type   | Default                              | Description                                         |
|-----------|--------|--------------------------------------|-----------------------------------------------------|
| `db_path` | string | `~/.local/share/cortex/rook/rook.db` | Single SQLite database path. `~` expands to `$HOME` |

Rook stores all local configuration/state in one SQLite database. Hexagonal boundaries stay at the port level, so replacing SQLite with a different adapter later requires a new adapter rather than changes to use cases or domain types.

### `[audit]`

Audit logging uses the shared `[database].db_path` SQLite database.

### `[auth.api_keys]`

Persistent client API key auth is disabled by default. When enabled, `/v1/*` requests authenticate against SQLite rows whose API key material is stored only as an HMAC-SHA256 hash.

```toml
[auth.api_keys]
enabled = true
allow_env_fallback = true
```

| Field                | Type | Default | Description                                                 |
|----------------------|------|---------|-------------------------------------------------------------|
| `enabled`            | bool | `false` | Use SQLite-backed client API key auth                      |
| `allow_env_fallback` | bool | `true`  | Permit legacy `CLIENT_API_KEYS` fallback                     |

When `enabled = true`, Rook resolves the hash secret in the following priority order:

1. **`API_KEY_HASH_SECRET`** environment variable (production / Docker — always use this).
2. **`api_key_secret.key`** next to the database — auto-generated and persisted on first run.
3. **Transient in-memory secret** — for `:memory:` or `file::memory:` targets that cannot persist files.

Production deployments **must** set `API_KEY_HASH_SECRET` via environment variable so the secret is not stored on disk and survives restarts. The `allow_env_fallback` option also permits the legacy `CLIENT_API_KEYS` fallback for local compatibility only.

### CLI: seed-admin

The `seed-admin` subcommand sets the initial admin password (stored as Argon2id hash):

```bash
rook seed-admin <password>
```

Requires `ROOK_CONFIG` and `API_KEY_HASH_SECRET` environment variables to be set. Use this for:

- Initial setup when no admin password exists
- E2E testing to bootstrap a known admin user

```bash
ROOK_CONFIG=dev/test-configs/rook.toml \
API_KEY_HASH_SECRET="your-secret" \
rook seed-admin admin123
```

### `[rate_limiting]`

Per-client rate limiting is disabled by default. When enabled, Rook enforces configurable rate limits for API keys and unauthenticated IP addresses.

```toml
[rate_limiting]
enabled = true
default_tier = "free"

[rate_limiting.tiers.free]
requests_per_minute = 100
requests_per_day = 1000
tokens_per_minute = 10000

[rate_limiting.tiers.pro]
requests_per_minute = 1000
requests_per_day = 100000
tokens_per_minute = 100000

[rate_limiting.tiers.enterprise]
requests_per_minute = 10000
requests_per_day = 10000000
tokens_per_minute = 1000000

[rate_limiting.ip_limits]
requests_per_minute = 30
```

| Field          | Type         | Default  | Description                                        |
|----------------|--------------|----------|----------------------------------------------------|
| `enabled`      | bool         | `false`  | Enable rate limiting middleware                    |
| `default_tier` | enum         | `"free"` | Fallback tier when API key has no explicit tier    |

**Tier configuration:**

Each tier (`free`, `pro`, `enterprise`) supports:

| Field                  | Type      | Description                               |
|------------------------|-----------|-------------------------------------------|
| `requests_per_minute`  | u32       | Maximum requests per minute (required)    |
| `requests_per_day`     | u32       | Maximum requests per day (optional)       |
| `tokens_per_minute`    | u32       | Maximum tokens per minute (optional)      |

**IP rate limits:**

Unauthenticated requests are rate limited by source IP address:

| Field                 | Type | Default | Description                            |
|-----------------------|------|---------|----------------------------------------|
| `requests_per_minute` | u32  | `30`    | Maximum requests per minute per IP     |

Authenticated requests bypass IP rate limiting and use API key tier limits instead.

**Rate limit headers:**

All responses include rate limit metadata:

- `X-RateLimit-Limit` — maximum requests allowed in the current window
- `X-RateLimit-Remaining` — remaining requests in the current window
- `X-RateLimit-Reset` — Unix timestamp when the limit resets

When rate limited, the response includes:

- HTTP 429 status
- `Retry-After` header (seconds until reset)
- Error body with `code: "RATE_LIMITED"`

**Admin API:**

When rate limiting is enabled, admin users can manage custom rate limit rules via `/api/rate-limits` endpoints. See [API Reference](api.md#rate-limit-admin-api) for details.

## Provider CRUD API

Provider connection management is disabled by default. When enabled, Rook mounts `/api/providers` endpoints for storing provider connection metadata and encrypted credentials. The provider registry is populated from the database on startup and refreshed after each CRUD operation.

```toml
[provider_crud]
enabled = true
```

| Field     | Type | Default | Description                       |
|-----------|------|---------|-----------------------------------|
| `enabled` | bool | `false` | Enable provider connection routes |

When `enabled = true`, both environment variables are required and must be non-empty:

| Variable                | Description                                      |
|-------------------------|--------------------------------------------------|
| `ENCRYPTION_PASSPHRASE` | Passphrase used to derive the credential key     |
| `ENCRYPTION_SALT`       | Base64url-no-pad encoded 16-byte derivation salt |

**Generating the encryption salt:**

```bash
# Generate a cryptographically secure 16-byte salt encoded as base64url without padding
openssl rand -base64 16 | tr -d '=' | tr '+/' '-_'
```

Example output: `Z3G83UBdTUkfGGWr-QDnQg`

**Environment variable setup:**

```bash
export ENCRYPTION_PASSPHRASE="your-secure-passphrase-min-12-chars"
export ENCRYPTION_SALT="Z3G83UBdTUkfGGWr-QDnQg"  # Use the generated value above
```

⚠️ **Important**: These environment variables must be set **before** starting Rook. Without them, `ManageConnections` will not initialize and the provider CRUD API will return 404 for all `/api/providers/*` routes.

Rollback: setting `enabled = false` unmounts the provider CRUD routes. It does not delete the SQLite database or stored connection rows.

### Dynamic Provider Registry

When `provider_crud.enabled = true`, the provider registry starts empty and is populated by calling `refresh_registry()` at startup — this reads all active connections from the database, decrypts credentials, and builds providers. The registry is also refreshed after each create, update, or delete operation.

If the initial refresh fails (e.g., encrypted connections with an incorrect key), Rook logs a warning and starts with an empty registry. Connections can be re-created or the server restarted once the correct key is available.

### Provider Management Dashboard

The Rook dashboard provides a web UI for managing provider connections at `http://localhost:8080/providers` (or your configured host/port).

**Features:**
- View all configured providers with status, latency, and priority
- Add new providers via dialog form (supports Ollama Cloud initially)
- Test provider connections before saving
- Configure advanced settings: max concurrent requests, default model, custom base URL
- View provider quotes and pricing information

**Adding a provider via UI:**

1. Navigate to `/providers` in the dashboard
2. Click "Add Provider"
3. Fill in the required fields:
   - **Name**: Descriptive name (e.g., "Ollama Production")
   - **API Key**: Your provider API key (from provider's dashboard)
   - **Base URL**: Provider endpoint (default provided)
   - **Priority**: Lower numbers = higher priority (0-255)
   - **Active**: Toggle to enable/disable without deletion
4. Optionally expand "Advanced Configuration" to set:
   - Max concurrent requests
   - Default model ID
5. Click "Test Connection" to verify credentials (optional)
6. Click "Save"

The provider will appear in the list immediately and be available for routing.

**Supported providers:**
- Ollama Cloud (API Key) — ✅ Implemented
- OpenAI (API Key / OAuth) — ✅ Implemented (streaming)
- Anthropic (API Key / OAuth) — planned
- Gemini (API Key / OAuth) — planned
- Ollama Local (no auth) — ✅ Implemented

**Multi-account support:**

You can add multiple connections for the same provider kind (e.g., "OpenAI Production" and "OpenAI Backup" with different API keys). Each connection gets a unique runtime ID and can have different priority, models, and configuration.

### Supported Provider Kinds

| Kind            | Auth required          | base_url default              |
|-----------------|------------------------|-------------------------------|
| `openai`        | API key or OAuth token | `https://api.openai.com`      |
| `anthropic`     | API key or OAuth token | `https://api.anthropic.com`   |
| `ollama`        | None (local)           | `http://localhost:11434`      |
| `ollama-cloud`  | API key (Bearer)       | `https://ollama.com`          |
| `gemini`        | API key or OAuth token | (uses Google's API)           |
| `groq`          | API key or OAuth token | (uses Groq's API)             |

## Routing Strategies

### `priority` (default)

Providers are tried in priority order. First available provider that supports the model is used.

### `round-robin`

Providers are rotated in round-robin order (among available providers that support the model).

### `model-based`

Selects provider by model ID prefix. Not yet fully implemented (falls back to priority).

## Full Example

```toml
[server]
host = "0.0.0.0"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 600

[audit]

[database]
db_path = "~/.local/share/cortex/rook/rook.db"

[provider_crud]
enabled = true
```

> Providers are added via the CRUD API after Rook starts:
> ```bash
> curl -X POST http://localhost:8080/api/providers \
>   -H "Content-Type: application/json" \
>   -d '{
>     "name": "openai-primary",
>     "provider_kind": "openai",
>     "auth_type": "api_key",
>     "credentials": { "api_key": "${OPENAI_API_KEY}" },
>     "is_active": true,
>     "priority": 1
>   }'
> ```

## Validation

Rook validates config on startup:

- `db_path` must be writable (directory must exist; file is created on first write)
- If `provider_crud.enabled = true`, `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT` must be set
- If `auth.api_keys.enabled = true`, `API_KEY_HASH_SECRET` must be set

There is no minimum provider requirement at startup — the registry starts empty and is populated from the database via `refresh_registry()`. If no connections exist in the database, Rook will return 503 for all routing requests until connections are added via the CRUD API.

## E2E Testing

### Prerequisites

1. Build the Docker image with the current code:
   ```bash
   docker build -f Dockerfile.dev -t rook:e2e .
   ```

2. Create a test config with API key management enabled:
   ```toml
   # dev/test-configs/rook-e2e.toml
   [server]
   host = "0.0.0.0"
   port = 8080

   [database]
   db_path = "/tmp/rook-e2e.db"

   [auth.api_keys]
   enabled = true
   allow_env_fallback = false
   ```

3. Start the container and seed the admin password:
   ```bash
   docker run -d --name rook-e2e \
     -p 8080:8080 \
     -v $(pwd)/dev/test-configs/rook-e2e.toml:/app/rook.toml:ro \
     -e ROOK_CONFIG=/app/rook.toml \
     -e API_KEY_HASH_SECRET="test-secret" \
     rook:e2e

   # Seed admin password
   docker exec rook-e2e rook seed-admin admin123

   # Test health
   curl http://localhost:8080/health
   ```

4. Run E2E tests:
   ```bash
   cd apps/rook/dashboard
   pnpm playwright test e2e/api-keys.spec.ts
   ```

### Quick E2E Test Script

```bash
./dev/e2e/run-api-keys-e2e.sh --test
```

See `dev/e2e/run-api-keys-e2e.sh` for the full script.
