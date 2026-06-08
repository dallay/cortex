# Spec Delta: providers-ui

> **Change:** 2026-06-07-provider-detail-polish
> **Capability:** providers-ui
> **Status:** draft

## ADDED Requirements

### Requirement: Provider Title as External Link

The system SHALL render the provider name in the `ProviderDetailsView` page header as an external link that opens the provider's official site in a new tab. The URL MUST be resolved from the `brandUrl` field of the catalog entry for the current kind (single source of truth). The anchor MUST declare `target="_blank"` and `rel="noopener noreferrer"`, MUST carry `aria-label="<ProviderName> — opens in new tab"`, and MUST NOT set `draggable="true"`. The link MUST render even when the kind has zero configured connections and MUST show a visual external-link affordance (an `ExternalLink` Lucide icon adjacent to the name, or an underline-on-hover treatment).

#### Scenario: New-tab external link on empty details

- **WHEN** the user navigates to `/providers/ollama-cloud` with 0 configured connections
- **THEN** the header renders `Ollama Cloud` as `<a href="https://ollama.com/cloud" target="_blank" rel="noopener noreferrer" aria-label="Ollama Cloud — opens in new tab">`
- **AND** the link is shown without `draggable="true"`

#### Scenario: Title link is keyboard-reachable and activates on Enter

- **WHEN** the user keyboard-tabs to the title link
- **THEN** the anchor receives a visible focus ring
- **AND** pressing Enter opens `brandUrl` in a new browser tab

#### Scenario: Screen reader announces target and new-tab behavior

- **WHEN** a screen reader (e.g. VoiceOver, NVDA) focuses the title link
- **THEN** it reads `Ollama Cloud — opens in new tab`
- **AND** the link is not announced as draggable

### Requirement: Branded Provider Icons

The system SHALL resolve every `ProviderKind` in the catalog to a branded vendor mark and render it in both the catalog grid (`ProviderCatalogCard`) and the detail page header (`ProviderDetailsView`). Catalog-card icons MUST be marked `aria-hidden="true"` (the visible card text already names the kind) and MUST be loaded with `loading="lazy"` (cards may sit below the fold). Detail-header icons MUST be loaded with `loading="eager"` (above-the-fold LCP candidate) and MUST carry `role="img"` with `aria-label="<ProviderName>"` (used as the standalone brand mark adjacent to the title link). Every icon MUST declare explicit `width` and `height` attributes to prevent CLS. If the asset for a kind is missing, the system MUST render a neutral `Server` Lucide fallback icon so the catalog is never broken, and MUST emit `console.warn` in development for the missing kind.

#### Scenario: Catalog cards show the vendor mark for each kind

- **WHEN** the catalog at `/providers` renders
- **THEN** every card shows the original vendor mark for its `ProviderKind` (not a generic Lucide icon)
- **AND** each catalog-card icon declares `width` and `height` and uses `loading="lazy"`

#### Scenario: Detail header shows the eager-loaded vendor mark

- **WHEN** the user navigates to `/providers/anthropic`
- **THEN** the header shows the Anthropic vendor mark adjacent to the title link
- **AND** the detail-header icon does NOT use `loading="lazy"` (eager load, LCP candidate)
- **AND** the detail-header icon carries `role="img"` and `aria-label="Anthropic"`

#### Scenario: Missing icon asset falls back gracefully

- **WHEN** a `ProviderKind` resolves to a missing icon asset
- **THEN** the catalog card and detail header render a neutral `Server` Lucide icon in its place
- **AND** `console.warn` reports the missing kind in development
- **AND** the catalog remains navigable (no broken-image icon, no thrown error)

## MODIFIED Requirements

### Requirement: Provider Details

The system SHALL display a details view at `/providers/:providerKind` listing all connections for the specified kind. The header SHALL show the provider's branded icon, the provider name (rendered as an external link to the official site — see "Provider Title as External Link"), the total connection count, and bulk action buttons (`Test All`, `Add`). The route SHALL validate `:providerKind` against the `ProviderKind` union.

(Previously: header showed the provider name as plain text with no branded icon and no external link.)

#### Scenario: Details with connections

- **WHEN** the user navigates to `/providers/ollama` with 3 Ollama connections
- **THEN** the header shows the branded Ollama icon, `Ollama Cloud` rendered as an external link, and `3 Connections`
- **AND** the page lists all 3 connections with name, status, model, priority, and proxy state

#### Scenario: Empty details state

- **WHEN** the user navigates to `/providers/openai` with 0 OpenAI connections
- **THEN** the header shows the branded OpenAI icon, `OpenAI` rendered as an external link, and `0 Connections`
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

### Requirement: Providers Catalog

The system SHALL display a catalog view at `/providers` listing every supported `ProviderKind` as a card grouped by category (`API Key`, `OAuth`, `Local`). Each card SHALL show the display name, configured-connection count, a kind-level enable toggle, a test-all action, and the kind's branded vendor mark (see "Branded Provider Icons"). The view SHALL support client-side category filtering and name search.

(Previously: cards were silent about iconography; implementation used a generic Lucide `ICONS` map.)

#### Scenario: Empty catalog

- **WHEN** the user navigates to `/providers` with no connections configured
- **THEN** the catalog shows every supported kind (`openai`, `anthropic`, `ollama`, `ollama-cloud`, `gemini`, `groq`) with `0 Connections`
- **AND** each card shows the branded vendor mark and an empty-state indicator for connections

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
- **AND** the clicked card showed the branded icon for the kind (not a generic Lucide icon)

## REMOVED Requirements

None.
