# Design: Provider Detail Polish

> **Change:** `2026-06-07-provider-detail-polish` · **Mode:** openspec · **Scope:** Frontend only (Vue 3 dashboard).

## 1. Architecture Overview

`apps/rook/dashboard/src/config/providerCatalog.ts` is the **single source of truth** for `ProviderKind` metadata (kind, displayName, brandUrl, iconFile). Consumers:

| Consumer                  | Reads                             | What it does                                           |
|---------------------------|-----------------------------------|--------------------------------------------------------|
| `ProviderIcon.vue` (new)  | `iconFile`                        | Resolves `kind` → `<img src="/providers/<iconFile>">`  |
| `ProviderCatalogCard.vue` | via `ProviderIcon`                | Branded mark + name (lazy)                             |
| `ProviderDetailsView.vue` | `displayName`, `brandUrl`         | Title-as-link + eager-loaded mark                      |
| `router/index.ts`         | `PROVIDER_KINDS.map(p => p.kind)` | Derives `VALID_PROVIDER_KINDS` — drift = compile error |

`brandUrl` is frontend-only; no wire change, no i18n key (URLs aren't translatable).

## 2. `ProviderIcon.vue` Component API

**Props:** `kind: ProviderKind` (required) · `loading?: 'eager' \| 'lazy' = 'lazy'` · `width?: number \| string = 32` · `height?: number \| string = 32` · `decorative?: boolean = true`.

**Behavior:** `decorative=true` → `aria-hidden="true"` (catalog cards; visible text names the kind). `decorative=false` → `role="img"` with `aria-label` (detail header; standalone brand mark). Catalog is `lazy`; detail header is `eager` (LCP candidate). `<img>` with `width`/`height`/`decoding="async"` for CLS safety; format is implicit via URL — no `format` field. Falls back to a Lucide `Server` icon + dev `console.warn` on 404.

The component is `<img v-if="src" :src="'/providers/'+entry.iconFile" ... v-bind="ariaProps">`. Adding a 7th kind requires no component change — only a new catalog entry.

## 3. `brandUrl` Population

Add to `CatalogEntry`: `readonly iconFile: string` (required) and `readonly brandUrl?: string` (optional — future kinds without a vendor page stay compile-safe).

| `kind`         | `iconFile`         | `brandUrl`                                    |
|----------------|--------------------|-----------------------------------------------|
| `openai`       | `openai.svg`       | `https://platform.openai.com/api-keys`        |
| `anthropic`    | `anthropic.png`    | `https://console.anthropic.com/settings/keys` |
| `ollama`       | `ollama.svg`       | `https://ollama.com`                          |
| `ollama-cloud` | `ollama-cloud.svg` | `https://ollama.com/cloud`                    |
| `gemini`       | `gemini.svg`       | `https://aistudio.google.com/apikey`          |
| `groq`         | `groq.svg`         | `https://console.groq.com/keys`               |

Detail view branches: `brandUrl` set → `<a>`; unset → plain `<h1>`.

## 4. Title-as-Link

```vue
<a v-if="entry.brandUrl" :href="entry.brandUrl" target="_blank"
   rel="noopener noreferrer"
   :aria-label="`${providerName} — opens in new tab`"
   class="inline-flex items-center gap-1.5 rounded-sm text-2xl
          font-semibold tracking-tight hover:underline
          focus-visible:outline focus-visible:outline-2
          focus-visible:outline-offset-2 focus-visible:outline-primary">
  {{ providerName }}
  <ExternalLink class="h-4 w-4" aria-hidden="true" />
</a>
<h1 v-else class="text-2xl font-semibold tracking-tight">{{ providerName }}</h1>
```

`focus-visible:outline` is Tailwind 4's native outline. The Lucide `ExternalLink` (16px) is `aria-hidden` because the link's `aria-label` already names the target and announces "opens in new tab" — single source of truth, no extra DOM.

## 5. Router Fix

```ts
// src/router/index.ts
import { PROVIDER_KINDS } from '@/config/providerCatalog'

// Catalog is the single source of truth. Adding a new kind now
// requires no router change — the guard and the param both update.
const VALID_PROVIDER_KINDS: readonly string[] = PROVIDER_KINDS.map(p => p.kind)
```

Drop the 8-line "we intentionally do not import" comment block and the 5-line hard-coded array. The `beforeEnter` at line 77 is unchanged.

## 6. Asset Strategy

**Authored SVGs (4):** `openai.svg`, `ollama.svg`, `ollama-cloud.svg`, `groq.svg` at `apps/rook/dashboard/public/providers/`. Style guide: 24×24 `viewBox`, `fill="currentColor"` (filled) or `stroke="currentColor" stroke-width="2"` (outlined), no embedded styles/fonts/animations, no inner `<title>`/`<desc>`. Themed via Tailwind `text-*` on the parent.

**Copied from OmniRoute (2) — LICENSE VERIFIED:** `tmp/OmniRoute/LICENSE` is **MIT** (© 2026 diegosouzapw). Both assets < 6 KB. Copy `anthropic-m.png` → `anthropic.png` and `gemini-cli.svg` → `gemini.svg`. Fallback: author fresh SVGs if off-brand.

## 7. Unit Test — `ProviderDetailsView.spec.ts`

Vitest 4 + `@vue/test-utils`. `vi.mock` the `useProviders` and `useAvailableModels` composables; mount via `createMemoryHistory` + `router.push('/providers/<kind>')`.

| # | Scenario                                                      | Assertion                                                |
|---|---------------------------------------------------------------|----------------------------------------------------------|
| 1 | Mount at `/providers/openai`                                  | Header text is "OpenAI"                                  |
| 2 | Mount at `/providers/ollama-cloud` (regression for issue #1)  | Renders without redirecting                              |
| 3 | Mount at `/providers/unknown-kind`                            | `vi.spyOn(router, 'replace')` called with `'/providers'` |
| 4 | Title link attrs                                              | `target="_blank"`, `rel="noopener noreferrer"`           |
| 5 | Title link `aria-label`                                       | Matches `/opens in new tab$/`                            |
| 6 | Detail-header icon is `role="img"` with `aria-label` for kind | `wrapper.find('img[role="img"]')` exists                 |

## 8. E2E Cleanup

Delete the entire `test.describe("Provider Management", ...)` block (lines 3-116) — 6 stale tests asserting against the legacy flat-CRUD layout (`<h1>Providers</h1>`, global "Add Provider" button, "Max Concurrent Requests" label). **Keep** the `test.describe("Provider Catalog — Ollama Cloud card", ...)` block (lines 118-191) intact.

Add **3 tests** to the modern block: (1) `Ollama Cloud card navigates to /providers/ollama-cloud` (regression for issue #1); (2) detail header is an external link with `target="_blank"` + `aria-label` announcing new tab; (3) branded icon is visible on the detail page (eager, not broken).

## 9. Risks

| # | Risk                                                                                                       | Mitigation                                                                                                            |
|---|------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------|
| 1 | Visual quality of 4 authored SVGs may not match OmniRoute style                                            | sdd-apply browser-snaps the 6 cards; sdd-verify requires parity.                                                      |
| 2 | OmniRoute assets are brand marks                                                                           | LICENSE is MIT — redistribution OK. Trademarks: fair-use on a self-hosted dashboard. Fallback to author-mode SVG.     |
| 3 | Router-level `beforeEnter` not unit-tested                                                                 | View-level `watch` bounce test (case #3) catches the same drift in practice. Direct router test tracked as follow-up. |
| 4 | `vue-router ^5.1.0` typo in `package.json`; `provider_crud.enabled` + encryption env vars required for E2E | Out of scope; pre-existing E2E setup gates on env vars.                                                               |

## 10. Implementation Plan (preview for sdd-tasks)

8 tasks in execution order. `sdd-tasks` will convert to checkboxes.

1. **Assets** — author 4 SVGs + copy 2 from OmniRoute into `apps/rook/dashboard/public/providers/`. Browser-verify at 32×32 and 64×64.
2. **Catalog** — add `iconFile` (required) and `brandUrl?` (optional) to `CatalogEntry`; populate for all 6 kinds.
3. **`ProviderIcon.vue`** — implement per §2 with `Server` Lucide fallback + dev `console.warn`.
4. **`ProviderCatalogCard.vue`** — drop `ICONS` map (lines 35-42) + 6 Lucide imports; render `<ProviderIcon :kind="item.kind" loading="lazy" decorative />`.
5. **`ProviderDetailsView.vue`** — wrap `<h1>` in `<a>` per §4; add eager `<ProviderIcon>` to header.
6. **Router** — import `PROVIDER_KINDS`, derive `VALID_PROVIDER_KINDS`, drop the drift comment.
7. **Unit test** — `ProviderDetailsView.spec.ts` (6 cases per §7); confirm `pnpm exec vitest run` green.
8. **E2E + verify** — delete stale block, add 3 new tests per §8. Run `just ci-lint-only`, `pnpm exec vue-tsc --noEmit`, `pnpm exec vitest run`, `just test-e2e`.
