# Proposal: Providers UI 3-Screen Refactor

> **Change name:** `providers-ui-3-screen-refactor`
> **Scope:** Frontend only. No backend changes.
> **Mode:** openspec (file-based persistence)

---

## 1. Why

The current `/providers` view is a flat table that does not scale beyond a handful of connections. The product vision — "an AI Provider Control Plane" with many providers and many connections per provider — requires a navigable hierarchy, not a single dense grid.

User intent (paraphrased): *"A futuro vamos a tener muchos providers"* and *"este es el AI Provider Control Plane."* The backend domain model (`Provider → Connection`, 5 `ProviderKind` values, `Credentials::ApiKey | OAuth`) is already correct and stable. The gap is purely in the dashboard UX.

This refactor lands the navigation skeleton and a per-kind details view, so adding the 6th, 7th, 8th provider later is a metadata entry, not a UI rewrite.

---

## 2. What Changes

### ADD — New views and components

| Path | Purpose |
|---|---|
| `apps/rook/dashboard/src/views/ProvidersCatalogView.vue` | New catalog: grid of `ProviderCatalogCard`s grouped by category (API Key, OAuth, Local) |
| `apps/rook/dashboard/src/views/ProviderDetailsView.vue` | `/providers/:providerKind` — header + list of connections for that kind, add/test-all/edit/delete actions |
| `apps/rook/dashboard/src/views/ProvidersQuotaView.vue` | `/providers/quota` — placeholder with mocked data + banner explaining per-provider variation + link to follow-up issue |
| `apps/rook/dashboard/src/components/ProviderCatalogCard.vue` | Reusable card (name, status pill, connection count, enable toggle, test action) |
| `apps/rook/dashboard/src/components/ProviderCategorySection.vue` | Wraps category header + grid of cards |
| `apps/rook/dashboard/src/components/ProviderCatalogFilter.vue` | Category filter chips + search input |
| `apps/rook/dashboard/src/composables/useProviderCatalog.ts` | Derived state: group connections by kind, count per kind, search/filter pipeline |
| `apps/rook/dashboard/src/config/providerCatalog.ts` | Static `PROVIDER_KINDS` metadata (displayName, runtimeId, defaultBaseUrl, supportsOAuth, description, iconName) |
| `apps/rook/dashboard/src/composables/useProviderConnections.ts` | New composable with `fetchById()` (and split from `useProviders` for kind vs. connection concerns — see Open Questions) |

### MODIFY — Existing files

| Path | Change |
|---|---|
| `apps/rook/dashboard/src/views/ProvidersView.vue` | Replace flat table with `ProvidersCatalogView` body. Keep file name (no rename). |
| `apps/rook/dashboard/src/components/AddProviderDialog.vue` | Add props: `providerKind: ProviderKind`, `providerId?: string`, `mode: 'create' \| 'edit'`. Controlled by parent via `v-model:open`. Add `authType` toggle (API Key / OAuth). Per-kind default `baseUrl`. |
| `apps/rook/dashboard/src/components/EmptyState.vue` | Internally wrap shadcn-vue `Empty` primitive. Public API (`title`, `description`, `icon`) unchanged. |
| `apps/rook/dashboard/src/router/index.ts` | Add `/providers/:providerKind` and `/providers/quota` routes. Validate `:providerKind` against `ProviderKind` union. |
| `apps/rook/dashboard/src/config/navigation.ts` | Replace `providersQuotes` item with `providersQuota` (→ `/providers/quota`). Remove `providersList` (catalog is the default landing for "Providers"). |
| `apps/rook/dashboard/src/views/sidebar/index.vue` | Extend global breadcrumb to 3 levels when `route.matched.length >= 3` (Home → Providers → `<kind>`). |
| `apps/rook/dashboard/src/composables/useNavigation.ts` | Register 5 new icons: `Sparkles`/`Bot`, `Brain`, `Server`/`Cpu`, `Stars`, `Zap`. |
| `apps/rook/dashboard/src/lib/api.ts` | Add `export type ProviderKind = 'openai' \| 'anthropic' \| 'ollama' \| 'gemini' \| 'groq'`. Tighten `providerKind` and `authType` fields from `string` to the union. |
| `apps/rook/dashboard/src/composables/useProviders.ts` | Add `fetchById(id)`. (Optional split — see Open Questions.) |
| `apps/rook/dashboard/src/i18n/en.json` and `es.json` | Add keys: `providers.catalog.*`, `providers.details.*`, `providers.kind.*` (per-kind display names + descriptions), `providers.dialog.edit.*`, `providers.quota.*`, `providers.filters.*`. **Mirror all keys in both files.** |
| `apps/rook/dashboard/src/components/AddProviderDialog.spec.ts` | Update test to pass new `providerKind` prop. Add OAuth form coverage. |
| `apps/rook/dashboard/e2e/providers.spec.ts` | Extend E2E: catalog → card → details → test → delete. |

### REMOVE

- Mock quotes data array in `ProvidersView.vue` (moved to `ProvidersQuotaView`).
- `nav.providersQuotes` and `breadcrumb.providersQuotes` i18n keys (replaced by `providersQuota`).

---

## 3. Capabilities

This refactor is a **UI restructure only**. The existing backend capability `provider-connections` (at `openspec/specs/provider-connections/spec.md`) defines the domain model, wire protocol, and endpoints — none of which change. No new requirements are introduced at the spec level.

### New Capabilities

- **None.** No new spec is created. The 3-screen UX is a frontend implementation concern, not a durable product contract.

### Modified Capabilities

- **None.** The `provider-connections` spec is about the wire/domain model (CRUD operations, credentials, health probe, optimistic locking). UI navigation hierarchy is out of scope for that spec.

> If `sdd-spec` later identifies a need to document the 3-screen UX as a cross-cutting capability, that is a separate decision — not required by this refactor.

---

## 4. Decisions (user-approved)

1. **Scope:** Option A+B — 3 screens + filter chips. No bulk actions, no Distribute Proxies, no static catalog of 228 providers.
2. **Routing pattern:** `/providers/:providerKind` groups by `ProviderKind` (e.g. `ollama` → 9 connections), not by connection id.
3. **Quota route:** Moved to its own route **`/providers/quota`** (singular). Mocked data kept for now. Follow-up issue tracks real per-provider quota.
4. **`EmptyState`:** Option C — wrap shadcn-vue `Empty` primitive internally. Public API stays `{ title, description, icon }` to avoid breaking existing callsites.
5. **Components:** No `shadcn-vue add` runs. All 65 components already installed; use what's there.
6. **Backend:** No backend changes. All 7 endpoints exist, including the newly shipped `POST /api/providers/test-credentials` (commit `541f5ce`).

---

## 5. Non-Goals (explicit)

- ❌ Bulk actions (select all, enable/disable/delete multi) — future PR.
- ❌ Distribute Proxies (auto-rebalance priority/weight) — future PR.
- ❌ Static catalog of 228 providers (OmniRoute parity) — future PR.
- ❌ OAuth flow implementation for OAuth-supporting providers (Gemini, etc.) — dialog supports the form shape only; no OAuth redirect yet.
- ❌ Per-provider quota implementation (real API integration) — placeholder page only; follow-up issue.
- ❌ Provider logos/brand icons — use lucide icons only.
- ❌ Real-time connection status (WebSocket / SSE push) — health check stays on-demand.
- ❌ Mobile-specific layout — desktop-first; mobile works but is not optimized.
- ❌ Backend changes of any kind (per Decision 6).

---

## 6. Affected Areas

| Area | Impact | Description |
|---|---|---|
| `apps/rook/dashboard/src/views/ProvidersView.vue` | Modified | Replaced by `ProvidersCatalogView` body (file name preserved) |
| `apps/rook/dashboard/src/components/AddProviderDialog.vue` | Modified | New `providerKind`/`mode`/`authType` props, controlled open state, OAuth form |
| `apps/rook/dashboard/src/components/EmptyState.vue` | Modified | Internal shadcn-vue `Empty` wrap, public API unchanged |
| `apps/rook/dashboard/src/router/index.ts` | Modified | Add `/providers/:providerKind` and `/providers/quota` |
| `apps/rook/dashboard/src/config/navigation.ts` | Modified | `providersQuota` replaces `providersQuotes`; drop `providersList` |
| `apps/rook/dashboard/src/views/sidebar/index.vue` | Modified | 3-level breadcrumb when `route.matched.length >= 3` |
| `apps/rook/dashboard/src/i18n/{en,es}.json` | Modified | New keys mirrored in both files |
| `apps/rook/dashboard/src/composables/useProviders.ts` | Modified | Add `fetchById` |
| `apps/rook/dashboard/src/lib/api.ts` | Modified | `ProviderKind` and `AuthType` type unions |
| `apps/rook/dashboard/src/composables/useNavigation.ts` | Modified | 5 new icon registrations |
| Backend (`crates/*`) | None | No changes |

---

## 7. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| i18n key drift between `en.json` and `es.json` (already 13-line gap) | High | Mirror every new key in both files; CI / `vue-tsc` will fail if missing. Do not fix pre-existing drift in this PR. |
| `AddProviderDialog.spec.ts` rewrite breaks unrelated tests | Med | Update test as part of the dialog refactor; keep shallowMount pattern. |
| Global breadcrumb extension to 3 levels breaks 2-level pages | Med | Guard with `route.matched.length >= 3`; smoke-test Home, ApiKeys, Combos, Models, Settings. |
| TypeScript union tightening in `lib/api.ts` surfaces pre-existing loose typing | Med | Fix or cast at the call site; do NOT change the wire protocol. |
| 5 new lucide icons add bundle weight | Low | ~5 KB tree-shaken; negligible for the catalog view. |
| Quota placeholder ships as "Coming soon" and is mistaken for shipped feature | Low | Banner + `Coming soon` empty state explicitly mark it as not real. |
| Provider CRUD limitation (TOML providers serve traffic, not SQLite) surprises users | Med | Add a `Tooltip`/banner on the catalog: "Saved providers are persisted but require a server restart to serve traffic." (Pre-existing limitation, surfaced as UX expectation.) |

---

## 8. Rollback Plan

Frontend-only change. Rollback = revert the PR (or `git revert`). No DB migrations, no API contract changes, no feature flags needed. The previous `ProvidersView.vue` flat-table behavior is fully recoverable from git history.

If the dialog refactor ships bugs, the safest interim state is to:
1. Keep `AddProviderDialog.vue` accepting the new props (back-compat by making `providerKind` optional, defaulting to `'ollama'`).
2. Land the catalog/details views first behind the new routes.
3. Cut over the `AddProviderDialog` callers last.

This staged cutover is the recommended `sdd-tasks` ordering.

---

## 9. Dependencies

- **shadcn-vue components** — all present (verified by `ls src/components/ui/`, 65 components). No `add` runs.
- **lucide-vue icons** — 5 new entries registered in `useNavigation.ts` icon registry.
- **`POST /api/providers/test-credentials`** — already shipped in commit `541f5ce` (see `tmp/plans/2026-06-06-test-credentials-endpoint-design.md`). Dialog's test-before-save flow consumes it.
- **i18n schema** — `MessageSchema = typeof en` already enforces parity; adding a key to `en.json` without `es.json` is a TS error.

---

## 10. Success Criteria

- [ ] `/providers` renders a catalog grid grouped by category (API Key, OAuth, Local), not a flat table.
- [ ] Clicking a card navigates to `/providers/:providerKind` and shows the list of connections for that kind.
- [ ] The "Add" action on both catalog and details views opens the same `AddProviderDialog` (controlled, with `providerKind` pre-filled on details).
- [ ] The dialog supports both `authType: 'apikey'` and `authType: 'oauth'` (form shape; OAuth flow is non-goal).
- [ ] The dialog "Test" action calls `POST /api/providers/test-credentials`; "Save" is disabled until `result.ok === true`.
- [ ] `/providers/quota` renders the placeholder page with mocked data and a follow-up issue link.
- [ ] Empty states (no connections for a kind) use the new `EmptyState` wrapper that internally uses shadcn-vue `Empty`.
- [ ] 3-level breadcrumb works on `/providers/:providerKind` without regressing 2-level pages.
- [ ] Filter chips (category) and search input filter the catalog client-side.
- [ ] `i18n/en.json` and `i18n/es.json` are in sync for all new keys.
- [ ] Unit tests: `AddProviderDialog.spec.ts` updated; new `ProviderDetailsView.spec.ts` and `ProviderCatalogCard.spec.ts` added.
- [ ] E2E: `e2e/providers.spec.ts` extended to cover catalog → card → details → test → delete.
- [ ] `pnpm typecheck` (vue-tsc), `cargo test`, and `just ci-lint-only` all pass.
- [ ] No new `shadcn-vue add` runs in the diff (verifiable via `git diff package.json`).

---

## 11. Open Questions (for sdd-spec / sdd-design)

1. **Should `useProviders` be split** into `useProviderCatalog` (kind-level) and `useProviderConnections` (connection-level)? **Recommendation: yes** — the catalog view does not need `create/update/remove`, and the details view does not need the per-kind grouping. Clean separation of concerns.
2. **Should the connection modal support editing**, not just create? **Recommendation: yes** — add `mode: 'create' | 'edit'` prop, reuse the same form, pre-fill from `fetchById(id)`. The backend already supports `PUT /api/providers/{id}` (requires `expectedUpdatedAt` for optimistic concurrency).
3. **Existing `AddProviderDialog.spec.ts`** — what is the cleanup path? (Verified during `sdd-apply`: dialog API changes require test rewrite. Plan to use shallowMount with `providerKind: 'ollama'` as the default case to keep the diff small.)
4. **Catalog metadata source of truth** — should `PROVIDER_KINDS` in `config/providerCatalog.ts` eventually be served from the backend (e.g. `GET /api/providers/kinds`)? For v1, static TS const is fine (5 kinds). Revisit when the 6th kind ships.
5. **OAuth form behavior** when no real OAuth flow exists — should the "Connect with OAuth" button be disabled with a "Coming soon" tooltip, or removed entirely? **Recommendation:** render the button as disabled with a tooltip pointing to the follow-up issue. Keeps the form shape forward-compatible.
