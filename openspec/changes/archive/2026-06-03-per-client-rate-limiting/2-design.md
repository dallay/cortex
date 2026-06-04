# Design: Per-Client Rate Limiting

## Technical Approach

`ApiKeyRateLimiter` is implemented but its parameter in
`crates/infrastructure/transport-axum/src/routes.rs:34` is `_`-prefixed and
never added to the layer stack. This change (1) wires it in, (2) replaces
hardcoded `tier_params()` with TOML `RateLimiterConfig`, (3) adds a sibling
`IpRateLimiter` for unauthenticated traffic, (4) extends `FallbackRouter` to
propagate upstream `Retry-After`, and (5) exposes admin CRUD. State is
in-memory (DashMap / `tokio::sync::Mutex`) — same pattern as
`LoginRateLimiter`.

## Architecture Decisions

| Decision                                           | Rationale                                                                                                                                    |
|----------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------|
| Reuse existing token-bucket in `ApiKeyRateLimiter` | Already battle-tested; rps derivable from `requests_per_minute / 60`. Sliding window can replace later without changing `RateLimitSnapshot`. |
| Keep `IpRateLimiter` separate                      | Mirrors `LoginRateLimiter`; IP and API-key limits evolve independently (own TOML section, own defaults).                                     |
| Custom rules in-process (DashMap)                  | Mutated at runtime via admin API; persistence not required for MVP, avoids hot-path async I/O.                                               |
| `Retry-After` propagation in `FallbackRouter`      | `router_impl.rs` already owns `CircuitState` — extend with `rate_limit_reset`, no new port, no handler-side re-decoding.                     |
| `RateLimitScope` in `shared-kernel`                | Same flavor as `ApiKeyTier` in `rook-core`; avoids a transport-axum dep from rook-usecases.                                                  |

## Middleware Chain (routes.rs)

```
Request
  → Security Headers (tower-http)
  → Cors (tower-http)
  → Body Limit (RequestBodyLimitLayer)
  → CsrfGuard (skips safe methods / public paths)
  → LoginRateLimiter (POST /login only)
  → IpRateLimiter (unauthenticated paths, NEW)
  → ApiKeyRateLimiter (authenticated paths, WIRED)
  → Authz → Route Handler → Provider
```

Both limiters run **before** `authz` to read headers it stamps (e.g.
`x-authz-tier`) — same pattern `LoginRateLimiter` uses for `ConnectInfo`.

## File Changes

| File                                                      | Action | Change                                                                                                               |
|-----------------------------------------------------------|--------|----------------------------------------------------------------------------------------------------------------------|
| `crates/infrastructure/transport-axum/src/routes.rs`      | Modify | Drop `_` prefix; add `.layer(from_fn_with_state(...))` for both limiters, ordered after `CsrfGuard`, before `authz`. |
| `…/transport-axum/src/middleware/api_key_rate_limiter.rs` | Modify | Accept `Arc<RateLimiterConfig>`; replace `tier_params()` with config lookup; stamp `x-ratelimit-*` headers.          |
| `…/transport-axum/src/middleware/ip_rate_limiter.rs`      | Create | `IpRateLimiter` + middleware; reuses `TokenBucket` from `login_rate_limiter.rs`; IP from `ConnectInfo`.              |
| `…/transport-axum/src/middleware/mod.rs`                  | Modify | `pub use IpRateLimiter;`                                                                                             |
| `…/transport-axum/src/handlers/{mod.rs,rate_limits.rs}`   | Create | `pub mod rate_limits;` + admin CRUD + status handler. Admin writes gated by `authz` Admin-scope.                     |
| `crates/domain/shared-kernel/src/{lib.rs,rate_limit.rs}`  | Create | `pub mod rate_limit;` exporting `RateLimitScope` + `RateLimitRule`.                                                  |
| `crates/application/rook-usecases/src/router_impl.rs`     | Modify | Add `rate_limit_reset` to `CircuitState`; `record_rate_limit()`; emit `RateLimitedError` carrying `retry_after`.     |
| `apps/rook/src/config.rs`                                 | Modify | New `RateLimiterConfig` / `TierConfig` / `IpRateLimitConfig`; add to `RookConfig`.                                   |
| `apps/rook/src/di.rs`                                     | Modify | Build `IpRateLimiter` + DashMap-backed `RateLimitRuleStore`; thread config into limiter via `with_config()`.         |
| `apps/rook/src/server.rs`                                 | Modify | Pass new fields; merge `/api/rate-limits`.                                                                           |

## Data Flow

```
HTTP → CsrfGuard
     → LoginRateLimiter (skip unless POST /login)
     → IpRateLimiter     → ConnectInfo → IpAddr → bucket.try_consume()? no → 429 + Retry-After
     → ApiKeyRateLimiter → tier=x-authz-tier; key=X-Authz-Auth-ID
                           try_consume(tier) → 429 + Retry-After + x-ratelimit-*
     → Authz → Handler → FallbackRouter
            on 429 from provider:
              retry_after = header("retry-after") || 60
              reset_at    = header("x-ratelimit-reset").parse()
              circuit.record_rate_limit(provider, retry_after, reset_at)
              error = RateLimitedError { provider, retry_after }
     → handler maps error → 429 + Retry-After
```

## Interfaces

```rust
// shared-kernel/src/rate_limit.rs
#[serde(rename_all = "kebab-case")]
pub enum RateLimitScope { ApiKey, IpAddress, Global }

pub struct RateLimitRule {
    pub id: String,
    pub scope: RateLimitScope,
    pub target: String,                                  // api_key_id | CIDR | "global"
    pub requests_per_minute: u32,
    pub requests_per_day: Option<u32>,
    pub tokens_per_minute: Option<u32>,
    pub burst: Option<u32>,
    pub provider_limits: HashMap<ProviderId, ProviderRateLimit>,
}

// apps/rook/src/config.rs
pub struct RateLimiterConfig {
    pub enabled: bool,
    pub default_tier: ApiKeyTier,
    pub tiers: HashMap<ApiKeyTier, TierConfig>,
    pub ip_limits: IpRateLimitConfig,
}
```

## Testing Strategy

| Layer       | What            | Approach                                                                                      |
|-------------|-----------------|-----------------------------------------------------------------------------------------------|
| Unit        | token bucket    | Keep existing `api_key_rate_limiter` tests; add config-driven tier and `IpRateLimiter` tests. |
| Unit        | `RateLimitRule` | Reject empty `target`; reject `Global` with non-`"global"` target.                            |
| Integration | chain order     | 429 with `Retry-After` **before** `authz`; 401/403 still come from `authz`.                   |
| Integration | 429 shape       | Free-tier over quota → 429 + `Retry-After` + `x-ratelimit-remaining: 0`.                      |
| Integration | refill          | Stub time, exhaust, advance 60s, assert refill.                                               |
| Integration | admin CRUD      | `POST` (admin) → 201; non-admin → 403; `GET .../status` returns counters.                     |
| Integration | upstream 429    | Mock provider `Retry-After: 30`; client response carries the same.                            |

## Migration / Rollout

No data migration. Each phase ships behind `rate_limiting.enabled`:

1. **Phase 1** — wire `ApiKeyRateLimiter` into `routes.rs`; deploy with `enabled = false`, flip to `true` to ship the fix.
2. **Phase 2** — TOML tiers replace `tier_params()`; defaults match current hardcoded values for parity.
3. **Phase 3** — `IpRateLimiter` added; off by default (`ip_limits` empty).
4. **Phase 4** — `FallbackRouter` records upstream 429s; metric first, client propagation second.
5. **Phase 5** — admin CRUD mounted under `/api/rate-limits`.

Rollback per phase: set `enabled = false` (or drop `[rate_limiting]`). No schema, no persisted state.

## Open Questions

- [ ] `IpRateLimiter` XFF trust: default **no** (`ConnectInfo` only); revisit if a reverse proxy is deployed.
- [ ] Admin scope: verify `authz` headers expose `is_admin`; if not, add to `authz::AuthzConfig`.
