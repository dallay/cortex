# Proposal: Provider Quota Tracking

> **GitHub issue**: [#132](https://github.com/dallay/cortex/issues/132)
> **Change**: `provider-quota-tracking`

## Intent

Replace the mocked/placeholder `/providers/quota` experience with trustworthy per-provider usage, historical trends, threshold warnings, and graceful degradation. The current code already records usage and serves a partial quota summary; the change will close the gaps in how those facts are captured, exposed, and presented.

## Problem

The current page displays recorded Cortex-routed tokens, estimated cost, and rate-limited request ratios, but does not render the historical trend data already returned by the API. It also has no refresh/staleness behavior and renders a global error when the endpoint or any required usage query fails. Most importantly, the word “quota” and current warning labels can imply provider account limits even though the backend mostly has request-history data and local thresholds. Provider account limits are not generally available from the current integrations.

## Product Contract — Decision Required

The term “quota/usage” currently blurs three separate things:

- **Measured usage:** tokens, requests, and estimated cost for traffic recorded by Cortex. This is not necessarily the provider account's total usage.
- **Observed provider signals:** upstream 429 responses and provider-supplied remaining/reset headers, if captured. These are observations, not necessarily a complete or current account balance.
- **Account limits:** authoritative provider maximum/remaining/consumed quota. Show this only when sourced from a verified provider API or explicitly configured by the user with clear provenance. Never derive or invent account limits from token counts, cost, rate-limit ratios, or `quotaWindowThresholds`.

**Recommended v1 contract:** ship usage/trends plus separately labeled observed rate-limit signals and local configured-threshold warnings; do not claim account quota/remaining values absent an authoritative source. The user must confirm whether any account-level integrations are required for v1 and whether to retain “Provider quota” as the page label or rename it to “Provider usage” / “Usage & limits.”

For **self-hosted Ollama**, account quota is not applicable/not provided; show measured routed usage where available, not a fabricated zero/unlimited limit. Cost is zero only when explicitly configured as zero, otherwise unknown. For **Ollama Cloud**, show upstream rate-limit/header observations only when actually captured; missing signals mean “not reported,” not “healthy” or “no limit.”

## Scope

### In Scope

- Replace the placeholder/mock contract in `providers-ui` REQ-8 with a truthful usage-and-quota-observation contract.
- Complete the existing `GET /api/providers/quota` data contract for per-provider measured usage, historical trends, and distinct warning/provenance/availability states.
- Capture and expose upstream provider rate-limit signals only where actually available, keeping them separate from Cortex's own API rate-limit headers and local configured thresholds.
- Render historical trends and clarify measured usage coverage, estimate/unknown cost, configured local threshold warnings, and provider-specific support states in the dashboard.
- Add refresh behavior and preserve the last successful result with a visible stale/degraded indication when refresh fails.
- Degrade gracefully for missing provider data, unsupported quota APIs, absent usage recorder/provider management, and local Ollama; avoid failing the whole page for a single provider/field gap where practical.
- Cover configured and unconfigured providers, missing usage, partial backend failures, threshold/rate-limit warnings, refresh errors, and Ollama local/Cloud distinctions.

### Out of Scope (unless user confirms otherwise)

- Inventing account limits or inferring them from recorded tokens, costs, 429 rates, or local thresholds.
- Claiming Cortex-routed usage is a provider account-wide total.
- Provider account-quota integrations without a confirmed authoritative API and provider-specific source.
- Changing request routing, retry/failover policy, or rejecting requests based on dashboard warning thresholds.
- Changing Cortex's own client API rate-limit behavior or presenting its `x-ratelimit-*` response headers as provider quota data.

## Capabilities

### Modified Capabilities

- **`providers-ui`**: Replace REQ-8's mocked placeholder page behavior with real usage/trend presentation, truthful quota-data provenance, warning semantics, refresh/stale states, and graceful per-provider behavior.
- **Provider usage/quota transport contract** (capability name to confirm during spec): Define `/api/providers/quota` response and degradation semantics for measured usage, observed upstream signals, local thresholds, authoritative limits (only if included), and data availability.

## Approach

Build on existing `UsageRecorderPort`, `usage_history`, `/api/providers/quota`, and `useProvidersQuota` rather than creating a parallel usage system. First lock down response semantics and source provenance; then close the selected capture/query gaps, update the dashboard to display trends and warning states, and verify partial failure plus local/cloud Ollama behavior. Keep the current URL stable by default while resolving the page terminology with the user.

## Key Questions for User

1. Confirm the recommended scope: usage and observed signals in v1; no provider account limits unless an authoritative integration is identified. Which provider APIs, if any, are explicitly required now?
2. Retain “Provider quota” or rename the UI to “Provider usage” / “Usage & limits” while keeping `/providers/quota` compatible?
3. Should warning thresholds be explicitly treated as locally configured thresholds over recorded metrics, distinct from provider-reported rate-limit events? What metric should they apply to?
4. Which trends are most useful: requests, tokens, estimated cost, 429s, and/or provider header snapshots? What refresh cadence is acceptable?

## Risks

| Risk                                                                           | Likelihood | Mitigation                                                                           |
|--------------------------------------------------------------------------------|------------|--------------------------------------------------------------------------------------|
| Dashboard implies an account-wide quota where only Cortex request usage exists | High       | Explicit provenance, Cortex-only coverage label, and no fabricated limits            |
| Existing `quotaWindowThresholds` are confused with provider limits             | High       | Name them local thresholds and keep warning semantics separate from upstream signals |
| Provider header support differs or is absent                                   | High       | Optional values with provider-specific availability; no false default values         |
| Partial database/provider failures erase useful data                           | Medium     | Define per-provider/field degradation and retain last successful dashboard result    |
| Cost estimate presented as authoritative billing                               | Medium     | Mark as estimate; show unknown when pricing is missing                               |

## Next Phase

After the user confirms the product contract, `sdd-spec` should create/update delta specs for `providers-ui` and the backend quota/usage API capability. Specs should include observable Given/When/Then scenarios for provenance separation, measured trends, local warnings, unavailable/stale data, partial failure, and Ollama local versus Cloud. No technical design or task breakdown is included in this proposal phase.
