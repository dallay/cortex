# Combo Execution — Specification

> **Purpose**: This document defines the execution logic for combo fallback chains, including the flow, error handling, circuit breaker integration, timeout management, and audit attribution. It describes HOW the system executes combos at runtime.

---

## 1. Overview

Combo execution is the process of trying a request against multiple providers in priority order until one succeeds. It is invoked by `RouteRequest::execute_combo()` when a combo ID is present in the request context.

### 1.1 Entry Points

1. **Via `X-Rook-Combo` header**: HTTP handler extracts header value, passes combo ID to request context
2. **Via `routing.default_combo` config**: If no header present, check config for default combo
3. **No combo specified**: Fall back to existing single-shot routing

### 1.2 Dependencies

- `ComboRepositoryPort` — load combo definition
- `ProviderRegistryPort` — resolve provider by ID
- `CircuitBreakerPort` — check provider health before attempting
- `AuditPort` — record per-step audit entries
- `UsageRecorderPort` — record per-step usage records

---

## 2. Execution Flow

### Requirement: Combo Execution Entry Point

The system SHALL execute a combo when:

1. The request has a `combo_id` in its context
2. A default combo is configured and no explicit combo ID is provided

#### Scenario: Explicit combo via header

- GIVEN a request with `X-Rook-Combo: abc123` header
- WHEN the request is processed
- THEN combo with ID "abc123" is loaded
- AND steps are executed in priority order

#### Scenario: Default combo from config

- GIVEN a request with no `X-Rook-Combo` header
- AND `routing.default_combo = "main-chain"` in config
- WHEN the request is processed
- THEN combo with name "main-chain" is loaded (by ID lookup)
- AND steps are executed in priority order

#### Scenario: No combo specified — single-shot routing

- GIVEN a request with no `X-Rook-Combo` header
- AND no `routing.default_combo` configured
- WHEN the request is processed
- THEN existing single-shot routing is used
- AND no combo execution occurs

---

### Requirement: Combo Execution Algorithm

The system SHALL execute combo steps in priority order using the following algorithm:

```
fn execute_combo(combo: Combo, request: CompletionRequest) -> Result<CompletionResponse, CortexError> {
    let steps = sort_by_priority_ascending(combo.steps);
    let mut errors: Vec<StepError> = [];

    for (index, step) in steps.enumerate() {
        // 1. Check circuit breaker
        if router.circuits.is_open(&step.provider_id) {
            log_warn!("Skipping combo step {}/{}: circuit open for provider={}",
                index + 1, steps.len(), step.provider_id);
            continue;
        }

        // 2. Resolve provider
        let provider = match registry.get(&step.provider_id) {
            Some(p) => p,
            None => {
                log_warn!("Provider '{}' not found in registry, skipping step", step.provider_id);
                continue;
            }
        };

        // 3. Execute with timeout
        let result = execute_with_timeout(provider, request, step_timeout).await;

        // 4. Handle result
        match result {
            Ok(response) => {
                record_audit(combo.id, index, step.provider_id, response);
                return Ok(response);
            }
            Err(error) => {
                if is_non_retryable(&error) {
                    // 4xx (except 429) — STOP
                    record_audit(combo.id, index, step.provider_id, error.clone());
                    return Err(error);
                }
                // 429, 5xx, network — CONTINUE
                record_audit(combo.id, index, step.provider_id, error.clone());
                errors.push(StepError { step, error });
                continue;
            }
        }
    }

    // All steps exhausted
    return Err(AllProvidersExhaustedError { steps_attempted: errors });
}
```

#### Scenario: First step succeeds — return immediately

- GIVEN a combo with 3 steps, priorities 1, 2, 3
- AND the request is a completion request
- WHEN `execute_combo()` is called
- THEN step 1 is attempted first
- AND if step 1 succeeds, the response is returned
- AND steps 2 and 3 are NOT attempted

#### Scenario: First step fails, second succeeds — continue and return

- GIVEN a combo with 3 steps
- AND step 1 fails with 503 Service Unavailable
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN step 1 is attempted and fails
- AND step 2 is attempted and succeeds
- AND the response from step 2 is returned
- AND step 3 is NOT attempted

#### Scenario: All steps fail — return AllProvidersExhaustedError

- GIVEN a combo with 2 steps
- AND step 1 fails with 503 Service Unavailable
- AND step 2 fails with 500 Internal Server Error
- WHEN `execute_combo()` is called
- THEN both steps are attempted
- AND the response is `AllProvidersExhaustedError`
- AND the error includes details of both failures

---

## 3. Error Handling

### Requirement: Non-Retryable Error Handling (STOP)

The system SHALL stop the combo chain immediately when a step returns a non-retryable error.

**Non-retryable errors** (4xx except 429):

| HTTP Status | Error Type           | Action                        |
|-------------|----------------------|-------------------------------|
| 400         | Bad Request          | STOP — client error           |
| 401         | Unauthorized         | STOP — invalid credentials    |
| 403         | Forbidden            | STOP — access denied          |
| 404         | Not Found            | STOP — resource not found     |
| 422         | Unprocessable Entity | STOP — invalid request format |

**Rationale**: These errors indicate client-side issues that cannot be fixed by trying another provider.

#### Scenario: 401 stops chain immediately

- GIVEN a combo with 3 steps
- AND step 1 returns 401 Unauthorized
- WHEN `execute_combo()` is called
- THEN step 1 is attempted and fails with 401
- AND the error is returned immediately
- AND steps 2 and 3 are NOT attempted
- AND audit records are created for step 1 only

#### Scenario: 400 stops chain immediately

- GIVEN a combo with 2 steps
- AND step 1 returns 400 Bad Request
- WHEN `execute_combo()` is called
- THEN the 400 error is returned immediately
- AND step 2 is NOT attempted

#### Scenario: 422 stops chain immediately

- GIVEN a combo with 3 steps
- AND step 1 returns 422 Unprocessable Entity
- WHEN `execute_combo()` is called
- THEN the 422 error is returned immediately
- AND no other steps are attempted

---

### Requirement: Retryable Error Handling (CONTINUE)

The system SHALL continue to the next step when a step returns a retryable error.

**Retryable errors**:

| HTTP Status   | Error Type                       | Action   |
|---------------|----------------------------------|----------|
| 429           | Too Many Requests                | CONTINUE |
| 500           | Internal Server Error            | CONTINUE |
| 502           | Bad Gateway                      | CONTINUE |
| 503           | Service Unavailable              | CONTINUE |
| 504           | Gateway Timeout                  | CONTINUE |
| Network Error | Connection refused, timeout, DNS | CONTINUE |

**Rationale**: These errors are transient and may succeed on a different provider.

#### Scenario: 503 triggers next step

- GIVEN a combo with 2 steps
- AND step 1 returns 503 Service Unavailable
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN step 1 is attempted and fails with 503
- AND step 2 is attempted
- AND the response from step 2 is returned

#### Scenario: 429 triggers next step

- GIVEN a combo with 2 steps
- AND step 1 returns 429 Too Many Requests
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN step 1 is attempted and fails with 429
- AND step 2 is attempted and succeeds
- AND the response from step 2 is returned

#### Scenario: Network error triggers next step

- GIVEN a combo with 2 steps
- AND step 1 fails with "connection refused"
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN step 1 is attempted and fails with network error
- AND step 2 is attempted and succeeds
- AND the response from step 2 is returned

#### Scenario: 500 triggers next step

- GIVEN a combo with 3 steps
- AND step 1 returns 500 Internal Server Error
- AND step 2 returns 500 Internal Server Error
- AND step 3 succeeds
- WHEN `execute_combo()` is called
- THEN all three steps are attempted
- AND the response from step 3 is returned

---

### Requirement: AllProvidersExhaustedError

When all steps fail, the system SHALL return `AllProvidersExhaustedError` with aggregated failure details.

```rust
pub struct AllProvidersExhaustedError {
    pub combo_id: ComboId,
    pub steps_attempted: Vec<ProviderStepResult>,
    pub total_steps: usize,
    pub total_latency_ms: u64,
}

pub struct ProviderStepResult {
    pub step_index: usize,
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub error: CortexError,
}
```

#### Scenario: All steps fail — detailed error returned

- GIVEN a combo with 3 steps
- AND step 1 fails with 503
- AND step 2 fails with 500
- AND step 3 fails with 429
- WHEN `execute_combo()` is called
- THEN `AllProvidersExhaustedError` is returned
- AND the error includes all 3 step results
- AND each result includes step_index, provider_id, model, and error

#### Scenario: Error aggregates provider details

- GIVEN `AllProvidersExhaustedError` is constructed
- WHEN the error is serialized
- THEN the response includes:
    - `combo_id`: UUID of the combo
    - `steps_attempted`: array of 3 results
    - `total_steps`: 3
    - `total_latency_ms`: sum of all step latencies

---

## 4. Circuit Breaker Integration

### Requirement: Circuit Breaker Check Before Each Step

The system SHALL check the circuit breaker state before attempting each step.

**Behavior**:

- Query `router.circuits.is_open(provider_id)`
- If `true` (circuit open): skip step, log warning, continue to next
- If `false` (circuit closed): proceed with step attempt

#### Scenario: Open circuit — step skipped

- GIVEN a combo with 2 steps
- AND the circuit breaker for provider in step 1 is open
- WHEN `execute_combo()` is called
- THEN step 1 is skipped with warning: "Skipping combo step 1/2: circuit open for provider=..."
- AND step 2 is attempted
- AND if step 2 succeeds, the response is returned

#### Scenario: Circuit opens mid-combo

- GIVEN a combo with 3 steps
- AND step 1 succeeds
- AND step 2's circuit was open when the combo started but closed by step 2
- WHEN `execute_combo()` is called
- THEN each step checks circuit state before execution
- AND step 2 succeeds or fails based on actual circuit state at execution time

#### Scenario: All circuits open — all steps skipped

- GIVEN a combo with 3 steps
- AND all 3 provider circuits are open
- WHEN `execute_combo()` is called
- THEN all 3 steps are skipped
- AND `AllProvidersExhaustedError` is returned with empty `steps_attempted`

---

## 5. Timeout Management

### Requirement: Per-Step Timeout

The system SHALL enforce a timeout for each provider call within a combo.

**Defaults**:

- Per-step timeout: 10 seconds (configurable via provider config)
- Combo timeout: 60 seconds (hard limit, not configurable)

**Implementation**:

- Each `provider.complete()` call is wrapped in `tokio::time::timeout()`
- If timeout expires: treat as network error, continue to next step
- Timeout error logged with `timeout_expired = true`

#### Scenario: Step times out — next step attempted

- GIVEN a combo with 2 steps
- AND step 1 times out after 10 seconds
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN step 1 is attempted and times out
- AND step 2 is attempted and succeeds
- AND the response from step 2 is returned

#### Scenario: Timeout error logged

- GIVEN a step times out
- WHEN the timeout occurs
- THEN a warning is logged: "Combo step timed out after 10000ms: provider={} model={}"
- AND a `TIMEOUT` event is recorded in audit

---

### Requirement: Overall Combo Timeout

The system SHALL enforce a hard limit on total combo execution time.

**Default**: 60 seconds from start to final response

**Behavior**:

- If total time exceeds 60s: cancel remaining steps, return `ComboTimeoutError`
- Timeout error takes precedence over partial results

#### Scenario: Combo times out before completion

- GIVEN a combo with 5 steps
- AND 4 steps have already consumed 58 seconds
- AND step 5 would take 10 seconds
- WHEN `execute_combo()` is called
- THEN step 5 is cancelled after 2 seconds (60s total reached)
- AND `ComboTimeoutError` is returned with `timeout_ms: 60000`

---

## 6. Audit Attribution

### Requirement: Per-Step Audit Records

The system SHALL create audit records for each step attempted, with attribution linking steps to the combo.

**Audit Record Fields** (in addition to standard fields):

- `request_id`: Same across all steps in a combo
- `provider_id`: Step-specific provider
- `model`: Step-specific model
- `combo_id`: UUID of the combo being executed
- `combo_step_index`: 0-based index of the step (0 = first)
- `combo_total_steps`: Total number of steps in the combo
- `combo_step_outcome`: `success | failed | skipped`

#### Scenario: First step succeeds — single audit record

- GIVEN a combo with 3 steps
- AND step 1 succeeds
- WHEN `execute_combo()` is called
- THEN exactly 1 audit record is created
- AND the record has `combo_id`, `combo_step_index: 0`, `combo_total_steps: 3`, `combo_step_outcome: success`

#### Scenario: Fallback creates multiple audit records

- GIVEN a combo with 3 steps
- AND step 1 fails with 503
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN 2 audit records are created
- AND record 1: `combo_step_index: 0`, `combo_step_outcome: failed`
- AND record 2: `combo_step_index: 1`, `combo_step_outcome: success`

#### Scenario: Skipped steps logged with skipped outcome

- GIVEN a combo with 3 steps
- AND step 1 circuit is open (skipped)
- AND step 2 succeeds
- WHEN `execute_combo()` is called
- THEN 2 audit records are created
- AND record 1: `combo_step_index: 0`, `combo_step_outcome: skipped`
- AND record 2: `combo_step_index: 1`, `combo_step_outcome: success`

#### Scenario: All steps fail — all failures audited

- GIVEN a combo with 3 steps
- AND all 3 steps fail
- WHEN `execute_combo()` is called
- THEN 3 audit records are created
- AND each has `combo_step_outcome: failed`
- AND each includes the error details

---

### Requirement: Audit Fire-and-Forget

Audit recording SHALL NOT block the retry loop.

**Implementation**:

- `audit.record()` is called without `await`
- If audit fails, a warning is logged
- Audit failure does NOT affect combo execution

#### Scenario: Audit failure does not block response

- GIVEN a combo with 2 steps
- AND step 1 fails (audit called)
- AND audit system is temporarily unavailable
- WHEN step 2 is attempted
- THEN audit failure is logged as a warning
- AND step 2 is still attempted
- AND if step 2 succeeds, the response is returned

---

## 7. Streaming Limitation

### Requirement: Combos Apply Before First Chunk

The system SHALL only apply combo logic before streaming begins. Once streaming starts, the combo is committed to that provider.

**Rationale**: Once HTTP response headers are sent (200 OK with chunked transfer encoding), the client is receiving data. Attempting to switch providers mid-stream would corrupt the response.

#### Scenario: Streaming request — combo applies before chunks

- GIVEN a streaming completion request
- AND a combo with 2 steps
- WHEN the request is processed
- THEN the combo executes step 1
- AND if step 1 fails with 503 before first chunk, step 2 is attempted
- AND if step 1 succeeds, streaming begins with step 1's response

#### Scenario: Stream fails mid-response — no retry

- GIVEN a streaming request that succeeded on step 1
- AND the stream is in progress
- AND step 1's stream fails with an error after 50 chunks
- WHEN the stream fails
- THEN the error is returned to the client
- AND no retry to step 2 is attempted
- AND this limitation is documented

**Documentation Requirement**:

- API documentation MUST state: "Combos only apply before the first chunk is sent. Once streaming begins, no fallback occurs."
- Error messages for mid-stream failures MUST mention this limitation.

---

## 8. Logging Specification

### Requirement: Structured Combo Logging

The system SHALL emit structured log events for combo execution.

| Event                           | Level | Fields                                                          |
|---------------------------------|-------|-----------------------------------------------------------------|
| Step attempted                  | INFO  | `combo_id`, `step_index`, `total_steps`, `provider_id`, `model` |
| Step skipped (circuit open)     | WARN  | `combo_id`, `step_index`, `provider_id`                         |
| Step skipped (provider missing) | WARN  | `combo_id`, `step_index`, `provider_id`                         |
| Step succeeded                  | INFO  | `combo_id`, `step_index`, `latency_ms`                          |
| Step failed                     | WARN  | `combo_id`, `step_index`, `error_code`, `error_message`         |
| All providers exhausted         | ERROR | `combo_id`, `steps_attempted_count`, `total_latency_ms`         |
| Combo timeout                   | ERROR | `combo_id`, `timeout_ms`, `steps_completed`                     |

#### Log Format Examples

```
INFO  Trying combo step 1/3: provider=openai-primary model=gpt-4o
WARN  Skipping combo step 1/3: circuit open for provider=openai-primary
INFO  Combo step 1 succeeded: latency_ms=245
WARN  Combo step 1 failed: error_code=503 error_message="Service Unavailable"
ERROR All providers exhausted for combo abc123: steps_attempted=3 total_latency_ms=15234
```

---

## 9. Non-Functional Requirements

### Performance

| Metric                 | Target | Notes                             |
|------------------------|--------|-----------------------------------|
| Combo lookup           | <1ms   | Index on `id`, SQLite primary key |
| Circuit breaker check  | <100μs | In-memory HashMap lookup          |
| Per-step audit (async) | <10ms  | Fire-and-forget, non-blocking     |
| Total combo overhead   | <5ms   | Overhead over single-shot routing |

### Reliability

| Requirement                        | Target                   |
|------------------------------------|--------------------------|
| Audit record completeness          | 100% of steps audited    |
| Audit fire-and-forget success rate | >99.9%                   |
| Combo execution atomicity          | Each step is independent |

### Observability

| Requirement               | Implementation                                           |
|---------------------------|----------------------------------------------------------|
| Trace context propagation | Same `request_id` across all steps                       |
| Metrics per step          | `combo_step_attempts_total`, `combo_step_failures_total` |
| Metrics per combo         | `combo_execution_duration_ms`, `combo_exhaustion_total`  |
