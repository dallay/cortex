# Exploration Findings: 2026-06-07-credential-validation-warning

> Captured by `sdd-explore` on 2026-06-07. The next phase (`sdd-propose`) should
> consume this document to draft the proposal without re-reading the codebase.

## 1. OpenSpec State — Confirmed

- `openspec/config.yaml` present and well-formed (mode: `openspec`, language:
  `rust`, edition `2021`, rust 1.81). 16 packages listed.
- `openspec/specs/` contains 22 domain folders. Relevant ones for this change:
  - `provider-connections/` — canonical wire + domain contract for test
    connection. **Primary delta target.**
  - `provider-connections-transport/` — HTTP DTO and routing spec. **Primary
    delta target (wire shape).**
  - `providers-ui/` — frontend UX spec. **Delta target (Save-button rule).**
  - `health-circuit-visibility/` — operator `/health` + `/api/resilience`
    observability. **NOT a delta target** (different surface).
  - `rate-limiting/` — server-side per-client 429 middleware. **NOT a delta
    target** (different concern).
  - `combo-execution/` — fallback chains. **NOT a delta target** (it lists 429
    as a retry trigger for routing — independent of test-connection UX).
  - `dynamic-provider-registry/` — registry-port contract. **NOT a delta
    target** (does not own test-connection response shape).
- `openspec/changes/` only contains `archive/`. **No active change conflicts.**
- `openspec/changes/2026-06-07-credential-validation-warning/` did not exist —
  created by this phase.

## 2. Archived Changes — No Overlap

| Change                                            | Relation       | Overlap verdict | Reason                                                                                                                                                                       |
| ------------------------------------------------- | -------------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `2026-06-04-health-circuit-visibility`            | Related        | **None**        | Owns `/health` and `/api/resilience` for circuit-breaker observability. Different endpoints, different shape (`CircuitStateSnapshot`). Test-connection path is untouched.   |
| `2026-06-06-providers-ui-3-screen-refactor`       | Touches UI     | **None (sequential)** | Restructured `/providers` to 3-screen flow (catalog → details → modal). The `AddProviderDialog.vue` test-result UI is the version this change will modify. **Additive, not replacing.** |
| `2026-06-02-api-key-scopes-and-restrictions`      | Tangential     | **None**        | API-key management features (scopes, restrictions). Test-connection logic is a separate concern.                                                                            |
| `2026-05-31-api-key-crud`                         | Tangential     | **None**        | API-key CRUD lifecycle. Does not touch `TestConnectionResult` shape.                                                                                                        |
| `2026-06-01-anthropic-sse-streaming`              | Unrelated      | **None**        | Anthropic SSE adapter for routing.                                                                                                                                           |
| `2026-06-01-format-translation-layer`             | Unrelated      | **None**        | OpenAI ↔ Anthropic wire format translation.                                                                                                                                  |
| `2026-06-03-per-client-rate-limiting`             | Unrelated      | **None**        | Server-side rate limit middleware.                                                                                                                                          |
| `2026-06-04-multi-step-fallback-chains`           | Unrelated      | **None**        | Combo execution / fallback routing.                                                                                                                                         |
| `2026-06-04-usage-tracking-token-counts-cost-estimation` | Unrelated | **None**        | Token accounting.                                                                                                                                                            |
| `2026-06-05-read-cache`                           | Unrelated      | **None**        | Cache feature.                                                                                                                                                               |
| `2026-05-31-dynamic-provider-registry`            | Unrelated      | **None**        | Registry port contract.                                                                                                                                                      |
| `2026-05-31-provider-connections`                 | Predecessor    | **None (already merged)** | This is the spec that defines the current `TestConnectionResult` shape — it's the **delta base**, not an overlap.                                                       |
| `2025-05-31-security-authz-architecture-notes`    | Notes          | **None**        | Architecture notes, no formal change artifacts.                                                                                                                              |

## 3. Code State — Confirmed (matches orchestrator context)

### 3.1 Domain

- **`crates/domain/rook-core/src/model.rs:348-395`** — `HealthStatus` enum has
  exactly 3 variants: `Healthy { provider, latency_ms }`, `Unhealthy { provider,
  latency_ms, error }`, `Unknown { provider, reason }`. Methods `is_healthy()`,
  `provider_id()`, `latency_ms()`, `last_error()` are intact.
- **`crates/domain/rook-core/src/provider_connection.rs:51-72`** — `TestStatus`
  enum has 5 variants used for persistence: `NeverTested`, `Active`,
  `Unhealthy`, `Expired`, `Unknown`. **`expired` is OAuth-specific** and is set
  by the test path *before* calling the runtime provider
  (`manage_connections.rs:163-179`). The new design must keep this 5-state
  semantics and not collapse `expired` into the new `warning` path.
- `ProviderKind` enum: `OpenAI`, `Anthropic`, `Ollama`, `OllamaCloud`, `Gemini`,
  `Groq` — **6 kinds**, not 5. The `OllamaCloud` variant is critical because
  the orchestrator's bug report is about Ollama Cloud (HTTP 429 weekly limit).

### 3.2 Application

- **`crates/application/rook-usecases/src/manage_connections.rs:474-525`** —
  `TestConnectionResult { ok: Option<bool>, status: String, latency_ms,
  error }`. The `from_health()` mapper translates `HealthStatus` →
  `TestConnectionResult`:
  - `Healthy` → `ok: Some(true)`, `status: "active"`, `latency_ms: Some(..)`, `error: None`
  - `Unhealthy` → `ok: Some(false)`, `status: "unhealthy"`, `latency_ms`, `error: Some(..)`
  - `Unknown` → `ok: None`, `status: "unknown"`, `latency_ms: None`, `error: Some(reason)`
  - `test_status_from_health()` mirrors this for persistence.
- **`manage_connections.rs:163-179`** — the `test()` function checks OAuth
  expiry BEFORE the runtime provider probe and returns
  `TestConnectionResult { ok: Some(false), status: "expired", latency_ms: None,
  error: Some("OAuth token expired at ...") }` directly. This is the
  pre-existing `expired` short-circuit that the new design must NOT break.

### 3.3 Transport (wire)

- **`crates/infrastructure/transport-axum/src/provider_dto.rs:160-167`** —
  `TestConnectionResponse` mirrors `TestConnectionResult` field-for-field with
  `#[serde(rename_all = "camelCase")]`. **No `warning` or `method` field today.**
- **`crates/infrastructure/transport-axum/src/provider_routes.rs:97-115`** —
  two handlers, `/api/providers/{id}/test` and
  `/api/providers/test-credentials`, both return `Json<TestConnectionResponse>`.
- **`apps/rook/dashboard/src/lib/api.ts:578-583`** — `TestConnectionResponse`
  interface:
  ```ts
  interface TestConnectionResponse {
    ok: boolean | null
    status: string
    latencyMs: number | null
    error: string | null
  }
  ```
  **No `warning` or `method` field today.** This file must be updated
  alongside the Rust DTO.

### 3.4 Provider implementations

| Provider        | File:line                                       | Current behavior                                                                                                                                                                                                                                                                                            |
| --------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `openai`        | `providers-openai/src/provider.rs:285-305`      | Single-step: `GET {base_url}/models` with Bearer auth. Any non-2xx → `Unhealthy { error: "HTTP {status}" }`. **No 401/403 differentiation. No 429 handling.**                                                                                                                                                |
| `anthropic`     | `providers-anthropic/src/lib.rs:362-367`        | Returns `Unknown { reason: "health_check_not_supported" }`. **No probe at all.**                                                                                                                                                                                                                            |
| `ollama`        | `providers-ollama/src/lib.rs:182-269`           | **2-step probe:** (1) `GET /api/tags` for reachability; (2) if `api_key` is set, `POST /api/chat` with minimal body for auth check. 401/403 → `Unhealthy { error: "auth rejected: HTTP 401 — ..." }`. 2xx → `Healthy`. **No 429 handling** — currently falls into the generic "POST /api/chat returned HTTP 429" bucket. **This is the bug.** |
| `gemini`        | `providers-gemini/src/lib.rs:99-104`            | Returns `Unknown { reason: "health_check_not_supported" }`. **No probe.**                                                                                                                                                                                                                                   |
| `groq`          | `providers-groq/src/lib.rs:271-276`              | Returns `Unknown { reason: "health_check_not_supported" }`. **No probe.**                                                                                                                                                                                                                                   |
| `ollama-cloud`  | NOT YET INSPECTED (treat as ollama variant)     | Need to confirm whether `OllamaCloud` is a separate impl or a config flag on `Ollama`. **Open question for proposal phase.**                                                                                                                                                                                 |

### 3.5 Dashboard UX

- **`apps/rook/dashboard/src/components/AddProviderDialog.vue:115`** —
  `testResult = ref<{ ok: boolean; message: string } | null>(null)` — **the
  local TypeScript shape is a SUB-SET of the wire.** It collapses
  `TestConnectionResponse` into `{ ok, message }`, losing `status`, `latencyMs`,
  and `error`. This means the existing UI has no way to distinguish "warning"
  from "ok" — the proposal must extend this ref to carry `status`/`warning`.
- **`AddProviderDialog.vue:143-145`** — `canSave = computed(() => canTest.value
  && testResult.value?.ok === true && !saving.value)`. **Save is enabled only
  when `ok === true`.** Unknown (`ok === null`) currently blocks Save. The new
  design must change this to `ok !== false` (or equivalent using a new
  `valid: boolean` field).
- **`AddProviderDialog.vue:527-551`** — test result block. Green check
  (`CheckCircle2`) when `ok`, red alert (`AlertCircle`) otherwise. **No
  yellow/warning state today.** The proposal must add a third icon (e.g.
  `AlertTriangle`) and yellow styling for the `warning` case.
- **`AddProviderDialog.spec.ts:703-729`** — existing tests cover `ok: true` →
  save enabled, `ok: false` → save disabled. **Need to add `ok: true +
  warning: "..."` → save enabled + yellow alert tests.**
- `apps/rook/dashboard/src/components/AddProviderDialog.vue:237, 244, 267, 292,
  296, 301, 307, 323` — all places where `testResult.value` is reset or
  assigned; the proposal needs to keep these in sync.

### 3.6 Tests

- **`crates/application/rook-usecases/tests/manage_connections_test_credentials.rs`**
  — uses `MockProvider` returning canned `HealthStatus` values; covers
  Healthy/Unhealthy/Unknown mappings. **Need to add cases for**: (a) `ok: true
  + warning: "Rate limited..."` mapping (which requires `HealthStatus` to
  express "credentials valid but quota exhausted"); (b) `ok: true + warning:
  "No API key configured"` (no-token path); (c) confirm `Expired` path is
  untouched.
- **`crates/infrastructure/transport-axum/tests/provider_routes.rs:206-236,
  358-...`** — has `TestConnectionResponse` serialization tests and a
  `test_status_enum_has_expected_variants` test. **Need to add serialization
  tests for the new shape** (camelCase `warning` and `method`).
- **`crates/infrastructure/providers-ollama/tests/`** — wiremock tests. **Need
  to add**: 429 response → `Healthy` (or new `Warning`) variant test; 401 → 
  `Unhealthy` test; success-after-reachability test.

## 4. Spec Coverage Gaps Found

| Gap                                                                                   | Why it matters for the change                                                                                                                  |
| ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `provider-connections/spec.md` does not enumerate 429 or quota status.                 | The change needs to ADD a "warning" status category to the `TestConnectionResult` contract. This is a delta on §7.3 of the spec.               |
| `provider-connections/spec.md:497` explicitly excludes "Rate limit enforcement from quota thresholds" from v1. | The change is intentionally re-introducing a SLICE of quota awareness (test-time only) — the proposal should note this.                       |
| `providers-ui/spec.md:85, 102` says "MUST require a successful credential test before Save is enabled." | The new design relaxes this: Save is enabled when `valid === true` regardless of warning. The proposal must update this requirement.           |
| `providers-ui/spec.md:96-98` says "displays the result (ok/error, latency)"           | The new design adds a third visual state (warning). The proposal must update this scenario.                                                     |
| No spec covers 429-from-upstream semantics in the test-connection path.                | The proposal should add a new requirement capturing "429 during a credential probe is a valid signal with a warning, not a failure."           |
| `provider-connections-transport/spec.md:240-265` documents 4 response shapes; missing is the warning shape. | The wire-spec delta needs to add 2-3 new response examples (warning, method-tagged, no-token).                                                |

## 5. Reference: OmniRoute pattern (already adopted in the orchestrator's brief)

> From `tmp/OmniRoute/src/lib/providers/validation.ts` — 3-layer probe
> (auth introspection, lightweight read, mini chat fallback) with 429 treated
> as `valid: true` + `warning: "Rate limited, but credentials are valid"`.
> **This is the design pattern the user already agreed to.**

Mapping to cortex:
- **Layer 1 (auth introspection)**: `Ollama::health_check` already has this for
  Ollama Cloud (POST /api/chat). Needs to be added to openai/gemini/groq.
- **Layer 2 (lightweight read)**: `GET /models` for OpenAI; `GET .../v1beta/models`
  for Gemini. Pattern is to use a cheap read endpoint to confirm the API key is
  accepted. This is what `Ollama::GET /api/tags` already does.
- **Layer 3 (mini chat fallback)**: minimal completion call. Optional — only
  needed if Layers 1+2 don't exist for a provider.

## 6. Open Questions for the Proposal Phase

> These should be resolved during `sdd-propose`, not blocked here.

1. **HealthStatus enum shape**: Add a 4th variant `Warning { provider, latency_ms, reason }`? Or add
   `Option<String>` warning field to `Healthy` and `Unknown`? Or keep the enum
   clean and attach the warning in the application layer when building
   `TestConnectionResult`? — Recommendation: **add a 4th variant** because (a)
   it's the simplest semantic — "valid but flagged"; (b) it lets the wire-shape
   decision live entirely in `TestConnectionResult`; (c) it makes
   `test_status_from_health` trivially extendable.
2. **`TestConnectionResult.valid` vs `ok`**: Rename `ok: Option<bool>` to
   `valid: bool`? Or keep `ok` and add a new `valid`? — Recommendation:
   **rename to `valid: bool`** since the user said wire-protocol breaking
   changes are acceptable and the 3-valued `Option<bool>` was always a code
   smell.
3. **Should `Expired` (OAuth) get a warning instead of a hard failure?** —
   Recommendation: **no**. The current `expired` semantics are "you must
   re-authorize before this connection is usable" — that IS a blocker and the
   Save button should remain disabled. This preserves the current behavior.
4. **`OllamaCloud` is its own `ProviderKind`** — does it have its own
   `ProviderPort` impl, or does it share Ollama's with a config flag? Need to
   confirm during propose phase so the new probe logic is wired into the right
   place.
5. **Should the 3-layer probe be uniform across providers, or per-provider
   where cheap probes exist?** — Recommendation: **per-provider, with a
   shared trait helper**. The orchestrator's "all providers" scope means we
   need at minimum: openai = GET /models; anthropic = no probe (Unknown);
   gemini = GET .../v1beta/models; groq = GET /openai/v1/models (or no probe
   — investigate); ollama = existing 2-step.

## 7. Quick File Map (for sdd-spec/sdd-apply)

**Files to MODIFY:**
- `crates/domain/rook-core/src/model.rs` — add `Warning` variant to `HealthStatus`.
- `crates/domain/rook-core/src/provider_connection.rs` — possibly add `Warning` variant to `TestStatus` (or NOT — the wire-only warning is enough; TBD in propose).
- `crates/application/rook-usecases/src/manage_connections.rs` — extend `TestConnectionResult` and `from_health`/`test_status_from_health`; update `canSave`-equivalent logic in `test()` to return `valid: true` for warning case.
- `crates/infrastructure/transport-axum/src/provider_dto.rs` — extend `TestConnectionResponse` with `warning`, `method`, and rename `ok` → `valid`.
- `crates/infrastructure/transport-axum/src/provider_routes.rs` — no signature change, just rebuild response.
- `crates/infrastructure/providers-openai/src/provider.rs` — split 429 from 401/403 from 5xx in `health_check`.
- `crates/infrastructure/providers-ollama/src/lib.rs` — split 429 from 401/403 from 5xx; add no-token-valid path.
- `crates/infrastructure/providers-gemini/src/lib.rs` — implement lightweight probe (GET models).
- `crates/infrastructure/providers-groq/src/lib.rs` — decide: implement probe OR keep `Unknown`. The orchestrator's decision 7 says "Unknown = valid + no warning" — could leave as-is.
- `apps/rook/dashboard/src/lib/api.ts` — extend `TestConnectionResponse` interface.
- `apps/rook/dashboard/src/components/AddProviderDialog.vue` — extend `testResult` ref, add yellow state, fix `canSave` to use `valid !== false`.
- `apps/rook/dashboard/src/components/AddProviderDialog.spec.ts` — add warning-case tests.

**Files to ADD:**
- (none expected — the change is additive within existing modules)

**Spec files to UPDATE (delta):**
- `openspec/specs/provider-connections/spec.md` — §7.3 / §10 AC add warning response shape + 429/quota rule.
- `openspec/specs/provider-connections-transport/spec.md` — add new response examples for warning.
- `openspec/specs/providers-ui/spec.md` — update "Test credentials" scenario + "Save" scenario for the new 3-state model.

**Spec files NOT to touch:**
- `openspec/specs/health-circuit-visibility/spec.md` — different concern.
- `openspec/specs/rate-limiting/spec.md` — different concern.
- `openspec/specs/combo-execution/spec.md` — different concern.
- `openspec/specs/dynamic-provider-registry/spec.md` — does not own the test-connection shape.
- `openspec/specs/api-key-*.md` — different concern.
