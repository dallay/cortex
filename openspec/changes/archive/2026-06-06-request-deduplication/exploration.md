# Exploration: Request Deduplication

## Current State

The system already has a comprehensive caching infrastructure with signature-based deduplication partially implemented:

### Request Flow Architecture

1. **Entry Points**:
   - `/v1/chat/completions` (OpenAI-compatible) → `chat_completions()` handler
   - `/v1/messages` (Anthropic-compatible) → `anthropic_messages()` handler
   - Both in `crates/infrastructure/transport-axum/src/routes.rs`

2. **Request Pipeline**:
   ```
   HTTP Handler (routes.rs)
     ↓
   CompletionRequest construction
     ↓
   RouteRequest::execute_with_format() or execute_stream_with_format()
     ↓
   Cache check (using cache_key)
     ↓
   Provider selection (RouterPort)
     ↓
   Provider execution
     ↓
   Cache storage (if cacheable)
     ↓
   Audit & Usage recording
   ```

3. **Key File Locations**:
   - **Request handling**: `crates/application/rook-usecases/src/route_request.rs`
   - **HTTP routes**: `crates/infrastructure/transport-axum/src/routes.rs`
   - **Cache handlers**: `crates/infrastructure/transport-axum/src/handlers/cache.rs`
   - **Domain model**: `crates/domain/rook-core/src/model.rs`
   - **Cache port**: `crates/domain/rook-core/src/ports.rs`

### Existing Caching Infrastructure

**Already Implemented**:

1. **Signature Generation** (`CompletionRequest::cache_key()`):
   - Located in `crates/domain/rook-core/src/model.rs:161-196`
   - Uses SHA-256 hash of semantic fields
   - Includes: model, messages, max_tokens, temperature, tools, tool_choice
   - Excludes: id, stream, metadata, restrictions
   - Returns 64-character hex string

2. **CacheKey Structure** (`shared-kernel/src/lib.rs:28-62`):
   ```rust
   pub struct CacheKey {
       pub request_id: RequestId,
       pub signature: String,  // SHA-256 hex (64 chars)
   }
   ```

3. **Cache Implementation** (`cache-memory/src/lib.rs`):
   - In-memory cache using `DashMap` (thread-safe concurrent HashMap)
   - TTL support with expiry tracking
   - LRU eviction when at capacity
   - Stats tracking (hits, misses, evictions)
   - **Already has `delete_by_signature(signature: &str)` method** (line 54)

4. **Cache Port** (`rook-core/src/ports.rs:104-117`):
   ```rust
   trait CachePort {
       async fn get(&self, key: &CacheKey) -> ...
       async fn set(&self, key: &CacheKey, value: &CompletionResponse, ttl: Duration) -> ...
       async fn delete(&self, key: &CacheKey) -> ...
       async fn clear(&self) -> ...
       async fn stats(&self) -> ...
       async fn delete_by_signature(&self, signature: &str) -> ...  // Already exists!
   }
   ```

5. **HTTP Cache Management Endpoints** (`handlers/cache.rs`):
   - `GET /api/cache/stats` — cache statistics
   - `DELETE /api/cache` — clear entire cache
   - `DELETE /api/cache/:signature` — delete by signature (ALREADY IMPLEMENTED!)

### Request Structure

`CompletionRequest` (from `rook-core/src/model.rs:142-155`):
```rust
pub struct CompletionRequest {
    pub id: RequestId,
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
    pub metadata: RequestMetadata,
    pub restrictions: ApiKeyRestrictions,
}
```

**Streaming vs Non-Streaming**:
- Non-streaming: `RouteRequest::execute_with_format()` — checks cache, caches response
- Streaming: `RouteRequest::execute_stream_with_format()` — **bypasses cache entirely** (line 243+)

### Configuration System

Cache configuration in `apps/rook/src/config.rs:223-255`:
```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: Option<usize>,
}
```

Default TTL: 5 minutes (`DEFAULT_CACHE_TTL` in `route_request.rs:35`)

Configuration validation rejects:
- TTL > 24 hours
- max_entries = 0

### Technology Stack

- **Language**: Rust
- **Web framework**: Axum (async HTTP)
- **Cache**: DashMap (lock-free concurrent HashMap)
- **Hashing**: SHA-256 via `sha2` crate + hex encoding
- **Database**: SQLite (via rusqlite)
- **Available dependencies**: Already has all needed crates

## Affected Areas

**Core domain (no changes needed)**:
- `crates/domain/rook-core/src/model.rs` — `cache_key()` method already generates signatures
- `crates/domain/shared-kernel/src/lib.rs` — `CacheKey` already holds signature
- `crates/domain/rook-core/src/ports.rs` — `CachePort::delete_by_signature()` already exists

**Infrastructure (minimal changes)**:
- `crates/infrastructure/cache-memory/src/lib.rs` — `delete_by_signature()` already implemented
- `crates/infrastructure/transport-axum/src/handlers/cache.rs` — DELETE endpoint already exists

**Application layer**:
- `crates/application/rook-usecases/src/route_request.rs` — cache integration point

**HTTP layer**:
- `crates/infrastructure/transport-axum/src/routes.rs` — cache routes already wired

## Approaches

### Approach 1: Document Existing Implementation ✅ **RECOMMENDED**

**What it is**: The signature-based deduplication is **already implemented**. Document it and add minimal enhancements.

**Pros**:
- Zero breaking changes
- Cache signature generation already deterministic (SHA-256)
- Delete-by-signature endpoint already exists
- Stats endpoint already tracks cache hits/misses
- Test coverage already exists

**Cons**:
- Streaming requests bypass cache (by design)
- No signature-to-responses mapping endpoint (minor enhancement needed)

**Effort**: **Low** — Primarily documentation + 1-2 small enhancements

**Implementation**:
1. Add `GET /api/cache/signatures` endpoint to list all cached signatures
2. Add `GET /api/cache/signature/:sig` endpoint to retrieve entries by signature
3. Document the existing behavior in specs
4. Add integration tests for multi-request deduplication

---

### Approach 2: Add Signature Persistence Layer

**What it is**: Store signature metadata in SQLite alongside in-memory cache.

**Pros**:
- Signatures survive restarts
- Can query historical deduplication patterns
- Can implement quotas/limits per signature

**Cons**:
- Adds complexity
- Performance overhead (disk writes on every cache hit)
- Memory cache and DB can drift out of sync
- Requires migration
- Not aligned with current in-memory-first design

**Effort**: **High** — New repository, migrations, sync logic

**Not recommended** — Adds complexity without clear benefit for the stated requirements.

---

### Approach 3: Extend Cache Stats with Signature Metrics

**What it is**: Track per-signature hit counts in the existing stats system.

**Pros**:
- Provides insight into deduplication effectiveness
- No schema changes
- Fits existing metrics infrastructure

**Cons**:
- Memory overhead (counters per signature)
- Stats reset on restart
- Not a core requirement

**Effort**: **Medium** — Counter tracking in cache layer

**Optional enhancement** — Consider only if metrics are explicitly requested.

## Recommendation

**Use Approach 1: Document and minimally enhance existing implementation.**

The system already implements signature-based request deduplication:

1. ✅ SHA-256 signatures are generated from semantic request fields
2. ✅ Cache lookup uses signatures to detect duplicates
3. ✅ Delete-by-signature endpoint exists
4. ✅ Stats tracking is operational
5. ✅ Test coverage exists

**Required work**:
1. **Spec**: Document the existing cache behavior and signature semantics
2. **Enhancement**: Add `GET /api/cache/signatures` endpoint (list all signatures with metadata)
3. **Enhancement**: Add `GET /api/cache/signature/:sig` endpoint (get all entries for a signature)
4. **Tests**: Add integration test demonstrating multi-request deduplication
5. **Documentation**: Update API docs with cache management endpoints

**Not in scope** (deliberate design decisions):
- Streaming requests bypass cache (cannot be deduplicated by signature)
- Signature persistence across restarts (in-memory cache resets on restart)
- Per-signature quotas/limits (not a current requirement)

## Risks

**Low risk overall** — most implementation already exists.

1. **Streaming bypass**: Streaming requests (`stream: true`) never hit the cache. This is intentional (streaming responses are consumed incrementally and cannot be stored as complete responses). **Mitigation**: Document this behavior clearly in specs.

2. **Cache eviction**: LRU eviction may drop popular signatures before TTL expires if `max_entries` is too low. **Mitigation**: Document `max_entries` tuning in operational guidance.

3. **Signature collision**: SHA-256 has negligible collision probability, but theoretically possible. **Mitigation**: Already handled — `CacheKey` includes both `request_id` (unique per request) and `signature`.

4. **Metadata drift**: Restrictions and metadata are excluded from signature, so two requests with identical semantic content but different restrictions share the same signature. **Mitigation**: This is intentional — restrictions are checked separately before cache lookup (lines 142-146 in `route_request.rs`).

## Ready for Proposal

**Yes** — proceed to `sdd-propose`.

The orchestrator should inform the user:

> Signature-based request deduplication is **already implemented** in Rook. The system generates SHA-256 signatures from request semantics (model, messages, parameters) and uses them as cache keys. A delete-by-signature endpoint already exists. The proposed work will document the existing behavior, add two list/get endpoints for signature inspection, and write integration tests to verify deduplication across multiple identical requests.
