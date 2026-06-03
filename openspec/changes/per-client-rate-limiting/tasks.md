# Tasks: Per-Client Rate Limiting

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 400-550 |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Suggested split | Single PR with 5 incremental phases |
| Delivery strategy | ask-on-risk |
| Chain strategy | size-exception |

Decision needed before apply: Yes
Chained PRs recommended: No
Chain strategy: size-exception
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Complete 5-phase implementation | PR 1 | All phases; tests/docs included; may request size:exception |

## Phase 1: Wire ApiKeyRateLimiter into routes.rs

- [x] 1.1 Remove `_` prefix from `api_key_rate_limiter` parameter in `transport-axum/src/routes.rs`
- [x] 1.2 Add `.layer(from_fn_with_state(...))` for `ApiKeyRateLimiter` after `CsrfGuard`, before `authz`
- [x] 1.3 Verify existing `ApiKeyRateLimiter` tests pass when wired
- [x] 1.4 Add integration test: authenticated request over tier limit returns 429 with `Retry-After`

## Phase 2: TOML tiers replace tier_params()

- [x] 2.1 Create `RateLimiterConfig`, `TierConfig`, `IpRateLimitConfig` in `apps/rook/src/config.rs`
- [x] 2.2 Add `[rate_limiting]` section with `enabled`, `default_tier`, `tiers` to TOML schema
- [x] 2.3 Modify `ApiKeyRateLimiter` to accept `Arc<RateLimiterConfig>` and replace `tier_params()` logic
- [x] 2.4 Update `apps/rook/src/di.rs` to build and inject `RateLimiterConfig` into limiter
- [x] 2.5 Add unit test: tier config parsed from TOML with 3 tiers (free/pro/enterprise)
- [x] 2.6 Add integration test: request using pro tier enforces correct rpm from config
- [x] 2.7 Add integration test: missing tier falls back to `default_tier` with warning log
- [x] 2.8 Add startup validation: reject `requests_per_minute = 0` or negative values

## Phase 3: IpRateLimiter added

- [x] 3.1 Create `transport-axum/src/middleware/ip_rate_limiter.rs` with `IpRateLimiter` struct
- [x] 3.2 Implement middleware fn extracting IP from `ConnectInfo<SocketAddr>`
- [x] 3.3 Reuse `TokenBucket` from `login_rate_limiter.rs` for per-IP buckets
- [x] 3.4 Add `IpRateLimiter` to middleware chain in `routes.rs` before `ApiKeyRateLimiter`
- [x] 3.5 Export `IpRateLimiter` in `transport-axum/src/middleware/mod.rs`
- [x] 3.6 Wire `IpRateLimiter` in `apps/rook/src/di.rs` with config
- [x] 3.7 Add unit test: unauthenticated request under IP limit is allowed
- [x] 3.8 Add integration test: unauthenticated request over IP limit returns 429
- [x] 3.9 Add integration test: authenticated request bypasses IP limiter

## Phase 4: FallbackRouter records upstream 429s

- [ ] 4.1 Add `rate_limit_reset` field to `CircuitState` in `rook-usecases/src/router_impl.rs`
- [ ] 4.2 Implement `record_rate_limit(provider, retry_after, reset_at)` method
- [ ] 4.3 Extract `Retry-After` header from provider 429 responses
- [ ] 4.4 Extract `X-RateLimit-Reset` header and parse as epoch timestamp
- [ ] 4.5 Emit `RateLimitedError` carrying `provider` and `retry_after` when upstream returns 429
- [ ] 4.6 Modify error handler to map `RateLimitedError` to 429 with client-facing `Retry-After`
- [ ] 4.7 Add integration test: mock provider 429 with `Retry-After: 30` → client receives same
- [ ] 4.8 Add integration test: provider 429 with `X-RateLimit-Reset` → circuit records backoff

## Phase 5: Admin CRUD mounted

- [ ] 5.1 Create `shared-kernel/src/rate_limit.rs` with `RateLimitScope` enum and `RateLimitRule` struct
- [ ] 5.2 Export `rate_limit` module in `shared-kernel/src/lib.rs`
- [ ] 5.3 Create `transport-axum/src/handlers/rate_limits.rs` with CRUD handlers
- [ ] 5.4 Implement `list_rules()` handler for `GET /api/rate-limits`
- [ ] 5.5 Implement `create_rule()` handler for `POST /api/rate-limits` with validation
- [ ] 5.6 Implement `update_rule()` handler for `PUT /api/rate-limits/:id`
- [ ] 5.7 Implement `delete_rule()` handler for `DELETE /api/rate-limits/:id`
- [ ] 5.8 Implement `get_status()` handler for `GET /api/rate-limits/:scope/:target/status`
- [ ] 5.9 Add DashMap-backed `RateLimitRuleStore` in `apps/rook/src/di.rs`
- [ ] 5.10 Mount `/api/rate-limits` routes in `apps/rook/src/server.rs`
- [ ] 5.11 Add authz guard requiring admin scope for all CRUD endpoints
- [ ] 5.12 Add unit test: `RateLimitRule` rejects empty `target`
- [ ] 5.13 Add integration test: admin POST creates rule → 201 with id
- [ ] 5.14 Add integration test: non-admin POST → 403
- [ ] 5.15 Add integration test: GET status returns current usage counters

## Phase 6: Documentation and Cleanup

- [ ] 6.1 Add `x-ratelimit-remaining`, `x-ratelimit-reset` headers to 200 responses
- [ ] 6.2 Update `docs/configuration.md` with `[rate_limiting]` section examples
- [ ] 6.3 Update `docs/api.md` with rate limit admin endpoints
- [ ] 6.4 Add example TOML config with all three tiers to `config.example.toml`
- [ ] 6.5 Verify all 6 proposal success criteria pass with integration tests
- [ ] 6.6 Run `just ci-local` to confirm fmt/clippy/test/doc green
