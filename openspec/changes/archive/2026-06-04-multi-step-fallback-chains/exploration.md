# Exploration: Multi-step Fallback Chains (Combos)

## Current State

### Routing Architecture

**Current flow** (from `route_request.rs`):

1. Cache check (if cacheable)
2. **Single provider selection** via `RouterPort::select()` — returns `Arc<dyn ProviderPort>`
3. Execute completion (or stream)
4. On success: cache + audit + usage recording
5. On failure: `router.on_failure()` + audit + usage recording → **request fails immediately**

**Key observation**: Today's routing is **single-shot**. One provider is selected, and if it fails, the entire request fails. There is no retry loop or fallback chain at the request level.

### FallbackRouter Implementation

Location: `crates/application/rook-usecases/src/router_impl.rs`

**Current behavior**:

- `select()` returns the **first available provider** that supports the model (filtered by circuit breaker state)
- `on_failure()` records failures and opens circuit breakers after 3 failures (30s cooldown)
- Circuit breaker prevents broken providers from being selected again until cooldown expires
- **Strategies supported**: Priority, RoundRobin, WeightedRandom, ModelBased

**Critical insight**: The router implements **passive fallback** (circuit breaker removes bad providers from the pool), but NOT **active fallback** (trying the next provider when one fails).

### Circuit Breaker State

Location: `router_impl.rs` lines 39-104

```rust
struct CircuitState {
    failures: u32,
    is_open: bool,
    last_failure: Option<DateTime<Utc>>,
    cooldown_until: Option<Instant>,
    rate_limit_reset: Option<u64>,  // Tracks upstream rate limit reset time
}
```

**Features**:

- Opens after 3 failures (`FAILURE_THRESHOLD`)
- 30s cooldown (`CIRCUIT_COOLDOWN`)
- Distinguishes between rate limits (429) and regular failures
- For 429s, respects provider's `retry_after_secs` and `reset_at`

**Reusability for Combos**: Circuit state can guide combo step selection — skip steps where `is_open()` returns true.

### Audit + Usage Tracking

**Audit schema** (`audit-sqlite/src/lib.rs`):

```rust
CREATE TABLE audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id  TEXT NOT NULL,
    provider    TEXT NOT NULL,  // Single provider per entry
    model       TEXT NOT NULL,
    status      TEXT NOT NULL,
    ...
    latency_ms  INTEGER NOT NULL,
    timestamp   TEXT NOT NULL
);
```

**Usage schema** (from `audit-sqlite/src/lib.rs` line 178):

```rust
INSERT INTO usage_history (
    request_id, provider, model, status,
    api_key_id, connection_id,
    tokens_prompt, tokens_completion, ...
    latency_ms, cost_usd, timestamp
)
```

**Current limitation**: Both tables record **one provider per request**. For combos, we'll need **one entry per step attempted**.

**Solution path**: Each combo step is a separate audit/usage entry with the same `request_id` but different providers. Existing schema already supports this — no migration needed.

### Persistence Layer (SQLite)

**Provider connections** (`provider-sqlite/src/repository.rs`):

```sql
CREATE TABLE provider_connections (
    id TEXT PRIMARY KEY,
    provider_kind TEXT,
    provider_runtime_id TEXT,
    name TEXT,
    priority INTEGER,
    is_active INTEGER,
    ... credentials ...
    created_at TEXT,
    updated_at TEXT
)
```

**Pattern for new `combos` table**:

```sql
CREATE TABLE combos (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    strategy TEXT NOT NULL,  -- 'priority' | 'weighted_random' | ...
    weights TEXT,            -- JSON array for weighted strategies
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE combo_steps (
    combo_id TEXT NOT NULL,
    step_order INTEGER NOT NULL,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    connection_id TEXT,      -- Optional: specific connection
    PRIMARY KEY (combo_id, step_order),
    FOREIGN KEY (combo_id) REFERENCES combos(id) ON DELETE CASCADE
);
```

**Location recommendation**: New crate `crates/infrastructure/combo-sqlite` (parallel to `provider-sqlite`, `audit-sqlite`).

### HTTP API Patterns

**Existing CRUD pattern** (`provider_routes.rs`):

```rust
Router::new()
    .route("/api/providers", get(list_providers))
    .route("/api/providers", post(create_provider))
    .route("/api/providers/{id}", get(get_provider))
    .route("/api/providers/{id}", put(update_provider))
    .route("/api/providers/{id}", delete(delete_provider))
```

**Combo routes will follow the same pattern**:

```rust
Router::new()
    .route("/api/combos", get(list_combos))
    .route("/api/combos", post(create_combo))
    .route("/api/combos/{id}", get(get_combo))
    .route("/api/combos/{id}", put(update_combo))
    .route("/api/combos/{id}", delete(delete_combo))
```

**Header handling** (from `routes.rs` line 344-392):

- Current: `X-Authz-Allowed-Models`, `X-Authz-Allowed-Providers` (for restrictions)
- New: `X-Rook-Combo: <combo-id>` (for per-request combo override)

### Configuration

**Current config** (`apps/rook/src/config.rs`):

```rust
pub struct RoutingConfig {
    pub strategy: StrategyConfig,  // priority | round-robin | model-based
}
```

**Proposed extension**:

```toml
[routing]
strategy = "priority"
default_combo = "primary-fallback"  # Optional: combo to use if no X-Rook-Combo header

[[combos]]
id = "primary-fallback"
name = "OpenAI → Anthropic → Ollama"
strategy = "priority"

  [[combos.steps]]
  provider = "openai-primary"
  model = "gpt-4o"
  
  [[combos.steps]]
  provider = "anthropic-primary"
  model = "claude-opus-4"
  
  [[combos.steps]]
  provider = "ollama-local"
  model = "llama3"
```

### Error Handling

**4xx vs 5xx distinction** (`shared-kernel/src/error.rs`):

Current error types:

- `RateLimitedError` (429) — has `retry_after_secs` and `reset_at`
- `ForbiddenError` (403) — for auth failures
- `AllProvidersExhaustedError` (503) — when no providers available
- `ProviderError` (generic 5xx)

**Error classification for combo logic**:

- **Stop chain**: 4xx errors (except 429) → client error, no point retrying with another provider
- **Continue chain**: 429, 5xx, network errors → provider-specific, try next step
- **Circuit breaker state**: Already available via `is_open()` → skip steps with open circuits

**Implementation note**: `route_request.rs` line 158 already calls `router.on_failure()`, which updates circuit state. Combo execution can reuse this.

## Affected Areas

### Domain Layer

- `crates/domain/rook-core/src/ports.rs` — RouterPort trait
    - Current: `select(&self, req) -> Arc<dyn ProviderPort>` (single provider)
    - **Decision**: Keep `select()` unchanged. Combo logic lives in a **new port** or as an **execution strategy inside RouteRequest**.

### Application Layer

- `crates/application/rook-usecases/src/route_request.rs` — **main integration point**
    - Lines 94-169: Current single-provider execution flow
    - **Change**: Add combo execution mode that loops through steps on retryable failures
    - Lines 355-389: `record_failure()` — already records per-provider, reusable for combos

- `crates/application/rook-usecases/src/router_impl.rs` — FallbackRouter
    - Lines 136-147: `available_providers()` filters by circuit state
    - **Reuse**: Combo executor can call `circuits.get(provider_id)` to check `is_open()` before trying a step

### Infrastructure Layer

- **New crate**: `crates/infrastructure/combo-sqlite`
    - Implements `ComboRepositoryPort` (CRUD for combos)
    - Schema: `combos` + `combo_steps` tables

- `crates/infrastructure/transport-axum/src/routes.rs`
    - Line 393-482: `chat_completions()` handler
    - **Change**: Extract `X-Rook-Combo` header, resolve combo, pass to `route_request`
    - New routes: `/api/combos` (CRUD endpoints)

- `crates/infrastructure/audit-sqlite/src/lib.rs`
    - Lines 64-107: `record()` implementation
    - **No schema change needed** — existing schema already supports multiple audit entries per `request_id`

### Configuration

- `apps/rook/src/config.rs`
    - Lines 130-151: `RoutingConfig`
    - **Addition**: `default_combo: Option<String>` + TOML combo definitions

### Domain Model

- `crates/domain/rook-core/src/model.rs`
    - **New types needed**:
        - `ComboId` (newtype wrapper, similar to `ConnectionId`)
        - `Combo` (struct with id, name, steps, strategy)
        - `ComboStep` (provider_id, model, connection_id, priority)

## Approaches

### Approach A: Combo as a First-Class Router Strategy

**Design**: Treat combos as a new `RoutingStrategy` variant that `FallbackRouter` understands.

**Architecture**:

```rust
pub enum RoutingStrategy {
    Priority,
    RoundRobin,
    WeightedRandom(Vec<f32>),
    ModelBased,
    Combo(ComboId),  // NEW: routes to a combo
}
```

**Flow**:

1. HTTP layer extracts `X-Rook-Combo` header → creates `RoutingStrategy::Combo(id)`
2. `RouteRequest` passes request to router with combo strategy
3. `FallbackRouter::select()` returns a **virtual provider** that wraps the combo chain
4. Virtual provider's `complete()` method executes the combo steps internally

**Pros**:

- Clean separation: combos live inside the routing layer
- Existing `RouterPort` trait unchanged
- Circuit breaker reuse is straightforward

**Cons**:

- Virtual provider is awkward — `ProviderPort` is not designed for multi-step behavior
- Audit/usage recording happens at the wrong layer (would record the virtual provider, not the real ones)
- Mixing routing (which provider) with execution (how to call it) violates SRP

**Effort**: Medium-High (refactoring routing abstraction)

---

### Approach B: Combo as Execution Logic in RouteRequest

**Design**: Keep routing simple (single provider selection). Add combo execution logic **inside `RouteRequest::execute()`**.

**Architecture**:

```rust
impl RouteRequest {
    pub async fn execute(&self, req: CompletionRequest) -> Result<CompletionResponse, CortexError> {
        if let Some(combo_id) = req.metadata.combo_id {
            return self.execute_combo(req, combo_id).await;
        }
        // Existing single-provider flow (unchanged)
        self.execute_single(req).await
    }

    async fn execute_combo(&self, req: CompletionRequest, combo_id: ComboId) -> Result<...> {
        let combo = self.combo_repository.find(&combo_id).await?;
        for step in combo.steps {
            if self.router.circuits.get(&step.provider_id).is_open() {
                continue;  // Skip providers with open circuits
            }
            let provider = self.router.get(&step.provider_id)?;
            match provider.complete(&req).await {
                Ok(resp) => return Ok(resp),  // Success → done
                Err(e) if e.is_rate_limited() || !e.is_4xx() => {
                    self.router.on_failure(&step.provider_id, &e).await;
                    self.record_failure(&req, &step.provider_id, ...).await;
                    continue;  // Try next step
                }
                Err(e) => return Err(e),  // 4xx → stop chain
            }
        }
        Err(CortexError::all_providers_exhausted())
    }
}
```

**Flow**:

1. HTTP layer extracts `X-Rook-Combo` → stores in `req.metadata.combo_id`
2. `RouteRequest::execute()` detects combo → calls `execute_combo()`
3. Loop through steps, skip open circuits, try each provider
4. Audit/usage recorded per step (existing `record_failure()` already works per-provider)

**Pros**:

- No changes to `RouterPort` or `FallbackRouter`
- Combo logic is **orthogonal** to routing strategies
- Audit/usage recording works out-of-the-box (same `request_id`, different providers)
- Easy to test — combo execution is a single method

**Cons**:

- `RouteRequest` becomes slightly more complex (but still cohesive)
- Combo repository is a new dependency for `RouteRequest`

**Effort**: Low-Medium

---

### Approach C: Combo as a Middleware Layer

**Design**: Insert a new `ComboRouter` between `transport-axum` and `RouteRequest`.

**Architecture**:

```
HTTP handler
  → ComboRouter::route(req) 
    → tries steps → RouteRequest::execute_single() for each step
```

**Pros**:

- Complete isolation of combo logic
- `RouteRequest` stays unchanged

**Cons**:

- Extra indirection layer
- Harder to share circuit breaker state (ComboRouter would need access to `FallbackRouter` internals)
- Audit/usage recording is tricky (ComboRouter doesn't have those dependencies)

**Effort**: Medium

## Recommendation

**Approach B: Combo as Execution Logic in RouteRequest**

### Why:

1. **Minimal invasiveness**: No changes to existing routing or provider abstractions
2. **Audit/usage transparency**: Each combo step automatically gets audit + usage entries with the same `request_id` but different `provider_id`
3. **Circuit breaker reuse**: Combo executor can check `router.circuits.get(provider_id).is_open()` before trying each step
4. **Testability**: Combo execution is a single async method — easy to unit test with mock providers
5. **Separation of concerns**: Routing (which provider) vs execution (how to try multiple) is clear

### Implementation sketch:

```rust
// New domain types (rook-core/src/model.rs)
pub struct ComboId(pub Uuid);
pub struct Combo {
    pub id: ComboId,
    pub name: String,
    pub steps: Vec<ComboStep>,
    pub strategy: ComboStrategy,
}
pub struct ComboStep {
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub connection_id: Option<ConnectionId>,
    pub priority: u32,
}
pub enum ComboStrategy {
    Priority,
    WeightedRandom { weights: Vec<f32> },
    RoundRobin,
    P2C,
    FillFirst,
}

// New port (rook-core/src/ports.rs)
#[async_trait]
pub trait ComboRepositoryPort: Send + Sync {
    async fn list(&self) -> Result<Vec<Combo>, RepositoryError>;
    async fn find(&self, id: &ComboId) -> Result<Option<Combo>, RepositoryError>;
    async fn create(&self, combo: &Combo) -> Result<(), RepositoryError>;
    async fn update(&self, combo: &Combo) -> Result<(), RepositoryError>;
    async fn delete(&self, id: &ComboId) -> Result<(), RepositoryError>;
}

// RouteRequest changes (rook-usecases/src/route_request.rs)
pub struct RouteRequest {
    router: Arc<dyn RouterPort>,
    cache: Arc<dyn CachePort>,
    audit: Arc<dyn AuditPort>,
    usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
    combo_repository: Option<Arc<dyn ComboRepositoryPort>>,  // NEW
    ...
}

impl RouteRequest {
    pub async fn execute(&self, req: CompletionRequest) -> Result<...> {
        // 1. Check cache (unchanged)
        // 2. Detect combo mode
        if let Some(combo_id) = req.metadata.combo_id {
            return self.execute_combo(req, combo_id).await;
        }
        // 3. Existing single-provider flow (unchanged)
        self.execute_single(req).await
    }
}
```

### Config example:

```toml
[routing]
strategy = "priority"
default_combo = "main-chain"

[[combos]]
id = "main-chain"
name = "OpenAI → Anthropic → Ollama"
strategy = "priority"
  [[combos.steps]]
  provider = "openai-primary"
  model = "gpt-4o"
  priority = 1
  
  [[combos.steps]]
  provider = "anthropic-primary"
  model = "claude-opus-4"
  priority = 2
  
  [[combos.steps]]
  provider = "ollama-local"
  model = "llama3"
  priority = 3
```

## Risks

### Risk 1: Latency accumulation

**Severity**: Medium  
**Description**: If the first 2 steps fail after 30s each (timeout), the total request latency could be 60s before succeeding.  
**Mitigation**: Add per-step timeout config (default 10s) + overall combo timeout (default 60s). Circuit breaker already skips known-bad providers.

### Risk 2: Cost explosion

**Severity**: Low  
**Description**: Multiple providers are attempted, but only one succeeds → user pays for all attempts (if providers charge per request, not just successful responses).  
**Mitigation**: Usage tracking already records each attempt separately. Cost reporting will show this naturally. Document clearly in combo feature docs.

### Risk 3: 4xx detection ambiguity

**Severity**: Medium  
**Description**: Some providers return 4xx for rate limits (e.g., 400 "rate limit exceeded" instead of 429).  
**Mitigation**: Extend `CortexError` with explicit error classification: `is_retryable()` method that checks both status code and error body patterns.

### Risk 4: Model compatibility

**Severity**: High  
**Description**: Different providers have different model capabilities (tool calling, vision, streaming). A combo might fail mid-chain if step 2 doesn't support what the request needs.  
**Mitigation**:

- Validate combo steps at creation time (check model compatibility)
- OR: Execute validation at runtime and skip incompatible steps (logged as warning in audit)

### Risk 5: Streaming complexity

**Severity**: High  
**Description**: Streaming requests cannot be retried mid-stream. If the first chunk succeeds but the 5th chunk fails, we cannot switch to another provider.  
**Mitigation**: For streaming, combo logic only applies **before the first chunk is sent**. Once streaming starts, it's committed to that provider. Document this limitation.

## Ready for Proposal

**Yes** — the codebase is well-structured for this change.

### What the orchestrator should tell the user:

> **Exploration complete for Combos (issue #39).**
>
> **Current state**: Rook uses single-provider routing with passive circuit breakers. When a provider fails, the request fails immediately — no retry with another provider.
>
> **Recommended approach**: Add combo execution logic inside `RouteRequest` (Approach B). This is the cleanest integration — no changes to routing abstractions, full audit/usage transparency, and natural circuit breaker reuse.
>
> **Key design decisions**:
> 1. **Domain model**: New `Combo`, `ComboStep`, `ComboStrategy` types in `rook-core`
> 2. **Persistence**: New `combo-sqlite` crate with `combos` + `combo_steps` tables
> 3. **HTTP API**: `/api/combos` CRUD endpoints (same pattern as `/api/providers`)
> 4. **Config**: TOML `[[combos]]` sections + optional `default_combo` in `[routing]`
> 5. **Error handling**: Stop chain on 4xx (except 429), continue on 5xx/429/network errors
> 6. **Circuit breaker integration**: Skip steps where `is_open() == true`
>
> **Risks to address in design**:
> - Latency accumulation → per-step + overall timeouts
> - Model compatibility → validation at combo creation or runtime
> - Streaming limitation → combos only apply before first chunk (document this)
>
> **Next step**: Create proposal with wire formats, error handling rules, and TOML config schema.
