# Health Check with Circuit Breaker Visibility — Specification

## Purpose

Expose circuit breaker state and health check data through public and authenticated endpoints to enable operators and monitoring systems to observe resilience behavior in real-time. This capability addresses the current limitation where circuit breaker state in `FallbackRouter` is internal and invisible to operators.

## Requirements

### Requirement: Enhanced Public Health Endpoint

The system MUST extend the existing `/health` endpoint to include circuit breaker state for each provider. The system MUST maintain backwards compatibility with existing consumers. The endpoint MUST NOT require authentication.

#### Scenario: Healthy provider with closed circuit

- GIVEN a provider "openai" is healthy
- AND the circuit breaker is closed
- WHEN a client calls `GET /health`
- THEN the response status is HTTP 200
- AND the response includes `providers[].circuit_state: "closed"`
- AND the response includes `providers[].failure_count: 0`
- AND the response includes `providers[].cooldown_until: null`

#### Scenario: Provider with open circuit

- GIVEN a provider "anthropic" has failed 3 times
- AND the circuit breaker is open until 2026-06-04T11:00:00Z
- WHEN a client calls `GET /health`
- THEN the response status is HTTP 200
- AND the response includes `providers[].circuit_state: "open"`
- AND the response includes `providers[].cooldown_until: "2026-06-04T11:00:00Z"`
- AND the response includes `providers[].failure_count: 3`

#### Scenario: Backwards compatibility for existing consumers

- GIVEN an existing monitoring tool that only reads `status` and `providers[].healthy` fields
- WHEN the tool calls `GET /health` after the enhancement
- THEN the tool continues to work without modification
- AND new fields are ignored by the tool (JSON forward compatibility)

### Requirement: Authenticated Resilience API

The system MUST provide a new `/api/resilience` endpoint that returns detailed circuit breaker state for all providers. The endpoint MUST require session authentication. The endpoint MUST include additional details not exposed in the public health endpoint.

#### Scenario: Authenticated request returns detailed state

- GIVEN a valid session cookie
- WHEN a client calls `GET /api/resilience`
- THEN the response status is HTTP 200
- AND the response includes per-provider circuit state
- AND the response includes `last_failure_message` for each provider
- AND the response includes `consecutive_failures` count
- AND the response includes `last_success_at` timestamp

#### Scenario: Unauthenticated request is rejected

- GIVEN no session cookie
- WHEN a client calls `GET /api/resilience`
- THEN the response status is HTTP 401
- AND the response body is `{"error": "Unauthorized"}`

#### Scenario: Circuit state matches FallbackRouter internal state

- GIVEN FallbackRouter has provider "gemini" circuit-opened with 5 failures
- WHEN an authenticated client calls `GET /api/resilience`
- THEN the response shows `gemini.circuit_state: "open"`
- AND the response shows `gemini.consecutive_failures: 5`
- AND the data matches the internal circuit breaker state exactly

### Requirement: Background Health Check Task

The system MUST spawn a background task that periodically calls `HealthCheck::refresh()` to update provider health status. The interval MUST be configurable via the `HEALTH_CHECK_INTERVAL_SECS` environment variable. The task MUST default to 30 seconds if not configured.

#### Scenario: Background task updates health status periodically

- GIVEN the background task is running with a 30-second interval
- AND a provider "openai" fails at time T
- WHEN 30 seconds elapse
- THEN the background task calls `HealthCheck::refresh()`
- AND the health status is updated to reflect the failure

#### Scenario: Background task respects configured interval

- GIVEN `HEALTH_CHECK_INTERVAL_SECS=10`
- WHEN the background task starts
- THEN the task calls `HealthCheck::refresh()` every 10 seconds

#### Scenario: Health check updates visible in endpoints immediately

- GIVEN the background task just completed a health check
- AND it discovered provider "ollama" is now healthy
- WHEN a client calls `GET /health` immediately after
- THEN the response reflects the updated "ollama" health status

### Requirement: Graceful Background Task Shutdown

The system MUST stop the background health check task cleanly during server shutdown. The task MUST NOT block shutdown. The system MUST NOT panic if the task is dropped during shutdown.

#### Scenario: Background task stops when server shuts down

- GIVEN the background task is running
- WHEN the server receives a shutdown signal
- THEN the background task exits on its next iteration
- AND shutdown completes within 5 seconds
- AND no panic occurs

#### Scenario: Background task uses weak reference to HealthCheck

- GIVEN the background task holds a weak reference to `HealthCheck`
- WHEN the server drops the `HealthCheck` Arc during shutdown
- THEN the background task detects the weak reference is invalid
- AND the task exits immediately

### Requirement: Circuit State Exposure Method

The system MUST provide a public method `circuit_states()` on `FallbackRouter` that returns a snapshot of all circuit breaker states. The method MUST NOT block routing decisions. The method MUST clone circuit state from the internal `DashMap`.

#### Scenario: circuit_states returns current state snapshot

- GIVEN FallbackRouter has 3 providers with different circuit states
- WHEN a caller invokes `router.circuit_states()`
- THEN the method returns a `Vec<(ProviderId, CircuitState)>` with 3 entries
- AND each entry includes `circuit_state`, `failure_count`, `cooldown_until`

#### Scenario: circuit_states does not block routing

- GIVEN a routing decision is in progress
- WHEN another thread calls `circuit_states()`
- THEN the routing decision completes without blocking
- AND the state query completes without blocking

## API Contracts

### GET /health

**Response (HTTP 200):**

```json
{
  "status": "healthy",
  "providers": [
    {
      "id": "openai",
      "name": "OpenAI",
      "healthy": true,
      "circuit_state": "closed",
      "failure_count": 0,
      "cooldown_until": null
    },
    {
      "id": "anthropic",
      "name": "Anthropic",
      "healthy": false,
      "circuit_state": "open",
      "failure_count": 3,
      "cooldown_until": "2026-06-04T11:00:00Z"
    }
  ]
}
```

**New Fields:**

- `circuit_state`: `"closed" | "open" | "half_open"`
- `failure_count`: non-negative integer
- `cooldown_until`: ISO 8601 timestamp or `null`

**Backwards Compatibility:** Existing fields (`status`, `healthy`) remain unchanged.

### GET /api/resilience

**Authentication:** Session cookie required (HTTP 401 if missing).

**Response (HTTP 200):**

```json
{
  "providers": [
    {
      "id": "openai",
      "circuit_state": "closed",
      "consecutive_failures": 0,
      "cooldown_until": null,
      "last_failure_at": null,
      "last_failure_message": null,
      "last_success_at": "2026-06-04T10:30:00Z"
    },
    {
      "id": "anthropic",
      "circuit_state": "open",
      "consecutive_failures": 5,
      "cooldown_until": "2026-06-04T11:00:00Z",
      "last_failure_at": "2026-06-04T10:45:00Z",
      "last_failure_message": "Connection timeout after 30s",
      "last_success_at": "2026-06-04T10:00:00Z"
    }
  ]
}
```

**Error Response (HTTP 401):**

```json
{
  "error": "Unauthorized"
}
```

## Edge Cases

### Empty Provider List

- GIVEN no providers are configured
- WHEN a client calls `GET /health`
- THEN the response is `{"status": "healthy", "providers": []}`

### Concurrent Circuit State Reads

- GIVEN 100 concurrent requests to `GET /api/resilience`
- WHEN all requests call `circuit_states()` simultaneously
- THEN all requests complete without deadlock
- AND all responses are consistent with a valid state snapshot

### Background Task During Shutdown

- GIVEN the background task is mid-refresh when shutdown starts
- WHEN the server begins shutdown
- THEN the current refresh completes
- AND the next iteration is skipped
- AND the task exits cleanly

### Provider Added After Task Start

- GIVEN the background task is running
- WHEN a new provider is added to the TOML config (requires restart)
- THEN after restart, the background task includes the new provider in health checks

### Circuit Opens During Health Check

- GIVEN the background task is checking provider health
- WHEN a circuit opens due to a concurrent routing failure
- THEN the health check completes with stale data
- AND the next iteration reflects the updated circuit state

## Non-Functional Requirements

### Performance

- Circuit state queries MUST complete in <10ms under normal load (<100 providers)
- Background health checks MUST NOT increase routing latency
- `DashMap` read contention MUST NOT degrade routing throughput by >5%

### Backwards Compatibility

- Existing `/health` consumers MUST continue to work without modification
- New fields MUST be additive only (no removals or renames)
- HTTP status codes for `/health` MUST remain unchanged (always 200)

### Observability

- Background task MUST log start/stop events at INFO level
- Background task MUST log health check completion at DEBUG level
- Circuit state changes MUST be logged at WARN level (already implemented in circuit breaker)

## Acceptance Criteria

- [ ] `/health` endpoint includes `circuit_state`, `cooldown_until`, `failure_count` for each provider
- [ ] `/api/resilience` endpoint returns detailed circuit state with session auth
- [ ] Background health check task runs at configured interval (default 30s)
- [ ] Background task shuts down within 5s of server shutdown signal
- [ ] `circuit_states()` method on `FallbackRouter` returns current state snapshot
- [ ] Integration test: circuit opens after 3 failures → `/health` shows `circuit_state: "open"`
- [ ] Integration test: background task updates health within interval after provider recovery
- [ ] Integration test: unauthenticated request to `/api/resilience` returns HTTP 401
- [ ] Unit test: `circuit_states()` does not block during concurrent routing
- [ ] `just ci-local` passes (fmt, clippy, test, doc, audit)
