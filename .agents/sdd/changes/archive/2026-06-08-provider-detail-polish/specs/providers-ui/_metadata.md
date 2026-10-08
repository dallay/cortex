---
name: providers-ui
displayName: Providers UI
version: 1.1.0
status: draft
kind: frontend
description: >
  Frontend UX for the Rook dashboard's AI provider management flows.
  Captures the 3-screen navigation model (Catalog → Details → Connection Modal),
  the quota placeholder page, and the EmptyState wrapper used across the dashboard.
  v1.1.0 adds branded vendor icons across the catalog and detail surface, and
  makes the provider name in the detail header an external link to the
  provider's official key-issuance page.
owners:
  - team/frontend
created: 2026-06-06
change: 2026-06-07-provider-detail-polish
relatedCapabilities:
  - provider-connections          # domain model & wire protocol
  - provider-connections-transport # HTTP DTOs and routing
nonGoals:
  - Backend changes of any kind
  - Bulk actions on the catalog (multi-select enable/disable/delete)
  - Distribute Proxies (auto-rebalance priority/weight)
  - Static catalog of 228 providers (OmniRoute parity)
  - OAuth authorization redirect/initiation (form shape only)
  - Per-provider quota implementation (placeholder only)
  - Real-time connection status push (WebSocket / SSE)
  - Mobile-optimized layout (desktop-first)
  - SVG theming via `currentColor` (deferred to a future iteration)
followUp:
  - "Real per-provider quota integration (mocked data today)"
  - "OAuth authorization flow for OAuth-supporting kinds"
  - "Bulk actions on the catalog"
  - "Distribute Proxies"
  - "Author SVG variants of the 4 newly-authored icons that match the OmniRoute visual language"
  - "Themable branded icons (dark/light mode via `currentColor`)"
---
