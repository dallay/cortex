# Proposal: coverage-and-bundle-analysis

## Intent

Fix SonarCloud Quality Gate failures and enable Codecov bundle analysis. Current gates fail due to missing tests for provider stream functions and duplicated code across providers (3.9% vs 3% threshold).同时需要为dashboard启用Codecov bundle analysis。

## Scope

### In Scope
- Extract shared provider utilities into a new `providers-core` crate
- Add unit tests for untested `stream()` functions (Anthropic, Groq, Ollama)
- Add unit tests for error mapping functions (`map_*_http_error`)
- Add unit tests for sanitization functions
- Install and configure `@codecov/vite-plugin` in dashboard
- Configure Vitest to emit coverage for Codecov upload

### Out of Scope
- Refactoring non-streaming provider code
- Adding integration tests (already covered)
- Modifying provider API implementations
- Dashboard E2E test coverage

## Approach

**Recommended: Option A — Deduplication + Tests**

Solve BOTH issues permanently by extracting shared utilities into `providers-core`:

### Phase 1: Create providers-core crate
Extract common utilities from all providers into `providers-core/src/`:
- `role.rs` — Role enum with `to_provider_string()` mapping
- `sse.rs` — `parse_event_text()`, `process_bytes()` helpers
- `validation.rs` — `validate_response()` template
- `request.rs` — `send_stream_request()` template
- `sanitize.rs` — body sanitization and truncation utilities

### Phase 2: Update providers to use providers-core
- Update `provider-openai`, `provider-anthropic`, `provider-groq`, `provider-ollama`
- Remove duplicated code, use imports from `providers-core`
- Expected: ~165 lines removed, reducing duplicated_lines_density below 3%

### Phase 3: Add unit tests
Create `providers-core/tests/` with tests for:
- `stream()` for Anthropic, Groq, Ollama (OpenAI already covered)
- `map_*_http_error` functions across all providers
- Sanitization functions
- `validate_response()` for Groq, Ollama, Anthropic

### Phase 4: Codecov bundle setup
In `apps/rook/dashboard/`:
```bash
pnpm add @codecov/vite-plugin
```
Update `vite.config.ts` to include the plugin, add `CODECOV_TOKEN` to CI env.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `providers-core/` | New | New crate for shared provider utilities |
| `provider-openai/src/` | Modified | Use providers-core imports, remove duplicated code |
| `provider-anthropic/src/` | Modified | Use providers-core, add stream tests |
| `provider-groq/src/` | Modified | Use providers-core, add stream tests |
| `provider-ollama/src/` | Modified | Use providers-core, add stream tests |
| `apps/rook/dashboard/` | Modified | Add Codecov vite plugin |
| `apps/rook/dashboard/vite.config.ts` | Modified | Configure Codecov plugin |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Breaking provider implementations | Low | Run full test suite after each provider update |
| Duplicated lines remain above threshold | Low | Verify before completing Phase 2 |
| Codecov token not available in CI | Medium | Use CI environment variable, fallback to no-upload for dev |
| Test coverage still insufficient | Medium | Run `cargo test` with coverage to verify before PR |

## Rollback Plan

1. Revert `providers-core` changes and restore duplicated code in providers
2. Remove `@codecov/vite-plugin` from dashboard `package.json` and vite config
3. All changes are additive until providers switch to providers-core imports
4. Use git tags/commits to isolate phases for granular rollback

## Dependencies

- `@codecov/vite-plugin` package (dashboard)
- `CODECOV_TOKEN` env var in CI (from GitHub Actions secrets)
- `providers-core` crate creation requires workspace membership

## Success Criteria

- [ ] SonarCloud `new_coverage` ≥ 80%
- [ ] SonarCloud `new_duplicated_lines_density` < 3%
- [ ] All 4 providers' `stream()` functions have unit tests
- [ ] All `map_*_http_error` functions have unit tests
- [ ] `cargo test --workspace` passes with no regressions
- [ ] Dashboard build uploads coverage to Codecov on CI
- [ ] Codecov bundle analysis shows in PR comments

## Tradeoffs

| Option | Pros | Cons |
|--------|------|------|
| **A: Deduplication + Tests** | Permanent fix for both issues; cleaner architecture; easier future changes | More work upfront; requires new crate |
| **B: Tests only** | Faster to implement; less risk of breaking changes | Duplicated lines stay; same patterns repeated across providers |
| **A + Codecov** | Comprehensive solution | Longer implementation time |

**Recommendation**: Option A — the deduplication directly addresses the duplicated lines issue while enabling better test coverage.