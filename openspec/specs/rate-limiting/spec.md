# Per-Client Rate Limiting — Specification

## Purpose

Defines per-client rate limiting behavior, the middleware chain, upstream 429 awareness, TOML configuration, and the admin CRUD API. Wires the existing `ApiKeyRateLimiter` into `routes.rs`, introduces a new `IpRateLimiter` for unauthenticated traffic, and surfaces rate limit management via `/api/rate-limits`.

## Affected Domains

| Domain           | Type     | Notes                                       |
|------------------|----------|---------------------------------------------|
| `rate-limiting`  | New      | Middleware, sliding window, tier resolution |
| `api-key`        | Modified | Per-key consumption and tier lookup         |
| `transport-axum` | Modified | Middleware chain, admin handlers            |
| `config`         | Modified | `[rate_limiting]` TOML section              |
| `router`         | Modified | `FallbackRouter` upstream 429 handling      |

---

## R1: Rate Limit Middleware

### Requirement: Rate Limit Middleware

The system MUST check rate limits BEFORE routing any request to a provider. The system MUST use a sliding window algorithm. The system MUST return HTTP 429 with a `Retry-After` header when a limit is exceeded. Rate limit state MUST be stored in-memory (acceptable for the MVP per architecture docs).

```gherkin
Scenario: Request under limit is allowed
  Given a rate limiter configured to 100 requests per minute
  When the client has made 99 requests in the current window
  Then the 100th request is allowed
  And the response includes "X-RateLimit-Remaining: 0"

Scenario: Request exactly at limit is allowed
  Given a rate limiter configured to 100 requests per minute
  When the client has made exactly 100 requests in the current window
  Then the 100th request is allowed

Scenario: Request over limit returns 429 with Retry-After
  Given a rate limiter configured to 100 requests per minute
  When the client has already consumed 100 requests in the current window
  Then the next request returns HTTP 429
  And the response includes a "Retry-After" header (seconds until reset)
  And the response includes an "X-RateLimit-Reset" header (epoch seconds)

Scenario: Rate limit runs before router
  Given a request that would be routed to provider "openai"
  When the request exceeds the configured limit
  Then no provider call is made
  And the client receives HTTP 429
```

---

## R2: Per-API-Key Rate Limiting

### Requirement: Per-API-Key Rate Limiting

The system MUST support per-API-key rate limits. Each API key MUST be associated with a tier (`Free`, `Pro`, `Enterprise`). Tier limits MUST be configurable via TOML. The system MUST extract the API key from `Authorization: Bearer <key>` OR `X-API-Key: <key>`.

```gherkin
Scenario: API key under rate limit is allowed
  Given an authenticated request with API key in tier "pro"
  And the pro tier limit is 600 requests per minute
  When the client makes 599 requests in one minute
  Then all requests are allowed
  And each response includes an "X-RateLimit-Remaining" header

Scenario: API key over rate limit returns 429
  Given an authenticated request with API key in tier "pro"
  And the pro tier limit is 600 requests per minute
  When the client makes 601 requests in one minute
  Then the 601st request returns HTTP 429
  And the response includes a "Retry-After" header
  And the response includes an "X-RateLimit-Reset" header

Scenario: API key extracted from Authorization Bearer header
  Given a request with header "Authorization: Bearer rook_live_abc123"
  When the rate limit middleware runs
  Then the key "rook_live_abc123" is used to resolve the tier and limits

Scenario: API key extracted from X-API-Key header
  Given a request with header "X-API-Key: rook_live_abc123" and no Authorization header
  When the rate limit middleware runs
  Then the key "rook_live_abc123" is used to resolve the tier and limits

Scenario: Key without explicit tier uses default_tier
  Given an API key with no explicit tier
  And "[rate_limiting].default_tier" is "free"
  When the key authenticates
  Then the free tier limits are applied to the request
```

---

## R3: Per-IP Rate Limiting

### Requirement: Per-IP Rate Limiting for Unauthenticated Requests

The system MUST enforce per-IP rate limits for unauthenticated requests. The system MUST extract the client IP from `X-Forwarded-For` (first hop) or fall back to `X-Real-IP`. IP rate limits MUST be configured separately from API key limits.

```gherkin
Scenario: Unauthenticated request under IP limit is allowed
  Given a request without an API key
  And the IP rate limit is 30 requests per minute
  When the same IP makes 29 requests in one minute
  Then all requests are allowed

Scenario: Unauthenticated request over IP limit returns 429
  Given a request without an API key
  And the IP rate limit is 30 requests per minute
  When the same IP makes 31 requests in one minute
  Then the 31st request returns HTTP 429
  And the response includes a "Retry-After" header

Scenario: Client IP resolved from X-Forwarded-For
  Given a request with header "X-Forwarded-For: 203.0.113.42, 10.0.0.1"
  When the IP rate limiter runs
  Then the client IP "203.0.113.42" is used for the rate limit bucket

Scenario: Client IP falls back to X-Real-IP
  Given a request with header "X-Real-IP: 198.51.100.7" and no X-Forwarded-For
  When the IP rate limiter runs
  Then the client IP "198.51.100.7" is used

Scenario: Authenticated request bypasses IP rate limit
  Given a request with a valid API key
  When the request enters the middleware chain
  Then the IP rate limiter is skipped
  And only the API key rate limit applies
```

---

## R4: Provider Rate Limit Awareness

### Requirement: Upstream 429 Detection and Backoff

The system MUST detect 429 responses from upstream providers. The system MUST extract `Retry-After` and `X-RateLimit-Reset` headers from provider responses. The system MUST back off from that provider for the indicated duration. The system MUST record the rate limit hit in the circuit breaker.

```gherkin
Scenario: Upstream 429 propagates Retry-After to client
  Given a request routed to provider "openai"
  When the provider returns HTTP 429 with header "Retry-After: 30"
  Then the client receives HTTP 429
  And the response includes "Retry-After: 30"

Scenario: X-RateLimit-Reset triggers provider backoff
  Given a request routed to provider "openai"
  When the provider returns HTTP 429 with header "X-RateLimit-Reset: 1717420000"
  Then the FallbackRouter records a backoff for "openai" until that timestamp
  And subsequent requests avoid "openai" until the backoff expires

Scenario: Rate limit hit recorded in circuit breaker
  Given a request routed to provider "anthropic"
  When the provider returns HTTP 429
  Then the circuit breaker for "anthropic" records one rate-limit failure
  And the circuit breaker state is queryable via the admin API

Scenario: Successful provider response records no backoff
  Given a request routed to provider "openai"
  When the provider returns HTTP 200
  Then no backoff is recorded for "openai"
  And no rate limit hit is logged against the client
```

---

## R5: Configuration

### Requirement: Rate Limit Tiers Configurable via TOML

The system MUST support TOML configuration for rate limit tiers. The system MUST support `requests_per_minute` and `requests_per_day` per tier. The system MUST support `tokens_per_minute` per tier. The system MUST support a `default_tier` for keys without an explicit tier.

```gherkin
Scenario: Tier config parsed from TOML
  Given a TOML config containing:
    """
    [rate_limiting]
    enabled = true
    default_tier = "free"

    [rate_limiting.tiers.pro]
    requests_per_minute = 600
    requests_per_day = 100000
    tokens_per_minute = 100000
    """
  When the application starts
  Then the pro tier is loaded with 600 rpm, 100000 rpd, 100000 tpm
  And "default_tier" is "free"

Scenario: Token budget enforced
  Given a tier configured with "tokens_per_minute = 100000"
  When a request consumes 100001 tokens within one minute
  Then the request is rejected with HTTP 429
  And the response includes "Retry-After"

Scenario: Daily request quota enforced
  Given a tier configured with "requests_per_day = 1000"
  When an API key on that tier makes 1001 requests in a calendar day
  Then the 1001st request returns HTTP 429
  And the response indicates a daily quota violation

Scenario: Missing tier section falls back to default_tier
  Given a TOML config without a "[rate_limiting.tiers.enterprise]" section
  When a request authenticates with an enterprise-tier API key
  Then the system applies the "default_tier" limits as a fallback
  And logs a warning that the explicit tier config is missing

Scenario: Invalid TOML config fails startup
  Given a TOML config where "requests_per_minute = 0" for a tier
  When the application starts
  Then startup fails with a configuration validation error
```

---

## R6: Admin API

### Requirement: Admin API for Rate Limit Rules

The system MUST expose the following admin endpoints under `/api/rate-limits`:

| Method   | Path                                     | Purpose                |
|----------|------------------------------------------|------------------------|
| `GET`    | `/api/rate-limits`                       | List all rules         |
| `POST`   | `/api/rate-limits`                       | Create a new rule      |
| `PUT`    | `/api/rate-limits/:id`                   | Update a rule          |
| `DELETE` | `/api/rate-limits/:id`                   | Delete a rule          |
| `GET`    | `/api/rate-limits/:scope/:target/status` | Current usage counters |

All admin endpoints MUST require admin session auth.

```gherkin
Scenario: List all rate limit rules
  Given 3 rate limit rules exist in storage
  When admin calls GET /api/rate-limits
  Then the response is HTTP 200 with a JSON array of 3 rules

Scenario: Create a rate limit rule
  Given admin sends POST /api/rate-limits with:
    """
    {
      "scope": "ApiKey",
      "target": "key_abc123",
      "requests_per_minute": 120,
      "requests_per_day": 5000,
      "tokens_per_minute": 50000
    }
    """
  When the request is valid
  Then the response is HTTP 201 with the new rule including its "id"
  And the rule is persisted to storage

Scenario: Update an existing rate limit rule
  Given a rate limit rule with id "rule_001"
  When admin calls PUT /api/rate-limits/rule_001 with new values
  Then the response is HTTP 200 with the updated rule
  And the rule in storage reflects the new values

Scenario: Delete a rate limit rule
  Given a rate limit rule with id "rule_001"
  When admin calls DELETE /api/rate-limits/rule_001
  Then the response is HTTP 204
  And the rule is no longer present in storage

Scenario: Get rate limit status for a key
  Given API key "key_abc123" has consumed 250 of 600 requests in the current minute
  When admin calls GET /api/rate-limits/api_key/key_abc123/status
  Then the response is HTTP 200 with current usage counters:
    """
    {
      "scope": "ApiKey",
      "target": "key_abc123",
      "current_minute_count": 250,
      "current_day_count": 4250,
      "remaining_minute": 350,
      "remaining_day": 750,
      "reset_at": "2026-06-03T13:55:00Z"
    }
    """

Scenario: Create rule with missing required field returns 400
  Given admin sends POST /api/rate-limits without a "scope" field
  When the request is received
  Then the response is HTTP 400 with code "VALIDATION_ERROR"

Scenario: Get status for non-existent target returns 404
  Given no rate limit rule exists for IP "198.51.100.99"
  When admin calls GET /api/rate-limits/ip_address/198.51.100.99/status
  Then the response is HTTP 404 with code "NOT_FOUND"
```

---

## Out of Scope (per proposal)

- Distributed rate limiting (Redis) — in-memory only for MVP
- Per-model rate limits (only per-client)
- Automatic tier migration from hardcoded to configurable rules
- Dashboard UI for rate limit management
