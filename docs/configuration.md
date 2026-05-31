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
```

> **Note:** Provider configuration is no longer in TOML. Providers are managed dynamically
> via the Provider CRUD API (`/api/providers`). See [Provider CRUD API](#provider-crud-api) below.

## Field Reference

### `[server]`

| Field   | Type   | Default         | Description                        |
|---------|--------|-----------------|------------------------------------|
| `host`  | string | `"127.0.0.1"`   | Bind address                       |
| `port`  | u16    | `8080`          | Listen port                        |

### `[routing]`

| Field      | Type   | Default     | Description                               |
|------------|--------|-------------|-------------------------------------------|
| `strategy` | enum   | `"priority"` | Routing strategy. See Routing Strategies  |

Supported values for `strategy`: `priority`, `round-robin`, `model-based`

### `[cache]`

| Field       | Type   | Default | Description                      |
|-------------|--------|---------|----------------------------------|
| `enabled`   | bool   | `true`  | Enable/disable response caching   |
| `ttl_secs`  | u64    | `300`   | Cache TTL in seconds             |

### `[database]`

| Field     | Type   | Default                                  | Description                                      |
|-----------|--------|------------------------------------------|--------------------------------------------------|
| `db_path` | string | `~/.local/share/cortex/rook/rook.db`     | Single SQLite database path. `~` expands to `$HOME` |

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

| Field                | Type   | Default | Description                              |
|----------------------|--------|---------|------------------------------------------|
| `enabled`            | bool   | `false` | Use SQLite-backed client API key auth    |
| `allow_env_fallback` | bool   | `true`  | Permit legacy `CLIENT_API_KEYS` fallback |

When `enabled = true`, `API_KEY_HASH_SECRET` is required and must be non-empty. Legacy `CLIENT_API_KEYS` may still be used for local compatibility only when `allow_env_fallback = true`.

## Provider CRUD API

Provider connection management is disabled by default. When enabled, Rook mounts `/api/providers` endpoints for storing provider connection metadata and encrypted credentials. The provider registry is populated from the database on startup and refreshed after each CRUD operation.

```toml
[provider_crud]
enabled = true
```

| Field     | Type   | Default | Description                       |
|-----------|--------|---------|-----------------------------------|
| `enabled` | bool   | `false` | Enable provider connection routes |

When `enabled = true`, both environment variables are required and must be non-empty:

| Variable                | Description                                      |
|-------------------------|--------------------------------------------------|
| `ENCRYPTION_PASSPHRASE` | Passphrase used to derive the credential key     |
| `ENCRYPTION_SALT`       | Base64url-no-pad encoded 16-byte derivation salt |

Rollback: setting `enabled = false` unmounts the provider CRUD routes. It does not delete the SQLite database or stored connection rows.

### Dynamic Provider Registry

When `provider_crud.enabled = true`, the provider registry starts empty and is populated by calling `refresh_registry()` at startup — this reads all active connections from the database, decrypts credentials, and builds providers. The registry is also refreshed after each create, update, or delete operation.

If the initial refresh fails (e.g., encrypted connections with an incorrect key), Rook logs a warning and starts with an empty registry. Connections can be re-created or the server restarted once the correct key is available.

### Supported Provider Kinds

| Kind       | Auth required           | base_url default                |
|------------|-------------------------|----------------------------------|
| `openai`   | API key or OAuth token  | `https://api.openai.com`        |
| `anthropic`| API key or OAuth token  | `https://api.anthropic.com`     |
| `ollama`   | None (local)            | `http://localhost:11434`         |
| `gemini`   | API key or OAuth token  | (uses Google's API)             |
| `groq`     | API key or OAuth token  | (uses Groq's API)               |

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
