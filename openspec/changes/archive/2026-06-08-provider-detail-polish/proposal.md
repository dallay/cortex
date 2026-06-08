# Proposal: Provider Detail Polish

> **Change name:** `2026-06-07-provider-detail-polish`
> **Scope:** Frontend only (Vue 3 dashboard at `apps/rook/dashboard/`). No backend changes.
> **Mode:** openspec (file-based persistence)

---

## 1. Why

The user reported that the **Ollama Cloud card on `/providers` does not navigate** to its CRUD view. Investigation surfaced **four sub-issues**, all touching the provider detail / CRUD surface, plus one unit-test gap. They are bundled into a single change for cohesion (one PR keeps router, catalog, view, icons, and tests consistent):

1. **Router kind-drift bug.** `apps/rook/dashboard/src/router/index.ts:14-20` hard-codes a 5-element `VALID_PROVIDER_KINDS` mirror of the catalog. The catalog grew to 6 kinds when `ollama-cloud` was added (see `config/providerCatalog.ts:156-175`); the router was not updated. The `beforeEnter` guard at line 79 silently redirects `ollama-cloud` back to `/providers`. The file header (lines 6-13) explicitly documents this drift as the **expected** failure mode — a clean fix, not a workaround, is overdue.
2. **Missing feature — title-as-link.** `views/ProviderDetailsView.vue:200-213` renders the provider display name as plain `<h1>` text. A user who lands on `/providers/ollama-cloud` with no API key has no path to the vendor's key-issuance page. The user wants the title to be a link to the provider's official site (one link per kind; e.g. `gemini` → `https://aistudio.google.com/apikey`, `ollama-cloud` → `https://ollama.com/cloud`).
3. **Missing feature — branded icons.** `components/ProviderCatalogCard.vue:35-42` resolves every kind to a generic Lucide icon (Cpu / Brain / Zap / Sparkles / Server / Cloud) via a static map. The user wants the **original vendor mark** for each provider. OmniRoute at `tmp/OmniRoute/public/providers/` ships usable assets for `anthropic` (`anthropic-m.png`, 5,133 bytes) and `gemini` (`gemini-cli.svg`, 745 bytes). For `openai`, `ollama`, `ollama-cloud`, `groq` there are no usable branded assets in OmniRoute — these will be **newly authored** as inline SVGs (NOT copied from a third party without a license), in the same visual language as the OmniRoute originals.
4. **Test rot.** 6 E2E tests at `e2e/providers.spec.ts:22-115` assume a legacy flat-CRUD layout (look for `<h1>Providers</h1>`, a global "Add Provider" button, a "Max Concurrent Requests" label, etc.) that was replaced by the `2026-06-06-providers-ui-3-screen-refactor` change. The second describe block (lines 118-191) is correct and uses modern `data-testid` selectors — **keep it**. The 6 stale tests must be deleted or rewritten.
5. **No unit test for `ProviderDetailsView`.** The route-guard bounce for invalid kinds is only covered by the `validKinds` set check inside the view's `watch` handler. A 5-line unit test would have caught the missing `ollama-cloud` in milliseconds.

**Modern-web best practices applied (per `modern-web-guidance` skill, MANDATORY for HTML/CSS/JS tasks):**

- External links: `target="_blank"` + `rel="noopener noreferrer"` (security guide §1.2, §1.5 — non-negotiable).
- External-link a11y: `aria-label="<Provider> — opens in new tab"` (or visually-hidden span) so screen readers announce the new-tab behavior.
- Icon a11y: `role="img"` + `aria-label="<Provider>"` on detail-header icons (decorative context); `aria-hidden="true"` on catalog-card icons (visible text already names the kind).
- LCP: detail-header icons are above the fold → **no** `loading="lazy"`. Catalog-grid cards may be off-screen → `loading="lazy"` is appropriate.
- SVG vs PNG: inline SVG preferred for theming (`currentColor`), zero extra HTTP request, Vite tree-shakes. PNG is acceptable for `anthropic.png` (the multicolor "A" mark does not SVG-cleanly without the source).
- CLS: every icon declares `width` and `height` (or `aspect-ratio`).

---

## 2. What Changes

### ADD — New files

| Path                                                        | Purpose                                                                                                                                                                                                                                           |
|-------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `apps/rook/dashboard/src/components/ProviderIcon.vue`       | Resolves `ProviderKind` → branded icon. Internally uses `<img>` with explicit `width` / `height`; supports `loading="lazy" \| "eager"` prop. Falls back to a neutral `Server` Lucide icon (with a `console.warn` in dev) if the asset is missing. |
| `apps/rook/dashboard/public/providers/openai.svg`           | Newly-authored inline SVG (stylized wordmark; not copied from a third party).                                                                                                                                                                     |
| `apps/rook/dashboard/public/providers/ollama.svg`           | Newly-authored inline SVG.                                                                                                                                                                                                                        |
| `apps/rook/dashboard/public/providers/ollama-cloud.svg`     | Newly-authored inline SVG.                                                                                                                                                                                                                        |
| `apps/rook/dashboard/public/providers/groq.svg`             | Newly-authored inline SVG.                                                                                                                                                                                                                        |
| `apps/rook/dashboard/public/providers/anthropic.png`        | Copy of `tmp/OmniRoute/public/providers/anthropic-m.png` (Anthropic's multicolor "A" mark, 5,133 bytes).                                                                                                                                          |
| `apps/rook/dashboard/public/providers/gemini.svg`           | Copy of `tmp/OmniRoute/public/providers/gemini-cli.svg` (Google's Gemini spark mark, 745 bytes).                                                                                                                                                  |
| `apps/rook/dashboard/src/views/ProviderDetailsView.spec.ts` | Unit test covering the `providerKindParam` watch + the invalid-kind bounce. Uses Vitest + `@vue/test-utils` `mount` with a stub router.                                                                                                           |

### MODIFY — Existing files

| Path                                                         | Change                                                                                                                                                                                                                                                                                                                                                                                                                 |
|--------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `apps/rook/dashboard/src/router/index.ts`                    | Remove the `VALID_PROVIDER_KINDS` mirror (lines 14-20) and its drift-acknowledging header (lines 6-13). Import `PROVIDER_KINDS` from `@/config/providerCatalog` and derive the valid set: `PROVIDER_KINDS.map(p => p.kind)`.                                                                                                                                                                                           |
| `apps/rook/dashboard/src/config/providerCatalog.ts`          | Add `readonly brandUrl?: string` to `CatalogEntry`. Populate for all 6 kinds: `openai` → `https://platform.openai.com/api-keys`, `anthropic` → `https://console.anthropic.com/settings/keys`, `gemini` → `https://aistudio.google.com/apikey`, `groq` → `https://console.groq.com/keys`, `ollama` → `https://ollama.com` (Ollama Cloud key page; local users can ignore), `ollama-cloud` → `https://ollama.com/cloud`. |
| `apps/rook/dashboard/src/views/ProviderDetailsView.vue`      | Wrap the page `<h1>` in `<a :href="entry.brandUrl" target="_blank" rel="noopener noreferrer" :aria-label="...">`. Render `<ProviderIcon :kind="entry.kind" loading="eager" />` above the title. Add a `tabindex` and `role="link"` semantics for AT if the link is purely visual.                                                                                                                                      |
| `apps/rook/dashboard/src/components/ProviderCatalogCard.vue` | Drop the `ICONS` map (lines 35-42). Render `<ProviderIcon :kind="item.kind" aria-hidden="true" loading="lazy" />` instead.                                                                                                                                                                                                                                                                                             |
| `apps/rook/dashboard/e2e/providers.spec.ts`                  | Delete the stale `test.describe("Provider Management", ...)` block (lines 3-116). Add 2-3 new tests inside the existing modern block: (a) the Ollama Cloud card navigates to `/providers/ollama-cloud` (regression for issue #1), (b) the detail page header is an external link with `target="_blank"` and the expected `aria-label`, (c) the branded icon is visible.                                                |
| `openspec/specs/providers-ui/spec.md`                        | Add requirement: **Provider Title as External Link** (h1 anchors to `brandUrl`, `target="_blank" rel="noopener noreferrer"`, `aria-label` announces "opens in new tab"). Add requirement: **Branded Provider Icons** (catalog cards and detail headers use the original vendor mark; catalog grid is lazy-loaded, detail header is eager; every icon declares `width` and `height` to prevent CLS).                    |

### REMOVE

- `apps/rook/dashboard/src/router/index.ts` — `VALID_PROVIDER_KINDS` constant (5 lines) and its drift-acknowledging header comment (8 lines).
- `apps/rook/dashboard/src/components/ProviderCatalogCard.vue` — the 8-line `ICONS` map and the 6 `@lucide/vue/dist/esm/icons/...` imports.
- `apps/rook/dashboard/e2e/providers.spec.ts` — the 6 stale tests in lines 22-115.

---

## 3. Capabilities

This change modifies the existing `providers-ui` spec (it adds 2 new requirements). The `provider-connections` spec is **not** modified — no wire format, no domain rules, and no `Credentials` type change. `brandUrl` is frontend-only metadata.

### New Capabilities

- **None.** No new spec file is created. The branded-icon and external-link concerns are scoped to the existing `providers-ui` UX spec.

### Modified Capabilities

- `providers-ui` — adding 2 new requirements (see "What Changes" → `openspec/specs/providers-ui/spec.md`). The router and `ProviderCatalogCard` are still UI implementation details, but the *behavior* (title links externally; icons are branded) is now part of the durable spec.

---

## 4. Decisions (user-approved)

1. **Single source of truth.** Import `PROVIDER_KINDS` into the router. The "router as self-contained bootstrap" rationale is no longer worth the drift tax. The catalog is small (6 entries) and `as const` frozen, so the import is type-safe.
2. **`brandUrl` is frontend-only.** Lives in `CatalogEntry` and is consumed only by `ProviderDetailsView`. No backend wire change, no i18n key (URLs are not translatable).
3. **Branded icon format.** Inline `<img src="/providers/<name>.{svg,png}" width=... height=... loading=... decoding="async">` for both SVG and PNG. Reasons: (a) browser caches PNGs that inline SVG cannot, (b) avoids Vite `?raw` re-encoding, (c) consistent `width` / `height` / `loading` API regardless of format. Inline SVG via Vue component is reserved for a future theming iteration.
4. **Icon resolution by filename extension.** The catalog holds the basename (`anthropic`); `ProviderIcon` resolves `.png` vs `.svg` from the actual file. Avoids polluting the catalog with a `format` field.
5. **Accessibility stance.** `aria-hidden="true"` on grid-card icons (visible text already names the kind). `role="img"` + `aria-label="<Provider>"` on detail-header icons. The external link uses `aria-label="<Provider> — opens in new tab"` (single source of truth; no extra DOM).
6. **E2E rewrite strategy.** Delete the entire `test.describe("Provider Management", ...)` block (lines 3-116) and add 2-3 new tests inside the existing modern block. Keeps the file compact and avoids a confusing mix of legacy and modern selectors.

---

## 5. Non-Goals

- ❌ Backend wire format or domain change. `brandUrl` is a frontend concern.
- ❌ `ProvidersQuotaView` real data (issue #132, unrelated).
- ❌ Per-provider OAuth flow, per-provider model import, or per-provider test playground.
- ❌ The `vue-router ^5.1.0` package.json typo (unrelated; tracked elsewhere).
- ❌ Modifying `provider-connections` spec or any backend `crates/*` code.
- ❌ Theming-driven SVG (`currentColor` switching). The 6 icons are static assets; dark/light mode is already handled by the card's `bg-primary/10` chip and the catalog's color tokens.
- ❌ Fixing pre-existing i18n drift between `en.json` and `es.json`.

---

## 6. Affected Areas

| Area                                                         | Impact   | Description                                          |
|--------------------------------------------------------------|----------|------------------------------------------------------|
| `apps/rook/dashboard/src/router/index.ts`                    | Modified | Mirror list removed; import catalog directly         |
| `apps/rook/dashboard/src/config/providerCatalog.ts`          | Modified | +1 field (`brandUrl`), populated for 6 kinds         |
| `apps/rook/dashboard/src/views/ProviderDetailsView.vue`      | Modified | Title-as-link, branded icon (eager)                  |
| `apps/rook/dashboard/src/components/ProviderCatalogCard.vue` | Modified | Drop Lucide `ICONS` map, use `<ProviderIcon>` (lazy) |
| `apps/rook/dashboard/src/components/ProviderIcon.vue`        | **New**  | Kind → icon resolver, fallback handling              |
| `apps/rook/dashboard/public/providers/*.{svg,png}`           | **New**  | 6 branded assets (~7 KB total)                       |
| `apps/rook/dashboard/src/views/ProviderDetailsView.spec.ts`  | **New**  | Unit test for kind-guard bounce                      |
| `apps/rook/dashboard/e2e/providers.spec.ts`                  | Modified | Delete 6 stale, add 2-3 new tests                    |
| `openspec/specs/providers-ui/spec.md`                        | Modified | +2 requirements                                      |
| Backend (`crates/*`)                                         | None     | —                                                    |

---

## 7. Risks

| Risk                                                                                                                                                                                                          | Likelihood | Mitigation                                                                                                                                                                                                                |
|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Router drift can recur** if the catalog exports a non-frozen array.                                                                                                                                         | Low        | `PROVIDER_KINDS` is `as const` frozen. The new unit test in `ProviderDetailsView.spec.ts` exercises the route guard. Add a typed unit test for the router's `beforeEnter` in a follow-up.                                 |
| **Type drift** between `authType` casing (`apikey` vs `apiKey`) in catalog vs backend.                                                                                                                        | Low        | The catalog's `authTypes` is `readonly AuthType[]` and the type is `as const` frozen. No `authType` field is added in this change.                                                                                        |
| **`provider_crud.enabled` flag** must be on for the new E2E to pass against a live backend.                                                                                                                   | Med        | The unit test mocks the composable, not the backend, so it runs without the flag. The E2E test setup already handles this; the new tests are no different.                                                                |
| **Missing icon asset** → no automatic fallback.                                                                                                                                                               | Med        | `ProviderIcon` falls back to a neutral `Server` Lucide icon (with a `console.warn` in dev). The fallback is the only way the absence is visible.                                                                          |
| **`vue-router ^5.1.0` typo** in `package.json` is unrelated to this change.                                                                                                                                   | Low        | Out of scope. Do not "fix" in this PR; that would conflate concerns.                                                                                                                                                      |
| **Bleeding-edge tool versions** may shift during the change (`vue-tsc`, Vite, Vitest).                                                                                                                        | Low        | Pin versions in the apply phase; do not bump major versions without a separate PR.                                                                                                                                        |
| **OmniRoute asset license** — the `anthropic-m.png` and `gemini-cli.svg` files are copied from a third-party project. The plan assumes they are redistributable; if not, swap to author-mode SVGs in `apply`. | Med        | sdd-apply will confirm the OmniRoute license before copying. If uncertain, fall back to the newly-authored SVGs (the same applies to Anthropic and Gemini).                                                               |
| **Newly-authored SVGs may not match the OmniRoute visual language.**                                                                                                                                          | Med        | The 4 newly-authored SVGs (openai, ollama, ollama-cloud, groq) are the implementation team's responsibility. sdd-design will set a style guide; sdd-apply will deliver them and verify visual consistency in the browser. |

---

## 8. Rollback Plan

Frontend-only change, single PR. Rollback = `git revert`. No DB migration, no API contract change, no feature flag.

If the `ProviderIcon` component ships a visual bug, ship a feature-flagged fallback to the existing Lucide map in `sdd-apply` (one-line `v-if`). If the new branded assets break the catalog layout, revert the `ProviderCatalogCard.vue` changes only; the router and spec changes can land independently.

---

## 9. Dependencies

- `tmp/OmniRoute/public/providers/anthropic-m.png` and `tmp/OmniRoute/public/providers/gemini-cli.svg` — copy under a permissive license. Both files are < 6 KB and present in the workspace. `sdd-apply` will confirm the OmniRoute license; if not redistributable, fall back to author-mode SVGs.
- `@iconify-json/simple-icons` and `@iconify/vue` added to the dashboard package for Iconify icon rendering. No `shadcn-vue add` runs.
- i18n: no new keys. URLs are not translatable.

---

## 10. Success Criteria

- [ ] `/providers/ollama-cloud` renders `ProviderDetailsView` (no silent redirect to `/providers`).
- [ ] The detail-page h1 is an `<a>` with `target="_blank" rel="noopener noreferrer"` and `aria-label="<Provider> — opens in new tab"`.
- [ ] All 6 catalog cards render the branded icon (no Lucide fallback in steady state).
- [ ] All 6 detail-page headers render the branded icon eagerly (no `loading="lazy"`).
- [ ] Every `<img>` and inline `<svg>` declares `width` and `height` (no CLS).
- [ ] `ProviderDetailsView.spec.ts` covers the invalid-kind bounce (mount with a mock route pushing an unknown kind → expect `router.replace('/providers')`).
- [ ] `e2e/providers.spec.ts` has zero stale tests; the modern Ollama Cloud block (lines 118-191) still passes; 2-3 new tests cover the icon and external link.
- [ ] `pnpm typecheck` (vue-tsc), `pnpm test`, and `just ci-lint-only` all pass.
- [ ] No new `shadcn-vue add`, no new npm package.
- [ ] Lighthouse accessibility score on `/providers/ollama-cloud` ≥ 95 (sanity check for the new aria-label).

---

## 11. Open Questions (for sdd-spec / sdd-design)

1. **Vite asset strategy** — `<img src="/providers/x.svg">` (cached) vs `import x from '/providers/x.svg?raw'` (inline). `<img>` is simpler and lets the browser cache. Inline is only worth it if we need `currentColor` theming. **Recommendation:** `<img>` for v1; revisit when dark/light SVG variants are needed.
2. **Icon resolution by extension** vs an explicit `format` field on the catalog. **Recommendation:** by extension. Avoids a new field; the catalog stays minimal.
3. **`aria-label` wording** — "opens in new tab" (verbatim) vs the icon convention of an `aria-label` that includes the provider name. **Recommendation:** `aria-label="<Provider> — opens in new tab"` on the link. Single source of truth; no extra DOM.
4. **Unit test scope** — just the bounce, or the full view mount with i18n? **Recommendation:** just the bounce. The full view mount is the E2E's job. Keeps the test 5 lines.
5. **E2E delete vs rewrite** — full `test.describe` removal (lines 3-116) vs surgical rewrite of each test. **Recommendation:** full removal. The legacy block is beyond repair (it asserts against `<h1>Providers</h1>`, a non-existent "Add Provider" button, and a "Max Concurrent Requests" label).
6. **Branded icon delivery** — 4 newly-authored SVGs in this PR, or punt to a follow-up issue and ship the 2 OmniRoute-licensed assets first? **Recommendation:** ship all 6 in this PR. The work is small (4 SVGs, ~1 KB each), and partial delivery looks unprofessional.
