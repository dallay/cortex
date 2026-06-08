# Delta for Providers UI

> **Change:** `2026-06-07-provider-detail-polish`  
> **Target spec:** `openspec/specs/providers-ui/spec.md`  
> **Type:** Delta (ADDED requirements only — no MODIFIED or REMOVED)

---

## ADDED Requirements

### Requirement: Provider Title as External Link

The provider details page (`/providers/:providerKind`) MUST render the provider display name as an external hyperlink to the provider's official key-management or sign-up page. The link MUST use `target="_blank"` with `rel="noopener noreferrer"` for security. The link MUST include an accessible label announcing that it opens in a new tab.

#### Scenario: Title links to provider site

- **GIVEN** user viewing `/providers/gemini`
- **WHEN** page renders
- **THEN** `<h1>` is wrapped in `<a href="https://aistudio.google.com/apikey">` with `target="_blank" rel="noopener noreferrer"` and `aria-label="Gemini — opens in new tab"`

#### Scenario: Click opens external tab

- **GIVEN** user viewing `/providers/ollama-cloud`
- **WHEN** user clicks title
- **THEN** new tab opens to `https://ollama.com/cloud`, current tab stays on `/providers/ollama-cloud`

#### Scenario: Keyboard accessible

- **GIVEN** user viewing `/providers/openai`
- **WHEN** user tabs to title and presses Enter
- **THEN** `https://platform.openai.com/api-keys` opens in new tab

#### Scenario: Screen reader announces external

- **GIVEN** screen reader user navigating `/providers/anthropic`
- **WHEN** title link is focused
- **THEN** announces "Anthropic — opens in new tab, link"

---

### Requirement: Branded Provider Icons

The catalog view (`/providers`) and the provider details page (`/providers/:providerKind`) MUST display the original vendor brand mark or logo for each provider kind. Catalog grid cards MUST lazy-load icons to improve initial page load performance. Detail page header icons MUST load eagerly (no lazy loading) as they are above the fold. Every icon MUST declare explicit `width` and `height` attributes to prevent Cumulative Layout Shift (CLS).

#### Scenario: Catalog shows branded icons

- **GIVEN** user navigating `/providers`
- **WHEN** page renders
- **THEN** each card shows provider's branded icon (OpenAI wordmark, Anthropic "A", Gemini spark), no Lucide fallback, all with `loading="lazy"`

#### Scenario: Detail header shows icon eagerly

- **GIVEN** user navigating `/providers/ollama-cloud`
- **WHEN** page renders
- **THEN** header shows Ollama Cloud icon above title with `loading="eager"`

#### Scenario: Icon prevents CLS

- **GIVEN** catalog or detail page rendering
- **WHEN** icon loads
- **THEN** no layout shift occurs; container reserves space via `width` and `height` attributes

#### Scenario: Missing icon fallback

- **GIVEN** provider kind has no asset in `/public/providers/`
- **WHEN** page attempts to render icon
- **THEN** neutral Lucide `Server` icon shown, console warning logged in dev mode

#### Scenario: Catalog icon accessibility

- **GIVEN** catalog card rendering
- **THEN** icon has `aria-hidden="true"` (visible text names provider); screen readers skip icon

#### Scenario: Detail icon accessibility

- **GIVEN** detail page rendering `/providers/groq`
- **THEN** icon has `role="img"` and `aria-label="Groq"`; screen readers announce "Groq, image"

---

## Coverage Summary

| Requirement                       | Happy Path | Edge Cases | Error States |
|-----------------------------------|------------|------------|--------------|
| Provider Title as External Link   | ✅ 4/4     | ✅ 1/1     | N/A          |
| Branded Provider Icons            | ✅ 4/4     | ✅ 2/2     | ✅ 1/1       |

**Total scenarios:** 11  
**ADDED requirements:** 2  
**MODIFIED requirements:** 0  
**REMOVED requirements:** 0

---

## Implementation Notes (informative, not normative)

- The provider's external URL is stored as `brandUrl` in `CatalogEntry` (`config/providerCatalog.ts`).
- Icon resolution is handled by a new `ProviderIcon.vue` component that accepts a `kind` prop and a `loading` prop.
- The 6 branded assets live in `apps/rook/dashboard/public/providers/` as `.svg` or `.png` files (~7 KB total).
- The catalog-to-detail navigation flow (clicking a card) is unchanged — only the visual presentation of the card icon and the detail header are modified.
