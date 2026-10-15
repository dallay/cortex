# Exploration: Provider Quota Tracking

> **GitHub issue**: [#132](https://github.com/dallay/cortex/issues/132)
> **Change slug**: `provider-quota-tracking`
> **Phase**: 1 — Explore
> **Artifact store**: `.agents/sdd/`

## Intent

Replace the quota dashboard's misleading mock/placeholder contract with real, per-provider usage, historical trends, provider-reported rate-limit observations where available, threshold warnings, and graceful degradation. This exploration records the current implementation and the contract decisions that must be made before writing delta specs.

## Existing SDD Contract and Precedent

- `.agents/sdd/specs/providers-ui/spec.md` REQ-8 (lines 122–131) currently requires a mocked `/providers/quota` view with tokens, limits, cost, a placeholder banner, and an issue reference. Issue #132 is the follow-up that changes this contract.
- Archived change `2026-06-06-providers-ui-3-screen-refactor/design.md` D7 concerns validation of the provider-kind route parameter in `beforeEnter`. It is unrelated to quota data or this change; retain its route-validation behavior if provider routes are touched.
- The archived change's quota design treated the page as pure presentation of mock data. Current implementation has advanced beyond that design: a backend endpoint and typed dashboard client are present. The new change should replace/refine the current behavior rather than reimplement that archived design.
- The archived provider-connections contract describes `quotaWindowThresholds` as read-only monitoring configuration, not provider account limits or enforcement.

## Current State

### Backend usage and persistence

- `rook_core::UsageEntry` records request usage by provider/model and optional connection ID, with prompt/completion/cache/reasoning token fields, optional cost, status, and timestamp (`crates/domain/rook-core/src/model.rs`). `UsageRecorderPort` supports record, count, summary, and cost queries (`crates/domain/rook-core/src/ports.rs`).
- `usage_history` is persisted by the V2 SQLite migration and `audit-sqlite` implementation. Existing summaries can be grouped by connection and time range, which is enough to build request/token/cost trends for recorded traffic.
- Provider response adapters expose token usage for the supported providers. This is usage observed on requests routed through Cortex, not necessarily total usage on the provider account (usage outside Cortex is not visible).
- `ProviderConnection.config.quota_window_thresholds` are local configured thresholds. They are not provider-issued maximums, remaining capacity, or account usage percentages.

### Existing quota API

- `GET /api/providers/quota` is registered in `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs` and returns per-provider summaries for six provider kinds, including configured/active connection counts, all-time/24-hour/7-day totals, seven daily trend points, rate-limited request ratio, warning/error thresholds, support notes, and a warning level.
- The handler aggregates usage by configured connection IDs and asks the recorder for summary, rate-limited count, and cost breakdown. It currently returns a whole-request 503 if the recorder or connection-management use case is unavailable, and an internal error if any individual summary query fails. One gap therefore hides all otherwise available providers.
- Provider support notes already acknowledge that public/account-level quota data is generally unavailable. However, the OpenAI note says limits are “inferred from local thresholds and observed usage,” and warning level compares the rate-limited-request ratio against configured threshold values. That risks making a local threshold look like a provider account quota.
- The endpoint's rate-limited observations are derived from stored request status. Provider adapters have some 429 handling and Ollama Cloud extracts `Retry-After` / `X-RateLimit-Reset` for error/backoff paths, but those observed upstream header values are not persisted or exposed by this summary endpoint. The `x-ratelimit-*` headers in `transport-axum/src/authz.rs` describe Cortex's own client API limiter; they must not be presented as upstream provider quota data.

### Dashboard

- `apps/rook/dashboard/src/lib/api.ts` contains typed response models and calls `/api/providers/quota`; `useProvidersQuota.ts` fetches once on mount and retains a single error state. There is no periodic refresh/polling or explicit per-provider data-quality state in the composable.
- `ProvidersQuotaView.vue` renders token usage over 24 hours, estimated cost over 7 days, the rate-limited ratio, support notes, and a warning badge. The response includes historical trend points, but the view does not render them. Several response dimensions (all-time, 7-day tokens, observed threshold context) are also not displayed.
- The visible table can read as a quota meter despite showing usage and rate-limited-request saturation rather than provider account limit/remaining data. API failure is rendered as a destructive error; stale/partial data and local Ollama's no-account-quota case are not represented as distinct, graceful states.

## Gaps and Product Contract Ambiguity

“Provider quota/usage” conflates at least three different facts, which the product and API must distinguish:

1. **Measured Cortex usage** — requests, tokens, and estimated cost recorded for traffic that passed through Cortex. These are measured totals, not whole-account totals. Cost may be unknown when pricing metadata is unavailable.
2. **Observed upstream rate-limit signals** — e.g. upstream 429 responses and provider-supplied remaining/reset headers, when an adapter actually receives and captures them. These are observations, may be intermittent or stale, and are not equivalent to a complete account quota.
3. **Provider account quota/limits** — a provider-defined maximum, current consumption, or remaining amount. This can only be displayed as such when obtained from an authoritative provider source/API or explicit user configuration with clear provenance. Do not infer or invent account limits from local `quotaWindowThresholds`, token totals, cost estimates, or rate-limit ratios.

The proposal must get agreement on whether v1 is a usage-and-observations dashboard (recommended) or includes account-level quota integrations for providers that expose them. It should define distinct provenance/availability states instead of a single ambiguous `support` or `warningLevel` label.

### Ollama/local behavior

Self-hosted Ollama has no third-party account quota to query. It should still show measured Cortex-routed request/token trends where available, and describe account quota as not applicable/not provided—not zero, unlimited, or exhausted. Cost should be zero only if explicitly configured/priced as zero; otherwise “unknown” is safer. For Ollama Cloud, show observed provider rate-limit/header signals only when available and clearly label them as observations. Missing headers or usage history should degrade to “unavailable/not reported,” not a fabricated quota or a page failure.

## Open Decisions for Proposal/Spec

- Keep the term “quota” in the route/navigation, or label the page “Usage & limits” / “Usage” to avoid implying an account-wide quota meter?
- Which providers, if any, have a verified authoritative quota/usage API in scope for v1? Otherwise explicitly defer account-level integrations.
- What are the exact provenance states and display semantics for measured usage, provider headers, configured local thresholds, unavailable data, stale data, and unknown cost?
- Should warnings use explicit user-configured local thresholds against a well-defined measured quantity, or observed 429/header signals, or both as separate warnings? Existing `quotaWindowThresholds` are not documented as observed-429 ratios.
- What refresh cadence and stale-data behavior should the view have? Avoid overlapping requests and preserve the last successful data if a later poll fails.
- Should API responses degrade per-provider/field when storage or a provider's data is missing, rather than fail the entire summary? Which failures should remain fatal?
- Should historical trends show request count, token usage, cost, 429s, and/or provider header snapshots? State the retention/data gaps clearly.

## Candidate Scope

- Define truthful distinctions between recorded usage, observed upstream rate-limit signals, locally configured thresholds, and authoritative account limits.
- Complete the current backend response/data-capture contract for the selected signals and partial-failure semantics.
- Update the dashboard to render historical trends, useful warnings, update/staleness state, and provider-specific unavailable/not-applicable behavior; remove the mock/placeholder contract.
- Cover representative provider support states, especially self-hosted Ollama versus Ollama Cloud.

## Risks

| Risk | Impact | Mitigation direction |
|---|---|---|
| Local thresholds or 429 ratios are presented as account quota percentages | Users make billing/capacity decisions from false data | Separate the concepts and label data provenance; never fabricate provider limits |
| Cortex request usage is mistaken for complete provider account usage | Under-counting when users also call provider APIs directly | Label all recorded usage as Cortex-routed and state coverage explicitly |
| One missing integration/storage error blanks all provider data | Dashboard becomes unusable during partial outages | Specify per-provider/field degradation and preserve last good data |
| Local Ollama gets a misleading quota meter or cost | False warning/zero/unlimited interpretation | Model quota as not applicable/not reported; keep usage and cost as separate facts |
| Existing response's 7-day trends and warning labels are inconsistent or unused | UI and API disagree on what a warning means | Define an observable warning contract and scenario coverage before implementation |

## Recommendation

Scope v1 as a real Cortex-routed usage and trend dashboard with clearly labeled upstream rate-limit observations where captured, plus explicitly local threshold warnings. Defer account-level quota claims/integrations until an authoritative data source is confirmed per provider. Preserve route compatibility unless the user chooses a terminology change. Treat local Ollama quota as not applicable, while still reporting its measured routed usage and unknown/explicitly configured cost correctly.
