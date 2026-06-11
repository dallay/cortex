# Tasks: Provider Detail Polish

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Low

### 1. License + copy 2 OmniRoute assets

- [x] Confirm `tmp/OmniRoute/LICENSE` is MIT (© 2026 diegosouzapw); add PR attribution. Create `dashboard/public/providers/`; copy `tmp/OmniRoute/public/providers/{anthropic-m.png → anthropic.png, gemini-cli.svg → gemini.svg}`.
- Acceptance: attribution in PR; both files load at dev server.

### 2. Author 4 branded SVGs

- [x] Author `dashboard/public/providers/{openai,ollama,ollama-cloud,groq}.svg` — 24×24 `viewBox`, `currentColor`, no styles/fonts/animations/`<title>`/`<desc>`; `ollama-cloud` distinct.
- Acceptance: all 4 render at 24×24 and 64×64, respond to `color`, each ≤ 4 KB.

### 3. Extend providerCatalog

- [x] Rename `logoIconName` → `iconFile: string` on `CatalogEntry`; add `readonly brandUrl?: string`; populate both for all 6 kinds.
- [x] `pnpm exec vue-tsc --noEmit -p dashboard`; fix every consumer.
- Acceptance: `vue-tsc` passes; `PROVIDER_KINDS` is the only place naming per-kind asset/URL.

### 4. Create ProviderIcon (TDD)

- [x] Create `dashboard/src/components/ProviderIcon.vue`: props `kind`, `loading`, `width`, `height`, `decorative`; `<img>`; aria; `Server` Lucide fallback + dev `console.warn` on `@error`.
- [x] Add `ProviderIcon.spec.ts` covering 4 cases.
- Acceptance: `pnpm exec vitest run ProviderIcon` green.

### 5. Wire ProviderIcon into ProviderCatalogCard

- [x] Drop `ICONS` + 6 Lucide imports in `ProviderCatalogCard.vue`; replace `<component :is="iconComponent">` with `<ProviderIcon :kind="item.kind" loading="lazy" :width="20" :height="20" decorative />`.
- Acceptance: card renders branded `<img>`; no Lucide generics; tests green.

### 6. Title-as-link in ProviderDetailsView

- [x] Replace header `<h1>` with design §4 anchor/h1 branching (anchor with `target="_blank" rel="noopener noreferrer"` + `aria-label` + `ExternalLink aria-hidden`; plain `<h1>` fallback).
- [x] Render `<ProviderIcon :kind="entry.kind" loading="eager" :width="40" :height="40" :decorative="false" />` above title; import `ExternalLink`.
- Acceptance: header shows branded icon + name as link; Copy Link returns `brandUrl`; tab focus has a ring.

### 7. Fix router kind-drift

- [x] In `dashboard/src/router/index.ts`, delete the 8-line "we intentionally do not import" comment and the 5-line `VALID_PROVIDER_KINDS` array. Add `import { PROVIDER_KINDS } from '@/config/providerCatalog'`; replace with `const VALID_PROVIDER_KINDS = PROVIDER_KINDS.map(p => p.kind) as readonly string[]`.
- Acceptance: `/providers/ollama-cloud` renders; `/providers/foo` bounces to `/providers`.

### 8. Add ProviderDetailsView.spec (TDD)

- [x] Vitest 4 + `@vue/test-utils`; `vi.mock('@/composables/useProviders')` and `vi.mock('@/composables/useAvailableModels')`; `createMemoryHistory` + `router.push('/providers/<kind>')`.
- [x] Cover 6 design §7 cases.
- Acceptance: `pnpm exec vitest run ProviderDetailsView` green.

### 9. Clean up stale E2E tests

- [x] In `dashboard/e2e/providers.spec.ts`, delete `test.describe("Provider Management", ...)` (lines 3-116); keep modern block (118-192). Add 3 tests: Ollama Cloud → `/providers/ollama-cloud`; detail header `<a target="_blank" rel="noopener noreferrer" aria-label="… — opens in new tab" href="https://ollama.com/cloud">`; branded icon visible.
- Acceptance: `pnpm exec playwright test e2e/providers.spec.ts` green.

### 10. Lint, type-check, tests

- [x] `cd dashboard && pnpm exec biome check --write src && pnpm exec vue-tsc --noEmit && pnpm exec vitest run && pnpm exec prettier --check src e2e`. Run `just ci-lint-only`.
- Acceptance: lint clean, types clean, unit + e2e green.

### 11. Browser-verify

- [ ] Start `pnpm dev` in `dashboard` (if not running). `/providers` — 6 branded cards. Click Ollama Cloud — `/providers/ollama-cloud` renders; header shows icon + "Ollama Cloud" link + `ExternalLink`; right-click → Copy Link returns `https://ollama.com/cloud`; tab focus has a ring.
- Acceptance: visual + a11y (Lighthouse ≥ 95) gut-check passes.
- **Status**: Code implementation complete. Manual browser verification required (dev server running on :4747).
