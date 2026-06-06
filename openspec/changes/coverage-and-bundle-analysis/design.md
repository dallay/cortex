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
| `crates/infrastructure/providers-core/Cargo.toml` | Create | Zero-dep crate manifest |
| `crates/infrastructure/providers-core/src/lib.rs` | Create | Public re-exports |
| `crates/infrastructure/providers-core/src/role.rs` | Create | Role enum + to_role_string() |
| `crates/infrastructure/providers-core/src/sse.rs` | Create | parse_event_text(), process_bytes() |
| `crates/infrastructure/providers-core/src/sanitize.rs` | Create | sanitize_body(), char_safe_truncate() |
| `crates/infrastructure/providers-core/src/request.rs` | Create | send_stream_request() template |
| `crates/infrastructure/providers-core/tests/stream_tests.rs` | Create | Wiremock-based stream() tests |
| `crates/infrastructure/providers-core/tests/sanitize_tests.rs` | Create | Sanitization + truncation tests |
| `crates/infrastructure/providers-core/tests/validation_tests.rs` | Create | validate_response() tests |
| `crates/infrastructure/providers-anthropic/src/lib.rs` | Modify | Replace duplicated code with providers-core imports |
| `crates/infrastructure/providers-groq/src/lib.rs` | Modify | Replace duplicated code with providers-core imports |
| `crates/infrastructure/providers-ollama/src/lib.rs` | Modify | Replace duplicated code with providers-core imports |
| `crates/infrastructure/providers-openai/src/provider.rs` | Modify | Replace duplicated code with providers-core imports |
| `apps/rook/dashboard/package.json` | Modify | Add @codecov/vite-plugin devDependency |
| `apps/rook/dashboard/vite.config.ts` | Modify | Add codecovVitePlugin to plugins array |

## Interfaces / Contracts

### providers-core API

```rust
// role.rs
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn to_role_string(&self) -> &'static str;
}

// sse.rs
pub fn parse_event_text(line: &str) -> Option<String>;
// Returns the content after "data: " prefix, or None if line doesn't match SSE data format.
// Filters out "[DONE]" messages.
// Does NOT parse JSON — caller handles provider-specific parsing.

pub fn process_bytes(
    bytes: &[u8],
    buffer: &mut SseBuffer,
) -> impl Iterator<Item = String>;
// Wrapper around SseBuffer::push() that converts bytes to strings safely.
// Skips invalid UTF-8 without panicking.

// sanitize.rs
pub fn sanitize_body(body: &str, max_chars: usize) -> String;
// Parses body as JSON, redacts keys matching SENSITIVE_KEYS, truncates to max_chars.
// Falls back to plain truncation if not valid JSON.

pub fn char_safe_truncate(s: &str, max_chars: usize) -> String;
// Truncates at character boundary (not byte), adds "… (truncated)" if cut.

// request.rs
pub async fn send_stream_request(
    client: &Client,
    builder: RequestBuilder,
    url: &str,
) -> Result<Response, HttpError>;
// Template for building and sending streaming requests with consistent error handling.
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