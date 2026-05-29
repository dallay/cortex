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

[audit]
db_path = "~/.local/share/cortex/rook/audit.db"

[provider_crud]
enabled = false
db_path = "~/.local/share/cortex/rook/providers.db"

# At least one provider is required
[[providers]]
id = "openai-primary"
kind = "openai"                    # required
api_key = "${OPENAI_API_KEY}"      # env var expansion supported
base_url = "https://api.openai.com"  # default for openai
models = ["gpt-4o", "gpt-4o-mini"]
timeout_secs = 60                  # default per provider kind

[[providers]]
id = "anthropic-primary"
kind = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
base_url = "https://api.anthropic.com"  # default for anthropic
models = ["claude-opus-4-5", "claude-sonnet-4-5"]
timeout_secs = 60

[[providers]]
id = "ollama-local"
kind = "ollama"
base_url = "http://localhost:11434"   # required for ollama (no auth)
models = ["llama3.2", "codellama"]
timeout_secs = 300                # ollama default is 300s (local models are slow)
```

## Field Reference

### `[server]`

| Field   | Type   | Default         | Description                        |
|---------|--------|-----------------|------------------------------------|
| `host`  | string | `"127.0.0.1"`   | Bind address                       |
| `port`  | u16    | `8080`          | Listen port                        |

### `[routing]`

| Field      | Type   | Default     | Description                               |
|------------|--------|-------------|-------------------------------------------|
| `strategy` | enum   | `"priority"`| Routing strategy. See Routing Strategies  |

Supported values for `strategy`: `priority`, `round-robin`, `model-based`

### `[cache]`

| Field       | Type   | Default | Description                      |
|-------------|--------|---------|----------------------------------|
| `enabled`   | bool   | `true`  | Enable/disable response caching   |
| `ttl_secs`  | u64    | `300`   | Cache TTL in seconds             |

### `[audit]`

| Field     | Type   | Description                        |
|-----------|--------|------------------------------------|
| `db_path` | string | SQLite DB path. `~` expands to `$HOME` |

### `[[providers]]`

| Field         | Type     | Required | Default                    | Description                        |
|---------------|----------|----------|----------------------------|------------------------------------|
| `id`          | string   | Yes      | —                          | Unique provider identifier         |
| `kind`        | string   | Yes      | —                          | Provider type. See Provider Kinds  |
| `api_key`     | string   | varies   | —                          | API key. Supports `${ENV_VAR}`     |
| `base_url`    | string   | varies   | Provider-specific          | API base URL                       |
| `models`      | array    | Yes      | —                          | List of supported model IDs        |
| `timeout_secs`| u64      | No       | Provider-specific (see below) | Request timeout in seconds     |

**Provider defaults for `timeout_secs`:**
- `openai`: 60s
- `anthropic`: 60s
- `gemini`: 60s
- `groq`: 60s
- `ollama`: 300s (local models are slower)

## Provider Kinds

| Kind       | api_key required? | base_url default                |
|------------|-------------------|---------------------------------|
| `openai`   | Yes               | `https://api.openai.com`        |
| `anthropic`| Yes               | `https://api.anthropic.com`     |
| `ollama`   | No                | `http://localhost:11434`         |
| `gemini`   | Yes               | (uses Google's API)             |
| `groq`     | Yes               | (uses Groq's API)               |

## Environment Variable Expansion

`api_key` fields support `${VAR}` syntax:
```toml
api_key = "${OPENAI_API_KEY}"   # reads from OPENAI_API_KEY env var
api_key = "${ANTHROPIC_KEY}"    # reads from ANTHROPIC_KEY env var
```

The `${}` wrapper is required. Bare `${VAR}` without closing brace is not expanded.

## Provider CRUD API

Provider connection CRUD is disabled by default. When enabled, Rook mounts `/api/providers` endpoints for storing provider connection metadata and encrypted credentials.

```toml
[provider_crud]
enabled = true
db_path = "~/.local/share/cortex/rook/providers.db"
```

| Field     | Type   | Default                                      | Description                       |
|-----------|--------|----------------------------------------------|-----------------------------------|
| `enabled` | bool   | `false`                                      | Enable provider connection routes |
| `db_path` | string | `~/.local/share/cortex/rook/providers.db`    | SQLite DB path; `~` expands       |

When `enabled = true`, both environment variables are required and must be non-empty:

| Variable                | Description                                      |
|-------------------------|--------------------------------------------------|
| `ENCRYPTION_PASSPHRASE` | Passphrase used to derive the credential key     |
| `ENCRYPTION_SALT`       | Base64url-no-pad encoded 16-byte derivation salt |

Rollback: setting `enabled = false` unmounts the provider CRUD routes. It does not delete the SQLite database or stored connection rows.

## Routing Strategies

### `priority` (default)
Providers are tried in config order. First available provider that supports the model is used.

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
db_path = "~/.local/share/cortex/rook/audit.db"

[[providers]]
id = "openai-primary"
kind = "openai"
api_key = "${OPENAI_API_KEY}"
models = ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo"]
timeout_secs = 60

[[providers]]
id = "anthropic-primary"
kind = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
models = ["claude-opus-4-5", "claude-sonnet-4-5", "claude-3-5-haiku"]
timeout_secs = 60

[[providers]]
id = "ollama-local"
kind = "ollama"
base_url = "http://localhost:11434"
models = ["llama3.2", "codellama"]
timeout_secs = 300

[[providers]]
id = "groq-fast"
kind = "groq"
api_key = "${GROQ_API_KEY}"
models = ["llama-3.1-8b-instant", "mixtral-8x7b-32768"]
timeout_secs = 30
```

## Validation

Rook validates config on startup:
- At least one provider must be configured
- Provider `kind` must be one of: `openai`, `anthropic`, `ollama`, `gemini`, `groq`
- `api_key` must be non-empty for providers that require it (unless the placeholder `${VAR}` is used, which resolves at runtime)
- `db_path` must be writable (directory must exist; file is created on first write)
