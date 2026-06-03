# Proposal: Per-Client Rate Limiting

## Intent

Implement configurable per-client rate limiting that throttles requests based on API key, IP address, or custom quota rules. The existing `ApiKeyRateLimiter` is implemented but unwired; rate limits are hardcoded per tier and not configurable. This change wires up the existing limiter, adds TOML-based configuration, per-IP limiting for unauthenticated requests, upstream 429 tracking, and an admin CRUD API for rate limit rules.

## Scope

### In Scope
- Wire up existing `ApiKeyRateLimiter` as proper middleware (remove `_` prefix, add to layer)
- Add TOML configuration section for rate limit tiers (Free/Pro/Enterprise with rpm/rpd/tpm)
- Per-IP rate limiter for unauthenticated requests (new `IpRateLimiter` middleware)
- Provider rate limit awareness — track per-client upstream 429s via `FallbackRouter`
- Admin API: GET/POST/PUT/DELETE `/api/rate-limits`
- Status endpoint: GET `/api/rate-limits/:scope/:target/status`

### Out of Scope
- Distributed rate limiting (Redis) — in-memory only for MVP
- Per-model rate limits (only per-client)
- Automatic tier migration from hardcoded to configurable rules
- Dashboard UI for rate limit management

## Approach

### Configuration (TOML)

```toml
[rate_limiting]
enabled = true
default_tier = "free"

[rate_limiting.tiers.free]
requests_per_minute = 60
requests_per_day = 1000
tokens_per_minute = 10000

[rate_limiting.tiers.pro]
requests_per_minute = 600
requests_per_day = 100000
tokens_per_minute = 100000

[rate_limiting.tiers.enterprise]
requests_per_minute = 6000
requests_per_day = 10000000
tokens_per_minute = 1000000
```

### Middleware Chain (routes.rs)

```
Request → IpRateLimiter (unauthenticated) → ApiKeyRateLimiter (authenticated) → Router
```

- `ApiKeyRateLimiter`: stamps consumption against key holder's tier config
- `IpRateLimiter`: token bucket per source IP for requests without API key

### Provider Rate Limit Awareness

Extend `FallbackRouter` to track per-client provider failures:
- On upstream 429, record client identifier + provider in circuit breaker state
- Extract `Retry-After` and `X-RateLimit-Reset` headers
- Return `429 Too Many Requests` to client with `Retry-After` when quota exhausted

### API Design

```
GET    /api/rate-limits              — list all rules (admin)
POST   /api/rate-limits              — create rule
PUT    /api/rate-limits/:id          — update rule
DELETE /api/rate-limits/:id          — delete rule
GET    /api/rate-limits/:scope/:target/status — current usage (key holder or admin)
```

### Data Model

```rust
RateLimitRule {
    id: String,
    scope: RateLimitScope, // ApiKey | IpAddress | Global
    target: String,
    requests_per_minute: u32,
    requests_per_day: Option<u32>,
    tokens_per_minute: Option<u32>,
    provider_limits: HashMap<ProviderId, ProviderRateLimit>,
}

ProviderRateLimit {
    requests_per_minute: u32,
    tokens_per_minute: Option<u32>,
    burst: Option<u32>,
}
```

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `transport-axum/src/routes.rs` | Modified | Wire `ApiKeyRateLimiter` and `IpRateLimiter` into middleware chain |
| `transport-axum/src/middleware/api_key_rate_limiter.rs` | Modified | Accept `RateLimitingConfig` from DI; remove `_` prefix |
| `transport-axum/src/middleware/ip_rate_limiter.rs` | New | Per-IP token bucket for unauthenticated requests |
| `apps/rook/src/config.rs` | Modified | Add `[rate_limiting]` TOML section with tier configs |
| `apps/rook/src/di.rs` | Modified | Wire `IpRateLimiter`, `RateLimitRuleRepository` into DI |
| `rook-usecases/src/router_impl.rs` | Modified | Track upstream 429s per client; use `Retry-After` / `X-RateLimit-Reset` |
| `shared-kernel/src/error.rs` | Modified | Ensure `RateLimitedError` covers all 429 variants |
| `transport-axum/src/handlers/rate_limits.rs` | New | Admin CRUD handlers + status endpoint |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| In-memory rate limit state resets on restart | High | Document as MVP limitation; Redis persistence is follow-up |
| Hardcoded tier values leak into existing auth paths | Med | Deprecate hardcoded paths; config is source of truth |
| `ApiKeyRateLimiter` not actually limiting (underscore bug) | Low | Add integration test that asserts 429 is returned on quota exceed |

## Rollback Plan

1. Revert `routes.rs` to `_api_key_rate_limiter` (disable middleware)
2. Remove `[rate_limiting]` section from TOML config
3. Revert `router_impl.rs` to discard upstream 429 tracking
4. Rollback is safe — no data migration required; in-memory state is ephemeral

## Dependencies

- `ApiKeyRateLimiter` in `transport-axum/src/middleware/api_key_rate_limiter.rs` (already implemented)
- `ApiKeyTier` in `rook-core/src/api_key.rs` (already implemented)
- `RateLimitedError` in `shared-kernel/src/error.rs` (already implemented)

## Success Criteria

- [ ] Authenticated request exceeding `requests_per_minute` returns `429 Too Many Requests` with `Retry-After`
- [ ] Unauthenticated request from same IP exceeding IP limit returns `429 Too Many Requests`
- [ ] `GET /api/rate-limits` lists all rules for admin principal
- [ ] `POST /api/rate-limits` creates a custom rule that overrides tier default for a specific key/IP
- [ ] `FallbackRouter` extracts and propagates `Retry-After` from upstream 429 responses
- [ ] `GET /api/rate-limits/:scope/:target/status` returns current usage counters
- [ ] All tier configs read from TOML, not hardcoded