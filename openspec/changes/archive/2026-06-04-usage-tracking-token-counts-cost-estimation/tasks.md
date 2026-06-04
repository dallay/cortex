# Tasks: Usage Tracking — Token Counts & Cost Estimation

## Review Workload Forecast

- Decision needed before apply: Yes
- Chained PRs recommended: Yes
- Chain strategy: chained-prs
- 400-line budget risk: High
- Estimated changed lines: 900-1500
- Delivery strategy: ask-on-risk
- Suggested work unit: Ship as slices: domain+ports+migration, SQLite repository+config, usecase propagation+recording, provider parsing, transport+retention+docs.

## Phase 1 — Domain and ports

- [x] T1.1 Extend `TokenUsage` with cache/reasoning fields and update constructors/tests first.
    - Files: `crates/domain/rook-core/src/model.rs`, affected tests constructing `TokenUsage`
    - Acceptance: Existing token usage remains compatible; new fields serialize as nullable optional dimensions.
    - Verification: `cargo test -p rook-core`

- [x] T1.2 Add usage domain types and exports.
    - Files: `crates/domain/rook-core/src/model.rs`, `crates/domain/rook-core/src/lib.rs`
    - Acceptance: `UsageEntry`, `UsageFilters`, `Pagination`, `UsageSummary`, and `CostBreakdown` match spec; pagination defaults to 100 and clamps to 1000.
    - Verification: `cargo test -p rook-core`

- [x] T1.3 Add `UsageRecorderPort` and connection lookup port method.
    - Files: `crates/domain/rook-core/src/ports.rs`, test fakes implementing `ProviderRepositoryPort`
    - Acceptance: Port includes `record`, `list`, `count`, `summary`, `cost_breakdown`; provider lookup returns `Result<Option<ConnectionId>, RepositoryError>`.
    - Verification: `cargo test -p rook-core --all-features`

## Phase 2 — SQLite migration and repository

- [x] T2.1 Add additive V2 migration for `usage_history`.
    - Files: `crates/infrastructure/db-migration/src/migrations/V2__usage_history.sql`
    - Acceptance: Table columns and six indexes exactly match spec; no V0/V1 edits.
    - Verification: `cargo test -p db-migration`

- [x] T2.2 Write repository tests before implementation for record/list/count/summary/cost/delete.
    - Files: `crates/infrastructure/audit-sqlite/src/lib.rs` or `crates/infrastructure/audit-sqlite/tests/*`
    - Acceptance: Tests cover complete records, null optional token fields, timestamp-desc pagination, filtered aggregates, cost grouping, and expired-row deletion.
    - Verification: `cargo test -p audit-sqlite usage`

- [x] T2.3 Implement `SqliteUsageRepository`.
    - Files: `crates/infrastructure/audit-sqlite/src/lib.rs`
    - Acceptance: Implements `UsageRecorderPort`; safely maps nullable SQL fields; filter SQL supports provider/model/api_key/connection/date/status.
    - Verification: `cargo test -p audit-sqlite usage`

- [x] T2.4 Implement runtime provider-to-connection lookup.
    - Files: `crates/infrastructure/provider-sqlite/src/repository.rs`
    - Acceptance: Active dynamic provider returns highest-priority `ConnectionId`; TOML/unknown providers return `Ok(None)`.
    - Verification: `cargo test -p provider-sqlite find_connection_id_by_runtime`

## Phase 3 — Config and cost calculation

- [x] T3.1 Add usage retention config and pricing config with tests first.
    - Files: `apps/rook/src/config.rs`, config tests
    - Acceptance: `[usage]` defaults to 90 days/6 hours; `[pricing.<provider>.<model>]` deserializes and lookup works, including quoted model segments for IDs containing dots.
    - Verification: `cargo test -p rook config`

- [x] T3.2 Add pure cost-estimation helper tests before usecase integration.
    - Files: `crates/application/rook-usecases/src/route_request.rs` or extracted helper module
    - Acceptance: Formula covers prompt/completion/cache read/cache creation; missing pricing returns `None` from the use-case cost helper and emits warning/metric path before persistence.
    - Verification: `cargo test -p rook-usecases estimate_cost`

- [x] T3.3 Implement cost calculation without provider adapter coupling.
    - Files: `crates/application/rook-usecases/src/route_request.rs`
    - Acceptance: Cost is calculated from config in usecase layer; unknown pricing is never represented as `0.0`.
    - Verification: `cargo test -p rook-usecases estimate_cost`

## Phase 4 — Request propagation and usecase recording

- [x] T4.1 Add request metadata fields and API-key extraction tests.
    - Files: `crates/domain/rook-core/src/model.rs`, `crates/infrastructure/transport-axum/src/authz.rs`
    - Acceptance: `x-authz-auth-id` maps to `Some(ApiKeyId)`; missing/empty/`public` maps to `None`.
    - Verification: `cargo test -p transport-axum extract_api_key_id`

- [x] T4.2 Wire nullable usage recorder, provider repository, and pricing through usecases.
    - Files: `crates/application/rook-usecases/src/lib.rs`, `crates/application/rook-usecases/src/route_request.rs`, usecase tests/fakes
    - Acceptance: `RouteRequest` can run with `usage_recorder: None`; provider connection lookup failures warn and continue.
    - Verification: `cargo test -p rook-usecases route_request`

- [x] T4.3 Record usage entries for non-streaming success/failure.
    - Files: `crates/application/rook-usecases/src/route_request.rs`
    - Acceptance: Success records latency as TTFT; failures before usage preserve nullable tokens/TTFT and do not fail client response because recording failed.
    - Verification: `cargo test -p rook-usecases non_stream_usage`

- [x] T4.4 Record streaming TTFT and final usage; instrument four fire-and-forget sites.
    - Files: `crates/application/rook-usecases/src/route_request.rs`
    - Acceptance: First successful chunk sets `ttft_ms`; success/failure recording errors emit `tracing::warn!` with `usage_record_failed = true` and increment `usage_record_failed_total` metric.
    - Verification: `cargo test -p rook-usecases streaming_usage`
    - Split note: If this diff grows, split into T4.4a TTFT recording and T4.4b warning/metric instrumentation.

## Phase 5 — Provider token extraction

- [x] T5.1 Extend OpenAI usage parsing and streaming request options.
    - Files: `crates/infrastructure/providers-openai/src/provider.rs`
    - Acceptance: Parses cached and reasoning tokens; streaming requests set `stream_options.include_usage = true`.
    - Verification: `cargo test -p providers-openai usage`

- [x] T5.2 Extend Anthropic usage parsing.
    - Files: `crates/infrastructure/providers-anthropic/src/lib.rs`
    - Acceptance: Non-streaming and message-delta paths map cache creation/read tokens; reasoning remains `None`.
    - Verification: `cargo test -p providers-anthropic usage`

- [x] T5.3 Add partial usage parsing for Ollama, Gemini, and Groq.
    - Files: `crates/infrastructure/providers-ollama/src/lib.rs`, `crates/infrastructure/providers-gemini/src/lib.rs`, `crates/infrastructure/providers-groq/src/lib.rs`
    - Acceptance: Prompt/completion totals are parsed where available; cache/reasoning dimensions remain `None`.
    - Verification: `cargo test -p providers-ollama -p providers-gemini -p providers-groq usage`
    - Split note: If this diff grows, split per provider.

## Phase 6 — Transport API endpoints

- [x] T6.1 Create usage handler query mapping tests before handlers.
    - Files: `crates/infrastructure/transport-axum/src/handlers/usage.rs`, `crates/infrastructure/transport-axum/src/handlers/mod.rs`
    - Acceptance: Query strings map to `UsageFilters`; invalid connection/status/date values return HTTP errors; limit clamps to 1000.
    - Verification: `cargo test -p transport-axum usage_query`

- [x] T6.2 Implement `GET /api/usage`, `/api/usage/summary`, and `/api/usage/cost` handlers.
    - Files: `crates/infrastructure/transport-axum/src/handlers/usage.rs`
    - Acceptance: List returns `{ entries, total }`; summary and cost call the port; unavailable recorder returns `503 USAGE_RECORDER_UNAVAILABLE`.
    - Verification: `cargo test -p transport-axum usage_handlers`

- [x] T6.3 Mount usage routes under management auth.
    - Files: `crates/infrastructure/transport-axum/src/routes.rs`, `crates/infrastructure/transport-axum/src/authz.rs`
    - Acceptance: Routes are always mounted; unavailable recorder returns `503 USAGE_RECORDER_UNAVAILABLE`; `/api/usage*` is classified as `Management`; GET needs no CSRF.
    - Verification: `cargo test -p transport-axum usage_routes`

## Phase 7 — Retention sweep and DI wiring

- [x] T7.1 Wire repositories, pricing, and config in DI.
    - Files: `apps/rook/src/di.rs`
    - Acceptance: Single shared provider repository is reused; concrete `SqliteUsageRepository` is stored for retention; `RookUsecases` receives nullable usage port.
    - Verification: `cargo test -p rook di`

- [x] T7.2 Add retention sweep task.
    - Files: `apps/rook/src/usage_retention.rs`, `apps/rook/src/server.rs` or `apps/rook/src/main.rs`
    - Acceptance: Startup sweep is awaited before serving; periodic sweep runs every configured interval after startup; failures warn without crashing.
    - Verification: `cargo test -p rook usage_retention`

## Phase 8 — Tests and documentation

Project convention: add tests in separate test targets/files where practical; do not introduce new inline `#[cfg(test)]` modules in production files.

- [x] T8.1 Add cross-layer integration coverage for successful routed usage record.
    - Files: relevant integration test crates under `crates/application/*`, `crates/infrastructure/*`, or `apps/rook/tests/*`
    - Acceptance: A routed success writes provider/model/api_key/connection/tokens/latency/TTFT/cost into `usage_history`.
    - Verification: `cargo test -p rook-usecases -p audit-sqlite -p transport-axum usage`

- [x] T8.2 Update configuration documentation after exact config shape is verified.
    - Files: `docs/configuration.md`
    - Acceptance: Documents `[usage]`, `[pricing.<provider>.<model>]`, quoted model segments for IDs containing dots, cache pricing defaults, and missing-pricing behavior.
    - Verification: `cargo test -p rook config && cargo test -p rook-usecases estimate_cost`

- [x] T8.3 Run focused and full verification gates.
    - Files: workspace verification only; no implementation files expected
    - Acceptance: Targeted tests pass, formatting/lints pass, and canonical local CI passes.
    - Verification: `cargo fmt --check && cargo test -p rook-core -p audit-sqlite -p provider-sqlite -p rook-usecases -p transport-axum && just ci-local`

## Final Verification Checklist

- [x] `cargo fmt --check`
- [x] `cargo test -p rook-core`
- [x] `cargo test -p db-migration`
- [x] `cargo test -p audit-sqlite usage`
- [x] `cargo test -p provider-sqlite find_connection_id_by_runtime`
- [x] `cargo test -p rook-usecases route_request streaming_usage estimate_cost`
- [x] `cargo test -p providers-openai -p providers-anthropic usage`
- [x] `cargo test -p providers-ollama -p providers-gemini -p providers-groq usage`
- [x] `cargo test -p transport-axum usage`
- [x] `cargo test -p rook config usage_retention di`
- [x] `just ci-local`
