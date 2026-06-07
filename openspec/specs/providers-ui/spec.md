# Providers UI Specification

> **Scope:** Frontend UX only. The domain model and wire protocol live in `provider-connections`; this spec covers the user-facing flows that consume those contracts.

## Purpose

Defines the Rook dashboard flows for discovering, configuring, and managing AI provider connections. Captures the 3-screen navigation model (Catalog → Details → Connection Modal), the quota placeholder, and the `EmptyState` wrapper used across the dashboard.

---

## Requirements

### Requirement: Providers Catalog

The system SHALL display a catalog view at `/providers` listing every supported `ProviderKind` as a card grouped by category (`API Key`, `OAuth`, `Local`). Each card SHALL show the display name, configured-connection count, a kind-level enable toggle, and a test-all action. The view SHALL support client-side category filtering and name search.

#### Scenario: Empty catalog

- **WHEN** the user navigates to `/providers` with no connections configured
- **THEN** the catalog shows every supported kind (`openai`, `anthropic`, `ollama`, `gemini`, `groq`) with `0 Connections`
- **AND** each card shows an empty-state indicator for connections

#### Scenario: Catalog with connections

- **WHEN** the user navigates to `/providers` with at least one connection
- **THEN** each card shows its configured count (e.g., `Ollama Cloud: 3 Connections`)
- **AND** the card is visually highlighted to indicate configured connections

#### Scenario: Filter by category

- **WHEN** the user clicks a category chip (e.g., `API Key`)
- **THEN** only kinds in that category are visible
- **AND** the chip shows an active state
- **AND** clicking the same chip again removes the filter

#### Scenario: Search the catalog

- **WHEN** the user types in the search input
- **THEN** only kinds whose name contains the search term are visible
- **AND** the match is case-insensitive

#### Scenario: Navigate to details

- **WHEN** the user clicks a kind card
- **THEN** the user is navigated to `/providers/:providerKind` (e.g., `/providers/ollama`)

---

### Requirement: Provider Details

The system SHALL display a details view at `/providers/:providerKind` listing all connections for the specified kind. The header SHALL show the provider name, total connection count, and bulk action buttons (`Test All`, `Add`). The route SHALL validate `:providerKind` against the `ProviderKind` union.

#### Scenario: Details with connections

- **WHEN** the user navigates to `/providers/ollama` with 3 Ollama connections
- **THEN** the header shows `Ollama Cloud` and `3 Connections`
- **AND** the page lists all 3 connections with name, status, model, priority, and proxy state

#### Scenario: Empty details state

- **WHEN** the user navigates to `/providers/openai` with 0 OpenAI connections
- **THEN** the header shows `OpenAI` and `0 Connections`
- **AND** the page shows an empty state with a prominent `Add your first OpenAI connection` CTA

#### Scenario: Test all connections

- **WHEN** the user clicks `Test All`
- **THEN** all connections for the kind are tested sequentially
- **AND** each connection's status indicator updates as its test completes

#### Scenario: Add from details

- **WHEN** the user clicks `Add`
- **THEN** the connection modal opens in create mode with `providerKind` pre-filled to the current kind

#### Scenario: Navigate back to catalog

- **WHEN** the user clicks the back link or breadcrumb
- **THEN** the user is navigated to `/providers`

---

### Requirement: Connection Modal

The system SHALL provide a modal dialog for creating and editing provider connections. The modal SHALL accept a `providerKind` prop and a `mode` prop (`'create' | 'edit'`) and SHALL be controlled by its parent via `v-model:open`. The modal MUST require a successful credential test before `Save` is enabled.

#### Scenario: Open in create mode

- **WHEN** the user clicks `Add` from the catalog or details view
- **THEN** the modal opens with `providerKind` pre-filled (or selectable if opened from catalog)
- **AND** the form shows fields for the selected `authType` (API Key for `apikey`, OAuth fields for `oauth`)

#### Scenario: Test credentials

- **WHEN** the user fills the form and clicks `Test`
- **THEN** the modal calls `POST /api/providers/test-credentials`
- **AND** displays the result (ok/error, latency)
- **AND** `Save` is disabled until the result is `ok`

#### Scenario: Save a new connection

- **WHEN** the test result is `ok` and the user clicks `Save`
- **THEN** the modal calls `POST /api/providers`
- **AND** the modal closes
- **AND** the catalog/details view refreshes to show the new connection

#### Scenario: Edit existing connection

- **WHEN** the modal opens in `edit` mode with a connection id
- **THEN** the form is pre-populated from the existing record
- **AND** saving calls `PUT /api/providers/{id}` with `expectedUpdatedAt`
- **AND** the API key field is empty (stored secrets are not re-exposed)

#### Scenario: Cancel without saving

- **WHEN** the user clicks `Cancel` or presses `Escape`
- **THEN** the modal closes without saving
- **AND** no mutation API call is made

---

### Requirement: Providers Quota Placeholder

The system SHALL display a placeholder quota view at `/providers/quota` with mocked data and a banner explaining that real per-provider quota integration is tracked in a follow-up issue.

#### Scenario: Navigate to quota

- **WHEN** the user navigates to `/providers/quota`
- **THEN** the page shows mocked quota data (tokens consumed, limits, cost)
- **AND** a banner explains that per-provider quota integration is a follow-up
- **AND** the page references the follow-up tracking issue

---

### Requirement: EmptyState Component

The system SHALL provide a reusable `EmptyState` component that wraps the shadcn-vue `Empty` primitive while preserving the existing public API (`title`, `description`, `icon` props).

#### Scenario: Backward-compatible callsites

- **WHEN** a component uses `<EmptyState title="..." description="..." :icon="..." />`
- **THEN** it renders correctly via the shadcn-vue `Empty` composition underneath
- **AND** no existing callsite requires changes

#### Scenario: New shadcn-vue features

- **WHEN** a new callsite uses shadcn-vue `Empty` slots or sub-components (e.g., `EmptyHeader`, `EmptyContent`)
- **THEN** the wrapper supports them via slot passthrough or new prop names

---

### Requirement: Catalog Metadata Source

The catalog view SHALL derive its kind-level metadata (displayName, runtimeId, defaultBaseUrl, supportsOAuth, description, iconName) from a static `PROVIDER_KINDS` constant in `apps/rook/dashboard/src/config/providerCatalog.ts`. Configured-connection counts per kind SHALL be derived from the live list returned by `GET /api/providers`.

#### Scenario: Display kind metadata

- **WHEN** the catalog renders a kind card
- **THEN** the card shows the kind's display name and description from `PROVIDER_KINDS`
- **AND** the connection count is the length of the live connections filtered by `providerKind`

#### Scenario: Adding a new kind

- **WHEN** a developer adds an entry to `PROVIDER_KINDS`
- **THEN** the new kind appears in the catalog without template changes
- **AND** the route `/providers/:providerKind` resolves only for kinds in the backend `ProviderKind` enum

---

## Delta (2026-06-07) — Credential Validation Warning

> Relaxes the Save-button gating rule in the Connection Modal so a
> credentials-valid result with a non-blocking warning (e.g. quota exhausted)
> still allows Save. Adds a third visual state (yellow) to the test-result
> block.

### MODIFIED: Connection Modal — Save button gating (supersedes line 85)

The Save button MUST be enabled iff `valid === true` regardless of `status`
or `warning` content. The `ok` and `warning` statuses both allow Save. Only
`unhealthy` and `expired` statuses disable Save. `unknown` also allows Save
(no probe was possible but credentials were not rejected).

| Test result                          | Save button     |
|--------------------------------------|-----------------|
| `valid: true, status: "ok"`          | **enabled**     |
| `valid: true, status: "warning"`     | **enabled**     |
| `valid: true, status: "unknown"`     | **enabled**     |
| `valid: false, status: "unhealthy"`  | **disabled**    |
| `valid: false, status: "expired"`    | **disabled**    |
| (no test run yet)                    | **disabled**    |

### MODIFIED: Test credentials scenario (supersedes lines 93-99)

The test result block MUST display three visual states: **success** (green
`CheckCircle2` icon), **warning** (yellow `AlertTriangle` icon, `text-yellow-600`
Tailwind class), **failure** (red `AlertCircle` icon). The `warning` field,
when present, MUST be shown with the yellow styling.

#### Scenario: Test credentials

- **WHEN** the user fills the form and clicks `Test`
- **THEN** the modal calls `POST /api/providers/test-credentials`
- **AND** displays the result with one of the three visual states (success / warning / failure) and the latency
- **AND** `Save` is enabled iff the response has `valid: true`

### MODIFIED: Save a new connection (supersedes line 102)

#### Scenario: Save a new connection

- **WHEN** the test result has `valid: true` (status `ok`, `warning`, or `unknown`) and the user clicks `Save`
- **THEN** the modal calls `POST /api/providers`
- **AND** the modal closes
- **AND** the catalog/details view refreshes to show the new connection

### ADDED: User sees a 429 warning

- **GIVEN** the user is configuring Ollama Cloud with a valid key that has hit its weekly quota
- **WHEN** the user clicks "Test connection"
- **THEN** a yellow alert appears with text "Rate limited, but credentials are valid"
- **AND** the Save button is enabled
- **AND** clicking Save persists the connection
- **AND** a subsequent `POST /api/providers/{id}/test` returns the same warning (transient — quota may refresh)

### ADDED: User sees an auth error

- **GIVEN** the user is configuring Ollama Cloud with an invalid key
- **WHEN** the user clicks "Test connection"
- **THEN** a red alert appears with text "auth rejected: HTTP 401 — check that your API key is valid and has access to the model"
- **AND** the Save button is disabled
