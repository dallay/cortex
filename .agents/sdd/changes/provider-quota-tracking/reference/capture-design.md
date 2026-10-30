# Design: Per-Provider Quota Integration

## Technical Approach

Keep local `usage_history` intact; add separate event-driven persistence for verified upstream response observations. Capture supported headers after `.send()` (including errors, before streaming body consumption); recording is best-effort and does not alter provider responses. Preserve `GET /api/providers/quota`, returning local usage/trend, 429 counts, and upstream observations as distinct data.

## Architecture Decisions

| Option                             | Tradeoff                                            | Decision                                     |
|------------------------------------|-----------------------------------------------------|----------------------------------------------|
| Infer quota from local usage/429s  | Conflates client activity with upstream limit state | Reject; only report observed upstream values |
| Poll account APIs                  | Endpoint credentials/scopes/cadence vary            | Out of scope; response-driven capture only   |
| Extend local history               | Risks mixing semantics                              | Separate additive SQLite observation table   |
| Threshold provider-wide aggregates | Hides differing limits/windows                      | Classify per connection, metric, and window  |

## Data Flow and Eligibility

```text
response headers → provider parser → recorder port → SQLite observations
  → quota API → upstream observations | local usage/trend | 429s
```

Store provider kind/runtime identity, optional connection ID, metric, window/scope, limit, remaining, reset, source, and original `observed_at`. Return/display observation age; never refresh its timestamp on read or call the latest stored sample fresh. A documented reset timestamp determines expiry; stale at/after reset. If documentation gives only a reset duration/window, expire at `observed_at + duration`. If reset is absent/malformed and no documented duration exists, retain otherwise valid data as historical with age and `expiry unknown`, but suppress current warning classification. Malformed/unusable limit data is unavailable for that metric. No sample is unavailable, not zero.

The existing connection config has per-connection `quotaWindowThresholds.warning/error`, finite fractions with error > warning (current UI defaults 0.8/0.95). Compare these only to `used_fraction = (limit - remaining) / limit`, when limit/remaining match in metric, window, and scope and the observation is eligible. Error threshold is critical; warning threshold is warning; below both is ok. Non-positive limit, remaining outside `[0, limit]`, malformed values, unknown expiry, or no connection thresholds means warning status unavailable. Never apply thresholds to 429 ratios or average incompatible observations/configs. Unmanaged observations remain visible without threshold classification.

The API already returns a local daily `trend`, but the quota view does not render it. Render it explicitly as local usage; render upstream observations as a separate, timestamped history without interpolation or implied polling. Reuse `config.usage.retention_days` and its startup/periodic sweep (default 90 days), deleting only quota rows.

## Vendor Evidence

- **OpenAI:** [response headers](https://platform.openai.com/docs/api-reference/response-headers) document request/token limit, remaining, and reset headers; these are rate-limit signals, not account-wide billing quota.
- **Anthropic:** [Messages API response headers](https://docs.anthropic.com/en/api/rate-limits) expose limit/remaining/reset; token values represent the most restrictive applicable limit. The separate [Rate Limits API](https://platform.claude.com/docs/en/manage-claude/rate-limits-api) exposes configured organization/workspace limits only, not remaining/headroom, and requires Admin API credentials. It is out of scope unless separately authorized.
- **Groq:** [rate limits](https://console.groq.com/docs/rate-limits) documents request RPD and token TPM response headers.
- **Gemini/Ollama:** reviewed sources do not establish equivalent response-header remaining/headroom support. Do not claim universal coverage or infer absence of all quota APIs.

## File Changes

| File                                                                                                                | Action | Description                                                       |
|---------------------------------------------------------------------------------------------------------------------|--------|-------------------------------------------------------------------|
| `crates/domain/rook-core/src/model.rs`, `ports.rs`                                                                  | Modify | Observation model and recorder port                               |
| `crates/infrastructure/db-migration/src/migrations/V6__provider_quota_observations.sql`                             | Create | Separate table/indexes                                            |
| `crates/infrastructure/audit-sqlite/src/lib.rs`                                                                     | Modify | Persistence and retention                                         |
| `crates/infrastructure/providers-{openai,anthropic,groq}/src/*`                                                     | Modify | Parse documented response headers                                 |
| `apps/rook/src/di.rs`, `usage_retention.rs`                                                                         | Modify | Wire recorder and retention                                       |
| `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs`, `provider_dto.rs`                            | Modify | Distinct observations/local data; remove 429-derived warning      |
| `apps/rook/dashboard/src/{lib/api.ts,composables/useProvidersQuota.ts,views/ProvidersQuotaView.vue,locales/*.json}` | Modify | Age, per-connection warning, and separate history/trend rendering |
| `.agents/sdd/specs/providers-ui/spec.md`                                                                            | Modify | Replace placeholder contract                                      |

## Testing Strategy

| Layer          | Focus                                                                                            |
|----------------|--------------------------------------------------------------------------------------------------|
| Unit           | Header parsing, reset validity/expiry, compatible fraction thresholds, no inference              |
| Integration    | Migration, retention, connection scoping, handler mapping                                        |
| Adapter/stream | Success/error/stream capture without response behavior changes                                   |
| UI             | Local daily trend vs upstream history; reset-eligible, stale, unknown-expiry, unavailable states |

## Migration / Rollout

Forward-only migration, no backfill; local history remains unchanged. No polling or feature flag.

## Open Questions

- Custom OpenAI-compatible base URL trust is unverified; safest default: capture only known provider kinds on canonical endpoints.
- No product-critical blocker: unknown expiry suppresses current warning; unassociated runtime observations have no configured threshold classification.
