# Design: coverage-and-bundle-analysis

## Technical Approach

Extract shared provider utilities into a new zero-dependency `providers-core` crate, then migrate each provider to use it. This eliminates ~165 lines of duplication while enabling unit tests for the shared code. Concurrently, configure Codecov bundle analysis in the dashboard.

**Relation to proposal**: Follows Option A — Deduplication + Tests, addressing both SonarCloud quality gate failures (duplicated lines density 3.9% → <3%, coverage gaps) and Codecov bundle analysis setup.

## Architecture Decisions

### Decision: providers-core dependency policy

**Choice**: Zero external service dependencies (like shared-kernel), but standard library helpers allowed.
**Alternatives considered**: Allow reqwest/cached HTTP clients — rejected because providers-core would then pull the async runtime into a shared utilities crate, creating coupling.
**Rationale**: shared-kernel demonstrates this pattern works. Providers-core utilities (SSE parsing, sanitization, role mapping) don't need HTTP — they operate on in-memory data already fetched by the provider.

### Decision: SSE parsing strategy

**Choice**: `parse_event_text(line: &str) -> Option<SseEvent>` returns raw SSE `data:` content as `String`, not a generic enum.
**Alternatives considered**: Parse into provider-specific enums in providers-core — rejected because each provider has different JSON shapes for SSE payloads.
**Rationale**: Keep providers-core focused on SSE framing (stripping `data: ` prefix, filtering `[DONE]`), not semantic parsing. Provider-specific JSON parsing stays in each provider crate.

### Decision: Migration order

**Choice**: Create providers-core first, then migrate providers one-by-one: OpenAI → Anthropic → Groq → Ollama.
**Alternatives considered**: Migrate all providers simultaneously — rejected due to high risk of breaking all providers at once.
**Rationale**: Sequential migration with full test run after each enables early detection of regressions.

### Decision: sanitize_body behavior

**Choice**: `sanitize_body(body: &str, max_chars: usize) -> String` applies JSON redaction of sensitive keys THEN char-safe truncation.
**Alternatives considered**: Truncation only — rejected because error responses often contain JSON with credentials.
**Rationale**: OpenAI provider already implements this two-step approach. Standardizing it in providers-core ensures consistent behavior.

## Data Flow

```
┌─────────────────────────────────────────────────────────┐
│                    providers-core                        │
│  ┌─────────┐  ┌──────────┐  ┌────────────┐  ┌────────┐ │
│  │  role   │  │   sse    │  │ sanitize   │  │request │ │
│  │  enum   │  │  parse   │  │ +truncate  │  │  send  │ │
│  └─────────┘  └──────────┘  └────────────┘  └────────┘ │
└───────────────────────┬─────────────────────────────────┘
                        │ imports
        ┌───────────────┼───────────────┬───────────────┐
        ▼               ▼               ▼               ▼
┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│ providers-   │ │ providers-   │ │ providers-   │ │ providers-   │
│ openai       │ │ anthropic    │ │ groq         │ │ ollama       │
│              │ │              │ │              │ │              │
│ stream()     │ │ stream()     │ │ stream()     │ │ stream()     │
│ complete()   │ │ complete()   │ │ complete()   │ │ complete()   │
│ + tests      │ │ + tests      │ │ + tests      │ │ + tests      │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/infrastructure/providers-core/Cargo.toml` | Create | Zero-dep crate (providers-core, no external service deps) |
| `crates/infrastructure/providers-core/src/lib.rs` | Create | Public re-exports: Role, role_to_string, SseEvent, parse_event_text, process_bytes, sanitize_body, char_safe_truncate, validate_response |
| `crates/infrastructure/providers-core/src/role.rs` | Create | Re-exports CoreRole from rook_core + role_to_string() standalone helper + unit tests |
| `crates/infrastructure/providers-core/src/sse.rs` | Create | parse_event_text() → Option<SseEvent>, process_bytes() iterator, SseEvent enum, SseBuffer struct |
| `crates/infrastructure/providers-core/src/sanitize.rs` | Create | sanitize_body() with recursive JSON redaction, char_safe_truncate(), SENSITIVE_KEYS constant |
| `crates/infrastructure/providers-core/src/request.rs` | Create | RequestTemplate, CommonHeaders, serialize_body, serialize_body_for_log |
| `crates/infrastructure/providers-core/src/validation.rs` | Create | validate_response(), validate_response_with_type() |
| `crates/infrastructure/providers-core/tests/stream_tests.rs` | Create | Wiremock-based stream() tests |
| `crates/infrastructure/providers-core/tests/sanitize_tests.rs` | Create | Sanitization + truncation tests |
| `crates/infrastructure/providers-core/tests/validation_tests.rs` | Create | validate_response() tests |
| `crates/infrastructure/provider-utils/Cargo.toml` | Create | Shared provider utilities crate (not a workspace member — standalone) |
| `crates/infrastructure/provider-utils/src/lib.rs` | Create | Public re-exports |
| `crates/infrastructure/provider-utils/src/error.rs` | Create | sanitize_error_body() with recursive JSON redaction |
| `crates/infrastructure/provider-utils/src/token_bucket.rs` | Create | TokenBucket, RetryAfterExt, rate_limit_headers() |
| `crates/infrastructure/providers-anthropic/src/lib.rs` | Modify | Replace duplicated SSE parsing + sanitize_body with providers-core imports; add stream() HTTP status fix |
| `crates/infrastructure/providers-groq/src/lib.rs` | Modify | Replace SSE parsing + validate_response with providers-core; remove unused validate_response |
| `crates/infrastructure/providers-ollama/src/lib.rs` | Modify | Replace from_utf8 with process_bytes; add 19 unit tests |
| `crates/infrastructure/providers-openai/src/provider.rs` | Modify | Replace local sanitize_body with providers_core::sanitize::sanitize_body |
| `crates/infrastructure/providers-openai/tests/error_mapping_tests.rs` | Create | Error mapping tests: 401, 429, 400 responses; extracted test helpers |
| `apps/rook/dashboard/package.json` | Modify | Add @codecov/vite-plugin devDependency |
| `apps/rook/dashboard/vite.config.ts` | Modify | Add codecovVitePlugin to plugins array |

## Interfaces / Contracts

### providers-core API

```rust
// role.rs — re-exports CoreRole from rook_core + provides role_to_string helper
pub use rook_core::Role;         // CoreRole enum from domain layer
pub use rook_core::RoleExt;      // trait with to_role_string() method
pub fn role_to_string(role: Role) -> &'static str;  // standalone helper for use in match arms

// sse.rs
#[derive(Debug, Clone)]
pub enum SseEvent {
    Data(String),
    Done,
}

pub fn parse_event_text(line: &str) -> Option<SseEvent>;
// Returns SseEvent::Data(content) for lines matching "data: <content>",
// SseEvent::Done for "[DONE]" messages, or None for other lines.
// Does NOT parse JSON — caller handles provider-specific parsing.

pub fn process_bytes(
    bytes: &[u8],
    buffer: &mut SseBuffer,
) -> impl Iterator<Item = String>;
// Wrapper around SseBuffer::push() that converts bytes to strings safely.
// Skips invalid UTF-8 without panicking.

// sanitize.rs
pub fn sanitize_body(body: &str) -> String;
// Parses body as JSON, recursively redacts keys matching SENSITIVE_KEYS (any nesting level),
// then truncates to MAX_LENGTH (200 chars).
// Falls back to plain truncation if not valid JSON.

pub fn char_safe_truncate(s: &str, max_chars: usize) -> String;
// Truncates at character boundary (not byte), adds "… (truncated)" if cut.

// request.rs
pub struct CommonHeaders { /* ... */ }
pub struct RequestTemplate { /* ... */ }
pub fn serialize_body<T: Serialize>(body: &T) -> Option<Vec<u8>>;
pub fn serialize_body_for_log<T: Serialize>(body: &T) -> String;

// validation.rs
pub fn validate_response(response: &Response) -> Result<(), ProviderError>;
// Checks response status code; returns Ok(()) for 2xx, Err(ProviderError) otherwise.
```

### SENSITIVE_KEYS constant

```rust
const SENSITIVE_KEYS: &[&str] = &[
    "api_key", "authorization", "token", "access_token",
    "secret", "headers",
];
```

### Dashboard Codecov Configuration

```typescript
// vite.config.ts
import { codecovVitePlugin } from '@codecov/vite-plugin'

export default defineConfig({
  plugins: [
    vue(),
    tailwindcss(),
    codecovVitePlugin({
      enableBundleAnalysis: process.env.CODECOV_TOKEN !== undefined,
      bundleName: 'rook-dashboard',
      uploadToken: process.env.CODECOV_TOKEN,
    }),
  ],
  // ... existing config
})
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | `parse_event_text` | Test data: prefix stripping, [DONE] filtering, empty lines |
| Unit | `char_safe_truncate` | ASCII, multi-byte UTF-8 (emoji), boundary conditions |
| Unit | `sanitize_body` | JSON redaction, plain text truncation, edge cases |
| Unit | Role enum mapping | All variants produce correct strings |
| Integration | `send_stream_request` | Wiremock: 200, 4xx, 5xx responses |
| Integration | `validate_response` | Wiremock: success and error status codes |
| Integration | Provider `stream()` | Wiremock: multi-event SSE, timeout, empty body |

### Wiremock patterns

```rust
// Example: stream() test structure
wiremock_server.stub()
    .matchers(method("POST"), path("/v1/messages"))
    .respond_with(status(200).body(sse_events.join("\n")))
    .expect(1)
    .named("anthropic_stream");

// SSE event fixtures
const SSE_EVENTS: &str = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":10}}
"#;
```

### Coverage targets

| Component | Target |
|-----------|--------|
| providers-core | 90%+ line coverage |
| providers-anthropic stream() | 80%+ |
| providers-groq stream() | 80%+ |
| providers-ollama stream() | 80%+ |
| Dashboard bundle | Configured for Codecov upload |

## Migration / Rollback

### Migration Phases

1. **Create providers-core** — Add crate, implement all modules, add tests
2. **Migrate OpenAI** — Update Cargo.toml, replace imports, verify `cargo test -p providers-openai`
3. **Migrate Anthropic** — Same pattern
4. **Migrate Groq** — Same pattern
5. **Migrate Ollama** — Same pattern
6. **Codecov setup** — Update dashboard package.json and vite.config.ts
7. **CI verification** — Run `just ci-local` to confirm all gates pass

### Rollback

- Git revert each provider migration individually
- Remove providers-core crate if all providers reverted
- Revert dashboard changes via `git checkout`

## Open Questions

- [ ] Should `providers-core` live in `crates/domain/` (like shared-kernel) or `crates/infrastructure/`?
  - **Recommendation**: `crates/infrastructure/` since providers are infrastructure and providers-core is provider-specific utilities, not domain model.
- [ ] Should we version providers-core separately or as part of the workspace version?
  - **Recommendation**: Workspace version only — it's an internal utility crate, not a public API.
- [ ] Do we need a feature flag for Codecov bundle analysis in dashboard?
  - **Recommendation**: No — the `enableBundleAnalysis: process.env.CODECOV_TOKEN !== undefined` conditional already provides the opt-in behavior.