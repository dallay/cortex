# Proposal: Per-Provider Quota Integration

## Intent

Preserve local request/token/cost history and separately show upstream quota/headroom only when actual response data supports it. The endpoint serves SQLite usage, but stores no upstream observations; the dashboard omits trends. Neither local usage nor 429 frequency is quota utilization.

## Scope

### In Scope
- Separate local usage history from upstream observation history.
- Persist supported real-response observations; otherwise show unavailable.
- Base warnings on reported limits when available; label 429 frequency separately.
- Reconcile stale quota-placeholder UI contract.

### Out of Scope
- Promising quota coverage for every provider, account, plan, metric, or window.
- Estimating upstream quota utilization from local usage, cost, configured thresholds, or 429 ratio.
- Background polling or guaranteed cadence; prefer response-driven capture unless design evidence justifies otherwise.
- Replacing or removing the existing local `usage_history` records.

## Capabilities

### New Capabilities
- `provider-quota`: Represent, persist, retrieve, and present upstream quota observations separately from local usage, including explicit unavailable states.

### Modified Capabilities
- `providers-ui`: Replace the stale mocked-placeholder contract with separate local-usage and upstream-quota presentation, including honest warning and unavailable semantics.

## Approach

Extend the endpoint and dashboard with distinct data types. Design verifies signals against current official provider docs; persist source/time separately. Do not synthesize values. Resolve aggregation, freshness/retention, and stream capture boundaries.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs`, `provider_dto.rs` | Modified | Separate local/upstream data |
| Provider adapters (`crates/infrastructure/providers-*`) | Modified | Capture verified signals |
| SQLite migration and quota persistence | New | Store upstream history separately |
| Dashboard quota view/composable/API types | Modified | Trends, unavailable state, separate 429 signal |
| `.agents/sdd/specs/providers-ui/spec.md` | Modified | Supersede stale placeholder requirement |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Signals vary/are absent | High | Verify docs; expose unavailable |
| Capture misses response metadata | Medium | Verify adapter lifecycle and test |
| Aggregation hides per-connection limits | Medium | Define scope and freshness |

## Rollback Plan

Revert adapter, endpoint, and UI changes. Keep `usage_history` untouched; handle observation storage with a safe forward migration.

## Success Criteria

- [ ] Local and upstream histories remain distinct.
- [ ] Persist only supported actual response data; otherwise report unavailable.
- [ ] Warnings use reported limits; 429 frequency is separate and never called utilization.
- [ ] Tests cover supported observations, absent data, and distinct local/upstream histories.

## Assumptions and Questions for Spec/Design

- Confirm response capture versus quota API/polling, freshness/retention, aggregation, and TOML providers.
- Confirm supported signals; coverage is not universal by assumption.
