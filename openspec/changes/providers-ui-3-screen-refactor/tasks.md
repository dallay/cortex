# Tasks: Providers UI 3-Screen Refactor

> **References**
> - Proposal: `openspec/changes/providers-ui-3-screen-refactor/proposal.md`
> - Spec: `openspec/changes/providers-ui-3-screen-refactor/specs/providers-ui/spec.md`
> - Design: `openspec/changes/providers-ui-3-screen-refactor/design.md`
> - Explore: `tmp/plans/2026-06-06-providers-ui-refactor-explore.md`

**Total tasks: 49** · **Peak parallelism: 3 streams** (phases 4.x + 5.x + 6.x after 1–3 done; phase 11.x independent).

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 1100–1400 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes (advisory) |
| Suggested split | PR A (Foundation) → PR B (Views & Routes) → PR C (Polish & Verify) |
| Delivery strategy | single-pr |
| Chain strategy | pending |

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Catalog data, composable, i18n keys, `EmptyState` wrap, dialog refactor + spec | PR A | Base = main |
| 2 | Catalog/Details/Quota views, routes, nav, breadcrumb | PR B | Stacked on PR A |
| 3 | Manual UI verify, follow-up issue, final CI + push | PR C | Stacked on PR B |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

## Phase 1: Foundational Data

- [x] **1.0** Milestone — static catalog, derived composable, i18n keys ready.
- [x] **1.1** `src/config/providerCatalog.ts` — export typed `PROVIDER_KINDS` (5 entries: `kind`, `displayName`, `category`, `defaultBaseUrl`, `logoIconName`).
  - Done when: `pnpm exec vue-tsc --noEmit` clean and 5 entries present.
  - Depends on: none · Parallel: 1.2, 1.3 · Touches: `apps/rook/dashboard/src/config/providerCatalog.ts`
- [x] **1.2** `src/composables/useProviderCatalog.ts` — `catalog: ComputedRef<CatalogEntry[]>` with `connectionCount`, `hasActiveConnections`.
  - Done when: composable derived from `useProviders().providers` groups by kind; smoke call shows counts.
  - Depends on: 1.1 · Parallel: 1.3 · Touches: `apps/rook/dashboard/src/composables/useProviderCatalog.ts`
- [x] **1.3** Add i18n keys `providers.catalog.*`, `providers.details.*`, `providers.quota.*`, `providers.form.*`, `providers.kind.*`, `nav.providersCatalog`, `nav.providersQuota` in BOTH locales.
  - Done when: `MessageSchema = typeof en` parity holds; `en.json` + `es.json` share every new key.
  - Depends on: none · Parallel: 1.2 · Touches: `apps/rook/dashboard/src/locales/en.json`, `apps/rook/dashboard/src/locales/es.json`

## Phase 2: Component Wrappers

- [ ] **2.0** Milestone — `EmptyState` wraps shadcn-vue `Empty`; no callsite regression.
- [ ] **2.1** Refactor `EmptyState.vue` — internally wrap shadcn-vue `Empty`; keep `{title, description, icon}` props + add default slot for actions.
  - Done when: existing 5 callsites (HomeView, ProvidersView, ApiKeysView, CombosView, EndpointsView) render unchanged.
  - Depends on: 1.0 · Parallel: 3.1 · Touches: `apps/rook/dashboard/src/components/EmptyState.vue`
- [ ] **2.2** Visual smoke check of `EmptyState` callsites after 2.1.
  - Done when: dev server shows 5 pages with no console errors or missing styles.
  - Depends on: 2.1 · Parallel: 3.2 · Touches: (visual, no file change)

## Phase 3: Connection Modal Refactor

- [ ] **3.0** Milestone — `AddProviderDialog` supports new props, edit mode, auth toggle, test-before-save.
- [ ] **3.1** Add props to `AddProviderDialog.vue`: `providerKind?: ProviderKind`, `mode: 'create'|'edit'`, `connectionId?: string`, `v-model:open`.
  - Done when: TS accepts all 4 props; `mode='create'` default still works.
  - Depends on: 1.0 · Parallel: 2.1 · Touches: `apps/rook/dashboard/src/components/AddProviderDialog.vue`
- [ ] **3.2** Add provider-kind selector (only when `providerKind` undefined) + auth-type `ToggleGroup` (apikey/oauth); OAuth form disabled with "not yet implemented" tooltip.
  - Done when: kind selector hidden in details flow; auth toggle swaps form; OAuth fields render disabled.
  - Depends on: 3.1 · Parallel: 3.3 · Touches: `apps/rook/dashboard/src/components/AddProviderDialog.vue`
- [ ] **3.3** Wire `testCredentials` flow; `Save` disabled until `testResult.ok === true`.
  - Done when: Test with valid key → Save enables; failed test → Save stays disabled with error shown.
  - Depends on: 3.1 · Parallel: 3.2 · Touches: `apps/rook/dashboard/src/components/AddProviderDialog.vue`
- [ ] **3.4** Update `AddProviderDialog.spec.ts` — new props, `mode='edit'`, OAuth case (keep `shallowMount` + `providerKind: 'ollama'` default).
  - Done when: `pnpm exec vitest run AddProviderDialog.spec.ts` passes.
  - Depends on: 3.3 · Parallel: none · Touches: `apps/rook/dashboard/src/components/AddProviderDialog.spec.ts`

## Phase 4: Catalog View

- [x] **4.0** Milestone — `/providers` renders grouped catalog with filter + search.
- [x] **4.1** `ProviderCatalogCard.vue` — Card with name, count badge, enable toggle, test button.
  - Done when: card renders from stubs and emits `test`/`toggle`.
  - Depends on: 1.0, 2.0, 3.0 · Parallel: 5.1, 5.2, 6.1 · Touches: `apps/rook/dashboard/src/components/ProviderCatalogCard.vue`
- [x] **4.2** `ProviderCatalogFilter.vue` — category chips + search input.
  - Done when: chips toggle active state; search emits `update:searchQuery`.
  - Depends on: 1.0, 2.0, 3.0 · Parallel: 4.1, 5.x, 6.x · Touches: `apps/rook/dashboard/src/components/ProviderCatalogFilter.vue`
- [x] **4.3** `ProviderCategorySection.vue` — section header + 2–3 col grid of cards.
  - Done when: section renders heading + responsive grid of stub cards.
  - Depends on: 4.1 · Parallel: 4.2, 5.x, 6.x · Touches: `apps/rook/dashboard/src/components/ProviderCategorySection.vue`
- [x] **4.4** Rewrite `ProvidersView.vue` as catalog (remove flat table, drop quotes tab, use new components).
  - Done when: `/providers` shows 5 kind cards grouped by category; old table + quotes tab gone.
  - Depends on: 4.2, 4.3 · Parallel: 5.2, 6.1 · Touches: `apps/rook/dashboard/src/views/ProvidersView.vue`

## Phase 5: Details View

- [x] **5.0** Milestone — `/providers/:providerKind` lists connections for that kind.
- [x] **5.1** `ConnectionListItem.vue` — row with status, model, priority, proxy, action buttons.
  - Done when: row renders from stub connection and emits `test`/`edit`/`delete`/`toggle-proxy`.
  - Depends on: 1.0, 2.0, 3.0 · Parallel: 4.1, 4.2, 4.3, 6.1 · Touches: `apps/rook/dashboard/src/components/ConnectionListItem.vue`
- [x] **5.2** `ProviderDetailsView.vue` — header (breadcrumb back, kind name, count, Test All, Add) + connection list + empty state.
  - Done when: `/providers/ollama` renders 3 stub rows; Add opens modal with `providerKind='ollama'`.
  - Depends on: 5.1 · Parallel: 4.4, 6.1 · Touches: `apps/rook/dashboard/src/views/ProviderDetailsView.vue`

## Phase 6: Quota View

- [x] **6.0** Milestone — `/providers/quota` placeholder ships.
- [x] **6.1** `ProvidersQuotaView.vue` — mocked table + `Alert` banner ("per-provider variation, follow-up") + link to follow-up issue.
  - Done when: route renders banner, mock rows, and external link.
  - Depends on: 1.0 · Parallel: 4.1, 4.2, 4.3, 4.4, 5.1, 5.2 · Touches: `apps/rook/dashboard/src/views/ProvidersQuotaView.vue`

## Phase 7: Routing & Navigation

- [x] **7.0** Milestone — new routes, validated `:providerKind`, nav updated.
- [x] **7.1** Add `/providers/:providerKind` route with `beforeEnter` validation in `router/index.ts`.
  - Done when: invalid kind redirects to `/providers`; valid kind renders Details view.
  - Depends on: 4.0, 5.0, 6.0 · Parallel: 7.2, 7.3, 7.4 · Touches: `apps/rook/dashboard/src/router/index.ts`
- [x] **7.2** Add `/providers/quota` route + remove old `/providers/quotes`.
  - Done when: `/providers/quota` resolves; old URL 404s.
  - Depends on: 6.0 · Parallel: 7.1, 7.3, 7.4 · Touches: `apps/rook/dashboard/src/router/index.ts`
- [x] **7.3** `config/navigation.ts` — replace `providersQuotes` with `providersQuota`, rename `providersList` → `providersCatalog`.
  - Done when: Providers subnav shows "Catalog" + "Quota"; no stale `providersQuotes` references in code.
  - Depends on: 1.3 · Parallel: 7.1, 7.2, 7.4 · Touches: `apps/rook/dashboard/src/config/navigation.ts`
- [x] **7.4** Register 5 new lucide icons in `useNavigation.ts` (`Sparkles`, `Brain`, `Server`, `Stars`, `Zap`) + quota icon (`Fuel` or similar).
  - Done when: catalog cards render their icons; zero `Icon "X" not found` warnings.
  - Depends on: 1.0 · Parallel: 7.1, 7.2, 7.3 · Touches: `apps/rook/dashboard/src/composables/useNavigation.ts`
  - _Note: no registry change needed — `useNavigation`'s `iconRegistry` is exclusive to the sidebar nav. Catalog card icons are resolved in `ProviderCatalogCard.vue`'s local map (Cpu/Sparkles/Brain/Zap/Server) and `ProvidersQuotaView.vue` imports `Fuel` directly from `@lucide/vue`. Both already in place from Phases 4 & 6._

## Phase 8: Breadcrumb Extension

- [x] **8.0** Milestone — 3-level breadcrumb on details, 2-level pages unaffected.
- [x] **8.1** `views/sidebar/index.vue` — render 3rd `BreadcrumbItem` when `route.matched.length >= 3`.
  - Done when: `/providers/ollama` shows Home → Providers → Ollama; Home/ApiKeys/Combos/Settings unchanged.
  - Depends on: 7.0 · Parallel: none · Touches: `apps/rook/dashboard/src/views/sidebar/index.vue`

## Phase 9: i18n Verification

- [ ] **9.0** Milestone — TS and unit-test layer clean.
- [ ] **9.1** `cd apps/rook/dashboard && pnpm exec vue-tsc --noEmit` — zero errors, no key mismatches.
  - Done when: clean exit; parity holds between `en.json` and `es.json`.
  - Depends on: 1.3 · Parallel: 9.2 · Touches: (CLI run)
- [ ] **9.2** `cd apps/rook/dashboard && pnpm exec vitest run` — all specs green.
  - Done when: updated `AddProviderDialog.spec.ts` + new specs all pass.
  - Depends on: 3.4 · Parallel: 9.1 · Touches: (CLI run)

## Phase 10: Manual UI Verification

- [ ] **10.0** Milestone — full UX flow validated in browser.
- [ ] **10.1** `cd apps/rook/dashboard && pnpm run build` succeeds.
- [ ] **10.2** Dev server up; `/providers` shows 5 cards grouped by category.
- [ ] **10.3** Click `API Key` chip — only API-key kinds visible; click again restores.
- [ ] **10.4** Type `ollama` in search — only Ollama card visible; case-insensitive.
- [ ] **10.5** Click Ollama card — navigates to `/providers/ollama`; breadcrumb Home → Providers → Ollama.
- [ ] **10.6** Click Add on details — modal opens with `providerKind='ollama'` pre-filled.
- [ ] **10.7** Test credentials with valid key — Save enables only on `ok: true`.
- [ ] **10.8** Save — modal closes; details list refreshes with new connection.
- [ ] **10.9** Navigate to `/providers/quota` — placeholder renders with banner + mock data.
- [ ] **10.10** DevTools console — zero errors, zero Vue warnings.
  - Depends on: 1.0–9.0 · Parallel: 11.1 · Touches: (manual + dev server)

## Phase 11: Follow-up Issue

- [ ] **11.0** Milestone — track real per-provider quota work.
- [ ] **11.1** Create GitHub/Linear issue "Per-provider quota integration" — body references the user decision, lists 5 providers' quota endpoints, links to follow-up work.
  - Done when: issue exists with title + body; URL captured in `state.yaml` notes.
  - Depends on: none · Parallel: 10.x · Touches: (external tracker)

## Phase 12: Final Validation

- [ ] **12.0** Milestone — change ready to push.
- [ ] **12.1** Run `just ci-local` (or: `cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --check` + `cargo test --workspace` + `pnpm exec tsc --noEmit` + `pnpm exec vitest run`).
  - Done when: all green; no new warnings.
  - Depends on: 10.0 · Parallel: 12.2 · Touches: (CLI runs)
- [ ] **12.2** Update `openspec/changes/providers-ui-3-screen-refactor/state.yaml` with completed phases.
  - Done when: `state.yaml` lists phases 1–10 done; `current_phase` points to next step.
  - Depends on: 12.1 · Parallel: 12.3 · Touches: `openspec/changes/providers-ui-3-screen-refactor/state.yaml`
- [ ] **12.3** Commit + push changes (user opens the PR).
  - Done when: clean commit log, remote branch updated, working tree clean locally.
  - Depends on: 12.2 · Parallel: none · Touches: git
