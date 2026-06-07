# Design: Provider Credential Validation Warning

> Maps the proposal to concrete code changes. The exploration (`findings.md`) and
> the proposal's decisions are settled — this document resolves the four
> remaining open questions and gives `sdd-apply` an implementable file-by-file
> plan.

## Open Questions Resolved

| # | Question                                                                         | Resolution                                                                                                                                                                                                                                                                                                                                           |
|---|----------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 1 | Does `OllamaCloud` share Ollama's `ProviderPort` impl?                           | **YES.** `apps/rook/src/di.rs:811-814`: "Ollama Cloud reuses the same `OllamaProvider` implementation — it differs only in the base URL and required Bearer auth." Both `ProviderKind::Ollama` and `ProviderKind::OllamaCloud` call `providers_ollama::OllamaProvider::new(config)`. The 2-step probe covers both. **No separate impl to refactor.** |
| 2 | Which step's `latency_ms` does the ollama 2-step probe report when step 2 fails? | **The failing step's latency.** Step-1 failure → step-1 latency. Step-2 failure (incl. 429) → step-2 latency. Both succeed → total elapsed. More honest about what took time.                                                                                                                                                                        |
| 3 | Wire `method` field shape                                                        | **Free-form `Option<String>`** (settled in proposal). Canonical values: `models_list \| v1beta_models \| tags_reachability \| chat_probe \| not_supported \| oauth_expired`.                                                                                                                                                                         |
| 4 | Yellow styling for the dashboard                                                 | **`AlertTriangle` from `lucide-vue-next` + Tailwind `text-yellow-600`.** `AlertTriangle` must be added to the import set at `AddProviderDialog.vue:4` (currently imports `AlertCircle, CheckCircle2, Loader2, Trash2`).                                                                                                                              |

## Architecture Decisions

| ADR   | Decision                                                                     | Tradeoff / why                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
|-------|------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **1** | Add 4th variant `Warning { provider, latency_ms, reason }` to `HealthStatus` | Cleanest semantic ("valid but flagged"). `is_healthy()` returns `true` for `Healthy \| Warning`. `test_status_from_health` maps `Warning -> TestStatus::Active` (transient, persist as active). `last_error()` returns `None` for `Warning`. `TestStatus` persistence enum stays 5-variant. **Rejected:** adding `warning: Option<String>` to `Healthy` (ambiguous `is_healthy()`); adding a 4th `TestStatus` variant (persistence shouldn't track transient wire state). |
| **2** | Rename `ok: Option<bool>` → `valid: bool` on `TestConnectionResult`          | Wire-breaking but acceptable (no released versions). `valid: true` for `Healthy`, `Warning`, `Unknown`; `valid: false` for `Unhealthy`, `Expired`. Dashboard `canSave` becomes `valid === true` (replaces buggy `ok === true` which blocked `Unknown`). Add `warning: Option<String>`, `method: Option<String>`.                                                                                                                                                          |
| **3** | Per-provider probe with shared classification helper                         | One-step (openai/gemini/groq) or two-step (ollama); Anthropic stays `Unknown`. Helper `classify_status_code(status: StatusCode) -> ProbeClassification` lives in `crates/domain/rook-core/src/probes.rs` (NEW). **Verify with `cargo tree -p rook-core`** that `reqwest::StatusCode` is reachable; if it would add a new direct dep, fall back to `u16`.                                                                                                                  |
| **4** | OllamaCloud shares Ollama's impl                                             | Verified (Q1 above). One provider refactor covers both `ProviderKind::Ollama` and `ProviderKind::OllamaCloud`.                                                                                                                                                                                                                                                                                                                                                            |
| **5** | Report failing step's latency in 2-step probes                               | `Instant::now()` snapshots per step; report the one that timed out.                                                                                                                                                                                                                                                                                                                                                                                                       |
| **6** | Free-form `method` field                                                     | `Option<String>` with documented canonical values. Type-safety upgrade is a non-breaking change later (only deserialization friction).                                                                                                                                                                                                                                                                                                                                    |
| **7** | Audit log captures warnings                                                  | `AuditPort::record_test_result(...)` gains `warning: Option<String>`. Operators need visibility into "test-credentials succeeded with warning" events.                                                                                                                                                                                                                                                                                                                    |

## File-by-File Implementation Plan

### Domain (`rook-core`)

| File                                           | Change                                                                                                                                                                                                                           |
|------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/domain/rook-core/src/model.rs:348-395` | Add `Warning { provider, latency_ms, reason }` to `HealthStatus`. Update `is_healthy()` (return `true` for `Healthy \| Warning`), `last_error()` (return `None` for both), `provider_id()`, `latency_ms()` to handle 4 variants. |
| `crates/domain/rook-core/src/probes.rs` (NEW)  | `pub enum ProbeClassification { Ok, RateLimited, AuthRejected(u16), ServerError(u16), ClientError(u16), NetworkError(String) }` and `classify_status_code(status) -> ProbeClassification`. `pub mod probes;` in `lib.rs`.        |

### Application (`rook-usecases`)

| File                                                                 | Change                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
|----------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/application/rook-usecases/src/manage_connections.rs:474-525` | Replace `TestConnectionResult { ok, status, latency_ms, error }` with `{ valid: bool, status: String, latency_ms, error, warning: Option<String>, method: Option<String> }`. Rewrite `from_health()` for 4 variants per ADR-2 mapping. Update `test_status_from_health()` with `Warning -> TestStatus::Active` arm. Add `method: Some("oauth_expired")` to the `Expired` short-circuit (line 179-184). Emit audit log entry at the end of `test_credentials()` and `test()`. |

### Transport (`transport-axum`)

| File                                                               | Change                                                                                                                                                                |
|--------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/infrastructure/transport-axum/src/provider_dto.rs:160-167` | `TestConnectionResponse` gets `valid: bool`, `warning`, `method`. `#[serde(rename_all = "camelCase")]`. Add `From<&TestConnectionResult> for TestConnectionResponse`. |
| `crates/infrastructure/transport-axum/src/provider_routes.rs`      | Both `/api/providers/{id}/test` and `/api/providers/test-credentials` use `TestConnectionResponse::from(&result)`. No signature change.                               |

### Providers

| File                                                             | Change                                                                                                                                                                                                                                                                                                           |
|------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/infrastructure/providers-openai/src/provider.rs:285-305` | Refactor `health_check()` to use `classify_status_code`. Add no-key short-circuit → `Warning { reason: "No API key configured. You can add one later via Edit." }`. Differentiate 401/403 (Unhealthy) from 429 (Warning). **Caveat:** URL is currently `/models`; verify it should be `/v1/models` per proposal. |
| `crates/infrastructure/providers-ollama/src/lib.rs:182-269`      | Same 429→Warning, no-key→Warning, `classify_status_code` adoption. Keep 2-step structure. Per-step latency snapshots. This refactor covers both Ollama and OllamaCloud (ADR-4).                                                                                                                                  |
| `crates/infrastructure/providers-anthropic/src/lib.rs`           | **NO CHANGE.** Stays at `Unknown { reason: "health_check_not_supported" }`.                                                                                                                                                                                                                                      |
| `crates/infrastructure/providers-gemini/src/lib.rs:99-104`       | Replace `Unknown` placeholder with `GET {base_url}/v1beta/models` probe. Bearer auth. No-key → `Warning`. Same `classify_status_code` mapping.                                                                                                                                                                   |
| `crates/infrastructure/providers-groq/src/lib.rs:271-276`        | Replace `Unknown` placeholder with `GET {base_url}/openai/v1/models` probe. Bearer auth. No-key → `Warning`.                                                                                                                                                                                                     |
| `apps/rook/src/di.rs:810-830`                                    | **NO CHANGE.** OllamaCloud wiring is correct. (Audit port signature update may touch the construction site if ADR-7 changes the port signature.)                                                                                                                                                                 |

### Dashboard

| File                                                           | Change                                                                                                                                                                                                                                                                                                                     |
|----------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `apps/rook/dashboard/src/lib/api.ts:578-583`                   | `TestConnectionResponse` TS interface: `valid: boolean`, `status: 'ok' \| 'warning' \| 'unhealthy' \| 'unknown' \| 'expired'`, `warning: string \| null`, `method: string \| null`.                                                                                                                                        |
| `apps/rook/dashboard/src/components/AddProviderDialog.vue`     | (1) Line 4: add `AlertTriangle` to imports. (2) Line 115: refactor `testResult` ref to carry `valid`, `status`, `warning`, `error`, `message` (derived). (3) Line 143-144: `canSave` → `testResult.value?.valid === true && !saving.value`. (4) Lines 527-548: add yellow `AlertTriangle` branch for `testResult.warning`. |
| `apps/rook/dashboard/src/components/AddProviderDialog.spec.ts` | Add: warning state (yellow, save enabled); invalid state (red, save disabled); no-warning healthy (green, save enabled).                                                                                                                                                                                                   |

## Test Strategy

| Layer                 | What                                                                                                               | Where                                                                           |
|-----------------------|--------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------|
| Unit (Rust, wiremock) | 429→Warning, 401→Unhealthy, 5xx→Unhealthy, no-key→Warning, network error→Unhealthy for each probe-capable provider | `crates/infrastructure/providers-{openai,ollama,gemini,groq}/tests/`            |
| Integration (Rust)    | `from_health` mapper for all 4 `HealthStatus` variants + `Expired` short-circuit                                   | `crates/application/rook-usecases/tests/manage_connections_test_credentials.rs` |
| Integration (Rust)    | 6-field `TestConnectionResponse` serialization; 7 wire examples from spec delta                                    | `crates/infrastructure/transport-axum/tests/provider_routes.rs`                 |
| Unit (Vitest)         | Yellow/red/green icon rendering, `canSave` logic                                                                   | `apps/rook/dashboard/src/components/AddProviderDialog.spec.ts`                  |
| E2E (Playwright)      | Stubbed 429 → yellow alert + Save enabled; stubbed 401 → red alert + Save disabled                                 | `apps/rook/dashboard/e2e/providers.spec.ts`                                     |

## Migration / Rollout

**No data migration needed.** Wire-protocol change is a clean break (no released
versions).

**Apply order (single PR, sequential commits):**

1. Domain (`HealthStatus::Warning` + `probes.rs`).
2. `TestConnectionResult` + `from_health` + `test_status_from_health`.
3. `TestConnectionResponse` DTO + route adapter.
4. Providers — `ollama` (most complex, 2-step) first, then `openai`, `gemini`, `groq`. `anthropic` untouched.
5. Audit port signature update (ADR-7).
6. Dashboard: `api.ts` → `AddProviderDialog.vue` → `.spec.ts`.
7. Spec deltas on `provider-connections`, `provider-connections-transport`, `providers-ui`.
8. `just ci-local` + targeted Playwright e2e.

**Rollback.** `git revert` of the change commits restores prior behavior. The
proposal's mitigation — collapse `Warning → Unhealthy` in `from_health()` — is a
one-line gate if a regression surfaces in production.

## Risks / Open Questions for Apply

1. **`reqwest` dep on `rook-core`.** Verify with `cargo tree -p rook-core` in apply-Task 1. If `StatusCode` would add a new direct dep, fall back to `u16` in `probes.rs` and translate at the call site.
2. **OpenAI probe URL.** Current code uses `/models` (`provider.rs:289`); proposal says `/v1/models`. Verify which is the canonical list-models endpoint before patching.
3. **Gemini auth header.** `/v1beta/models` accepts `Authorization: Bearer {key}` (recommended per Google docs) or `?key={api_key}` (legacy). Prefer Bearer for consistency.
4. **Groq base URL.** Verify default in `providers-groq` config (likely `https://api.groq.com/openai/v1`); the probe is `GET {base_url}/models`.
5. **`TestStatus::Active` semantic for `Warning`.** Confirmed: warnings are transient, the connection itself is Active. No follow-up needed.
