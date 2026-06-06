# Tasks: coverage-and-bundle-analysis

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 700-900 (providers-core ~400, migrations ~200, tests ~300) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1: providers-core crate; PR 2: OpenAI migration; PR 3: Anthropic migration; PR 4: Groq+Ollama migration; PR 5: Dashboard Codecov |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: stacked-to-main|feature-branch-chain|size-exception|pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | providers-core crate with all modules + unit tests | PR 1 | Base: main; tests pass before migrating providers |
| 2 | OpenAI provider uses providers-core | PR 2 | Base: PR 1; focused migration, easy rollback |
| 3 | Anthropic provider uses providers-core | PR 3 | Base: PR 2; sequential migration |
| 4 | Groq + Ollama providers use providers-core | PR 4 | Base: PR 3; both similar scope, can combine |
| 5 | Dashboard Codecov setup | PR 5 | Base: PR 4; independent of Rust changes |

## Phase 1: providers-core crate creation

- [ ] 1.1 Create `crates/infrastructure/providers-core/Cargo.toml` with zero deps, edition 2021 (effort: small)
- [ ] 1.2 Create `providers-core/src/lib.rs` with public re-exports for role, sse, sanitize, validation, request modules (effort: small)
- [ ] 1.3 Create `providers-core/src/role.rs` — Role enum with to_role_string() for System/User/Assistant/Developer → provider strings (effort: small)
- [ ] 1.4 Create `providers-core/src/sse.rs` — parse_event_text() filters `data: ` prefix and `[DONE]`, process_bytes() with SseBuffer (effort: medium)
- [ ] 1.5 Create `providers-core/src/validation.rs` — validate_response() returns Ok for 2xx, Err(CortexError::provider) for non-2xx (effort: small)
- [ ] 1.6 Create `providers-core/src/request.rs` — send_stream_request() template with common headers, timeout, returns CortexResult<Response> (effort: small)
- [ ] 1.7 Create `providers-core/src/sanitize.rs` — sanitize_body() (JSON redaction + char-safe truncation to 200), char_safe_truncate() for UTF-8 safety (effort: medium)
- [ ] 1.8 Add `providers-core` to workspace Cargo.toml members list (effort: tiny)
- [ ] 1.9 Add unit tests for role.rs, sse.rs, sanitize.rs, validation.rs in `providers-core/tests/` using mockito or similar (effort: medium)

## Phase 2: Migrate providers (one at a time)

- [ ] 2.1 Add providers-core dependency to `providers-openai/Cargo.toml` (effort: tiny)
- [ ] 2.2 In providers-openai/src/provider.rs: replace duplicated role mapping with `use providers_core::role::Role` and `Role::to_role_string()` (effort: small)
- [ ] 2.3 In providers-openai/src/provider.rs: replace duplicated sanitize code with `use providers_core::sanitize::{sanitize_body, char_safe_truncate}` (effort: small)
- [ ] 2.4 Add error mapping tests for OpenAI: 401→auth_failed, 429 with Retry-After→rate_limited_with_reset, 400 sanitization (effort: medium)
- [ ] 2.5 Add providers-core dependency to `providers-anthropic/Cargo.toml` (effort: tiny)
- [ ] 2.6 In providers-anthropic/src/lib.rs: replace duplicated code (sanitize, SSE parsing, error mapping) with providers_core imports (effort: small)
- [ ] 2.7 Add stream() tests for Anthropic: HTTP 400 error, timeout, empty SSE response (effort: medium)
- [ ] 2.8 Add providers-core dependency to `providers-groq/Cargo.toml` (effort: tiny)
- [ ] 2.9 In providers-groq/src/lib.rs: replace duplicated code with providers_core imports (effort: small)
- [ ] 2.10 Add stream() tests for Groq: HTTP 429 rate limit, timeout, empty SSE response (effort: medium)
- [ ] 2.11 Add providers-core dependency to `providers-ollama/Cargo.toml` (effort: tiny)
- [ ] 2.12 In providers-ollama/src/lib.rs: replace duplicated code with providers_core imports (effort: small)
- [ ] 2.13 Add stream() tests for Ollama: HTTP 400 error, timeout, empty SSE response (effort: medium)

## Phase 3: Dashboard Codecov setup

- [ ] 3.1 Add @codecov/vite-plugin to dashboard package.json devDependencies (effort: tiny)
- [ ] 3.2 Update dashboard vite.config.ts: import codecovVitePlugin, add to plugins array with bundleName: "rook-dashboard", enableBundleAnalysis conditional on CODECOV_TOKEN (effort: small)

## Phase 4: Verification

- [ ] 4.1 Run `cargo test -p providers-core` — all new unit tests must pass (effort: tiny)
- [ ] 4.2 Run `cargo test -p providers-openai -p providers-anthropic -p providers-groq -p providers-ollama` — all provider tests pass (effort: tiny)
- [ ] 4.3 Run `cargo test --workspace` — full test suite passes (effort: tiny)
- [ ] 4.4 Run `just clippy` — no warnings (effort: tiny)
- [ ] 4.5 Verify SonarCloud quality gate passes (new_coverage ≥ 80%, new_duplicated_lines_density < 3%) (effort: tiny)
- [ ] 4.6 Verify Codecov bundle analysis uploads successfully when CODECOV_TOKEN is set (effort: tiny)

## Implementation Order

1. **providers-core first** — This is the foundation. All provider migrations depend on it. Write tests alongside each module.
2. **OpenAI migration second** — Most mature provider, easiest to migrate first as a pattern.
3. **Anthropic migration third** — Sequential migration catches issues early before touching more providers.
4. **Groq + Ollama migration fourth** — Both are similar scope, combined in one PR for efficiency.
5. **Dashboard Codecov fifth** — Independent of Rust changes, can be done in parallel or after.
6. **Verification last** — Run full CI locally before claiming completion.