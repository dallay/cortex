# Design: Provider Quota Tracking

## Summary

Evolve the existing `GET /api/providers/quota` endpoint and `/providers/quota` dashboard route into a truthful **Provider usage** view. Reuse the persisted Cortex request-usage history and provider-connection configuration; do not add account-quota integrations or imply that Cortex-routed history is a provider-wide account balance. Keep the existing route stable.

The backend will expose measured usage, data completeness, observed rate-limited outcomes, local threshold configuration/warnings, and account-quota availability as separate concepts. The dashboard will render the existing seven daily trend buckets, clearly label estimates and coverage, and retain its last good response across failed refreshes. Upstream headers are not exposed in v1: although some adapters parse retry/reset values while constructing errors, those values are currently transient and are not recorded with usage history. The only upstream-like signal included is the stored `RequestStatus::RateLimited` outcome for routed requests; it is a count/ratio of observed rate-limited requests, not a remaining quota or health guarantee.

## Current Architecture and Constraints

- `apps/rook/src/di.rs` constructs one `SqliteUsageRepository`, passes it to `RookUsecases`, and optionally supplies provider-management (`ManageConnections`) according to provider CRUD configuration. `/api/providers/quota` is registered in `crates/infrastructure/transport-axum/src/routes.rs` and is protected by the existing admin middleware.
- `rook_core::UsageEntry` persists request ID, provider/model, status, optional connection ID, optional token counts, optional cost, and timestamp. `UsageRecorderPort` exposes summary/count/cost-breakdown operations. `V2__usage_history.sql` already stores nullable `cost_usd`; no schema migration is required for usage or cost completeness.
- `SqliteUsageRepository::summary` coalesces missing token values to zero, and `cost_breakdown` coalesces nullable cost to zero. Therefore the current API cannot distinguish absent usage from a measured zero or unknown cost from zero cost using the current aggregates alone. Cost completeness must be added to the recorder aggregate contract.
- `route_request.rs` records provider request outcomes and token/cost usage. Provider 429 outcomes become `RequestStatus::RateLimited`; response headers are not part of `UsageEntry`. OpenAI and Groq map retry/reset headers into errors; Ollama Cloud extracts `Retry-After` / `X-RateLimit-Reset` on 429 paths. This does not constitute a durable, generalized header-observation API. Do not add unsupported header names, defaults, or synthetic reset/remaining values.
- Provider `quota_window_thresholds` are stored per connection. The current handler averages them by provider kind and compares them with the seven-day rate-limited request ratio. Averaging heterogeneous connection thresholds obscures provenance and is not a meaningful effective threshold. The new design evaluates each configured connection against its own seven-day rate-limited ratio and summarizes the highest resulting severity at provider level.
- Dashboard API types live in `apps/rook/dashboard/src/lib/api.ts`; `useProvidersQuota.ts` currently fetches once and holds a single error state. `ProvidersQuotaView.vue` presents a table but omits the returned trend. The app has Vue/Vitest, English/Spanish locale resources, and chart wrappers under `src/components/ui/chart/`; reuse those wrappers rather than introducing a charting package.

## API and Data Contract

Keep `GET /api/providers/quota`, the admin authorization, and `generatedAt` / `items`. Replace ambiguous quota fields with explicit names and availability metadata. Proposed camelCase shape (exact Rust DTO names may follow repository conventions):

```json
{
  "generatedAt": "RFC3339 timestamp",
  "items": [{
    "providerKind": "openai",
    "connectionCount": 1,
    "activeConnectionCount": 1,
    "coverage": "cortex_routed",
    "usageAvailability": "available",
    "accountQuota": { "availability": "not_provided" },
    "localThresholds": [
      { "warning": 0.7, "error": 0.9, "connectionCount": 1 }
    ],
    "rateLimitObservation": {
      "source": "recorded_routed_request_status",
      "availability": "available",
      "window": "last_7d",
      "rateLimitedRequests": 1,
      "requests": 20,
      "ratio": 0.05
    },
    "localWarningLevel": "ok",
    "allTime": { "availability": "available", "requests": 20, "...": "..." },
    "last24h": { "availability": "no_data", "...": "..." },
    "last7d": { "availability": "available", "...": "..." },
    "trend": [{
      "date": "YYYY-MM-DD",
      "usageAvailability": "available",
      "requests": 1,
      "rateLimitedRequests": 0,
      "totalTokens": 100,
      "costUsd": null,
      "costAvailability": "unknown"
    }]
  }]
}
```

Contract rules:

1. `coverage` is always `cortex_routed`; request/token totals cover traffic recorded by Cortex only. No field represents total provider-account consumption.
2. A period/window has `available`, `no_data`, or `unavailable` status. `no_data` means the query succeeded and returned no matching records; numeric counts in that case are zero as a measured absence in the selected interval, not a provider account quota. `unavailable` means the relevant recorder query failed. Preserve successful fields/windows when a different query fails.
3. Cost is an estimate from configured pricing, not provider-reported billing. Represent cost as nullable and accompany it with `costAvailability` (`complete`, `partial`, `unknown`, `no_data`, `unavailable`). If every matched record has priced cost, return the sum and `complete`; if some are unpriced, return null and `partial`; if records exist and none have priced cost, return null and `unknown`; if no records exist, return null and `no_data`. Never serialize an unknown aggregate as `0`.
4. `rateLimitObservation` is derived only from persisted routed request statuses. For successful queries it reports the number of rate-limited outcomes and request denominator for the selected seven-day interval. A missing/failing query is `unavailable`; zero rate-limited outcomes is a measured zero and does not mean healthy, unlimited, or no provider limit. It must not include Cortex client-facing `x-ratelimit-*` middleware data.
5. `localThresholds` retains the actual configured warning/error values, grouped by identical pair with a count rather than averaged. Evaluate each connection's seven-day rate-limited ratio against that connection's own pair; aggregate to `localWarningLevel` using the highest observed severity. If the ratio or thresholds for a connection cannot be evaluated, do not classify that connection as `ok`; report provider warning availability as unavailable/degraded (or a distinct `unknown` level if used consistently in the DTO/UI). These values are monitoring thresholds only and do not enforce routing.
6. `accountQuota.availability` is `not_provided` for providers without an authoritative source and `not_applicable` for self-hosted Ollama. No account maximum, consumed amount, remaining amount, zero, or unlimited value is included. Ollama Cloud remains `not_provided`; only the existing recorded routed usage and rate-limited outcomes are available in v1.
7. Do not return provider-supplied retry/reset header snapshots in this change. Adapter-level parsing is transient and not a consistent persisted observation; capturing headers would require a separate domain/data-flow design, storage/retention and security review. The API should say no header observation is available/not captured rather than imply no limit exists.

## Backend Design and Data Flow

1. `list_provider_quota` still requires `UsageRecorderPort` and provider-management listing; if either dependency is absent, or provider listing itself fails, retain the current request-level service/internal error. These dependencies are necessary to establish the configured-provider population and actual usage source. Do not silently emit an all-zero dashboard.
2. Build provider-kind rows from the seven supported `ProviderKind` values and the connections returned by `ManageConnections::list`. Keep rows for unconfigured provider kinds, but mark usage `no_data` / configuration `not_configured`; do not imply they have zero account quota.
3. Query usage by configured connection IDs for all-time, trailing 24-hour, trailing seven-day, and seven calendar-day trend buckets (UTC, consistent with the existing date bucket behavior). For each provider/window, treat usage summary, rate-limited count, and cost completeness as independently fallible measures. Return each successful measure; mark only the failed measure/window unavailable and continue processing other providers and buckets. A failed provider/window must not `?` out of the entire endpoint.
4. Keep the current `UsageRecorderPort` operations and extend `CostBreakdown` with cost-known and cost-unknown entry counts (or an equivalent cost-completeness aggregate) so the endpoint can distinguish nullable costs. `SqliteUsageRepository::cost_breakdown` should calculate `SUM(cost_usd)` and `COUNT(cost_usd)` plus total matching record count using SQL aggregates under the existing filter set. Existing cost consumers can continue using the numeric sum; the provider-usage response uses the new completeness fields. Empty window is represented as `no_data`, not a cost of zero.
5. Change `ProviderQuotaSummaryResponse`, window/trend DTOs in `provider_dto.rs` from the current `support`, `note`, `warning_threshold`, `error_threshold`, `observed_rate_limited_ratio`, and `warning_level` terminology to the explicit contract above. The handler should contain narrowly scoped aggregation/classification helpers; remove the misleading OpenAI support note that says limits are inferred. No authoritative-account-quota adapter, new usage-history table, or header migration is included.
6. `UsageEntry` already carries `connection_id`, status, nullable cost, and timestamps; no change to request recording, adapter metadata, connection configuration, or DI is needed. Thresholds come from each `ProviderConnection.config.quota_window_thresholds`. Classification uses the seven-day rate-limited ratio already calculated by the endpoint, per connection, preserving the current observable metric while making local provenance explicit.

### Query/performance note

The current handler issues summary/count/cost operations per connection, per time window and trend day. The first implementation should preserve the existing port boundary and avoid changing routing or the recorder's general query model. Avoid spawning unbounded per-connection tasks because the SQLite repository serializes access through a mutex. If test or profiling evidence shows this endpoint is too expensive, a batch-by-connection-IDs aggregate can be a follow-up optimization; it is not required to meet this contract.

## Dashboard Design

- Rename the page title to **Provider usage** in navigation/page copy and update subtitle/help text to state “Cortex-routed” coverage and that it is not total provider-account usage. Keep `/providers/quota` route compatibility.
- Update `lib/api.ts` types to mirror the new DTO, including nullable costs, explicit availability enums, threshold groups, and warning/account-quota provenance. Keep `useApi().getProvidersQuota()` path unchanged.
- Upgrade `useProvidersQuota.ts` to expose `refresh`, `loading`/`refreshing`, `lastSuccessfulAt`, `refreshError`, and `isStale`. Retain the previous successful response on refresh error. Start a 60-second periodic refresh when mounted, prevent overlapping requests, support a manual refresh button, and clear the timer on unmount. If the initial load fails and no data exists, show the initial error/empty state; if a later refresh fails, show a non-blocking stale banner and keep all last-good values visible.
- Update `ProvidersQuotaView.vue` to explain coverage and provenance. Use per-provider status text instead of a generic healthy badge; distinguish “no recorded data”, “unavailable”, local-threshold warning, observed rate-limited requests, and account quota not provided/not applicable. Show unknown/partial cost as unknown rather than currency zero. Keep row rendering resilient to unknown future provider kinds.
- Render the API's seven-day trend points with the existing chart primitives in `components/ui/chart` and `@unovis/vue`; plot requests and total tokens, and estimated cost only for points with known/complete cost. Preserve unknown cost gaps (do not coerce null to zero). Include an accessible table/list equivalent or text summary for chart data and empty/unavailable states. No new chart dependency is planned.
- Add/replace localization keys for the page, labels, availability/provenance explanations, stale state, warning semantics, and chart labels in both `src/locales/en.json` and `src/locales/es.json`. The current visible page renders the server-provided `note` as raw text; replace that with typed/localized provenance copy rather than provider-authored free-form claims.

## Files Expected to Change

### Rust backend

- `crates/domain/rook-core/src/model.rs` — extend `CostBreakdown` with known/unknown cost entry counts, preserving the existing total-cost numeric field for current consumers.
- `crates/infrastructure/audit-sqlite/src/lib.rs` — implement completeness aggregates in `cost_breakdown`; add repository tests for all-known, mixed-known/unknown, all-unknown, and empty filtered windows.
- `crates/infrastructure/transport-axum/src/provider_dto.rs` — replace/extend quota response DTOs with provenance and availability-aware types; use nullable cost fields.
- `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs` — independent field/window error degradation, explicit cost states, per-connection local threshold evaluation, correct Ollama/account-quota availability, and UTC trend aggregation.
- `crates/infrastructure/transport-axum/tests/` (likely a new `provider_quota` integration test target, or existing suitable route test) — endpoint serialization and aggregate behavior tests with fake recorder/manage-connections, covering partial failure and all seven provider kinds.
- `crates/domain/rook-core/src/ports.rs` and implementations/mocks of `UsageRecorderPort` only if changing the aggregate interface requires updating trait-local/application test doubles. Prefer extending `CostBreakdown` over adding another port method.

### Dashboard

- `apps/rook/dashboard/src/lib/api.ts` — new response types.
- `apps/rook/dashboard/src/composables/useProvidersQuota.ts` — retain-last-good, polling, overlap prevention, refresh/stale state.
- `apps/rook/dashboard/src/views/ProvidersQuotaView.vue` — truthful summary, trend visualization, partial/unavailable states, refresh affordance and accessibility fallback.
- `apps/rook/dashboard/src/locales/en.json`, `apps/rook/dashboard/src/locales/es.json` — translated terminology and state labels.
- `apps/rook/dashboard/src/composables/useProvidersQuota.spec.ts` and `apps/rook/dashboard/src/views/ProvidersQuotaView.spec.ts` (new) — timer/refresh/error-retention and rendered provenance/trend/unknown-cost tests, following existing Vitest + Vue Test Utils conventions.
- `apps/rook/dashboard/src/__tests__/providersI18n.spec.ts` — extend smoke coverage for new usage keys in both locales.

No changes are expected to `V2__usage_history.sql`, provider adapter HTTP handling, request routing, DI construction, authorization, or navigation route paths.

## Test Strategy

1. **SQLite aggregate tests:** verify nullable `cost_usd` distinction and completeness counts for empty, explicitly zero-priced, fully priced, partly priced, and fully unknown records; confirm filters by connection and date are honored.
2. **Backend handler tests:** deterministic fixed timestamps and fake recorder failures per query. Assert Cortex-only coverage, provider-kind rows, UTC buckets, per-connection threshold semantics, nullable unknown costs, no fabricated account limits, self-hosted Ollama `not_applicable`, Ollama Cloud `not_provided`, and that a failed cost/count/window/provider query does not discard successful data elsewhere. Verify missing recorder/provider-management remains an intentional request-level error.
3. **DTO serialization tests:** check camelCase names, `null` for unknown cost, explicit availability/source values, and absence of legacy misleading `support` / generic healthy/account-limit fields. Do not assert volatile database implementation details beyond externally visible contract.
4. **Dashboard composable tests:** fake timers and API responses for initial success, periodic/manual refresh, overlapping request suppression, refresh failure with last-good response retained and `isStale`, recovery clearing stale state, and unmount timer cleanup.
5. **Dashboard view/i18n tests:** render usage label and Cortex coverage notice, trend points, unknown-cost text, unavailable/no-data distinction, observed rate-limited wording, local threshold provenance, Ollama local/Cloud quota states, and stale refresh banner. Ensure key labels resolve in English and Spanish. Keep chart accessibility fallback assertions.
6. Run focused Rust crate and dashboard Vitest/typecheck/build checks during implementation; final verification follows repository policy (`just ci-local`).

## Decisions and Deferred Work

- **No provider-account quota integrations in v1.** The approved product contract excludes them unless a verified authoritative source is added in a later change.
- **No upstream header persistence in v1.** Adapter parsing is currently transient and provider-specific. Persisting header values would require an explicit typed observation model, retention policy, safe header allowlist, and design for connection attribution; do not invent this during implementation.
- **Rate-limit ratio remains the metric for existing local thresholds**, but the thresholds are evaluated per connection and labeled as local monitoring thresholds. They are not provider limits, not account utilization, and do not affect request routing.
- **Cost completeness is the one persistence-adjacent contract gap** addressed without a migration: `cost_usd` is already nullable in SQLite, and aggregate counts can reveal whether the sum is complete.
- **Refresh interval is 60 seconds** with one in-flight request at a time; this is dashboard refresh only and does not change backend polling or provider traffic.

## Risks / Implementation Guardrails

- Do not coerce `Option<f64>` cost into `0.0` in DTO conversion, chart data, localization formatting, or tests.
- Separate request status observations from actual provider remaining/reset headers; no header is surfaced unless future work captures it durably with a verified source.
- Avoid the old “Healthy” label for a zero rate-limited ratio. Zero observed rate-limited requests is not evidence that there is no limit or that the provider is healthy.
- Avoid dropping an entire provider or response when a local query fails; keep failures scoped and visible. Only absent core dependencies/listing remain request-level errors.
- Preserve existing admin authorization and route path. Do not alter provider retry, failover, or enforcement behavior.
