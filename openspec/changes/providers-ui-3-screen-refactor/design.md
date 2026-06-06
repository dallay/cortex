# Design: Providers UI 3-Screen Refactor

> **Change:** `providers-ui-3-screen-refactor`
> **Mode:** openspec (file-based persistence)

## 1. Architecture Overview

```
+-----------------+      route param      +-------------------+
| ProvidersView   | ─── /providers ──→   | Catalog (cards    |
| (Catalog)       |                       | grouped by       |
|                 |                       | category)         |
+-----------------+                       +-------------------+
        │                                          │ click card
        │ /providers/quota                         ▼
        ▼                              +-------------------------+
+--------------------+                 | ProviderDetailsView     |
| ProvidersQuotaView |                 | /providers/:kind        |
| (placeholder)      |                 | - list of connections   |
+--------------------+                 | - Test All / Add CTAs   |
                                       +-------------------------+
                                                  │
                                                  ▼ open modal
                                       +-------------------------+
                                       | AddProviderDialog       |
                                       | (controlled v-model)    |
                                       +-------------------------+
```

| Layer | Owns | Reads from |
|---|---|---|
| Composables | providers list, catalog derivation, mutations | `lib/api.ts` |
| Views | page-level state (active filter, search query) | composables |
| Components (cards/lists) | UI state (open dialog, hover) | props + emits |
| Config | static kind metadata, navigation | constants |

## 2. File Structure

| File | Action | Purpose |
|---|---|---|
| `src/views/ProvidersView.vue` | MODIFY | Becomes catalog (table body removed) |
| `src/views/ProviderDetailsView.vue` | NEW | `/providers/:providerKind` |
| `src/views/ProvidersQuotaView.vue` | NEW | `/providers/quota` placeholder |
| `src/components/AddProviderDialog.vue` | MODIFY | Add `providerKind`, `mode`, controlled `v-model:open` |
| `src/components/EmptyState.vue` | MODIFY | Internally wrap shadcn-vue `Empty`; public API unchanged |
| `src/components/ProviderCatalogCard.vue` | NEW | Clickable card → details |
| `src/components/ProviderCategorySection.vue` | NEW | Category header + grid of cards |
| `src/components/ProviderCatalogFilter.vue` | NEW | Category chips + search input |
| `src/components/ConnectionListItem.vue` | NEW | Row in details view (test/edit/delete actions) |
| `src/composables/useProviderCatalog.ts` | NEW | Derived state: per-kind entries with counts |
| `src/composables/useProviders.ts` | MODIFY | Add `fetchById(id)` |
| `src/config/providerCatalog.ts` | NEW | Static `PROVIDER_KINDS` metadata |
| `src/config/navigation.ts` | MODIFY | Replace quotes with quota sub-item |
| `src/router/index.ts` | MODIFY | 2 new routes, validate `:providerKind` |
| `src/composables/useNavigation.ts` | MODIFY | 5 new lucide icons |
| `src/lib/api.ts` | MODIFY | Tighten `providerKind`/`authType` to unions |
| `src/views/sidebar/index.vue` | MODIFY | 3-level breadcrumb when `route.matched.length >= 3` |
| `src/locales/en.json` + `es.json` | MODIFY | New keys under `providers.{catalog,details,form,kind,quota}` |
| `src/components/AddProviderDialog.spec.ts` | MODIFY | Pass `providerKind` prop; add OAuth case |
| `e2e/providers.spec.ts` | MODIFY | Catalog → card → details → test → delete |

## 3. Component Composition

| Component | Props (new in **bold**) | Emits | Wraps |
|---|---|---|---|
| `ProviderCatalogCard` | `**providerKind**`, `**displayName**`, `**connectionCount**`, `**category**`, `**hasActiveConnections**` | `**test**`, `**toggle**` | `Card`, `Badge`, `Button`, `Switch` |
| `ProviderCatalogFilter` | `**categories**`, `**activeCategory**`, `**searchQuery**` | `**update:activeCategory**`, `**update:searchQuery**` | `Toggle` chips, `InputGroup` |
| `ProviderCategorySection` | `**category**`, `**entries**` | — | `<h2>` + grid of cards |
| `ConnectionListItem` | `**connection**` | `**test**`, `**edit**`, `**delete**`, `**toggle-proxy**` | `Card`, `StatusBadge`, `Button` |
| `AddProviderDialog` | `**providerKind?`**, `**mode = 'create'`**, `**connectionId?`** | `**update:open**` | shadcn-vue `Dialog`, `FieldGroup`/`Field`, `ToggleGroup` (auth type) |
| `EmptyState` | same (`title`, `description`, `icon`) | — | shadcn-vue `Empty` + `EmptyHeader` + `EmptyTitle` + `EmptyDescription` + `EmptyContent` (default slot) |

`ProvidersView` (catalog) groups `useProviderCatalog().entries` by `category` and renders a `ProviderCategorySection` per group. `ProviderDetailsView` subscribes to `useProviders().providers` and filters by `route.params.providerKind`. `ProvidersQuotaView` is pure presentational (mock data + `Alert` banner).

## 4. Architecture Decisions

| # | Decision | Alternatives | Rationale |
|---|---|---|---|
| D1 | Keep `useProviders`, add thin `useProviderCatalog` derived composable | Split into `useProviderCatalog` + `useProviderConnections` (full split) | Catalog only needs counts + displayName; full split adds 2 files for 5 kinds. Single `useProviders` source of truth, derived `ComputedRef` is enough. |
| D2 | No Pinia store | Add Pinia store for providers | Providers data is request-scoped, < 50 records, mutated via 7 composable methods. Pinia is overkill. |
| D3 | Static `PROVIDER_KINDS` in TS | Backend-served `GET /api/providers/kinds` | 5 kinds are stable; revisit at 6th kind. Avoid premature API surface. |
| D4 | Wrap shadcn-vue `Empty` inside existing `EmptyState` (public API unchanged) | Replace `EmptyState` with direct shadcn usage | Back-compat with all current callsites (ProvidersView, HomeView, etc.) without test changes. |
| D5 | Catalog is route `/providers` (not `/providers/catalog`) | Add `/providers/catalog` | Keeps current URL stable for the primary "Providers" entry point; no redirect required. |
| D6 | Modal controlled by parent via `v-model:open` | Self-managed `open` ref (today) | Required so both catalog and details can own the modal's lifecycle and reset cleanly. |
| D7 | `providerKind` route param validated in `beforeEnter` | Validate in view's `onMounted` | Catch invalid URLs at navigation time, redirect to catalog (better UX, no flash of error). |
| D8 | OAuth form is rendered disabled with notice | Remove OAuth option entirely | Keeps form shape forward-compatible when OAuth flow lands (follow-up issue). |

## 5. Routing Changes

```ts
// router/index.ts — add to children of '/' route
{
  path: 'providers/:providerKind',
  name: 'Provider Details',
  component: () => import('@/views/ProviderDetailsView.vue'),
  meta: { requiresAuth: true },
  beforeEnter: (to) => {
    const valid = ['openai', 'anthropic', 'ollama', 'gemini', 'groq']
    if (!valid.includes(to.params.providerKind as string)) {
      return { name: 'Providers' }
    }
  },
},
{
  path: 'providers/quota',
  name: 'Providers Quota',
  component: () => import('@/views/ProvidersQuotaView.vue'),
  meta: { requiresAuth: true },
},
// REMOVE: 'providers/quotes' route
```

## 6. Navigation Config

```ts
// config/navigation.ts — providers sub-items
items: [
  { titleKey: 'nav.providersCatalog', url: '/providers' },      // RENAMED
  { titleKey: 'nav.providersQuota',   url: '/providers/quota' }, // RENAMED + new path
]
```

## 7. i18n Schema (en.json — mirror to es.json)

```json
{
  "providers": {
    "catalog": {
      "title": "Providers", "description": "...",
      "searchPlaceholder": "Search providers",
      "totalCount": "Total: {configured}/{total}",
      "filterAll": "All", "filterApiKey": "API Key",
      "filterOauth": "OAuth", "filterLocal": "Local",
      "noResults": "No providers match your search"
    },
    "details": {
      "backToCatalog": "Back to Providers",
      "connectionsCount": "{count} Connections",
      "testAll": "Test All", "add": "Add Connection",
      "emptyTitle": "No {provider} connections yet",
      "emptyDescription": "Add your first connection to get started"
    },
    "form": {
      "name": "Name", "apiKey": "API Key",
      "authType": "Authentication",
      "authTypeApiKey": "API Key", "authTypeOauth": "OAuth",
      "oauthNotImplemented": "OAuth flow is not yet implemented",
      "validationModel": "Validation Model",
      "priority": "Priority", "routingWeight": "Routing Weight",
      "advancedSettings": "Advanced Settings",
      "test": "Test", "save": "Save", "cancel": "Cancel"
    },
    "kind": {
      "openai": "OpenAI", "anthropic": "Anthropic",
      "ollama": "Ollama Cloud", "gemini": "Gemini", "groq": "Groq"
    },
    "quota": {
      "title": "Provider Quotas",
      "description": "Track token consumption and limits per provider",
      "placeholderNotice": "Real per-provider quota integration is coming. Each provider exposes quota differently in their API.",
      "followUpIssue": "Follow-up tracking issue"
    }
  }
}
```

Remove `nav.providersQuotes` + `breadcrumb.providersQuotes` from both files.

## 8. AddProviderDialog Refactor

| Behavior | Today | Target |
|---|---|---|
| `providerKind` | hardcoded `'ollama'` | prop, optional (catalog case shows selector) |
| `mode` | implicit create | prop `'create' \| 'edit'`, default `'create'` |
| Auth type | implicit `apiKey` | `ToggleGroup` (apikey / oauth); OAuth form is disabled with notice |
| Edit prefill | n/a | name/model/priority from `fetchById(id)`; apiKey left empty |
| Open trigger | self-managed `ref(false)` | `v-model:open` from parent; close → `resetForm()` |
| Validation | name + apiKey required | + priority 1-100; Save enabled only when `testResult.ok === true` |
| Test flow | `testCredentials(payload)` | same; result drives Save button |

Use `FieldGroup` + `Field` per shadcn-vue form conventions. The `useProviderForm` composable is **deferred** — keep validation inline in the dialog for v1.

## 9. EmptyState Wrapper

```vue
<template>
  <Empty>
    <EmptyHeader>
      <EmptyMedia v-if="icon" variant="icon"><component :is="icon" /></EmptyMedia>
      <EmptyTitle v-if="title">{{ title }}</EmptyTitle>
      <EmptyDescription v-if="description">{{ description }}</EmptyDescription>
    </EmptyHeader>
    <EmptyContent v-if="$slots.default"><slot /></EmptyContent>
  </Empty>
</template>
```

Public API unchanged: `{ title, description, icon }` props + default slot. No callsite changes required.

## 10. Risks & Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| i18n drift between en/es (pre-existing 13-line gap) | High | Mirror every new key; `MessageSchema = typeof en` enforces it. Do NOT fix pre-existing drift. |
| 3-level breadcrumb regresses 2-level pages | Med | Guard with `route.matched.length >= 3`; smoke-test Home, API Keys, Combos, Settings. |
| `AddProviderDialog.spec.ts` rewrite breaks unrelated tests | Med | Keep `shallowMount` pattern; default `providerKind: 'ollama'` in spec helper. |
| shadcn-vue `Empty` composition differs from current | Low | Test in browser during `sdd-verify`; public API kept identical. |
| TypeScript union tightening surfaces loose call sites | Med | Add type guard helper; do not change wire protocol. |
| Quota placeholder mistaken for shipped feature | Low | `Alert` banner + `EmptyState` ("Coming soon") explicit. |
| Provider CRUD limitation (TOML serves traffic, not SQLite) | Med | Tooltip on catalog: "Saved providers require a server restart to serve traffic." |
| 5 new lucide icons add bundle weight | Low | ~5 KB tree-shaken; negligible. |

## 11. Verification Plan

```bash
# Backend untouched — only smoke check
cargo check --workspace --all-targets

# Frontend type & unit checks
cd apps/rook/dashboard && pnpm exec vue-tsc --noEmit
cd apps/rook/dashboard && pnpm exec vitest run
```

**Manual browser flow:**
1. `/providers` → catalog grid grouped by category, all 5 kinds visible
2. Click `API Key` chip → only API Key kinds; click again → all
3. Type `ollama` in search → only Ollama card
4. Click Ollama card → navigates to `/providers/ollama`
5. Details view header shows `Ollama Cloud` + connection count + `Test All` + `Add` buttons
6. Click `Add` → modal opens with `providerKind = 'ollama'` pre-filled, auth type `apikey` selected
7. Fill name + apiKey, click `Test` → see result; `Save` enables only on `ok`
8. Click `Save` → modal closes, list refreshes
9. Click breadcrumb `Providers` → returns to catalog
10. Sidebar → `Provider Quota` → `/providers/quota` → see banner + mock data
11. DevTools console → zero errors, zero Vue warnings
12. `pnpm exec vitest run` → all specs pass (including updated `AddProviderDialog.spec.ts`)

**Verification gates per project AGENTS.md:**
- `just fmt-check` clean
- `just clippy` clean (no backend changes)
- `just test-unit` green
- i18n parity: `en.json` line count == `es.json` line count for the `providers` subtree
