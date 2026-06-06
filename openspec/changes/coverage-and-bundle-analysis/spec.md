# Delta for coverage-and-bundle-analysis

## ADDED Requirements

### Requirement: providers-core crate MUST be created

The system SHALL create a new `providers-core` crate at `crates/infrastructure/providers-core/` containing shared utilities extracted from provider implementations.

#### Scenario: providers-core crate structure

- GIVEN the workspace Cargo.toml
- WHEN the change is applied
- THEN a new crate `providers-core` MUST exist at `crates/infrastructure/providers-core/`
- AND MUST contain modules: `role.rs`, `sse.rs`, `validation.rs`, `request.rs`, `sanitize.rs`
- AND MUST be added to the workspace members list

#### Scenario: role_to_string() helper

- GIVEN `providers_core::role::role_to_string(CoreRole)` is called
- WHEN passed a `rook_core::Role` variant
- THEN it MUST return the correct provider-specific string mapping:
  - `System` → `"system"`
  - `User` → `"user"`
  - `Assistant` → `"assistant"`
  - `Developer` → `"developer"`

### Requirement: SSE parsing helpers MUST be available

The system MUST provide SSE parsing utilities in `providers_core::sse`.

#### Scenario: parse_event_text() helper

- GIVEN an SSE event text string (a single line)
- WHEN `parse_event_text(line: &str)` is called
- THEN it MUST return `Some(SseEvent::Data(content))` for lines with `data: ` prefix
- AND MUST return `Some(SseEvent::Done)` for lines containing `[DONE]`
- AND MUST return `None` for lines that don't match SSE data format
- NOTE: Does NOT parse JSON — caller handles provider-specific parsing

#### Scenario: process_bytes() helper

- GIVEN a byte slice and an `SseBuffer`
- WHEN `process_bytes(bytes, sse_buffer)` is called
- THEN it MUST push valid UTF-8 strings to the SSE buffer
- AND MUST skip invalid UTF-8 sequences without panicking
- AND MUST return an iterator yielding each complete SSE line as `String`

### Requirement: validate_response() generic function MUST be provided

The system MUST provide a generic response validation function in `providers_core::validation`.

#### Scenario: Successful response validation

- GIVEN a successful HTTP response (2xx status)
- WHEN `validate_response(resp)` is called
- THEN it MUST return `Ok(())`

#### Scenario: Failed response validation

- GIVEN a failed HTTP response (non-2xx status)
- WHEN `validate_response(resp)` is called
- THEN it MUST return `Err(ProviderError)` with status code and sanitized body

### Requirement: Sanitization utilities MUST be provided

The system MUST provide body sanitization in `providers_core::sanitize`.

#### Scenario: sanitize_body() truncation

- GIVEN a body string longer than 200 characters
- WHEN `sanitize_body(body)` is called
- THEN it MUST truncate to 200 characters
- AND MUST append `… (truncated)` if truncation occurred

#### Scenario: sanitize_body() JSON redaction

- GIVEN a JSON body with sensitive keys
- WHEN `sanitize_body(body)` is called
- THEN it MUST recursively redact values for keys matching SENSITIVE_KEYS at ANY nesting level
- AND MUST replace sensitive values with `"(redacted)"`
- AND MUST preserve non-sensitive data unchanged

#### Scenario: sanitize_body() nested sensitive keys

- GIVEN a JSON body: `{"error": {"token": "secret123"}, "api_key": "sk-12345"}`
- WHEN `sanitize_body()` is called
- THEN both the nested `"token"` AND the top-level `"api_key"` MUST be redacted
- AND the non-sensitive `"error"` key MUST be preserved

#### Scenario: char_safe_truncate() UTF-8 safety

- GIVEN a string with multi-byte UTF-8 characters
- WHEN `char_safe_truncate(s, max)` is called
- THEN it MUST count characters, not bytes
- AND MUST NOT split a multi-byte character

---

## ADDED Requirements (Test Coverage)

### Requirement: stream() functions MUST have unit tests

Each provider's `stream()` function MUST have unit tests covering error scenarios.

#### Scenario: Anthropic stream() error response test

- GIVEN an AnthropicProvider with a mocked client
- WHEN `stream()` receives an HTTP 400 error response
- THEN it MUST return an error containing the sanitized body
- AND the error MUST be a `CortexError::invalid_request`

#### Scenario: Anthropic stream() timeout test

- GIVEN an AnthropicProvider with a short timeout
- WHEN `stream()` encounters a timeout
- THEN it MUST return a `CortexError::provider` with timeout message

#### Scenario: Anthropic stream() empty response test

- GIVEN an AnthropicProvider
- WHEN `stream()` receives an empty SSE response
- THEN it MUST return an empty stream
- AND MUST NOT panic

#### Scenario: Groq stream() error response test

- GIVEN a GroqProvider with a mocked client
- WHEN `stream()` receives an HTTP 429 rate limit response
- THEN it MUST return a `CortexError::rate_limited` or `CortexError::rate_limited_with_reset`

#### Scenario: Groq stream() timeout test

- GIVEN a GroqProvider with a short timeout
- WHEN `stream()` encounters a timeout
- THEN it MUST return a `CortexError::provider` with timeout message

#### Scenario: Groq stream() empty response test

- GIVEN a GroqProvider
- WHEN `stream()` receives an empty SSE response
- THEN it MUST return an empty stream
- AND MUST NOT panic

#### Scenario: Ollama stream() error response test

- GIVEN an OllamaProvider with a mocked client
- WHEN `stream()` receives an HTTP 400 error response
- THEN it MUST return an error containing the sanitized body

#### Scenario: Ollama stream() timeout test

- GIVEN an OllamaProvider with a short timeout
- WHEN `stream()` encounters a timeout
- THEN it MUST return a `CortexError::provider` with timeout message

#### Scenario: Ollama stream() empty response test

- GIVEN an OllamaProvider
- WHEN `stream()` receives an empty SSE response
- THEN it MUST return an empty stream
- AND MUST NOT panic

### Requirement: Error mapping functions MUST have unit tests

Each provider's `map_*_http_error` function MUST have unit tests.

#### Scenario: OpenAI error mapping - 401

- GIVEN a401 response from OpenAI
- WHEN `map_openai_http_error()` is called
- THEN it MUST return `CortexError::auth_failed`

#### Scenario: OpenAI error mapping - 429 with Retry-After

- GIVEN a 429 response with `Retry-After: 120` header
- WHEN `map_openai_http_error()` is called
- THEN it MUST return `CortexError::rate_limited_with_reset` with retry_secs=120

#### Scenario: OpenAI error mapping - 400

- GIVEN a 400 response with JSON body
- WHEN `map_openai_http_error()` is called
- THEN it MUST sanitize the body before including in error

#### Scenario: Anthropic error mapping - 401

- GIVEN a 401 response from Anthropic
- WHEN `map_anthropic_http_error()` is called
- THEN it MUST return `CortexError::auth_failed`

#### Scenario: Anthropic error mapping - 429

- GIVEN a 429 response
- WHEN `map_anthropic_http_error()` is called
- THEN it MUST return `CortexError::rate_limited`

#### Scenario: Groq error mapping - 401

- GIVEN a 401 response from Groq
- WHEN `map_groq_http_error()` is called
- THEN it MUST return `CortexError::auth_failed`

#### Scenario: Ollama error mapping - 401

- GIVEN a 401 response from Ollama
- WHEN `map_ollama_http_error()` is called
- THEN it MUST return `CortexError::auth_failed`

### Requirement: Sanitization functions MUST have unit tests

#### Scenario: sanitize_body() with sensitive JSON

- GIVEN a JSON body: `{"api_key": "secret123", "data": "ok"}`
- WHEN `sanitize_body()` is called
- THEN the result MUST contain `"api_key": "(redacted)"`
- AND MUST contain `"data": "ok"`
- AND MUST NOT contain `"secret123"`

#### Scenario: sanitize_body() with nested sensitive keys

- GIVEN a nested JSON body: `{"error": {"token": "secret123"}, "api_key": "sk-12345"}`
- WHEN `sanitize_body()` is called
- THEN both the nested `token` AND the top-level `api_key` MUST be redacted
- AND `error` content MUST be preserved

#### Scenario: sanitize_body() with plain text

- GIVEN a plain text body longer than 200 chars
- WHEN `sanitize_body()` is called
- THEN the result MUST be exactly 200 + `… (truncated)` chars

#### Scenario: char_safe_truncate() with UTF-8

- GIVEN a string with emoji: `"hello 😀 world"`
- WHEN `char_safe_truncate(s, 8)` is called
- THEN the result MUST be `"hello 😀"` (6 chars + truncation marker)
- AND MUST NOT corrupt the emoji

#### Scenario: char_safe_truncate() with ASCII

- GIVEN `"hello world"` and max=5
- WHEN `char_safe_truncate()` is called
- THEN the result MUST be `"hello… (truncated)"`

---

## ADDED Requirements (Codecov Bundle Analysis)

### Requirement: Dashboard MUST include @codecov/vite-plugin

The system MUST add `@codecov/vite-plugin` to the dashboard dependencies.

#### Scenario: Codecov plugin installation

- GIVEN the dashboard `package.json`
- WHEN the change is applied
- THEN `@codecov/vite-plugin` MUST be added to dependencies
- AND the version MUST be compatible with the current Vite version

### Requirement: Dashboard vite.config.ts MUST configure Codecov

The system MUST configure the Codecov plugin in `vite.config.ts`.

#### Scenario: Codecov plugin configuration

- GIVEN `@codecov/vite-plugin` is installed
- WHEN `vite.config.ts` is updated
- THEN the plugin MUST be added to the `plugins` array
- AND MUST be configured with `bundleName: "rook-dashboard"`

#### Scenario: Codecov token-based activation

- GIVEN the vite config includes Codecov plugin
- WHEN the dashboard is built
- THEN the plugin SHOULD only upload when `CODECOV_TOKEN` environment variable is present
- AND SHOULD gracefully skip upload when token is absent

---

## MODIFIED Requirements

### Requirement: providers-openai MUST use providers-core imports

The `providers-openai` crate MUST import shared utilities from `providers-core` instead of duplicating them.

(Previously: All utilities defined inline in provider.rs)

#### Scenario: OpenAI uses providers-core sanitize

- GIVEN `providers-core` is available
- WHEN `providers-openai` is compiled
- THEN `sanitize_body()` MUST be imported from `providers_core::sanitize`
- AND local `sanitize_body` definition MUST be removed

#### Scenario: OpenAI uses providers-core role mapping

- GIVEN `providers-core` is available
- WHEN `providers-openai` is compiled
- THEN role string conversion MUST use `providers_core::role::role_to_string()`
- AND local role mapping MUST be removed

### Requirement: providers-anthropic MUST use providers-core imports

The `providers-anthropic` crate MUST import shared utilities from `providers-core`.

(Previously: `sanitize_body()`, `sanitize_body()`, SSE parsing defined inline)

#### Scenario: Anthropic uses providers-core sanitize

- GIVEN `providers-core` is available
- WHEN `providers-anthropic` is compiled
- THEN `sanitize_body()` MUST be imported from `providers_core::sanitize`
- AND local definition MUST be removed

#### Scenario: Anthropic uses providers-core SSE parsing

- GIVEN `providers-core` is available
- WHEN `providers-anthropic` is compiled
- THEN SSE parsing MUST use `providers_core::sse::parse_event_text` and `process_bytes`
- AND local SSE parsing MUST be removed

### Requirement: providers-groq MUST use providers-core imports

The `providers-groq` crate MUST import shared utilities from `providers-core`.

(Previously: Duplicated utilities defined inline)

#### Scenario: Groq uses providers-core imports

- GIVEN `providers-core` is available
- WHEN `providers-groq` is compiled
- THEN `sanitize_body()`, SSE parsing, and validation MUST be imported from `providers-core`
- AND local duplicate definitions MUST be removed

### Requirement: providers-ollama MUST use providers-core imports

The `providers-ollama` crate MUST import shared utilities from `providers-core`.

(Previously: Duplicated utilities defined inline)

#### Scenario: Ollama uses providers-core imports

- GIVEN `providers-core` is available
- WHEN `providers-ollama` is compiled
- THEN `sanitize_body()`, SSE parsing, and validation MUST be imported from `providers-core`
- AND local duplicate definitions MUST be removed

---

## Quality Gate Requirements

### Requirement: SonarCloud new_coverage MUST be ≥ 80%

After the change, the SonarCloud quality gate `new_coverage` MUST be at least 80%.

#### Scenario: Coverage threshold met

- GIVEN `cargo test --workspace` passes
- WHEN coverage is calculated for new code
- THEN `new_coverage` MUST be ≥ 80%

### Requirement: SonarCloud new_duplicated_lines_density MUST be < 3%

After the change, the SonarCloud quality gate `new_duplicated_lines_density` MUST be below 3%.

#### Scenario: Duplication threshold met

- GIVEN `providers-core` extracts ~165 lines
- WHEN duplicated lines density is calculated
- THEN `new_duplicated_lines_density` MUST be < 3%

### Requirement: All existing tests MUST continue passing

The change MUST NOT break any existing tests.

#### Scenario: Full test suite passes

- GIVEN `cargo test --workspace` is run
- THEN all 645+ existing tests MUST pass
- AND no test regressions MUST occur

### Requirement: New tests MUST pass

All newly added tests MUST pass.

#### Scenario: New provider tests pass

- GIVEN new tests for `stream()`, error mapping, sanitization
- WHEN tests are run
- THEN all new tests MUST pass

#### Scenario: Dashboard tests pass

- GIVEN dashboard tests are run
- THEN all dashboard tests MUST pass
